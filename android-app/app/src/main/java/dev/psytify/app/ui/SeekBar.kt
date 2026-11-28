package dev.psytify.app.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.stateDescription
import dev.psytify.app.design.LocalArtPx
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace

/**
 * The player's progress bar, as tall as a touch target: a tap seeks, a drag scrubs and seeks on
 * release. While the finger is down the caller shows `scrub` instead of the playing position.
 * `state` is what TalkBack says of the position.
 * Square thumb, lines on whole art pixels.
 */
@Composable
fun SeekBar(
    fraction: Float,
    state: String,
    scrub: Float?,
    onScrub: (Float?) -> Unit,
    onSeek: (Float) -> Unit,
    modifier: Modifier = Modifier,
) {
    val px = LocalArtPx.current
    val shown = (scrub ?: fraction).coerceIn(0f, 1f)
    Canvas(
        modifier
            .fillMaxWidth()
            .height(PsytifySpace.TOUCH_TARGET)
            .pointerInput(Unit) {
                detectTapGestures { offset -> onSeek((offset.x / size.width).coerceIn(0f, 1f)) }
            }
            .pointerInput(Unit) {
                var last = 0f
                detectHorizontalDragGestures(
                    onDragStart = { offset ->
                        last = (offset.x / size.width).coerceIn(0f, 1f)
                        onScrub(last)
                    },
                    onDragEnd = {
                        onSeek(last)
                        onScrub(null)
                    },
                    onDragCancel = { onScrub(null) },
                    onHorizontalDrag = { change, _ ->
                        last = (change.position.x / size.width).coerceIn(0f, 1f)
                        onScrub(last)
                    },
                )
            }
            .semantics {
                contentDescription = "Avanzamento"
                // «1:23 di 3:45» rather than a percentage; volume keys and swipes move it by steps.
                stateDescription = state
                progressBarRangeInfo = ProgressBarRangeInfo(shown, 0f..1f)
                setProgress { value ->
                    onSeek(value.coerceIn(0f, 1f))
                    true
                }
            },
    ) {
        val line = 2f * px
        val thumb = (if (scrub != null) 6f else 4f) * px
        val y = ((size.height - line) / 2f).toInt().toFloat()
        val filled = (size.width * shown).toInt().toFloat()
        drawRect(PsytifyColors.LINE_STRONG, Offset(0f, y), Size(size.width, line))
        drawRect(PsytifyColors.TEXT_PRIMARY, Offset(0f, y), Size(filled, line))
        val thumbX = (filled - thumb / 2f).coerceIn(0f, size.width - thumb).toInt().toFloat()
        val thumbY = ((size.height - thumb) / 2f).toInt().toFloat()
        drawRect(PsytifyColors.TEXT_PRIMARY, Offset(thumbX, thumbY), Size(thumb, thumb))
    }
}
