package dev.psytify.app.ui

import android.os.SystemClock
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.graphics.graphicsLayer
import dev.psytify.app.design.LocalArtPx
import dev.psytify.app.design.LocalReduceMotion
import kotlin.math.roundToInt
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.min
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.psytify.app.core.CoreHub
import dev.psytify.app.core.Playback
import dev.psytify.app.design.AnimatedSprite
import dev.psytify.app.design.IconButton
import dev.psytify.app.design.Pill
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SpriteImage
import dev.psytify.app.design.ap
import dev.psytify.app.design.rememberNotch
import dev.psytify.app.rust.PlaybackAction
import dev.psytify.app.rust.MenuView
import dev.psytify.app.rust.PlayerState
import dev.psytify.app.rust.PsytifyCore
import dev.psytify.app.rust.RepeatMode
import dev.psytify.app.rust.formatClock
import kotlinx.coroutines.delay

/** The player's distance from the screen edges. */
private val GUTTER = 24.dp
/** Previous and next: bigger than the switches at the sides. */
private val SKIP = 56.dp
/** The small cover next to the title while the queue takes the cover's place. */
private val MINI_COVER = 52.dp
/** The times under the bar move once a second at most; a quarter keeps them on the second. */
private const val TICK_MS = 250L
/** How far down a whole back gesture moves the player, as a share of its height. */
private const val BACK_DROP = 0.2f

/** What fills the player above the controls: the media slot, the lyrics or the queue. */
enum class PlayerPanel { MEDIA, LYRICS, QUEUE }

/**
 * The full-screen player on the black canvas, as in the Iced phone layout: the media slot (the
 * cover, or the clip in its place), title and artist, the progress bar with the times under it, the
 * big controls and, at the bottom, «Testo» and «Coda». With the lyrics or the queue open, they take
 * the slot's place under a small cover row, and the controls stay where they are.
 */
@Composable
fun PlayerScreen(
    playback: Playback,
    core: PsytifyCore,
    panel: PlayerPanel,
    onPanel: (PlayerPanel) -> Unit,
    clipOn: Boolean,
    onClipOn: (Boolean) -> Unit,
    onClose: () -> Unit,
    onMenu: (MenuView) -> Unit = {},
    backProgress: Float = 0f,
) {
    val snapshot = playback.snapshot
    val track = snapshot.track
    val video by CoreHub.video.collectAsStateWithLifecycle()
    val lyrics by CoreHub.lyrics.collectAsStateWithLifecycle()
    val px = LocalArtPx.current
    // A predictive back gesture lowers the player with the finger, in whole art pixels.
    val lowered = if (LocalReduceMotion.current) 0f else backProgress
    Column(
        Modifier
            .fillMaxSize()
            .graphicsLayer { translationY = (((lowered * BACK_DROP * size.height) / px).roundToInt() * px).toFloat() }
            .background(PsytifyColors.SURFACE_CANVAS)
            .semantics { paneTitle = "Player" }
            .systemBarsPadding()
            .padding(horizontal = GUTTER - PsytifySpace.SPACE_3),
        verticalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_4),
    ) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            IconButton(PsytifySprites.ICON_CHEVRON_DOWN, "Chiudi il player", onClose)
            Box(Modifier.weight(1f), contentAlignment = Alignment.Center) {
                PsytifyText("In riproduzione", PsytifyType.TYPE_META, PsytifyColors.TEXT_SECONDARY)
            }
            Spacer(Modifier.width(PsytifySpace.TOUCH_TARGET))
        }
        Column(Modifier.weight(1f).padding(horizontal = PsytifySpace.SPACE_3)) {
            if (track == null) {
                Spacer(Modifier.weight(1f))
            } else if (panel != PlayerPanel.MEDIA) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clickable(
                            remember { MutableInteractionSource() },
                            indication = null,
                            onClickLabel = "Torna alla copertina",
                        ) { onPanel(PlayerPanel.MEDIA) }
                        .semantics { liveRegion = LiveRegionMode.Polite },
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Cover(track.coverUrl, MINI_COVER)
                    Spacer(Modifier.width(PsytifySpace.SPACE_4))
                    Column(Modifier.weight(1f)) {
                        PsytifyText(track.title, PsytifyType.TYPE_TRACK_TITLE, PsytifyColors.TEXT_PRIMARY)
                        PsytifyText(detail(track.artist, track.album), PsytifyType.TYPE_ARTIST, PsytifyColors.TEXT_SECONDARY)
                    }
                }
                Spacer(Modifier.height(PsytifySpace.SPACE_4))
                if (panel == PlayerPanel.QUEUE) {
                    QueueList(
                        core,
                        snapshot.queueVersion,
                        playing = snapshot.state == PlayerState.Playing,
                        onMenu = onMenu,
                        modifier = Modifier.weight(1f),
                    )
                } else {
                    LyricsPanel(lyrics, playback, core, Modifier.weight(1f))
                }
            } else {
                BoxWithConstraints(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                    MediaSlot(track, video, clipOn, onClipOn, playback, core, min(maxWidth, maxHeight))
                }
                Spacer(Modifier.height(PsytifySpace.SPACE_5))
                // One live region: TalkBack says the next track when it starts.
                Column(Modifier.semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite }) {
                    PsytifyText(track.title, PsytifyType.TYPE_HEADING, PsytifyColors.TEXT_PRIMARY)
                    Spacer(Modifier.height(PsytifySpace.SPACE_2))
                    PsytifyText(detail(track.artist, track.album), PsytifyType.TYPE_ARTIST, PsytifyColors.TEXT_SECONDARY)
                }
            }
        }
        Progress(playback, core, Modifier.padding(horizontal = PsytifySpace.SPACE_3))
        Controls(playback, core)
        Row(
            Modifier.fillMaxWidth().padding(bottom = PsytifySpace.SPACE_3),
            horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_3, Alignment.CenterHorizontally),
        ) {
            val lyricsOn = panel == PlayerPanel.LYRICS
            val queueOn = panel == PlayerPanel.QUEUE
            Pill("Testo", on = lyricsOn, onClick = { onPanel(if (lyricsOn) PlayerPanel.MEDIA else PlayerPanel.LYRICS) })
            Pill("Coda", on = queueOn, onClick = { onPanel(if (queueOn) PlayerPanel.MEDIA else PlayerPanel.QUEUE) })
        }
    }
}

