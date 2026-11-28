package dev.psytify.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.layout.LocalPinnableContainer
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.zIndex
import dev.psytify.app.design.AnimatedSprite
import dev.psytify.app.design.IconButton
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SpriteImage
import dev.psytify.app.rust.MenuView
import dev.psytify.app.rust.PlaybackAction
import dev.psytify.app.rust.PsytifyCore
import dev.psytify.app.rust.QueueEntry
import dev.psytify.app.rust.QueuePage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import kotlin.math.roundToInt

/** Entries asked of the bridge at a time: a screen and a half of rows. */
private const val PAGE = 50

/** After a drop, how long the moved rows wait for the core's new queue before they go back. */
private const val SETTLE_MS = 1_500L

/** Near the list's edges a dragged row scrolls the list: this band, at most this fast. */
private val EDGE = 56.dp
private val EDGE_SPEED = 1_200.dp

/**
 * A row being dragged: where it was, its top and the finger's place in the list's viewport when the
 * drag started, how far the finger moved and how far the list scrolled since. The target counts
 * `finger + scrolled`; while held, a copy of the row is drawn over the list at `top + finger`, under
 * the finger even when the list scrolls the row's own place out of view (a lazy list does not draw
 * an item off screen, pinned or not).
 */
private data class Drag(
    val from: Int,
    val top: Float,
    val startY: Float,
    val finger: Float = 0f,
    val scrolled: Float = 0f,
    val dropped: Boolean = false,
    val settled: Float? = null,
) {
    val dy: Float get() = settled ?: (finger + scrolled)
}

/**
 * The queue in the player, as in the Iced phone layout: the playing track, then the next ones. Rows
 * are read from the bridge a page at a time (`queuePage`), only the pages on screen, and read again
 * when the core publishes another queue (`queueVersion`). A tap plays the entry, the cross removes
 * it, a long-press opens its «Altro» sheet, and the grip drags a next entry to another place
 * (`PlaybackAction.Move`, the core's rule).
 */
