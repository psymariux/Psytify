package dev.psytify.app.ui

import android.content.Context
import android.content.pm.ApplicationInfo
import android.net.Uri
import android.util.Log
import android.view.TextureView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LifecycleStartEffect
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.VideoSize
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.ResolvingDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import dev.psytify.app.core.Playback
import dev.psytify.app.design.AnimatedSprite
import dev.psytify.app.design.DISSOLVE_STEPS
import dev.psytify.app.design.LocalArtPx
import dev.psytify.app.design.Pill
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SecondaryButton
import dev.psytify.app.design.ditherOut
import dev.psytify.app.design.rememberDissolve
import dev.psytify.app.rust.ClipStatus
import dev.psytify.app.rust.PsytifyCore
import dev.psytify.app.rust.TrackItem
import dev.psytify.app.rust.VideoMeasure
import dev.psytify.app.rust.VideoSnapshot
import dev.psytify.app.rust.VideoSync
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap
import kotlin.math.abs
import kotlinx.coroutines.delay

private const val TAG = "PsytifyVideo"
/** The drift check while the clip plays: the core's beat. */
private const val BEAT_MS = 250L
/** A debug log line of the drift every this many beats (2 s), and at every correction. */
private const val LOG_EVERY = 8
/** The scheme of the clip addresses ExoPlayer sees: the signed one is resolved only when loading. */
private const val SCHEME = "psytify-clip"

/**
 * The full player's media slot, a square: the cover or, when the track has a clip and the person
 * chose «Video», the clip (16:9, centred) playing muted in its place. The two swap with the 4-frame
 * ordered-dither dissolve (a cut under reduced motion). The core asks for the clip only while this
 * shows it and the app is in front; ExoPlayer exists only while the clip is on screen (and for the
 * dissolve back to the cover), so nothing decodes at rest.
 */
@Composable
fun MediaSlot(
    track: TrackItem,
    video: VideoSnapshot?,
    clipOn: Boolean,
    onClipOn: (Boolean) -> Unit,
    playback: Playback,
    core: PsytifyCore,
    side: Dp,
) {
    val hasClip = video?.hasClip == true
    val wanted = clipOn && hasClip
    // Only while the app is in front: a hidden slot asks the core for nothing.
    LifecycleStartEffect(wanted) {
        if (wanted) core.showVideo(true)
        onStopOrDispose { if (wanted) core.showVideo(false) }
    }
    val status = video?.status
    var heldSeq by remember { mutableStateOf<ULong?>(null) }
    if (wanted && status is ClipStatus.Ready) heldSeq = status.seq
    var framed by remember { mutableStateOf(false) }
    val step = rememberDissolve(gone = wanted && framed)
    val seq = heldSeq
    // The player stays for the dissolve back to the cover, then goes.
    val playerUp = seq != null && ((wanted && status is ClipStatus.Ready) || step > 0)
    if (!playerUp) {
        framed = false
        if (!wanted) heldSeq = null
    }
    Box(Modifier.size(side), contentAlignment = Alignment.Center) {
        if (playerUp) {
            ClipPlayer(core, seq, playback, onFrame = { framed = true }, onStopped = { framed = false }, Modifier)
        }
        if (step < DISSOLVE_STEPS) {
            Cover(track.coverUrl, side, Modifier.ditherOut(step, LocalArtPx.current))
        }
        if (wanted) ClipNote(status, framed, core, Modifier.align(Alignment.TopStart))
        if (hasClip) {
            Row(
                Modifier.align(Alignment.BottomStart).padding(PsytifySpace.SPACE_3),
                horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_2),
            ) {
                Pill("Copertina", on = !clipOn, onClick = { onClipOn(false) })
                Pill("Video", on = clipOn, onClick = { onClipOn(true) })
            }
        }
    }
}

