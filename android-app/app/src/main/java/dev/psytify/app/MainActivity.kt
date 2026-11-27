package dev.psytify.app

import android.Manifest
import android.content.ComponentName
import android.content.pm.PackageManager
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import com.google.common.util.concurrent.ListenableFuture
import dev.psytify.app.core.CoreHub
import dev.psytify.app.design.PsytifyTheme
import dev.psytify.app.playback.PlaybackService
import dev.psytify.app.rust.PlayerState
import dev.psytify.app.ui.PsytifyApp
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    /** Binds the media service while the activity is visible, so it exists before playback starts. */
    private var controller: ListenableFuture<MediaController>? = null
    private var notificationsAsked = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // The canvas is always dark: light system bar icons, whatever the system theme.
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
        )
        // Debug builds only (src/debug): `--es fixture NAME` shows fake social and jam views.
        DebugHooks.fixture(intent)
        setContent { PsytifyTheme { PsytifyApp() } }
        askNotificationsAtFirstPlay()
    }

    override fun onStart() {
        super.onStart()
        CoreHub.core?.setVisible(true)
        val token = SessionToken(this, ComponentName(this, PlaybackService::class.java))
        controller = MediaController.Builder(this, token).buildAsync()
    }

    override fun onStop() {
        controller?.let(MediaController::releaseFuture)
        controller = null
        // Hidden: the core saves now and stops if nothing plays for a while (psytify-mobile).
        CoreHub.core?.setVisible(false)
        super.onStop()
    }

    /**
     * Android 13+: the media and download notifications need the permission; ask when something
     * first plays or downloads, not at launch.
     */
    private fun askNotificationsAtFirstPlay() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) return
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.RESUMED) {
                if (notificationsAsked) return@repeatOnLifecycle
                val playing = CoreHub.playback.map { it?.snapshot?.state == PlayerState.Playing }
                val downloading = CoreHub.downloads.map { it.running }
                playing.combine(downloading) { play, download -> play || download }.first { it }
                notificationsAsked = true
                requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
            }
        }
    }
}
