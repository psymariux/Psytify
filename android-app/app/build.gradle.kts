import com.android.build.api.variant.ApplicationAndroidComponentsExtension

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// Built by scripts/android-kotlin.sh before Gradle runs: jniLibs/<abi>/libpsytify_mobile.so (cargo-ndk)
// and kotlin/ (the UniFFI bindings read from that library).
val rust = layout.buildDirectory.dir("generated/rust")

// The release tag (`vX.Y.Z`, PSYTIFY_BUILD_TAG, the one the core compares with the latest release),
// or the version below for builds without one. The code is 2·2²⁴ + X·2¹⁶ + Y·2⁸ + Z: above the Iced
// APK's (2²⁴ + the same, from cargo-apk), so the Kotlin app installs over it and keeps its data.
val release = providers.environmentVariable("PSYTIFY_BUILD_TAG").orNull
    ?.removePrefix("v")
    ?.takeIf { Regex("""\d{1,3}\.\d{1,3}\.\d{1,3}""").matches(it) }
    ?: "0.2.0"
val releaseCode = release.split('.').map(String::toInt).let { (major, minor, patch) ->
    (2 shl 24) + (major shl 16) + (minor.coerceAtMost(255) shl 8) + patch.coerceAtMost(255)
}

android {
    namespace = "dev.psytify.app"
    compileSdk = 37
    // The NDK of scripts/android-env.sh: AGP uses its llvm-strip on the Rust library.
    ndkVersion = providers.environmentVariable("NDK_VERSION").getOrElse("27.2.12479018")

    defaultConfig {
        applicationId = "dev.psytify.app"
        minSdk = 26
        targetSdk = 35
        versionCode = releaseCode
        versionName = release
        // The ABIs the Rust core is built for: JNA and AndroidX bring libraries for more (armeabi,
        // mips, x86), which would install on a phone where libpsytify_mobile.so is missing.
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    buildTypes {
        debug {
            // Installs next to the Iced app (`dev.psytify.app`, signed with the release key) instead of
            // clashing with it.
            applicationIdSuffix = ".kotlin"
        }
        release {
            // Unsigned here: scripts/android-kotlin.sh aligns it and signs it with the release key.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    buildFeatures {
        compose = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        jniLibs {
            // Android 15 phones with 16 KB pages need the libraries stored uncompressed and aligned.
            useLegacyPackaging = false
        }
    }
}

kotlin {
    jvmToolchain(21)
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

/** Copies the pixel sprites (design/assets/pixel/export/1x) and the fonts into generated resources. */
abstract class DesignResources : DefaultTask() {
    @get:InputFiles
    abstract val sprites: ConfigurableFileCollection

    @get:InputFiles
    abstract val fonts: ConfigurableFileCollection

    @get:OutputDirectory
    abstract val output: DirectoryProperty

    @get:Inject
    abstract val files: FileSystemOperations

    @TaskAction
    fun copy() {
        files.sync {
            // Not scaled by density: sprites are drawn at whole multiples of the art pixel, unfiltered.
            from(sprites) { into("drawable-nodpi") }
            // Resource names are lower case: `Inter-SemiBold.ttf` becomes `inter_semibold.ttf`.
            from(fonts) {
                into("font")
                rename { it.lowercase().replace('-', '_') }
            }
            into(output)
        }
    }
}

val repo = rootDir.parentFile
val designResources = tasks.register<DesignResources>("designResources") {
    sprites.from(fileTree(repo.resolve("design/assets/pixel/export/1x")) { include("*.png") })
    fonts.from(
        repo.resolve("design/assets/fonts/Inter-Regular.ttf"),
        repo.resolve("design/assets/fonts/Inter-Medium.ttf"),
        repo.resolve("design/assets/fonts/Inter-SemiBold.ttf"),
        repo.resolve("design/assets/fonts/DepartureMono-Regular.otf"),
    )
    output.set(layout.buildDirectory.dir("generated/design-res"))
}

val checkRust = tasks.register("checkRust") {
    val lib = rust.map { it.file("kotlin/dev/psytify/app/rust/psytify_mobile.kt").asFile }
    doLast {
        check(lib.get().exists()) { "The Rust library and its bindings are missing: run scripts/android-kotlin.sh" }
    }
}
tasks.named("preBuild") { dependsOn(checkRust) }

extensions.getByType(ApplicationAndroidComponentsExtension::class.java).onVariants { variant ->
    variant.sources.res?.addGeneratedSourceDirectory(designResources, DesignResources::output)
    variant.sources.jniLibs?.addStaticSourceDirectory(rust.get().dir("jniLibs").asFile.path)
    variant.sources.kotlin?.addStaticSourceDirectory(rust.get().dir("kotlin").asFile.path)
}

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.foundation)
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    debugImplementation(libs.compose.ui.tooling)
    implementation(libs.activity.compose)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.media3.session)
    implementation(libs.media3.common)
    // ExoPlayer plays the current track's clip, muted and with its audio track disabled, in the
    // full player's media slot (stage 7); the audio stays in the Rust core. Same Media3 release as
    // the session, so one copy of media3-common; GStreamer has no maintained Android build.
    implementation(libs.media3.exoplayer)
    implementation(libs.coroutines.android)
    implementation("${libs.jna.get()}@aar")
    testImplementation(libs.junit)
}