/** While the clip is on its way, or after it failed: a line on the slot. */
@Composable
private fun ClipNote(status: ClipStatus?, framed: Boolean, core: PsytifyCore, modifier: Modifier) {
    val failed = status as? ClipStatus.Failed
    if (failed == null && framed) return
    if (failed == null && status !is ClipStatus.Loading && status !is ClipStatus.Ready) return
    Column(
        modifier.padding(PsytifySpace.SPACE_3).background(PsytifyColors.SURFACE_CANVAS).padding(PsytifySpace.SPACE_3),
    ) {
        if (failed != null) {
            PsytifyText("Il videoclip non si è aperto.", PsytifyType.TYPE_BODY, PsytifyColors.TEXT_PRIMARY, maxLines = 3)
            Box(Modifier.padding(top = PsytifySpace.SPACE_3)) { SecondaryButton("Riprova", { core.retryVideo() }) }
        } else {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_3)) {
                AnimatedSprite(PsytifySprites.LOADER, running = true, tint = PsytifyColors.TEXT_SECONDARY)
                PsytifyText("Preparo il videoclip…", PsytifyType.TYPE_META, PsytifyColors.TEXT_SECONDARY)
            }
        }
    }
}

/**
 * ExoPlayer on a TextureView for clip `seq`, aligned to the core's audio at its beat. The player goes
 * as soon as the app leaves the screen (a stopped activity does not recompose, so this does not wait
 * for the core's answer to `showVideo(false)`); back in front, a new one opens the clip the core
 * publishes again.
 */
@Composable
private fun ClipPlayer(
    core: PsytifyCore,
    seq: ULong,
    playback: Playback,
    onFrame: () -> Unit,
    onStopped: () -> Unit,
    modifier: Modifier,
) {
    val context = LocalContext.current
    val owner = LocalLifecycleOwner.current
    var aspect by remember { mutableFloatStateOf(16f / 9f) }
    var life by remember { mutableIntStateOf(0) }
    val clip = remember(life) { Clip(context, core, onFrame) { aspect = it } }
    DisposableEffect(clip, owner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP && !clip.released) {
                clip.release()
                onStopped()
            } else if (event == Lifecycle.Event.ON_START && clip.released) {
                life++
            }
        }
        owner.lifecycle.addObserver(observer)
        onDispose {
            owner.lifecycle.removeObserver(observer)
            clip.release()
        }
    }
    LaunchedEffect(clip, seq) { clip.open(seq, playback.positionMs(), playback.snapshot.running) }
    // A playback change (pause, resume, seek, a new track) is followed at once…
    LaunchedEffect(clip, playback) { clip.beat(force = false) }
    // …and while it plays the drift is checked at the beat. Paused or in the background, nothing runs.
    LaunchedEffect(clip, playback.snapshot.running) {
        owner.lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (playback.snapshot.running) {
                delay(BEAT_MS)
                clip.beat(force = false)
            }
        }
    }
    key(clip) {
        AndroidView(
            factory = { TextureView(it).also(clip::attach) },
            modifier = modifier.aspectRatio(aspect),
        )
    }
}

/**
 * One ExoPlayer and its alignment. Video only: the audio track is disabled (never decoded, no audio
 * output opened) and the volume is 0. The signed address is kept here, keyed by `seq`, and handed to
 * the HTTP stack when a load starts; ExoPlayer's media item only carries `psytify-clip:<seq>`.
 */
private enum class Stage { OPENING, SETTLING, ALIGNED }

