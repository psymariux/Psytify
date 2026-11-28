package dev.psytify.app.playback

import android.app.PendingIntent
import android.content.Intent
import androidx.annotation.OptIn
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.DefaultMediaNotificationProvider
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import dev.psytify.app.MainActivity
import dev.psytify.app.R
import dev.psytify.app.core.CoreHub
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/**
 * The foreground media service: notification, lock screen and headset buttons. Media3 promotes it to
 * the foreground while the session's player plays and lets it go when it pauses; the player is a
 * mirror of the core ([MirrorPlayer]), which keeps the audio.
 */
@OptIn(UnstableApi::class)
class PlaybackService : MediaSessionService() {
    private val scope = MainScope()
    private var session: MediaSession? = null

    override fun onCreate() {
        super.onCreate()
        CoreHub.start(applicationContext)
        val player = MirrorPlayer { command -> CoreHub.core?.media(command) }
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        session = MediaSession.Builder(this, player).setSessionActivity(open).build()
        setMediaNotificationProvider(
            DefaultMediaNotificationProvider.Builder(this).build().apply { setSmallIcon(R.drawable.ic_stat_psytify) },
        )
        scope.launch {
            val hasNext = CoreHub.playback.map { it?.snapshot?.hasNext ?: false }.distinctUntilChanged()
            CoreHub.nowPlaying
                .combine(hasNext) { now, next -> now to next }
                .collect { (now, next) -> player.show(now, next) }
        }
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    /**
     * Swiped away from recents: with nothing playing and nothing downloading there is no reason to
     * wait for the idle stop.
     */
    override fun onTaskRemoved(rootIntent: Intent?) {
        val player = session?.player
        if ((player == null || !player.playWhenReady) && !CoreHub.downloads.value.running) {
            CoreHub.stopNow()
            stopSelf()
        }
    }

    override fun onDestroy() {
        scope.cancel()
        session?.run {
            player.release()
            release()
        }
        session = null
        super.onDestroy()
    }
}
