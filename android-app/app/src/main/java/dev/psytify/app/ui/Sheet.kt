package dev.psytify.app.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
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
import androidx.compose.ui.layout.layout
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import dev.psytify.app.design.LocalArtPx
import dev.psytify.app.design.LocalReduceMotion
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.Sprite
import dev.psytify.app.design.SpriteImage
import dev.psytify.app.design.ap
import dev.psytify.app.design.rememberTopNotch
import dev.psytify.app.rust.MenuChoice
import dev.psytify.app.rust.MenuItem
import dev.psytify.app.rust.MenuView
import kotlin.math.roundToInt

/**
 * A bottom sheet over everything: the scrim behind (a tap on it closes), the raised surface with
 * the strong top line and the notched top corners, the handle. It slides up in `MOTION_PANEL`,
 * in whole art pixels.
 */
@Composable
fun BottomSheet(title: String, onDismiss: () -> Unit, content: @Composable ColumnScope.() -> Unit) {
    BackHandler(onBack = onDismiss)
    val still = LocalReduceMotion.current
    val rise = remember { Animatable(if (still) 0f else 1f) }
    LaunchedEffect(Unit) {
        if (!still) rise.animateTo(0f, tween(PsytifySpace.MOTION_PANEL_MS, easing = LinearOutSlowInEasing))
    }
    val px = LocalArtPx.current
    val shape = rememberTopNotch(PsytifySpace.NOTCH_2_AP)
    // The scrim closes on a tap, but is no TalkBack target (one would swallow the sheet's text):
    // back closes the sheet there.
    Box(
        Modifier
            .fillMaxSize()
            .background(PsytifyColors.SCRIM)
            .pointerInput(onDismiss) { detectTapGestures { onDismiss() } },
        contentAlignment = Alignment.BottomCenter,
    ) {
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            val most = maxHeight * 0.8f
            Column(
                Modifier
                    .fillMaxWidth()
                    .heightIn(max = most)
                    .layout { measurable, constraints ->
                        val placeable = measurable.measure(constraints)
                        val shift = ((rise.value * placeable.height) / px).roundToInt() * px
                        layout(placeable.width, placeable.height) { placeable.place(0, shift) }
                    }
                    .background(PsytifyColors.SURFACE_RAISED, shape)
                    .border(ap(PsytifySpace.BORDER_W_AP), PsytifyColors.LINE_STRONG, shape)
                    // A tap inside the sheet does not reach the scrim.
                    .pointerInput(Unit) { detectTapGestures {} }
                    .semantics { paneTitle = title }
                    .navigationBarsPadding()
                    .imePadding(),
            ) {
                Box(Modifier.fillMaxWidth().padding(vertical = PsytifySpace.SPACE_3), contentAlignment = Alignment.Center) {
                    Box(
                        Modifier
                            .size(ap(PsytifySpace.SHEET_HANDLE_W_AP), ap(PsytifySpace.SHEET_HANDLE_H_AP))
                            .background(PsytifyColors.STONE),
                    )
                }
                content()
            }
        }
    }
}

/** A row of a sheet: 56 dp, the icon, the label; pressed, the selected surface. */
@Composable
fun SheetRow(
    label: String,
    sprite: Sprite?,
    onClick: () -> Unit,
    enabled: Boolean = true,
    destructive: Boolean = false,
    checked: Boolean = false,
) {
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = PsytifySpace.SHEET_ITEM_H)
            .background(if (pressed && enabled) PsytifyColors.SURFACE_SELECTED else PsytifyColors.SURFACE_RAISED)
            .clickable(interaction, indication = null, enabled = enabled, role = Role.Button, onClick = onClick)
            .semantics { if (checked) stateDescription = "selezionato" }
            .padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_2),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val tint = when {
            !enabled -> PsytifyColors.ICON_DISABLED
            pressed -> PsytifyColors.ICON_ACTIVE
            else -> PsytifyColors.ICON_IDLE
        }
        Box(Modifier.width(PsytifySpace.ICON), contentAlignment = Alignment.Center) {
            if (sprite != null) SpriteImage(sprite, tint = tint)
        }
        Spacer(Modifier.width(PsytifySpace.SPACE_4))
        val color = when {
            !enabled -> PsytifyColors.TEXT_DISABLED
            destructive -> PsytifyColors.TEXT_ERROR
            else -> PsytifyColors.TEXT_PRIMARY
        }
        PsytifyText(label, PsytifyType.TYPE_BODY, color, Modifier.weight(1f))
        if (checked) SpriteImage(PsytifySprites.ICON_CHECK, tint = PsytifyColors.ICON_ACTIVE)
    }
}

