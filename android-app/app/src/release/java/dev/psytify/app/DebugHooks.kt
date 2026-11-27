package dev.psytify.app

import android.content.Intent

/** A release build has no fixtures: nothing to do. */
object DebugHooks {
    @Suppress("UNUSED_PARAMETER")
    fun fixture(intent: Intent?) {}
}
