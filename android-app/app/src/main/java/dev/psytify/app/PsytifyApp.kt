package dev.psytify.app

import android.app.Application
import dev.psytify.app.core.CoreHub

/** Starts the core with the process: the activity and the media service share it. */
class PsytifyApp : Application() {
    override fun onCreate() {
        super.onCreate()
        CoreHub.start(this)
    }
}
