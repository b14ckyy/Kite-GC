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

// Optional release key: a gitignored `key.properties` next to this file (storeFile, storePassword,
// keyAlias, keyPassword). Without it release builds fall back to the debug signing config.
val keystoreProperties = Properties().apply {
    val propFile = file("key.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

// versionCode for Google Play from the version name (Tauri's ignores the pre-release part — every 2.0.0-*
// would be 2000000):  major * 10_000_000 + minor * 100_000 + patch * 1_000 + stage * 10 + build
//   stage: -bN / -betaN = N (1..39) · -rcN = 40 + N · final = 99 · -dev = 0 (installs, never for Play);
//          any other suffix stops the build (it would collide with -dev on Play).
//   build: KITE_ANDROID_BUILD 0..9 (default 0) — a re-upload of the SAME version; Play never takes a code twice.
//   2.0.0-b1 = 20000010 · 2.0.0-rc1 = 20000410 · 2.0.0 = 20000990 · 2.0.1 = 20001990
// PERMANENT: a code can never go down again; 2000001 (the first scheme's 2.0.0-b1) is spent on Play.
fun playVersionCode(name: String, build: Int): Int {
    val m = Regex("""^(\d+)\.(\d+)\.(\d+)(?:-([A-Za-z]+)\.?(\d+)?)?$""").matchEntire(name)
        ?: throw GradleException("versionName '$name' is not MAJOR.MINOR.PATCH[-bN|-rcN|-dev]")
    val major = m.groupValues[1].toInt()
    val minor = m.groupValues[2].toInt()
    val patch = m.groupValues[3].toInt()
    val kind = m.groupValues[4].lowercase()
    val n = m.groupValues[5].toIntOrNull() ?: 0
    if (major > 209 || minor > 99 || patch > 99) {
        throw GradleException("versionName '$name': major must stay below 210, minor and patch below 100, for the versionCode")
    }
    if (build !in 0..9) {
        throw GradleException("KITE_ANDROID_BUILD must be 0..9, got $build")
    }
    val stage = when (kind) {
        "" -> 99
        "beta", "b" -> if (n in 1..39) n else throw GradleException("versionName '$name': beta number must be 1..39")
        "rc" -> if (n in 1..39) 40 + n else throw GradleException("versionName '$name': rc number must be 1..39")
        "dev" -> 0
        else -> throw GradleException("versionName '$name': unknown pre-release '$kind' (use bN, rcN or dev)")
    }
    return major * 10_000_000 + minor * 100_000 + patch * 1_000 + stage * 10 + build
}

android {
    compileSdk = 36
    // AGP strips the Rust library and extracts its symbols only with an NDK of exactly `ndkVersion`; its
    // default is rarely the installed one, and then the library ships unstripped, with no symbols file.
    System.getenv("NDK_HOME")?.takeIf { it.isNotBlank() }?.let { home ->
        val props = file("$home/source.properties")
        if (props.exists()) {
            ndkPath = home
            ndkVersion = Properties().apply { props.inputStream().use { load(it) } }.getProperty("Pkg.Revision")
        }
    }
    namespace = "com.kitegc.app"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.kitegc.app"
        minSdk = 24
        targetSdk = 36
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0.0")
        versionCode = playVersionCode(
            versionName!!,
            providers.environmentVariable("KITE_ANDROID_BUILD").orNull?.trim()?.ifEmpty { null }.let { raw ->
                (raw ?: "0").toIntOrNull()
                    ?: throw GradleException("KITE_ANDROID_BUILD must be a number 0..9, got '$raw'")
            },
        )
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
            // KITE_ANDROID_DEV_ID=1: own application id `com.kitegc.app.dev`, so the debug build installs BESIDE
            // the Play copy with its own data. Opt-in: `tauri android dev` still starts the OTHER copy, so start
            // the .dev one yourself (phone-emu.ps1 -Device -Beside does). Every debug build is named "Kite Dev".
            if (providers.environmentVariable("KITE_ANDROID_DEV_ID").orNull?.trim() == "1") {
                applicationIdSuffix = ".dev"
            }
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
        }
        getByName("release") {
            // An unsigned APK cannot be installed at all, so without `key.properties` fall back to the debug
            // config. Debug-signed builds are for testing only: not for Play, and the keystore differs per
            // machine, so a CI build and a local one cannot be installed over each other.
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
            // Function names for Play's crash and ANR reports; the cargo release profile has no line tables.
            ndk { debugSymbolLevel = "SYMBOL_TABLE" }
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

// Per variant on purpose: a `packaging {}` inside a build type is the module-wide one, and release then
// ships the library unstripped too.
androidComponents {
    onVariants(selector().withBuildType("debug")) { variant ->
        variant.packaging.jniLibs.keepDebugSymbols.add("**/*.so")
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