@Composable
fun QueueList(
    core: PsytifyCore,
    queueVersion: ULong,
    playing: Boolean,
    onMenu: (MenuView) -> Unit,
    modifier: Modifier = Modifier,
) {
    var head by remember { mutableStateOf<QueuePage?>(null) }
    val pages = remember(queueVersion) { mutableStateMapOf<Int, QueuePage>() }
    var drag by remember { mutableStateOf<Drag?>(null) }
    val list = rememberLazyListState()
    var viewport by remember { mutableStateOf(0f) }
    LaunchedEffect(queueVersion) {
        head = withContext(Dispatchers.Default) { core.queuePage(0u, 0u) }
        // The core published the queue the drop asked for (or another): the rows are where it says.
        drag = null
    }
    val dropped = drag?.dropped == true
    LaunchedEffect(dropped) {
        if (dropped) {
            delay(SETTLE_MS)
            drag = null
        }
    }
    val queue = head ?: return
    val total = queue.total.toInt()
    // The list starts at the current entry: what was already played is not shown, as on the phone.
    val start = queue.current?.toInt() ?: 0
    val firstMovable = queue.current?.let { it.toInt() + 1 } ?: 0
    val density = LocalDensity.current
    // Rows are as tall as `PHONE_ROW_H`, taller with a big font: the measured one counts.
    var rowPx by remember { mutableStateOf(with(density) { PsytifySpace.PHONE_ROW_H.toPx() }) }
    val edgePx = with(density) { EDGE.toPx() }
    val speedPx = with(density) { EDGE_SPEED.toPx() }
    // While a row is held near an edge, the list scrolls a little each frame (only then).
    val holding = drag?.takeIf { !it.dropped } != null
    LaunchedEffect(holding) {
        if (!holding) return@LaunchedEffect
        var last = withFrameNanos { it }
        while (true) {
            val now = withFrameNanos { it }
            val seconds = (now - last) / 1_000_000_000f
            last = now
            val held = drag?.takeIf { !it.dropped } ?: break
            val y = held.startY + held.finger
            val speed = when {
                y < edgePx -> -speedPx * ((edgePx - y) / edgePx).coerceIn(0f, 1f)
                y > viewport - edgePx -> speedPx * ((y - (viewport - edgePx)) / edgePx).coerceIn(0f, 1f)
                else -> 0f
            }
            if (speed != 0f) {
                val moved = list.scrollBy(speed * seconds)
                drag?.takeIf { !it.dropped }?.let { drag = it.copy(scrolled = it.scrolled + moved) }
            }
        }
    }
    fun entry(position: Int): QueueEntry? {
        val page = pages[position / PAGE] ?: return null
        return page.entries.getOrNull(position - page.offset.toInt())
    }
    fun target(of: Drag): Int = (of.from + (of.dy / rowPx).roundToInt()).coerceIn(firstMovable, total - 1)

    Column(modifier) {
        PsytifyText(queue.summary, PsytifyType.TYPE_META, PsytifyColors.TEXT_SECONDARY)
        Spacer(Modifier.height(PsytifySpace.SPACE_3))
        if (total == 0) {
            PsytifyText(
                "La coda è vuota. Scegli un brano per iniziare.",
                PsytifyType.TYPE_BODY,
                PsytifyColors.TEXT_SECONDARY,
                Modifier.weight(1f),
                maxLines = 2,
            )
            return@Column
        }
        val rowAt: @Composable (Int) -> Unit = { position ->
            val pageIndex = position / PAGE
            LaunchedEffect(queueVersion, pageIndex) {
                if (pages[pageIndex] == null) {
                    val page = withContext(Dispatchers.Default) {
                        core.queuePage((pageIndex * PAGE).toUInt(), PAGE.toUInt())
                    }
                    if (page.version == queueVersion) pages[pageIndex] = page
                }
            }
            // While a row is dragged, the rows between its place and the target make room.
            val shown = drag
            val shift = when {
                shown == null -> 0f
                position == shown.from -> shown.dy
                else -> {
                    val to = target(shown)
                    when {
                        shown.from < position && position <= to -> -rowPx
                        to <= position && position < shown.from -> rowPx
                        else -> 0f
                    }
                }
            }
            val lifted = shown?.from == position
            // The held row is drawn by the copy over the list; its place keeps the gesture.
            val hidden = lifted && shown?.dropped == false
            val movable = position >= firstMovable && total - firstMovable > 1
            QueueRow(
                entry = entry(position),
                current = position == queue.current?.toInt(),
                playing = playing,
                movable = movable,
                lifted = lifted,
                modifier = Modifier
                    .zIndex(if (lifted) 1f else 0f)
                    .graphicsLayer {
                        translationY = shift
                        alpha = if (hidden) 0f else 1f
                    }
                    .onSizeChanged { if (it.height > 0) rowPx = it.height.toFloat() },
                onPlay = { core.playback(PlaybackAction.PlayAt(position.toUInt())) },
                onRemove = { core.playback(PlaybackAction.Remove(position.toUInt())) },
                onMenu = { core.queueMenu(queueVersion, position.toUInt())?.let(onMenu) },
                // TalkBack cannot drag: one place up or down is an action of the row.
                onMoveUp = if (movable && position > firstMovable) {
                    { core.playback(PlaybackAction.Move(position.toUInt(), (position - 1).toUInt())) }
                } else {
                    null
                },
                onMoveDown = if (movable && position < total - 1) {
                    { core.playback(PlaybackAction.Move(position.toUInt(), (position + 1).toUInt())) }
                } else {
                    null
                },
                onDragStart = { fingerInRow ->
                    if (drag == null) {
                        val top = list.layoutInfo.visibleItemsInfo
                            .find { it.key == "$queueVersion:$position" }?.offset?.toFloat() ?: 0f
                        drag = Drag(position, top = top, startY = top + fingerInRow)
                    }
                },
                onDrag = { dy -> drag?.takeIf { !it.dropped }?.let { drag = it.copy(finger = it.finger + dy) } },
                onDragEnd = {
                    val done = drag
                    if (done != null && !done.dropped) {
                        val to = target(done)
                        if (to != done.from) {
                            core.playback(PlaybackAction.Move(done.from.toUInt(), to.toUInt()))
                            drag = done.copy(dropped = true, settled = (to - done.from) * rowPx)
                        } else {
                            drag = null
                        }
                    }
                },
            )
        }
        Box(Modifier.weight(1f).clipToBounds()) {
            LazyColumn(Modifier.fillMaxSize().onSizeChanged { viewport = it.height.toFloat() }, state = list) {
                if (queue.current != null) {
                    item(key = "label:now") { Label("In riproduzione") }
                    item(key = "$queueVersion:$start") { rowAt(start) }
                }
                if (firstMovable < total) {
                    item(key = "label:next") { Label("Successivi") }
                    items(count = total - firstMovable, key = { index -> "$queueVersion:${firstMovable + index}" }) { index ->
                        rowAt(firstMovable + index)
                    }
                }
            }
            drag?.takeIf { !it.dropped }?.let { held ->
                QueueRow(
                    entry = entry(held.from),
                    current = false,
                    playing = playing,
                    movable = true,
                    lifted = true,
                    modifier = Modifier
                        .graphicsLayer { translationY = held.top + held.finger }
                        .clearAndSetSemantics {},
                    onPlay = {},
                    onRemove = {},
                    onMenu = {},
                    onMoveUp = null,
                    onMoveDown = null,
                    onDragStart = {},
                    onDrag = {},
                    onDragEnd = {},
                )
            }
        }
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Spacer(Modifier.weight(1f))
            Box(
                Modifier
                    .height(PsytifySpace.TOUCH_TARGET)
                    .clickable(remember { MutableInteractionSource() }, indication = null, role = Role.Button) {
                        core.playback(PlaybackAction.ClearQueue)
                    }
                    .padding(horizontal = PsytifySpace.SPACE_3),
                contentAlignment = Alignment.Center,
            ) {
                PsytifyText("Svuota", PsytifyType.TYPE_BUTTON, PsytifyColors.TEXT_SECONDARY)
            }
        }
    }
}

