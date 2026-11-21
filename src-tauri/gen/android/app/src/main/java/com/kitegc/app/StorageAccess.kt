// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

package com.kitegc.app

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.graphics.drawable.Icon
import android.os.Build
import android.service.chooser.ChooserAction
import android.net.Uri
import android.provider.DocumentsContract
import androidx.activity.result.ActivityResultLauncher
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * The user-chosen raw-log folder (Settings → Data) for the Android build, called from
 * `android/storage.rs` over JNI — Strings only, no callbacks. A SAF grant gives `content://`
 * documents, not paths: the raw writers stay app-private and closed files are moved in
 * ([moveFilesToTree]).
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

    /** The picker's result callback (main thread). The persistable grant keeps the folder writable
     *  in later sessions. */
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
                // A provider without persistable grants loses access with this process; every later
                // move would fail.
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

    private const val ACTION_SAVE_TO_DEVICE = "com.kitegc.app.EXPORT_SAVE_TO_DEVICE"

    /** The file of the share sheet that is open, and the one whose "save to device" action was tapped. */
    @Volatile private var sharedPath: String? = null
    @Volatile private var saveRequest: String? = null
    private var saveReceiver: BroadcastReceiver? = null

    /** The path the user asked to save to the device from the share sheet, once; null when none. Polled
     *  by the Rust side — no callbacks into Rust (see the class note). */
    @JvmStatic
    fun takeSaveRequest(): String? {
        val r = saveRequest
        saveRequest = null
        return r
    }

    /** Hand [path] to the share sheet as a FileProvider URI (a raw file:// URI throws since API 24). A
     *  non-empty [saveLabel] adds a "save to device" action (API 34+), fetched via [takeSaveRequest].
     *  Returns null on success, the error message otherwise. */
    @JvmStatic
    fun shareFile(path: String, mime: String, saveLabel: String): String? {
        return try {
            val f = File(path)
            if (!f.isFile) return "file not found: $path"
            val uri = FileProvider.getUriForFile(activity, activity.packageName + ".fileprovider", f)
            val send = Intent(Intent.ACTION_SEND).apply {
                type = mime
                putExtra(Intent.EXTRA_STREAM, uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            val chooser = Intent.createChooser(send, f.name)
            if (saveLabel.isNotEmpty() && Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                addSaveAction(chooser, saveLabel)
            }
            saveRequest = null
            sharedPath = path
            activity.runOnUiThread {
                activity.startActivity(chooser)
            }
            null
        } catch (e: Exception) {
            "share failed: ${e.message}"
        }
    }

    private fun addSaveAction(chooser: Intent, label: String) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE) return
        if (saveReceiver == null) {
            val receiver = object : BroadcastReceiver() {
                override fun onReceive(context: Context?, intent: Intent?) {
                    saveRequest = sharedPath
                }
            }
            // On the application context: a receiver of the activity dies with it when Android
            // recreates the activity, and this one is registered once per process.
            ContextCompat.registerReceiver(
                activity.applicationContext, receiver, IntentFilter(ACTION_SAVE_TO_DEVICE),
                ContextCompat.RECEIVER_NOT_EXPORTED
            )
            saveReceiver = receiver
        }
        val tap = PendingIntent.getBroadcast(
            activity, 0,
            Intent(ACTION_SAVE_TO_DEVICE).setPackage(activity.packageName),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val action = ChooserAction.Builder(
            Icon.createWithResource(activity, android.R.drawable.ic_menu_save), label, tap
        ).build()
        chooser.putExtra(Intent.EXTRA_CHOOSER_CUSTOM_ACTIONS, arrayOf(action))
    }

    /**
     * Move the files in [paths] (one per line) flat into the tree [treeUri]: write, read the size
     * back, then delete the local file; one already there with the same size counts as written.
     * Returns null on success, else the last error; a file that failed stays in the app.
     */
    @JvmStatic
    fun moveFilesToTree(treeUri: String, paths: String): String? {
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

            var moved = 0
            var failed: String? = null
            for (path in paths.split('\n')) {
                val f = File(path)
                if (!f.isFile) continue
                val existing = children[f.name]
                if (existing == null || existing.second != f.length()) {
                    val dest = existing?.first ?: DocumentsContract.createDocument(
                        resolver, parentUri, "application/octet-stream", f.name
                    )
                    if (dest == null) {
                        failed = "could not create ${f.name} in the shared folder"
                        continue
                    }
                    // "wt" = write-truncate: a rewritten file must not keep a longer stale tail.
                    val opened = resolver.openOutputStream(dest, "wt")?.use { out ->
                        f.inputStream().use { it.copyTo(out) }
                    }
                    if (opened == null) {
                        failed = "could not open ${f.name} in the shared folder for writing"
                        continue
                    }
                    val written = resolver.query(
                        dest, arrayOf(DocumentsContract.Document.COLUMN_SIZE), null, null, null
                    )?.use { c -> if (c.moveToFirst() && !c.isNull(0)) c.getLong(0) else -1L } ?: -1L
                    // A provider that reports no size cannot confirm the copy: the file stays here.
                    if (written != f.length()) {
                        failed = "${f.name} arrived with $written of ${f.length()} bytes — kept in the app"
                        continue
                    }
                }
                if (f.delete()) moved++
            }
            android.util.Log.i("StorageAccess", "moved $moved file(s) into $treeDocId")
            failed
        } catch (e: SecurityException) {
            "the folder grant is gone (revoked or the folder was deleted) — pick the folder again: ${e.message}"
        } catch (e: Exception) {
            "move failed: ${e.message}"
        }
    }
}
