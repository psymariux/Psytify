package dev.psytify.app.ui

import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.animateScrollBy
import androidx.compose.foundation.gestures.scrollBy
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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.sp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleStartEffect
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import dev.psytify.app.core.Playback
import dev.psytify.app.design.AnimatedSprite
import dev.psytify.app.design.LocalReduceMotion
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SecondaryButton
import dev.psytify.app.design.ap
import dev.psytify.app.rust.LyricLine
import dev.psytify.app.rust.LyricsCursor
import dev.psytify.app.rust.LyricsSnapshot
import dev.psytify.app.rust.LyricsStatus
import dev.psytify.app.rust.PlayerState
import dev.psytify.app.rust.PsytifyCore
import kotlinx.coroutines.delay

/** Where the current verse sits, from the top, as a share of the height: the upper third. */
private const val ANCHOR = 0.3f
/** The cursor wakes just after a change, not just before. */
private const val WAKE_MARGIN_MS = 2L
/** Space left of the verses: the cursor block and the gap to the text. */
private val GUTTER = PsytifySpace.SPACE_4
/** Widths of the grey verses while the text arrives, as shares of the line. */
private val SKELETON = listOf(0.86f, 0.62f, 0.92f, 0.48f, 0.74f, 0.58f)
/** `MOTION_LYRICS`, the design's ease-out. */
private val EASE_OUT = CubicBezierEasing(0.2f, 0f, 0f, 1f)

/**
 * «Testo» in the full player: the core loads the lyrics only while this is on screen with the app
 * in front. Synced text puts the current verse at the upper third with the cursor block (lime while
 * playing, stone in pause) and glides there in `MOTION_LYRICS` (a jump under reduced motion); a tap
 * on a verse seeks there. Which verse and when the next change comes are the Rust side's: the panel
 * sleeps until then, and with the track paused nothing runs.
 */
@Composable
fun LyricsPanel(lyrics: LyricsSnapshot?, playback: Playback, core: PsytifyCore, modifier: Modifier = Modifier) {
    LifecycleStartEffect(core) {
        core.showLyrics(true)
        onStopOrDispose { core.showLyrics(false) }
    }
    val playing = playback.snapshot.state == PlayerState.Playing
    Box(modifier.fillMaxWidth()) {
        when (val status = lyrics?.status) {
            null, LyricsStatus.Loading -> Loading()
            LyricsStatus.NoTrack -> Note("Scegli un brano per leggere il testo.")
            LyricsStatus.Missing -> Missing(playing)
            is LyricsStatus.Failed -> Column(Modifier.padding(start = GUTTER)) {
                PsytifyText(status.message, PsytifyType.TYPE_BODY, PsytifyColors.TEXT_SECONDARY, maxLines = 3)
                Spacer(Modifier.height(PsytifySpace.SPACE_4))
                // A new «shows the lyrics» makes the core try again.
                SecondaryButton("Riprova", { core.showLyrics(true) })
            }
            is LyricsStatus.Plain -> PsytifyText(
                status.text,
                PsytifyType.TYPE_LYRIC,
                PsytifyColors.TEXT_SECONDARY,
                Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(start = GUTTER, bottom = PsytifySpace.SPACE_6),
                maxLines = Int.MAX_VALUE,
            )
            is LyricsStatus.Synced -> Synced(lyrics.version, status.lines, playback, core)
        }
    }
}

@Composable
private fun Synced(version: ULong, lines: List<LyricLine>, playback: Playback, core: PsytifyCore) {
    var cursor by remember(version) { mutableStateOf(LyricsCursor(null, 0u, null)) }
    // Ask for the cursor now, then sleep until the change it names; a new snapshot starts over. In the
    // background nothing wakes.
    val owner = LocalLifecycleOwner.current
    LaunchedEffect(version, playback) {
        owner.lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                val next = core.lyricsCursor(version, playback.positionMs().toULong())
                cursor = next
                if (!playback.snapshot.running) break
                val wait = next.nextChangeMs ?: break
                delay(wait.toLong() + WAKE_MARGIN_MS)
            }
        }
    }
    val playing = playback.snapshot.state == PlayerState.Playing
    val reduce = LocalReduceMotion.current
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val anchor = maxHeight * ANCHOR
        val anchorPx = with(LocalDensity.current) { anchor.roundToPx() }
        val list = rememberLazyListState()
        val current = cursor.line?.toInt()
        // Item 0 is the space above the first verse, so verse i is item i + 1.
        LaunchedEffect(version, current, anchorPx) {
            val target = (current ?: -1) + 1
            val shown = list.layoutInfo.visibleItemsInfo.firstOrNull { it.index == target }
            // The space above the first verse goes to the top; a verse goes to the anchor.
            val goal = if (target == 0) 0 else anchorPx
            when {
                shown != null && !reduce -> list.animateScrollBy(
                    (shown.offset - goal).toFloat(),
                    tween(PsytifySpace.MOTION_LYRICS_MS, easing = EASE_OUT),
                )
                shown != null -> list.scrollBy((shown.offset - goal).toFloat())
                target == 0 -> list.scrollToItem(0)
                else -> {
                    list.scrollToItem(target)
                    list.scrollBy(-anchorPx.toFloat())
                }
            }
        }
        LazyColumn(Modifier.fillMaxSize(), state = list) {
            item(key = "top") { Spacer(Modifier.height(anchor)) }
            itemsIndexed(lines, key = { index, _ -> index }) { index, line ->
                Verse(
                    line,
                    tone = when {
                        current == null || index > current -> Tone.Next
                        index < current -> Tone.Past
                        else -> Tone.Now
                    },
                    dots = if (index == current) cursor.dots.toInt() else 0,
                    playing = playing,
                    onTap = { core.lyricsSeek(version, index.toUInt()) },
                )
            }
            item(key = "bottom") { Spacer(Modifier.height(maxHeight - anchor)) }
        }
    }
}