private fun detail(artist: String, album: String?): String = if (album == null) artist else "$artist · $album"

@Composable
private fun Progress(playback: Playback, core: PsytifyCore, modifier: Modifier = Modifier) {
    val snapshot = playback.snapshot
    val duration = snapshot.track?.durationMs?.toLong() ?: 0L
    var now by remember { mutableLongStateOf(SystemClock.elapsedRealtime()) }
    LaunchedEffect(playback, snapshot.running) {
        now = SystemClock.elapsedRealtime()
        while (snapshot.running) {
            delay(TICK_MS)
            now = SystemClock.elapsedRealtime()
        }
    }
    var scrub by remember { mutableStateOf<Float?>(null) }
    val position = scrub?.let { (it * duration).toLong() } ?: playback.positionMs(now)
    val fraction = if (duration > 0) position.toFloat() / duration else 0f
    Column(modifier) {
        SeekBar(
            fraction = fraction,
            state = "${formatClock(position.toULong())} di ${formatClock(duration.toULong())}",
            scrub = scrub,
            onScrub = { scrub = it },
            onSeek = { target ->
                if (duration > 0) core.playback(PlaybackAction.SeekTo((target * duration).toLong().toULong()))
            },
        )
        // The seek bar says the times; these are for the eyes.
        Row(Modifier.fillMaxWidth().clearAndSetSemantics {}) {
            PsytifyText(formatClock(position.toULong()), PsytifyType.TYPE_NUMERIC_PIXEL, PsytifyColors.TEXT_SECONDARY)
            Spacer(Modifier.weight(1f))
            PsytifyText(formatClock(duration.toULong()), PsytifyType.TYPE_NUMERIC_PIXEL, PsytifyColors.TEXT_SECONDARY)
        }
    }
}

@Composable
private fun Controls(playback: Playback, core: PsytifyCore) {
    val snapshot = playback.snapshot
    val send = core::playback
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(
            PsytifySprites.ICON_SHUFFLE,
            "Riproduzione casuale",
            { send(PlaybackAction.ToggleShuffle) },
            tint = if (snapshot.shuffle) PsytifyColors.STATE_ON else PsytifyColors.ICON_IDLE,
            state = if (snapshot.shuffle) "attiva" else "disattivata",
        )
        IconButton(PsytifySprites.ICON_PREVIOUS, "Precedente", { send(PlaybackAction.Previous) }, Modifier.size(SKIP))
        PlayButton(snapshot.state) { send(PlaybackAction.Toggle) }
        IconButton(
            PsytifySprites.ICON_NEXT,
            "Successivo",
            { send(PlaybackAction.Next) },
            Modifier.size(SKIP),
            tint = if (snapshot.hasNext) PsytifyColors.ICON_ACTIVE else PsytifyColors.ICON_DISABLED,
        )
        val repeat = snapshot.repeat
        IconButton(
            if (repeat == RepeatMode.ONE) PsytifySprites.ICON_REPEAT_ONE else PsytifySprites.ICON_REPEAT,
            "Ripeti",
            { send(PlaybackAction.CycleRepeat) },
            tint = if (repeat == RepeatMode.OFF) PsytifyColors.ICON_IDLE else PsytifyColors.STATE_ON,
            state = when (repeat) {
                RepeatMode.OFF -> "disattivato"
                RepeatMode.ALL -> "tutta la coda"
                RepeatMode.ONE -> "questo brano"
            },
        )
    }
}

/** The primary square: the biggest control of the screen, white face on its hard ledge. */
@Composable
private fun PlayButton(state: PlayerState, onClick: () -> Unit) {
    val shape = rememberNotch(PsytifySpace.NOTCH_1_AP)
    val playing = state == PlayerState.Playing
    Box(
        Modifier
            .size(PsytifySpace.PHONE_PLAY_BUTTON)
            .clickable(remember { MutableInteractionSource() }, indication = null, role = Role.Button, onClick = onClick)
            .semantics {
                contentDescription = when {
                    state == PlayerState.Loading -> "Caricamento"
                    playing -> "Pausa"
                    else -> "Riproduci"
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier
                .size(PsytifySpace.PHONE_PLAY_BUTTON - ap(PsytifySpace.LEDGE_Y_AP))
                .offset(ap(PsytifySpace.LEDGE_X_AP), ap(PsytifySpace.LEDGE_Y_AP))
                .background(PsytifyColors.LEDGE_PRIMARY, shape),
        )
        Box(
            Modifier
                .size(PsytifySpace.PHONE_PLAY_BUTTON - ap(PsytifySpace.LEDGE_Y_AP))
                .background(PsytifyColors.PURE_WHITE, shape),
            contentAlignment = Alignment.Center,
        ) {
            if (state == PlayerState.Loading) {
                AnimatedSprite(PsytifySprites.LOADER, running = true, tint = PsytifyColors.TEXT_ON_LIGHT)
            } else {
                SpriteImage(
                    if (playing) PsytifySprites.ICON_PAUSE else PsytifySprites.ICON_PLAY,
                    tint = PsytifyColors.TEXT_ON_LIGHT,
                )
            }
        }
    }
}

