package dev.psytify.app.core

import android.content.Context

/**
 * Gives the Rust side the JavaVM and the application context (`ndk-context`), which the audio
 * output (cpal, AAudio) reads when it opens. A NativeActivity does this by itself; this app does it
 * once, before [dev.psytify.app.rust.PsytifyCore.start]. See `mobile/src/android.rs`.
 */
object NativeContext {
    init {
        // The same library the UniFFI bindings load through JNA: this load only registers the JNI call.
        System.loadLibrary("psytify_mobile")
    }

    external fun init(context: Context)
}
