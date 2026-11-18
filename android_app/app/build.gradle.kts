import java.util.Properties

plugins {
    id("com.android.application")
}

// ── version ─────────────────────────────────────────────────────────────────
// Kept in ../version.properties rather than inline, so `build_android.ps1 -Release`
// can bump the code without editing this file (and so a merge conflict on it is a
// one-line conflict). Missing file -> 1/"0.1.0", i.e. a fresh clone still builds.
val versionProps = Properties().apply {
    val f = rootProject.file("version.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}
val appVersionCode = (versionProps.getProperty("versionCode") ?: "1").trim().toInt()
val appVersionName = (versionProps.getProperty("versionName") ?: "0.1.0").trim()

// ── release signing ─────────────────────────────────────────────────────────
// An unsigned APK CANNOT BE INSTALLED, and `assembleRelease` produced exactly that
// until 2026-08-03 — which is why the only installable builds were debug ones,
// signed with the throwaway debug keystore Android generates per machine. A debug
// key is fine for your own phone and useless for handing the app to anyone: it is
// not stable across machines, and Play will not take it.
//
// The keystore and its passwords are SECRETS and are read from outside the repo, in
// this order (first complete set wins):
//
//   1. environment:  HERO3_KEYSTORE, HERO3_KEYSTORE_PASSWORD,
//                    HERO3_KEY_ALIAS, HERO3_KEY_PASSWORD
//   2. gradle properties (put them in %USERPROFILE%\.gradle\gradle.properties,
//      NOT in this project):  hero3.keystore, hero3.keystorePassword,
//                             hero3.keyAlias, hero3.keyPassword
//
// With neither, the release build still succeeds but is UNSIGNED and the build
// prints how to fix it. Nothing here ever writes a secret into the build output.
fun secret(env: String, prop: String): String? =
    (System.getenv(env) ?: providers.gradleProperty(prop).orNull)?.takeIf { it.isNotBlank() }

val ksPath = secret("HERO3_KEYSTORE", "hero3.keystore")
val ksPass = secret("HERO3_KEYSTORE_PASSWORD", "hero3.keystorePassword")
val ksAlias = secret("HERO3_KEY_ALIAS", "hero3.keyAlias")
val ksKeyPass = secret("HERO3_KEY_PASSWORD", "hero3.keyPassword")
val ksFile = ksPath?.let { file(it) }
val canSign = ksFile != null && ksFile.exists() && ksPass != null && ksAlias != null && ksKeyPass != null

android {
    // ★ `namespace` (the JAVA package) and `applicationId` (the INSTALL identity) are
    // deliberately different since 2026-08-03.
    //
    // The Java class is `net.wie.emulator.MainActivity` and STAYS there: the Rust side
    // looks it up BY NAME through JNI (`orientation.rs`, `server_prompt.rs`), so moving
    // the class would break the orientation toggle and the server/account dialogs at
    // runtime — silently, since a JNI class lookup fails at call time, not at build.
    // Only the installed identity is renamed, which is all Play cares about.
    namespace = "net.wie.emulator"
    compileSdk = 35
    ndkVersion = "25.1.8937393"

    defaultConfig {
        // ★ PERMANENT once uploaded to Play: it is the app's identity there and cannot
        // be changed afterwards. It is also the on-device data path
        // (/data/data/<applicationId>/), so changing it makes the app a DIFFERENT app —
        // a fresh install with no access to the previous one's saves.
        applicationId = "com.hero3remake.app"
        minSdk = 26        // AAudio (next-stage audio) needs API 26; S25 = API 36
        targetSdk = 35
        versionCode = appVersionCode
        versionName = appVersionName
        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    if (canSign) {
        signingConfigs {
            create("release") {
                storeFile = ksFile
                storePassword = ksPass
                keyAlias = ksAlias
                keyPassword = ksKeyPass
                // Both schemes: v1 is what API 26 verifies, v2 is what everything
                // since verifies and what Play expects.
                enableV1Signing = true
                enableV2Signing = true
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            if (canSign) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // The Rust cdylib is produced by cargo-ndk into src/main/jniLibs; it is
    // already stripped, so let AGP keep it as-is.
    packaging {
        jniLibs {
            keepDebugSymbols += "**/*.so"
        }
    }
}

// games-activity 4.4.0 drags in the pre-1.8 split kotlin-stdlib-jdk7/jdk8
// (1.6.21) artifacts, whose classes were folded into the main kotlin-stdlib in
// 1.8+. That collides with appcompat 1.7.0's kotlin-stdlib 1.8.22 (duplicate
// class). Forcing the split artifacts to 1.8.22 (now empty stubs) resolves it.
configurations.all {
    resolutionStrategy {
        force(
            "org.jetbrains.kotlin:kotlin-stdlib-jdk7:1.8.22",
            "org.jetbrains.kotlin:kotlin-stdlib-jdk8:1.8.22",
        )
    }
}

// Say it plainly at the top of a release build rather than letting someone discover
// it when the APK will not install on a tester's phone.
if (gradle.startParameter.taskNames.any { it.contains("Release", ignoreCase = true) }) {
    if (canSign) {
        logger.lifecycle("==> release signing: ${ksFile!!.absolutePath} (alias $ksAlias), versionCode $appVersionCode")
    } else {
        logger.warn(
            "==> WARNING: release build will be UNSIGNED and CANNOT BE INSTALLED.\n" +
                "    Set HERO3_KEYSTORE / HERO3_KEYSTORE_PASSWORD / HERO3_KEY_ALIAS / HERO3_KEY_PASSWORD,\n" +
                "    or hero3.keystore / hero3.keystorePassword / hero3.keyAlias / hero3.keyPassword in\n" +
                "    %USERPROFILE%\\.gradle\\gradle.properties. See build_android.ps1 -NewKeystore." +
                (if (ksPath != null && ksFile?.exists() != true) "\n    (configured keystore not found: $ksPath)" else "")
        )
    }
}

dependencies {
    // Provides the com.google.androidgamesdk.GameActivity Java class whose
    // native methods are implemented inside libmain.so (android-activity 0.6.1
    // bundles the matching GameActivity 4.4.0 C++).
    implementation("androidx.games:games-activity:4.4.0")
    // GameActivity 4.4.0 extends androidx.appcompat.app.AppCompatActivity, so
    // appcompat (which brings androidx.core) must be on the classpath and the
    // activity theme must be a Theme.AppCompat descendant.
    implementation("androidx.appcompat:appcompat:1.7.0")
}
