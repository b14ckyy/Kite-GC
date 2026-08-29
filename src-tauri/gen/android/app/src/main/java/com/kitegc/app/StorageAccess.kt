// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

package com.kitegc.app

import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import androidx.activity.result.ActivityResultLauncher
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * User-chosen storage folders (Settings → Flight Logbook) for the Android build.
 *
 * The Rust side (`android/storage.rs`) calls the static methods here over JNI — same contract as
 * [UsbSerial]: primitives and Strings only, no callbacks back into Rust, one error string to fetch
 * after a failure.
 *
 * The model is scoped storage, deliberately: the user grants **one folder** through the system tree
 * picker ([pickFolder]), Kite takes a persistable grant on exactly that folder, and no storage
 * permission appears in the manifest. Because a SAF grant provides `content://` documents rather
 * than POSIX paths, nothing *lives* in the folder — the SQLite database and the raw-log writers
 * need real paths and stay app-private — instead the session-end mirror ([syncDirToTree]) copies
 * the artefacts into it through the ContentResolver. That copy is what survives an uninstall,
 * which is the reason a user picks a folder at all.
 */
object StorageAccess {
    /** How long [pickFolder] waits for the user to finish with the system picker. Generous on
     *  purpose — browsing to a folder and creating a new one takes as long as it takes. */
    private const val PICK_TIMEOUT_MS = 180_000L

    private lateinit var activity: MainActivity
    private var launcher: ActivityResultLauncher<Uri?>? = null

    private var pending: CountDownLatch? = null
    @Volatile private var pickedUri: String? = null
    @Volatile private var lastError: String? = null

    /** Wire up from [MainActivity.onCreate]; [launcher] is the activity-result contract that must be
     *  registered as a field initializer (before the activity reaches STARTED). */
    fun init(activity: MainActivity, launcher: ActivityResultLauncher<Uri?>) {
        this.activity = activity
        this.launcher = launcher
    }

    /** The picker's result callback (runs on the main thread). Take the persistable grant — the
     *  whole point: it outlives this process, so the mirror still has access next session. */
    fun onFolderPicked(uri: Uri?) {
        if (uri == null) {
            pickedUri = null // cancelled — not an error
        } else {
            try {
                activity.contentResolver.takePersistableUriPermission(
                    uri,
                    Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                )
                pickedUri = uri.toString()
            } catch (e: SecurityException) {
                // A provider that refuses persistable grants is useless as a mirror target: the
                // grant would die with this process and every later sync would fail confusingly.
                lastError = "That location does not allow a lasting grant — pick a folder on the " +
                    "device storage or SD card (${e.message})"
            }
        }
        pending?.countDown()
    }

    /**
     * Open the system folder picker and wait for the answer. Returns the granted folder's **tree
     * URI** (the value the setting stores), or null when the user cancelled; a refused grant
     * reports through [getLastError]. Called from a Rust worker thread — the launch hops to the
     * main thread, the wait stays here.
     */
    @JvmStatic
    fun pickFolder(): String? {
        val l = launcher ?: run { lastError = "folder picker not initialised"; return null }
        lastError = null
        pickedUri = null
        val latch = CountDownLatch(1)
        pending = latch
        activity.runOnUiThread { l.launch(null) }
        try {
            if (!latch.await(PICK_TIMEOUT_MS, TimeUnit.MILLISECONDS)) {
                lastError = "folder picker timed out"
                return null
            }
        } catch (e: InterruptedException) {
            Thread.currentThread().interrupt()
            lastError = "interrupted while waiting for the folder picker"
            return null
        } finally {
            pending = null
        }
        return pickedUri
    }

    /** The error behind the last null result, if any — same fetch-after-failure contract as UsbSerial. */
    @JvmStatic
    fun getLastError(): String? = lastError

    /**
     * Mirror every regular file in [srcDir] into the granted tree at [treeUri]. A file already
     * present with the same size is skipped (raw logs never change after close; the DB snapshot is
     * freshly written when it differs), anything else is created or rewritten. Flat on purpose —
     * the two source dirs (raw-log staging, DB snapshot) hold no subdirectories.
     *
     * Returns null on success, the error message otherwise (the Rust side turns that into a
     * `Result::Err` and logs it — a failed mirror must never take the session teardown down).
     */
    @JvmStatic
    fun syncDirToTree(treeUri: String, srcDir: String): String? {
        return try {
            val tree = Uri.parse(treeUri)
            val resolver = activity.contentResolver
            val treeDocId = DocumentsContract.getTreeDocumentId(tree)
            val parentUri = DocumentsContract.buildDocumentUriUsingTree(tree, treeDocId)

            // One children query up front: name → (documentUri, size). Per-file existence probes
            // would be a resolver round-trip each; this is a single cursor.
            val children = HashMap<String, Pair<Uri, Long>>()
            val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(tree, treeDocId)
            resolver.query(
                childrenUri,
                arrayOf(
                    DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                    DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                    DocumentsContract.Document.COLUMN_SIZE
                ),
                null, null, null
            )?.use { c ->
                while (c.moveToNext()) {
                    val id = c.getString(0)
                    val name = c.getString(1) ?: continue
                    val size = if (c.isNull(2)) -1L else c.getLong(2)
                    children[name] = Pair(DocumentsContract.buildDocumentUriUsingTree(tree, id), size)
                }
            }

            var copied = 0
            val files = File(srcDir).listFiles() ?: return null // nothing staged yet — fine
            for (f in files) {
                if (!f.isFile) continue
                val existing = children[f.name]
                if (existing != null && existing.second == f.length()) continue
                val dest = existing?.first ?: DocumentsContract.createDocument(
                    resolver, parentUri, "application/octet-stream", f.name
                ) ?: return "could not create ${f.name} in the shared folder"
                // "wt" = write-truncate: a rewritten file must not keep a longer stale tail.
                resolver.openOutputStream(dest, "wt")?.use { out ->
                    f.inputStream().use { it.copyTo(out) }
                } ?: return "could not open ${f.name} in the shared folder for writing"
                copied++
            }
            android.util.Log.i("StorageAccess", "mirror: $copied file(s) from $srcDir into $treeDocId")
            null
        } catch (e: SecurityException) {
            "the folder grant is gone (revoked or the folder was deleted) — pick the folder again: ${e.message}"
        } catch (e: Exception) {
            "mirror failed: ${e.message}"
        }
    }
}
