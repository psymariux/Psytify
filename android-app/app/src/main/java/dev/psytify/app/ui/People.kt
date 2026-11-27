package dev.psytify.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import dev.psytify.app.design.AnimatedSprite
import dev.psytify.app.design.GhostButton
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SecondaryButton
import dev.psytify.app.design.SpriteImage
import dev.psytify.app.design.ap
import dev.psytify.app.design.rememberNotch
import dev.psytify.app.design.rememberPixelCircle
import dev.psytify.app.rust.ButtonKind
import dev.psytify.app.rust.RowAction
import dev.psytify.app.rust.Row as PageRow

/**
 * A person's row (48 dp, the desktop's `PersonRow`): the avatar in the pixel circle, the name, the
 * status with the lime live dot while they listen, and the row's buttons on the right.
 */
@Composable
fun PersonRow(row: PageRow.Person, onTap: () -> Unit, onAction: (RowAction) -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = PsytifySpace.PHONE_ROW_H)
            .clickable(remember { MutableInteractionSource() }, indication = null, role = Role.Button, onClick = onTap)
            .padding(start = PsytifySpace.PHONE_GUTTER, end = PsytifySpace.SPACE_2),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Avatar(row.avatarUrl, row.name)
        Spacer(Modifier.width(PsytifySpace.SPACE_4))
        Column(Modifier.weight(1f)) {
            PsytifyText(row.name, PsytifyType.TYPE_TRACK_TITLE, PsytifyColors.TEXT_PRIMARY)
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (row.live) {
                    Box(Modifier.size(ap(3)).background(PsytifyColors.STATE_ON))
                    Spacer(Modifier.width(PsytifySpace.SPACE_2))
                }
                PsytifyText(row.status, PsytifyType.TYPE_ARTIST, PsytifyColors.TEXT_SECONDARY)
            }
        }
        row.buttons.forEach { button ->
            Spacer(Modifier.width(PsytifySpace.SPACE_2))
            when (button.kind) {
                ButtonKind.SECONDARY -> SecondaryButton(
                    button.label,
                    { onAction(button.action) },
                    selected = button.on,
                    enabled = button.enabled,
                )
                ButtonKind.GHOST -> GhostButton(button.label, { onAction(button.action) }, enabled = button.enabled)
            }
        }
    }
}

/** A portrait in the pixel circle, or the initial on the raised surface without one. */
@Composable
fun Avatar(url: String?, name: String, size: androidx.compose.ui.unit.Dp = PsytifySpace.AVATAR_S) {
    if (url != null) {
        Cover(url, size, Modifier.clip(rememberPixelCircle()))
    } else {
        Box(
            Modifier.size(size).clip(rememberPixelCircle()).background(PsytifyColors.SURFACE_SELECTED),
            contentAlignment = Alignment.Center,
        ) {
            PsytifyText(name.take(1).uppercase(), PsytifyType.TYPE_BUTTON, PsytifyColors.TEXT_PRIMARY)
        }
    }
}

/**
 * The room card of the Jam page: the code in the pixel type with «Copia» and `LIVE` once the room
 * is open (the loader and «Connessione in corso…» before), who is there as avatars in a row, and
 * «Esci dalla jam».
 */
@Composable
fun JamRoomCard(row: PageRow.JamRoom, onAction: (RowAction) -> Unit) {
    val shape = rememberNotch(PsytifySpace.NOTCH_1_AP)
    Column(
        Modifier
            .padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_3)
            .fillMaxWidth()
            .background(PsytifyColors.SURFACE_RAISED, shape)
            .border(ap(PsytifySpace.BORDER_W_AP), PsytifyColors.LINE_STRONG, shape)
            .padding(PsytifySpace.SPACE_5),
    ) {
        val code = row.code
        if (code == null) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                AnimatedSprite(PsytifySprites.LOADER, running = true, tint = PsytifyColors.ICON_IDLE)
                Spacer(Modifier.width(PsytifySpace.SPACE_3))
                PsytifyText("Connessione in corso…", PsytifyType.TYPE_BODY, PsytifyColors.TEXT_SECONDARY)
            }
        } else {
            Row(verticalAlignment = Alignment.CenterVertically) {
                PsytifyText("Codice", PsytifyType.TYPE_META, PsytifyColors.TEXT_SECONDARY, Modifier.weight(1f))
                LiveBadge()
            }
            Spacer(Modifier.height(PsytifySpace.SPACE_2))
            Row(verticalAlignment = Alignment.CenterVertically) {
                PsytifyText(
                    code,
                    PsytifyType.TYPE_NUMERIC_PIXEL,
                    PsytifyColors.TEXT_PRIMARY,
                    Modifier.weight(1f).semantics { contentDescription = "Codice ${code.toList().joinToString(" ")}" },
                )
                SecondaryButton("Copia", { onAction(RowAction.COPY) }, leading = {
                    SpriteImage(PsytifySprites.ICON_COPY, tint = PsytifyColors.ICON_ACTIVE)
                })
            }
            if (row.members.isNotEmpty()) {
                Spacer(Modifier.height(PsytifySpace.SPACE_4))
                FlowRow(
                    horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_2),
                    verticalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_2),
                ) {
                    row.members.forEach { name -> Avatar(null, name) }
                }
            }
        }
        Spacer(Modifier.height(PsytifySpace.SPACE_4))
        SecondaryButton("Esci dalla jam", { onAction(RowAction.LEAVE) })
    }
}

/** `LIVE` in the pixel type on lime: the room is open (lime marks a live state). */
@Composable
fun LiveBadge(modifier: Modifier = Modifier) {
    Box(
        modifier.background(PsytifyColors.STATE_ON).padding(horizontal = ap(2), vertical = ap(1)),
        contentAlignment = Alignment.Center,
    ) {
        PsytifyText("LIVE", PsytifyType.TYPE_LABEL, PsytifyColors.TEXT_ON_LIGHT)
    }
}