private enum class Tone { Past, Now, Next }

/** One verse: the cursor block on the current one, three dots for an instrumental pause. */
@Composable
private fun Verse(line: LyricLine, tone: Tone, dots: Int, playing: Boolean, onTap: () -> Unit) {
    val role = PsytifyType.TYPE_LYRIC
    val lineHeight = with(LocalDensity.current) { role.lineHeight.sp.toDp() }
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(remember { MutableInteractionSource() }, indication = null, role = Role.Button, onClick = onTap)
            .padding(bottom = PsytifySpace.SPACE_4)
            .semantics { if (line.pause) contentDescription = "Pausa strumentale" },
    ) {
        Box(Modifier.width(GUTTER).height(lineHeight), contentAlignment = Alignment.CenterStart) {
            if (tone == Tone.Now) {
                Box(
                    Modifier
                        .size(ap(2), ap(3))
                        .background(if (playing) PsytifyColors.STATE_ON else PsytifyColors.STONE),
                )
            }
        }
        if (line.pause) {
            Row(
                Modifier.height(lineHeight),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(ap(2)),
            ) {
                repeat(3) { dot ->
                    Box(
                        Modifier
                            .size(ap(2))
                            .background(if (dot < dots) PsytifyColors.WARM_WHITE else PsytifyColors.STONE),
                    )
                }
            }
        } else {
            PsytifyText(
                line.text,
                role,
                when (tone) {
                    Tone.Past -> PsytifyColors.TEXT_TERTIARY
                    Tone.Now -> PsytifyColors.TEXT_PRIMARY
                    Tone.Next -> PsytifyColors.TEXT_SECONDARY
                },
                Modifier.weight(1f),
                maxLines = Int.MAX_VALUE,
            )
        }
    }
}

/** Grey verses the size of real ones while the text arrives. */
@Composable
private fun Loading() {
    val height = with(LocalDensity.current) { (PsytifyType.TYPE_LYRIC.size * 0.8f).sp.toDp() }
    BoxWithConstraints(Modifier.fillMaxWidth().padding(start = GUTTER, top = PsytifySpace.SPACE_3)) {
        val width = maxWidth
        Column(verticalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_5)) {
            SKELETON.forEach { share ->
                Box(Modifier.size(width * share, height).background(PsytifyColors.SURFACE_RAISED))
            }
        }
    }
}

@Composable
private fun Note(text: String) {
    PsytifyText(text, PsytifyType.TYPE_BODY, PsytifyColors.TEXT_SECONDARY, Modifier.padding(start = GUTTER), maxLines = 3)
}

/** No text for the track: Nastro listens while it plays. */
@Composable
private fun Missing(playing: Boolean) {
    Column(
        Modifier.fillMaxSize().padding(PsytifySpace.SPACE_6),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        AnimatedSprite(if (playing) PsytifySprites.NASTRO_LISTEN else PsytifySprites.NASTRO_IDLE, running = true)
        Spacer(Modifier.height(PsytifySpace.SPACE_5))
        PsytifyText("Nessun testo per questo brano", PsytifyType.TYPE_HEADING, PsytifyColors.TEXT_PRIMARY, maxLines = 3, align = TextAlign.Center)
        Spacer(Modifier.height(PsytifySpace.SPACE_2))
        PsytifyText("LRCLIB non ha il testo di questo brano.", PsytifyType.TYPE_BODY, PsytifyColors.TEXT_SECONDARY, maxLines = 3, align = TextAlign.Center)
    }
}

