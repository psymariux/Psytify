package dev.psytify.app.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import dev.psytify.app.design.LocalArtPx
import dev.psytify.app.design.LocalReduceMotion
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.SpriteImage
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/** How far the row goes before the release adds it (dp), as in the Iced phone layout. */
private val THRESHOLD = 88.dp

/** Past the threshold the row follows the finger by this fraction: the limit is felt. */
private const val RESISTANCE = 0.3f

/**
 * A row that slides to the right to go to the queue: the strip under it shows the queue icon and
 * lights up past the threshold; on release past it `onSwipe` runs, then the row snaps back. A
 * vertical drag is the list's scroll: the gesture starts only after a horizontal slop.
 */
@Composable
fun SwipeToQueue(enabled: Boolean, onSwipe: () -> Unit, content: @Composable () -> Unit) {
    if (!enabled) {
        // The sideways drag is still taken, so it does not end as a tap on the row.
        Box(Modifier.pointerInput(Unit) { detectHorizontalDragGestures { change, _ -> change.consume() } }) { content() }
        return
    }
    val offset = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val threshold = with(LocalDensity.current) { THRESHOLD.toPx() }
    val px = LocalArtPx.current
    val still = LocalReduceMotion.current
    suspend fun back() = if (still) offset.snapTo(0f) else offset.animateTo(0f, tween(PsytifySpace.MOTION_PANEL_MS))
    val shown = offset.value
    val armed = shown >= threshold
    Box(
        Modifier
            .fillMaxWidth()
            .heightIn(min = PsytifySpace.PHONE_ROW_H)
            .pointerInput(still) {
                var raw = 0f
                detectHorizontalDragGestures(
                    onDragStart = { raw = 0f },
                    onDragEnd = {
                        if (offset.value >= threshold) onSwipe()
                        scope.launch { back() }
                    },
                    onDragCancel = { scope.launch { back() } },
                ) { change, amount ->
                    change.consume()
                    val before = offset.value >= threshold
                    raw = (raw + amount).coerceAtLeast(0f)
                    val eased = if (raw > threshold) threshold + (raw - threshold) * RESISTANCE else raw
                    scope.launch { offset.snapTo(eased) }
                    if (!before && eased >= threshold) haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                }
            },
    ) {
        if (shown > 0f) {
            Box(
                Modifier
                    .matchParentSize()
                    .background(if (armed) PsytifyColors.WARM_WHITE else PsytifyColors.SURFACE_SELECTED)
                    .padding(start = PsytifySpace.SPACE_6),
                contentAlignment = Alignment.CenterStart,
            ) {
                Box(Modifier.width(PsytifySpace.ICON), contentAlignment = Alignment.Center) {
                    SpriteImage(
                        PsytifySprites.ICON_QUEUE,
                        tint = if (armed) PsytifyColors.TEXT_ON_LIGHT else PsytifyColors.ICON_IDLE,
                    )
                }
            }
        }
        // The row moves in whole art pixels.
        Box(Modifier.offset { IntOffset((shown / px).roundToInt() * px, 0) }) { content() }
    }
}
