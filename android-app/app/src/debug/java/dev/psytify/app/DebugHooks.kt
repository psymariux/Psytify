package dev.psytify.app

import android.content.Intent
import android.util.Log
import dev.psytify.app.core.CoreHub

/**
 * Debug builds only. `adb shell am start -n <app>/dev.psytify.app.MainActivity --es fixture NAME`
 * shows the fake social and jam views `NAME` (`psytify-mobile`'s `fixtures`, built with
 * `FIXTURES=1 scripts/android-kotlin.sh`); without that feature the call does nothing.
 */
object DebugHooks {
    fun fixture(intent: Intent?) {
        val name = intent?.getStringExtra("fixture") ?: return
        val shown = CoreHub.core?.useFixture(name) ?: false
        Log.i("Psytify", if (shown) "Fixture $name" else "No fixture $name in this build")
    }
}
