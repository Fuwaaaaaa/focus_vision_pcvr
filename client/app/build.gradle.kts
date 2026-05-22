plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}
android {
    namespace = "com.focusvision.pcvr"
    compileSdk = 34
    ndkVersion = "26.1.10909125"
    defaultConfig {
        applicationId = "com.focusvision.pcvr"
        minSdk = 29
        targetSdk = 34
        versionCode = 3
        versionName = "3.0.0-rc3"
        ndk { abiFilters += listOf("arm64-v8a") }
        externalNativeBuild {
            cmake { arguments += listOf("-DANDROID_STL=c++_shared", "-DANDROID_PLATFORM=android-29") }
        }
    }
    externalNativeBuild {
        cmake { path = file("src/main/cpp/CMakeLists.txt"); version = "3.22.1" }
    }

    // Release signing. Keystore is supplied via environment variables in
    // CI; locally a developer can either set the same env vars before
    // `gradle assembleRelease` or fall back to the debug keystore.
    //
    // ANDROID_KEYSTORE_PATH       absolute path to the .keystore file
    // ANDROID_KEYSTORE_PASSWORD   keystore password
    // ANDROID_KEY_ALIAS           key alias inside the keystore
    // ANDROID_KEY_PASSWORD        key password (often == keystore password)
    //
    // If ANDROID_KEYSTORE_PATH is unset OR the file does not exist, the
    // signingConfig is omitted and `assembleRelease` falls back to the
    // debug-signed APK (sideloadable, Play-Store-incompatible). CI's
    // android-build job creates an ephemeral keystore for this fallback
    // path so the workflow stays green even on PRs from forks.
    signingConfigs {
        create("release") {
            val keystorePath = System.getenv("ANDROID_KEYSTORE_PATH")
            if (keystorePath != null && file(keystorePath).exists()) {
                storeFile = file(keystorePath)
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD") ?: ""
                keyAlias = System.getenv("ANDROID_KEY_ALIAS") ?: "focus_vision_pcvr"
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD") ?: storePassword
            }
        }
    }

    buildTypes {
        getByName("release") {
            // Only use the release signing config if the keystore was
            // actually found — otherwise Gradle would fail with "null
            // storeFile". The fallback is the (default) debug signing,
            // which still produces a parseable APK suitable for sideload.
            val keystorePath = System.getenv("ANDROID_KEYSTORE_PATH")
            if (keystorePath != null && file(keystorePath).exists()) {
                signingConfig = signingConfigs.getByName("release")
            }
            // R8 code+resource shrinking. The Kotlin layer is essentially
            // just NativeActivity + companion loadLibrary; the bulk of the
            // app is C++ (libfocusvision_native.so), which R8 does not
            // touch. Rules in proguard-rules.pro keep JNI callbacks alive.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
}
