package dev.psytify.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.ui.unit.dp
import android.os.SystemClock
import dev.psytify.app.core.Playback
import dev.psytify.app.design.AnimatedSprite
import dev.psytify.app.design.IconButton
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.ap
import dev.psytify.app.rust.PlaybackAction
import dev.psytify.app.rust.PlayerState
import dev.psytify.app.rust.SocialMarks
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import kotlinx.coroutines.delay

/**
 * In a jam: the group (its live sprite while the room is open) and `LIVE`; a tap opens the Jam page.
 * While connecting, the group alone.
 */
@Composable
private fun JamMarker(marks: SocialMarks, onJam: () -> Unit) {
    val label = if (marks.live) "Jam dal vivo, ${marks.members} persone" else "Jam, connessione in corso"
    Row(
        Modifier
            .height(PsytifySpace.TOUCH_TARGET)
            .clickable(remember { MutableInteractionSource() }, indication = null, role = Role.Button, onClick = onJam)
            .semantics { contentDescription = label }
            .padding(horizontal = PsytifySpace.SPACE_2),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        AnimatedSprite(
            if (marks.live) PsytifySprites.ICON_PEOPLE_LIVE else PsytifySprites.ICON_PEOPLE,
            running = marks.live,
            tint = if (marks.live) PsytifyColors.STATE_ON else PsytifyColors.ICON_IDLE,
        )
        if (marks.live) {
            Spacer(Modifier.width(PsytifySpace.SPACE_2))
            LiveBadge()
        }
    }
}

/** How often the progress line moves while a track plays and the bar is on screen. */
private const val PROGRESS_TICK_MS = 250L

/**
 * The mini player above the bottom edge: progress line, cover, title and artist, play or pause and
 * next. The equaliser in lime marks the track that is playing (the one state lime is for).
 */
@Composable
fun MiniPlayer(
    playback: Playback,
    marks: SocialMarks,
    send: (PlaybackAction) -> Unit,
    onOpen: () -> Unit,
    onJam: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val snapshot = playback.snapshot
    val track = snapshot.track ?: return
    val playing = snapshot.state == PlayerState.Playing
    var now by remember { mutableLongStateOf(SystemClock.elapsedRealtime()) }
    LaunchedEffect(playback, snapshot.running) {
        now = SystemClock.elapsedRealtime()
        while (snapshot.running) {
            delay(PROGRESS_TICK_MS)
            now = SystemClock.elapsedRealtime()
        }
    }
    val duration = track.durationMs.toLong()
    val fraction = if (duration > 0) playback.positionMs(now).toFloat() / duration else 0f

    Column(modifier.fillMaxWidth().background(PsytifyColors.SURFACE_CHROME)) {
        Box(Modifier.fillMaxWidth().height(ap(1)).background(PsytifyColors.LINE_SUBTLE)) {
            Box(Modifier.fillMaxWidth(fraction.coerceIn(0f, 1f)).fillMaxHeight().background(PsytifyColors.TEXT_PRIMARY))
        }
        Row(
            Modifier
                .fillMaxWidth()
                .heightIn(min = PsytifySpace.PHONE_MINI_PLAYER_H)
                .padding(start = PsytifySpace.SPACE_3),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // Cover, title and artist are one target; as a live region, TalkBack says the new track
            // (or the error that replaced the artist) when it changes.
            Row(
                Modifier
                    .weight(1f)
                    .heightIn(min = PsytifySpace.PHONE_MINI_PLAYER_H)
                    .clickable(remember { MutableInteractionSource() }, indication = null, onClickLabel = "Apri il player", onClick = onOpen)
                    .semantics { liveRegion = LiveRegionMode.Polite },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Cover(track.coverUrl, 40.dp)
                Spacer(Modifier.width(PsytifySpace.SPACE_4))
                Column(Modifier.weight(1f).padding(vertical = PsytifySpace.SPACE_2)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        if (playing) {
                            AnimatedSprite(PsytifySprites.EQ, running = true, tint = PsytifyColors.STATE_ON)
                            Spacer(Modifier.width(PsytifySpace.SPACE_2))
                        }
                        PsytifyText(track.title, PsytifyType.TYPE_TRACK_TITLE, PsytifyColors.TEXT_PRIMARY)
                    }
                    val state = snapshot.state
                    if (state is PlayerState.Failed) {
                        PsytifyText(state.message, PsytifyType.TYPE_META, PsytifyColors.TEXT_ERROR)
                    } else {
                        PsytifyText(track.artist, PsytifyType.TYPE_ARTIST, PsytifyColors.TEXT_SECONDARY)
                    }
                }
            }
            if (marks.jam) JamMarker(marks, onJam)
            if (snapshot.state == PlayerState.Loading) {
                Box(
                    Modifier.size(PsytifySpace.TOUCH_TARGET).semantics { contentDescription = "Caricamento" },
                    contentAlignment = Alignment.Center,
                ) {
                    AnimatedSprite(PsytifySprites.LOADER, running = true)
                }
            } else {
                IconButton(
                    sprite = if (playing) PsytifySprites.ICON_PAUSE else PsytifySprites.ICON_PLAY,
                    description = if (playing) "Pausa" else "Riproduci",
                    onClick = { send(PlaybackAction.Toggle) },
                )
            }
            IconButton(
                sprite = PsytifySprites.ICON_NEXT,
                description = "Successivo",
                onClick = { send(PlaybackAction.Next) },
                tint = if (snapshot.hasNext) PsytifyColors.ICON_ACTIVE else PsytifyColors.ICON_DISABLED,
            )
        }
    }
}

