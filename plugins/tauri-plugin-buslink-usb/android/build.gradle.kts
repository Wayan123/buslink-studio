import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "id.co.arkamaya.buslink.usb"
    compileSdk = 36

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("com.fasterxml.jackson.core:jackson-databind:2.15.3")
    // MIT. Resolved from JitPack (the app's root build.gradle.kts scopes JitPack to com.github.mik3y).
    implementation("com.github.mik3y:usb-serial-for-android:3.12.0")
    testImplementation("junit:junit:4.13.2")
    implementation(project(":tauri-android"))
}
