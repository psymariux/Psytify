// The Kotlin + Compose app of Psytify (docs/transformation/android-kotlin.md). The logic is the Rust
// core, built by scripts/android-kotlin.sh into build/rust (jniLibs and UniFFI bindings) before Gradle
// runs; Gradle only builds the Kotlin side and packages it.
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

plugins {
    // Provisions the JDK 21 toolchain if the machine has none (the system JDK may be newer than AGP supports).
    id("org.gradle.toolchains.foojay-resolver-convention") version "1.0.0"
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "psytify-android"
include(":app")
