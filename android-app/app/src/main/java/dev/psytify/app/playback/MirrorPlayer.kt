package dev.psytify.app.playback

import android.graphics.Bitmap
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import androidx.annotation.OptIn
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import androidx.media3.common.SimpleBasePlayer
import androidx.media3.common.util.UnstableApi
import com.google.common.util.concurrent.ListenableFuture
import com.google.common.util.concurrent.SettableFuture
import dev.psytify.app.rust.Cover
import dev.psytify.app.rust.MediaCommand
import dev.psytify.app.rust.NowPlaying
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer

/**
 * The player the media session sees. It plays nothing: the audio is the Rust core's (rodio on
 * AAudio). It shows what the core decided the system should show ([NowPlaying], sent only at real
 * transitions) and turns the session's buttons into [MediaCommand]s for the core.
 *
 * Media3 knows "next" and "previous" only as playlist items, so the current track sits in a playlist
 * of up to three: a placeholder before it (the core's Previous restarts or goes back) and one after
 * it while the queue has a next track.
 */
@OptIn(UnstableApi::class)
class MirrorPlayer(private val send: (MediaCommand) -> Unit) : SimpleBasePlayer(Looper.getMainLooper()) {
    private val main = Handler(Looper.getMainLooper())
    private var now: NowPlaying? = null
    private var hasNext = false
    private var shownAt = 0L
    private var artwork: ByteArray? = null
    private var artworkOf: Cover? = null
    /** Completes when the core shows the result of a button, so the session keeps its optimistic state until then. */
    private var waiting: SettableFuture<Unit>? = null

    /** The core's latest decision. Main thread only. */
    fun show(now: NowPlaying?, hasNext: Boolean) {
        this.now = now
        this.hasNext = hasNext
        shownAt = SystemClock.elapsedRealtime()
        val cover = now?.artwork
        if (cover != artworkOf) {
            artworkOf = cover
            artwork = cover?.let(::encode)
        }
        settle()
        invalidateState()
    }

    override fun getState(): State {
        val commands = Player.Commands.Builder()
            .addAll(
                COMMAND_PLAY_PAUSE,
                COMMAND_SEEK_TO_NEXT,
                COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
                COMMAND_SEEK_TO_PREVIOUS,
                COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM,
                COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM,
                COMMAND_GET_CURRENT_MEDIA_ITEM,
                COMMAND_GET_METADATA,
                COMMAND_GET_TIMELINE,
            )
            .build()
        val now = now ?: return State.Builder()
            .setAvailableCommands(commands)
            .setPlaybackState(STATE_IDLE)
            .build()
        val metadata = MediaMetadata.Builder()
            .setTitle(now.title)
            .setArtist(now.artist)
            .setAlbumTitle(now.album)
            .apply { artwork?.let { setArtworkData(it, MediaMetadata.PICTURE_TYPE_FRONT_COVER) } }
            .build()
        val current = MediaItemData.Builder(CURRENT)
            .setMediaItem(MediaItem.Builder().setMediaId(CURRENT).setMediaMetadata(metadata).build())
            .setMediaMetadata(metadata)
            .setDurationUs(now.durationMs.toLong() * 1000)
            .build()
        val playlist = buildList {
            add(MediaItemData.Builder(BEFORE).build())
            add(current)
            if (hasNext) add(MediaItemData.Builder(AFTER).build())
        }
        val elapsed = if (now.playing) SystemClock.elapsedRealtime() - shownAt else 0
        val position = (now.positionMs.toLong() + elapsed).coerceIn(0, now.durationMs.toLong())
        return State.Builder()
            .setAvailableCommands(commands)
            .setPlaylist(playlist)
            .setCurrentMediaItemIndex(1)
            .setPlaybackState(STATE_READY)
            .setPlayWhenReady(now.playing, PLAY_WHEN_READY_CHANGE_REASON_USER_REQUEST)
            .setContentPositionMs(PositionSupplier.getExtrapolating(position, if (now.playing) 1f else 0f))
            .build()
    }

    override fun handleSetPlayWhenReady(playWhenReady: Boolean): ListenableFuture<*> =
        ask(if (playWhenReady) MediaCommand.Play else MediaCommand.Pause)

    override fun handleSeek(mediaItemIndex: Int, positionMs: Long, seekCommand: Int): ListenableFuture<*> =
        ask(
            when {
                mediaItemIndex > 1 -> MediaCommand.Next
                mediaItemIndex < 1 || seekCommand == COMMAND_SEEK_TO_PREVIOUS -> MediaCommand.Previous
                else -> MediaCommand.SeekTo(positionMs.coerceAtLeast(0).toULong())
            },
        )

    override fun handleRelease(): ListenableFuture<*> {
        settle()
        return SettableFuture.create<Unit>().apply { set(Unit) }
    }

    private fun ask(command: MediaCommand): ListenableFuture<*> {
        settle()
        val future = SettableFuture.create<Unit>()
        waiting = future
        send(command)
        // The core may have nothing to show (a seek that does not move): do not wait forever.
        main.postDelayed({ if (waiting === future) settle() }, ANSWER_MS)
        return future
    }

    private fun settle() {
        waiting?.set(Unit)
        waiting = null
    }

    private companion object {
        const val CURRENT = "current"
        const val BEFORE = "before"
        const val AFTER = "after"
        const val ANSWER_MS = 1500L

        /** Media3 takes artwork as encoded bytes: the core's RGBA square, as a PNG, once per cover. */
        fun encode(cover: Cover): ByteArray? {
            val side = cover.side.toInt()
            if (side <= 0 || cover.rgba.size != side * side * 4) return null
            val bitmap = Bitmap.createBitmap(side, side, Bitmap.Config.ARGB_8888)
            bitmap.copyPixelsFromBuffer(ByteBuffer.wrap(cover.rgba))
            return ByteArrayOutputStream().use { out ->
                bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
                bitmap.recycle()
                out.toByteArray()
            }
        }
    }
}