private fun MenuItem.sprite(): Sprite = when (choice) {
    is MenuChoice.PlayNext -> PsytifySprites.ICON_NEXT
    is MenuChoice.Enqueue -> PsytifySprites.ICON_QUEUE
    is MenuChoice.Radio -> PsytifySprites.ICON_RADIO
    is MenuChoice.ShowPlaylists -> PsytifySprites.ICON_LIST
    is MenuChoice.NewPlaylist -> PsytifySprites.ICON_PLUS
    is MenuChoice.AddTo -> PsytifySprites.ICON_LIST
    is MenuChoice.RemoveFromPlaylist -> PsytifySprites.ICON_TRASH
    is MenuChoice.GoToArtist -> PsytifySprites.ICON_ACCOUNT
    is MenuChoice.GoToAlbum -> PsytifySprites.ICON_LIBRARY
    is MenuChoice.ToggleFollow -> if (on) PsytifySprites.ICON_CHECK else PsytifySprites.ICON_USER_PLUS
    is MenuChoice.Copy -> PsytifySprites.ICON_COPY
    is MenuChoice.Share -> PsytifySprites.ICON_PEOPLE
    is MenuChoice.Download -> PsytifySprites.ICON_DOWNLOAD
    is MenuChoice.ShowQualities -> PsytifySprites.ICON_DOWNLOAD
    is MenuChoice.DownloadAs -> PsytifySprites.ICON_DOWNLOAD
    is MenuChoice.RemoveDownload -> PsytifySprites.ICON_TRASH
}

/**
 * The «Altro» sheet of a track, as the bridge decided it: the track on top, then the items. «Aggiungi
 * a una playlist» shows the playlists in place of the items (with «Nuova playlist…» first).
 */
@Composable
fun TrackSheet(menu: MenuView, onChoose: (MenuChoice) -> Unit, onDismiss: () -> Unit) {
    var playlists by remember(menu.id) { mutableStateOf(false) }
    var qualities by remember(menu.id) { mutableStateOf(false) }
    BottomSheet("Altro: ${menu.title}", onDismiss) {
        Row(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_3)
                .semantics(mergeDescendants = true) { heading() },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ArtView(menu.art, PsytifySpace.COVER_S)
            Spacer(Modifier.width(PsytifySpace.SPACE_4))
            Column(Modifier.weight(1f)) {
                PsytifyText(menu.title, PsytifyType.TYPE_TRACK_TITLE, PsytifyColors.TEXT_PRIMARY)
                PsytifyText(menu.subtitle, PsytifyType.TYPE_ARTIST, PsytifyColors.TEXT_SECONDARY)
            }
        }
        Box(Modifier.fillMaxWidth().height(ap(PsytifySpace.BORDER_W_AP)).background(PsytifyColors.LINE_SUBTLE))
        Column(Modifier.verticalScroll(rememberScrollState()).padding(vertical = PsytifySpace.SPACE_2)) {
            if (playlists) {
                BackHandler { playlists = false }
                SheetRow("Indietro", PsytifySprites.ICON_CHEVRON_LEFT, { playlists = false })
                SheetRow("Nuova playlist…", PsytifySprites.ICON_PLUS, { onChoose(MenuChoice.NewPlaylist) })
                menu.playlists.forEach { playlist ->
                    SheetRow(
                        playlist.name,
                        PsytifySprites.ICON_LIST,
                        { onChoose(MenuChoice.AddTo(playlist.id)) },
                        checked = playlist.has,
                    )
                }
            } else if (qualities) {
                BackHandler { qualities = false }
                SheetRow("Indietro", PsytifySprites.ICON_CHEVRON_LEFT, { qualities = false })
                menu.qualities.forEach { quality ->
                    SheetRow(
                        quality.label,
                        PsytifySprites.ICON_DOWNLOAD,
                        { onChoose(MenuChoice.DownloadAs(quality.quality)) },
                        checked = quality.on,
                    )
                }
            } else {
                menu.items.forEach { item ->
                    SheetRow(
                        item.label,
                        item.sprite(),
                        onClick = {
                            when (item.choice) {
                                is MenuChoice.ShowPlaylists -> playlists = true
                                is MenuChoice.ShowQualities -> qualities = true
                                else -> onChoose(item.choice)
                            }
                        },
                        enabled = item.enabled,
                    )
                }
            }
        }
    }
}