@Composable
private fun Label(text: String) {
    PsytifyText(
        text,
        PsytifyType.TYPE_META,
        PsytifyColors.TEXT_SECONDARY,
        Modifier.padding(top = PsytifySpace.SPACE_3, bottom = PsytifySpace.SPACE_2),
    )
}

/** A row as tall as `PHONE_ROW_H`; a placeholder of the same height until its page arrives. */
@Composable
private fun QueueRow(
    entry: QueueEntry?,
    current: Boolean,
    playing: Boolean,
    movable: Boolean,
    lifted: Boolean,
    modifier: Modifier,
    onPlay: () -> Unit,
    onRemove: () -> Unit,
    onMenu: () -> Unit,
    onMoveUp: (() -> Unit)?,
    onMoveDown: (() -> Unit)?,
    onDragStart: (Float) -> Unit,
    onDrag: (Float) -> Unit,
    onDragEnd: () -> Unit,
) {
    val haptics = LocalHapticFeedback.current
    // The lifted row stays composed while the list scrolls its place out of view: its gesture
    // lives in it.
    val pinnable = LocalPinnableContainer.current
    DisposableEffect(lifted, pinnable) {
        val pin = if (lifted) pinnable?.pin() else null
        onDispose { pin?.release() }
    }
    val fill = when {
        lifted -> PsytifyColors.SURFACE_RAISED
        current -> PsytifyColors.SURFACE_SELECTED
        else -> PsytifyColors.SURFACE_CANVAS
    }
    Row(
        modifier
            .fillMaxWidth()
            .heightIn(min = PsytifySpace.PHONE_ROW_H)
            .background(fill)
            .semantics {
                if (current) stateDescription = if (playing) "in riproduzione" else "brano corrente"
                customActions = listOfNotNull(
                    onMoveUp?.let { up -> CustomAccessibilityAction("Sposta su") { up(); true } },
                    onMoveDown?.let { down -> CustomAccessibilityAction("Sposta giù") { down(); true } },
                )
            }
            .combinedClickable(
                remember { MutableInteractionSource() },
                indication = null,
                role = Role.Button,
                onLongClickLabel = "Altro",
                onLongClick = {
                    haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                    onMenu()
                },
                onClick = onPlay,
            ),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val track = entry?.track
        if (movable) {
            val start by rememberUpdatedState(onDragStart)
            val move by rememberUpdatedState(onDrag)
            val end by rememberUpdatedState(onDragEnd)
            Box(
                Modifier
                    .size(PsytifySpace.TOUCH_TARGET, PsytifySpace.PHONE_ROW_H)
                    // Not a TalkBack target: the row's «Sposta su» and «Sposta giù» do it.
                    .clearAndSetSemantics {}
                    .pointerInput(Unit) {
                        detectDragGestures(
                            onDragStart = { offset ->
                                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                start(offset.y)
                            },
                            onDragEnd = { end() },
                            onDragCancel = { end() },
                        ) { change, amount ->
                            change.consume()
                            move(amount.y)
                        }
                    },
                contentAlignment = Alignment.Center,
            ) {
                SpriteImage(PsytifySprites.ICON_GRIP, tint = PsytifyColors.ICON_IDLE)
            }
        } else {
            Spacer(Modifier.width(PsytifySpace.SPACE_2))
        }
        if (track == null) {
            Box(
                Modifier
                    .width(PsytifySpace.COVER_S)
                    .height(PsytifySpace.COVER_S)
                    .background(PsytifyColors.SURFACE_RAISED)
                    .semantics { contentDescription = "Caricamento" },
            )
            Spacer(Modifier.weight(1f))
            return@Row
        }
        Cover(track.coverUrl, PsytifySpace.COVER_S)
        Spacer(Modifier.width(PsytifySpace.SPACE_4))
        Column(Modifier.weight(1f).padding(vertical = PsytifySpace.SPACE_2)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                // Lime only while it plays: the row's surface already says which entry is current.
                if (current && playing) {
                    AnimatedSprite(PsytifySprites.EQ, running = true, tint = PsytifyColors.STATE_ON)
                    Spacer(Modifier.width(PsytifySpace.SPACE_2))
                }
                PsytifyText(track.title, PsytifyType.TYPE_TRACK_TITLE, PsytifyColors.TEXT_PRIMARY)
            }
            PsytifyText(track.artist, PsytifyType.TYPE_ARTIST, PsytifyColors.TEXT_SECONDARY)
        }
        IconButton(PsytifySprites.ICON_CLOSE, "Togli dalla coda", onRemove, tint = PsytifyColors.ICON_IDLE)
    }
}
