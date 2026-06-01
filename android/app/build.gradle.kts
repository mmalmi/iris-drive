plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

import org.gradle.api.tasks.Exec

val repoRoot = layout.projectDirectory.dir("../..")
val rustOutputDir = layout.projectDirectory.dir("src/main/jniLibs")
val releaseKeystorePath = System.getenv("ANDROID_KEYSTORE_PATH")?.takeIf { it.isNotBlank() }
val releaseKeystorePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")?.takeIf { it.isNotBlank() }
val releaseKeyAlias = System.getenv("ANDROID_KEY_ALIAS")?.takeIf { it.isNotBlank() }
val releaseKeyPassword = System.getenv("ANDROID_KEY_PASSWORD")?.takeIf { it.isNotBlank() }
val irisDriveVersionName = providers.gradleProperty("irisDriveVersionName").orElse("0.1.0")
val irisDriveVersionCode = providers.gradleProperty("irisDriveVersionCode").orElse("1")
val hasReleaseSigning =
    releaseKeystorePath != null &&
        releaseKeystorePassword != null &&
        releaseKeyAlias != null &&
        releaseKeyPassword != null

android {
    namespace = "to.iris.drive.app"
    compileSdk = 36
    testBuildType = "uiTest"

    defaultConfig {
        applicationId = "to.iris.drive"
        minSdk = 26
        targetSdk = 36
        versionCode = irisDriveVersionCode.get().toInt()
        versionName = irisDriveVersionName.get()
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        manifestPlaceholders["documentsProviderAuthority"] = "to.iris.drive.documents"
        buildConfigField("String", "DOCUMENTS_PROVIDER_AUTHORITY", "\"to.iris.drive.documents\"")

        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    if (hasReleaseSigning) {
        signingConfigs {
            create("releaseEnv") {
                storeFile = file(releaseKeystorePath!!)
                storePassword = releaseKeystorePassword
                keyAlias = releaseKeyAlias
                keyPassword = releaseKeyPassword
            }
        }
    }

    buildTypes {
        debug {
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
        }
        create("uiTest") {
            initWith(getByName("debug"))
            applicationIdSuffix = ".uitest"
            versionNameSuffix = "-uitest"
            matchingFallbacks += listOf("debug")
            manifestPlaceholders["documentsProviderAuthority"] = "to.iris.drive.uitest.documents"
            buildConfigField(
                "String",
                "DOCUMENTS_PROVIDER_AUTHORITY",
                "\"to.iris.drive.uitest.documents\"",
            )
        }
        release {
            isMinifyEnabled = false
            if (hasReleaseSigning) {
                signingConfig = signingConfigs.getByName("releaseEnv")
            }
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

kotlin {
    jvmToolchain(17)
}

tasks.register<Exec>("buildRustArm64") {
    workingDir = repoRoot.asFile
    commandLine(
        "cargo",
        "ndk",
        "--target",
        "arm64-v8a",
        "--platform",
        "26",
        "--output-dir",
        rustOutputDir.asFile.absolutePath,
        "build",
        "--package",
        "iris-drive-app-core",
        "--release",
    )
}

tasks.matching { task ->
    task.name in listOf("mergeDebugNativeLibs", "mergeUiTestNativeLibs", "mergeReleaseNativeLibs")
}.configureEach {
    dependsOn("buildRustArm64")
}

dependencies {
    implementation("androidx.activity:activity-compose:1.11.0")
    implementation("androidx.camera:camera-camera2:1.4.2")
    implementation("androidx.camera:camera-lifecycle:1.4.2")
    implementation("androidx.camera:camera-view:1.4.2")
    implementation("androidx.core:core:1.17.0")
    implementation("androidx.compose.foundation:foundation:1.9.2")
    implementation("androidx.compose.material3:material3:1.4.0")
    implementation("androidx.compose.ui:ui:1.9.2")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.4")
    implementation("com.google.mlkit:barcode-scanning:17.3.0")

    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20260522")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4-android:1.9.2")
    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    androidTestImplementation("androidx.test:runner:1.7.0")
    debugImplementation("androidx.compose.ui:ui-test-manifest:1.9.2")
    add("uiTestImplementation", "androidx.compose.ui:ui-test-manifest:1.9.2")
}
