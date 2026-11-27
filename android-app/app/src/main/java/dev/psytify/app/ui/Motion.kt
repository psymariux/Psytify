package dev.psytify.app.ui

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.database.ContentObserver
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.provider.Settings
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.LifecycleStartEffect

/** What Android asks of the motion: animations removed (the animator scale at 0), battery saver. */
data class SystemMotion(val animationsOff: Boolean, val powerSaver: Boolean)

private fun read(context: Context): SystemMotion {
    val scale = Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f)
    val power = context.getSystemService(PowerManager::class.java)?.isPowerSaveMode ?: false
    return SystemMotion(animationsOff = scale == 0f, powerSaver = power)
}

/**
 * The system's motion, followed live while the app is in front: the battery saver's broadcast and
 * an observer on the animator scale (the quick settings tile changes them with the app open). Read
 * again at each start, since nothing listens while the app is hidden.
 */
@Composable
fun rememberSystemMotion(): SystemMotion {
    val context = LocalContext.current
    var motion by remember { mutableStateOf(read(context)) }
    LifecycleStartEffect(context) {
        motion = read(context)
        val refresh = { motion = read(context) }
        val power = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) = refresh()
        }
        val filter = IntentFilter(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED)
        // The system's broadcast reaches a receiver that is not exported.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            context.registerReceiver(power, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            context.registerReceiver(power, filter)
        }
        val scale = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) = refresh()
        }
        val uri = Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE)
        context.contentResolver.registerContentObserver(uri, false, scale)
        onStopOrDispose {
            context.unregisterReceiver(power)
            context.contentResolver.unregisterContentObserver(scale)
        }
    }
    return motion
}