private class Clip(
    context: Context,
    private val core: PsytifyCore,
    private val onFrame: () -> Unit,
    private val onAspect: (Float) -> Unit,
) : Player.Listener {
    private val debug = context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
    private val addresses = ConcurrentHashMap<Long, String>()
    private val sync: VideoSync = core.videoSync()
    private var seq: ULong? = null
    /** Opening, then gone to the audio's position and settling, then aligned at each beat. */
    private var stage = Stage.OPENING
    /** Whether the audio plays, as last opened: the player starts only once it settled. */
    private var playing = false
    private var beats = 0
    var released = false
        private set

    val player: ExoPlayer = run {
        val upstream = DefaultDataSource.Factory(context)
        val resolving = ResolvingDataSource.Factory(upstream) { spec ->
            if (spec.uri.scheme != SCHEME) return@Factory spec
            val key = spec.uri.schemeSpecificPart.toLongOrNull()
            val address = key?.let(addresses::get) ?: throw IOException("This clip is no longer published")
            spec.withUri(Uri.parse(address))
        }
        ExoPlayer.Builder(context)
            .setMediaSourceFactory(DefaultMediaSourceFactory(resolving))
            .setHandleAudioBecomingNoisy(false)
            .setWakeMode(C.WAKE_MODE_NONE)
            .build()
            .also { player ->
                player.trackSelectionParameters = player.trackSelectionParameters
                    .buildUpon()
                    .setTrackTypeDisabled(C.TRACK_TYPE_AUDIO, true)
                    .build()
                player.volume = 0f
                player.repeatMode = Player.REPEAT_MODE_OFF
                player.addListener(this)
            }
    }

    fun attach(view: TextureView) {
        player.setVideoTextureView(view)
    }

    /** Opens clip `seq` at the audio's position; an upgrade replaces the item in the same player. */
    fun open(seq: ULong, startMs: Long, playing: Boolean) {
        if (released) return
        val address = core.clipUrl(seq) ?: return
        addresses.clear()
        addresses[seq.toLong()] = address
        this.seq = seq
        this.playing = playing
        stage = Stage.OPENING
        player.setMediaItem(MediaItem.fromUri("$SCHEME:$seq"), startMs)
        player.playWhenReady = false
        player.prepare()
        if (debug) Log.d(TAG, "open clip $seq")
    }

    /** Measures the player and applies what the Rust side decides. */
    fun beat(force: Boolean) {
        if (released) return
        // A settle seek that never left READY: settled all the same.
        if (stage == Stage.SETTLING && !force && player.playbackState == Player.STATE_READY) stage = Stage.ALIGNED
        if (stage != Stage.ALIGNED && !force) return
        val state = player.playbackState
        val duration = player.duration.takeIf { it != C.TIME_UNSET && it > 0 } ?: 0L
        val measure = VideoMeasure(
            positionMs = player.currentPosition.coerceAtLeast(0).toULong(),
            durationMs = duration.toULong(),
            paused = !player.playWhenReady,
            ended = state == Player.STATE_ENDED,
            buffering = state != Player.STATE_READY,
        )
        val adjust = sync.align(measure, force)
        adjust.pause?.let { player.playWhenReady = !it }
        adjust.seekMs?.let { player.seekTo(it.toLong()) }
        if (abs(player.playbackParameters.speed - adjust.speed) > 0.001f) player.setPlaybackSpeed(adjust.speed)
        val moved = adjust.seekMs != null || adjust.pause != null
        if (debug && (moved || force || beats % LOG_EVERY == 0)) {
            Log.d(
                TAG,
                "drift=${adjust.driftMs} ms lead=${adjust.leadMs} ms speed=${"%.3f".format(adjust.speed)}" +
                    " seek=${adjust.seekMs ?: "-"} pause=${adjust.pause ?: "-"} force=$force",
            )
        }
        beats++
    }

    override fun onPlaybackStateChanged(playbackState: Int) {
        if (playbackState != Player.STATE_READY) return
        when (stage) {
            // Ready where it opened: go to where the audio is now, then settle.
            Stage.OPENING -> {
                stage = Stage.SETTLING
                player.seekTo(sync.audioMs().toLong())
                player.playWhenReady = playing
            }
            // Settled: the core's rules from here, first forced (pause, position, lead).
            Stage.SETTLING -> {
                stage = Stage.ALIGNED
                beat(force = true)
            }
            Stage.ALIGNED -> {}
        }
    }

    override fun onRenderedFirstFrame() {
        onFrame()
    }

    override fun onVideoSizeChanged(videoSize: VideoSize) {
        if (videoSize.width > 0 && videoSize.height > 0) {
            onAspect(videoSize.width * videoSize.pixelWidthHeightRatio / videoSize.height)
        }
    }

    override fun onPlayerError(error: PlaybackException) {
        val failed = seq ?: return
        // The code name only: the exception's text may carry the address.
        Log.w(TAG, "Clip $failed did not play: ${error.errorCodeName}")
        core.videoFailed(failed)
    }

    fun release() {
        if (released) return
        released = true
        player.removeListener(this)
        player.release()
        addresses.clear()
        sync.close()
        if (debug) Log.d(TAG, "released the player of clip $seq")
    }
}
