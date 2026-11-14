import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

// Optional real signing key. Drop a `key.properties` next to this file with storeFile,
// storePassword, keyAlias and keyPassword to sign releases for distribution; it is gitignored, so a
// private key never lands in the repository. Without it, release builds fall back to the debug
// signing config below — see the `release` build type.
val keystoreProperties = Properties().apply {
    val propFile = file("key.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

// versionCode for Google Play, derived from the version name. Play wants a higher code for every
// upload, and the one Tauri writes to tauri.properties ignores the pre-release part — every 2.0.0-*
// build would be 2000000.
//
//   major * 1_000_000 + minor * 10_000 + patch * 100 + stage
//   stage: -bN (or -betaN) = N (1..39) · -rcN = 40 + N (41..79) · final = 99 · -dev = 0
//   Any other suffix stops the build: it would share stage 0 with -dev and collide on Play.
//
//   2.0.0-b1 = 2000001 · 2.0.0-rc1 = 2000041 · 2.0.0 = 2000099 · 2.0.1 = 2000199 · 10.0.0 = 10000099
//
// This scheme is PERMANENT once a build is on Play: a code can never go down again. A build with
// stage 0 (-dev) installs, but is never meant for Play.
fun playVersionCode(name: String): Int {
    val m = Regex("""^(\d+)\.(\d+)\.(\d+)(?:-([A-Za-z]+)\.?(\d+)?)?$""").matchEntire(name)
        ?: throw GradleException("versionName '$name' is not MAJOR.MINOR.PATCH[-bN|-rcN|-dev]")
    val major = m.groupValues[1].toInt()
    val minor = m.groupValues[2].toInt()
    val patch = m.groupValues[3].toInt()
    val kind = m.groupValues[4].lowercase()
    val n = m.groupValues[5].toIntOrNull() ?: 0
    if (minor > 99 || patch > 99) {
        throw GradleException("versionName '$name': minor and patch must stay below 100 for the versionCode")
    }
    val stage = when (kind) {
        "" -> 99
        "beta", "b" -> if (n in 1..39) n else throw GradleException("versionName '$name': beta number must be 1..39")
        "rc" -> if (n in 1..39) 40 + n else throw GradleException("versionName '$name': rc number must be 1..39")
        "dev" -> 0
        else -> throw GradleException("versionName '$name': unknown pre-release '$kind' (use bN, rcN or dev)")
    }
    return major * 1_000_000 + minor * 10_000 + patch * 100 + stage
}

android {
    compileSdk = 36
    namespace = "com.kitegc.app"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.kitegc.app"
        minSdk = 24
        targetSdk = 36
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0.0")
        versionCode = playVersionCode(versionName!!)
    }
    signingConfigs {
        keystoreProperties.getProperty("storeFile")?.let { storePath ->
            create("release") {
                storeFile = file(storePath)
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            // An APK with no signature cannot be installed at all — Android rejects it outright as an
            // invalid package, which is what an unconfigured release build produces. Use the real key
            // when `key.properties` supplies one, otherwise fall back to the debug config so that a
            // plain `tauri android build --apk` still yields something installable.
            //
            // Debug-signed builds are for testing only: they cannot go on Play, and because the debug
            // keystore is generated per machine, a build from CI and a build from your laptop have
            // different signatures — Android then refuses to install one over the other until the old
            // copy is uninstalled.
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
            isMinifyEnabled = true
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    buildFeatures {
        buildConfig = true
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")