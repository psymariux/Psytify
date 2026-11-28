package dev.psytify.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import dev.psytify.app.core.CoreHub
import dev.psytify.app.core.Notice
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SecondaryButton
import dev.psytify.app.design.ap
import dev.psytify.app.design.rememberNotch
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay

/** How long a notice stays, and how many wait behind it (older ones go). */
private const val NOTICE_MS = 3_500L
private const val WAITING = 3

/**
 * The core's notices, one at a time, as a floating strip (the hard shadow of the floating
 * elements). The timer runs only while a notice is on screen.
 */
@Composable
fun Notices(modifier: Modifier = Modifier) {
    var shown by remember { mutableStateOf<Notice?>(null) }
    val queue = remember { Channel<Notice>(WAITING, onBufferOverflow = kotlinx.coroutines.channels.BufferOverflow.DROP_OLDEST) }
    LaunchedEffect(Unit) { CoreHub.notices.collect { queue.trySend(it) } }
    LaunchedEffect(Unit) {
        for (notice in queue) {
            shown = notice
            delay(NOTICE_MS)
            shown = null
        }
    }
    val notice = shown ?: return
    val shape = rememberNotch(PsytifySpace.NOTCH_1_AP)
    Box(modifier.fillMaxWidth().padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_3)) {
        Box(
            Modifier
                .matchParentSize()
                .offset(ap(PsytifySpace.SHADOW_FLOAT_X_AP), ap(PsytifySpace.SHADOW_FLOAT_Y_AP))
                .background(PsytifyColors.SHADOW_FLOAT, shape),
        )
        Box(
            Modifier
                .fillMaxWidth()
                .background(PsytifyColors.SURFACE_RAISED, shape)
                .border(ap(PsytifySpace.BORDER_W_AP), PsytifyColors.LINE_STRONG, shape)
                .padding(horizontal = PsytifySpace.SPACE_5, vertical = PsytifySpace.SPACE_4)
                .semantics { liveRegion = if (notice.error) LiveRegionMode.Assertive else LiveRegionMode.Polite },
        ) {
            val color = if (notice.error) PsytifyColors.TEXT_ERROR else PsytifyColors.TEXT_PRIMARY
            PsytifyText(notice.text, PsytifyType.TYPE_BODY, color, maxLines = 3)
        }
    }
}

/** The core did not start again after a stop: why, and «Riprova». */
@Composable
fun CoreProblem(reason: String, onRetry: () -> Unit, modifier: Modifier = Modifier) {
    Column(
        modifier
            .fillMaxWidth()
            .background(PsytifyColors.SURFACE_RAISED)
            .padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_4),
    ) {
        Column(Modifier.semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Assertive }) {
            PsytifyText("Psytify non è ripartito", PsytifyType.TYPE_HEADING, PsytifyColors.TEXT_PRIMARY, maxLines = 3)
            Spacer(Modifier.height(PsytifySpace.SPACE_1))
            PsytifyText(reason, PsytifyType.TYPE_BODY, PsytifyColors.TEXT_ERROR, maxLines = 3)
        }
        Spacer(Modifier.height(PsytifySpace.SPACE_3))
        SecondaryButton("Riprova", onRetry)
    }
}
