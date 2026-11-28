package dev.psytify.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import dev.psytify.app.rust.MenuView
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.Dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.psytify.app.design.AnimatedSprite
import dev.psytify.app.design.IconButton
import dev.psytify.app.design.Pill
import dev.psytify.app.design.PrimaryButton
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SecondaryButton
import dev.psytify.app.design.SpriteImage
import dev.psytify.app.design.ap
import dev.psytify.app.design.rememberPixelCircle
import dev.psytify.app.rust.Art
import dev.psytify.app.rust.DownloadMark
import dev.psytify.app.rust.RowAction
import dev.psytify.app.rust.Header
import dev.psytify.app.rust.PageAction
import dev.psytify.app.rust.PageHandle
import dev.psytify.app.rust.PageListener
import dev.psytify.app.rust.PageStatus
import dev.psytify.app.rust.PageView
import dev.psytify.app.rust.PsytifyCore
import dev.psytify.app.rust.Route
import dev.psytify.app.rust.RowPage
import dev.psytify.app.rust.RowState
import dev.psytify.app.rust.Tap
import dev.psytify.app.rust.Tone
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import dev.psytify.app.rust.Row as PageRow

/** Rows asked of the bridge at a time past the first ones the view carries. */
private const val ROW_PAGE = 100

/** An open page of the core: its latest view as a flow. Closing it closes the page in the bridge. */
class OpenPage(core: PsytifyCore, route: Route) : PageListener {
    private val state = MutableStateFlow<PageView?>(null)
    val view: StateFlow<PageView?> = state.asStateFlow()
    val handle: PageHandle = core.openPage(route, this)

    override fun pageChanged(page: PageView) {
        state.value = page
    }

    fun close() = handle.close()
}

/** Opens the page at `route` while this is composed. */
@Composable
fun rememberPage(core: PsytifyCore, route: Route): OpenPage {
    val page = remember(core, route) { OpenPage(core, route) }
    DisposableEffect(page) { onDispose { page.close() } }
    return page
}

/**
 * A page the core built (`PageView`): the header, then the rows. Every decision is the core's: a
 * tap goes back as (page version, row) and comes back as what to open, if anything. The first rows
 * come with the view; the rest are read a page at a time while they scroll in.
 */
@Composable
fun PageScreen(
    page: OpenPage,
    onTap: (Tap) -> Unit,
    modifier: Modifier = Modifier,
    onBack: (() -> Unit)? = null,
    onMenu: (MenuView) -> Unit = {},
    top: (LazyListScope.() -> Unit)? = null,
    jamStart: (@Composable (PageRow.JamStart) -> Unit)? = null,
) {
    val view by page.view.collectAsStateWithLifecycle()
    val shown = view
    val version = shown?.version ?: 0UL
    val more = remember(version) { mutableStateMapOf<Int, RowPage>() }
    val handle = page.handle
    LazyColumn(modifier, contentPadding = PaddingValues(bottom = PsytifySpace.SPACE_6)) {
        if (onBack != null) {
            item(key = "bar") { TopBar(shown?.title.orEmpty(), onBack) }
        }
        top?.invoke(this)
        if (shown == null) return@LazyColumn
        shown.header?.let { header ->
            item(key = "header") {
                PageHeader(header) { action -> onTap(handle.act(version, action)) }
            }
        }
        when (val status = shown.status) {
            is PageStatus.Loading -> item(key = "loading") { Waiting() }
            is PageStatus.Failed -> item(key = "failed") {
                Problem(status.message) { onTap(handle.act(version, PageAction.RETRY)) }
            }
            is PageStatus.Empty -> item(key = "empty") { Empty(status.message) }
            is PageStatus.Ready -> {}
        }
        val first = shown.rows
        val total = shown.total.toInt()
        items(count = total, key = { index -> first.getOrNull(index)?.key() ?: "row:$version:$index" }) { index ->
            val row = first.getOrNull(index) ?: run {
                val chunk = index / ROW_PAGE
                LaunchedEffect(version, chunk) {
                    if (more[chunk] == null) {
                        val loaded = withContext(Dispatchers.Default) {
                            handle.rows((chunk * ROW_PAGE).toUInt(), ROW_PAGE.toUInt())
                        }
                        if (loaded.version == version) more[chunk] = loaded
                    }
                }
                more[chunk]?.let { loaded -> loaded.rows.getOrNull(index - loaded.offset.toInt()) }
            }
            PageRowView(
                row,
                onTap = { onTap(handle.tap(version, index.toUInt())) },
                onChip = { chip -> handle.chip(version, index.toUInt(), chip.toUInt()) },
                onMenu = { handle.menu(version, index.toUInt())?.let(onMenu) },
                onSwipe = { handle.enqueue(version, index.toUInt()) },
                onAction = { action -> onTap(handle.rowAction(version, index.toUInt(), action)) },
                jamStart = jamStart,
            )
        }
    }
}

private fun PageRow.key(): String = when (this) {
    is PageRow.Heading -> key
    is PageRow.Note -> key
    is PageRow.Track -> key
    is PageRow.Item -> key
    is PageRow.Chips -> key
    is PageRow.Action -> key
    is PageRow.Download -> key
    is PageRow.Saved -> key
    is PageRow.Person -> key
    is PageRow.JamStart -> key
    is PageRow.JamRoom -> key
}

@Composable
private fun TopBar(title: String, onBack: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = PsytifySpace.PHONE_TOP_BAR_H).padding(horizontal = PsytifySpace.SPACE_2),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(PsytifySprites.ICON_CHEVRON_LEFT, "Indietro", onBack)
        PsytifyText(title, PsytifyType.TYPE_META, PsytifyColors.TEXT_SECONDARY, Modifier.weight(1f))
    }
}

/** The header of an object page: picture, kind, name, the artist link, the facts and the buttons. */
@Composable
private fun PageHeader(header: Header, onAction: (PageAction) -> Unit) {
    Column(Modifier.fillMaxWidth().padding(horizontal = PsytifySpace.PHONE_GUTTER)) {
        Spacer(Modifier.height(PsytifySpace.SPACE_3))
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
            ArtView(header.art, PsytifySpace.COVER_M)
        }
        Spacer(Modifier.height(PsytifySpace.SPACE_5))
        PsytifyText(header.kicker, PsytifyType.TYPE_META, PsytifyColors.TEXT_SECONDARY)
        Spacer(Modifier.height(PsytifySpace.SPACE_1))
        PsytifyText(
            header.title,
            PsytifyType.TYPE_DISPLAY,
            PsytifyColors.TEXT_PRIMARY,
            Modifier.semantics { heading() },
            maxLines = 2,
        )
        header.subtitle?.let { subtitle ->
            val link = if (header.subtitleOpens) {
                Modifier.clickable(remember { MutableInteractionSource() }, indication = null, role = Role.Button) {
                    onAction(PageAction.OPEN_ARTIST)
                }
            } else {
                Modifier
            }
            PsytifyText(
                subtitle,
                PsytifyType.TYPE_ARTIST,
                PsytifyColors.TEXT_PRIMARY,
                link.padding(vertical = PsytifySpace.SPACE_2),
            )
        }
        if (header.meta.isNotEmpty()) {
            PsytifyText(header.meta, PsytifyType.TYPE_META, PsytifyColors.TEXT_SECONDARY)
        }
        if (header.buttons.isNotEmpty()) {
            Spacer(Modifier.height(PsytifySpace.SPACE_4))
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_3),
                verticalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_2),
            ) {
                header.buttons.forEach { button ->
                    if (button.primary) {
                        PrimaryButton(button.label, { onAction(button.action) }, enabled = button.enabled)
                    } else {
                        SecondaryButton(button.label, { onAction(button.action) }, selected = button.on, enabled = button.enabled)
                    }
                }
            }
        }
        header.note?.let { note ->
            Spacer(Modifier.height(PsytifySpace.SPACE_3))
            PsytifyText(note, PsytifyType.TYPE_META, PsytifyColors.TEXT_TERTIARY, maxLines = 4)
        }
        Spacer(Modifier.height(PsytifySpace.SPACE_4))
    }
}

/** A cover, a portrait in the pixel circle, or the drawn picture of a library entry. */
@Composable
fun ArtView(art: Art, size: Dp) {
    when (art) {
        is Art.Cover -> Cover(art.url, size)
        is Art.Round -> Cover(art.url, size, Modifier.clip(rememberPixelCircle()))
        is Art.Liked -> Drawn(size) { SpriteImage(PsytifySprites.LIKED_COVER, tint = PsytifyColors.TEXT_PRIMARY) }
        is Art.Downloads -> Drawn(size) { SpriteImage(PsytifySprites.ICON_DOWNLOADED, tint = PsytifyColors.ICON_IDLE) }
        is Art.Search -> Drawn(size) { SpriteImage(PsytifySprites.ICON_SEARCH, tint = PsytifyColors.ICON_IDLE) }
    }
}

@Composable
private fun Drawn(size: Dp, content: @Composable () -> Unit) {
    Box(Modifier.size(size).background(PsytifyColors.SURFACE_RAISED), contentAlignment = Alignment.Center) {
        content()
    }
}

@Composable
private fun PageRowView(
    row: PageRow?,
    onTap: () -> Unit,
    onChip: (Int) -> Unit,
    onMenu: () -> Unit,
    onSwipe: () -> Unit,
    onAction: (RowAction) -> Unit,
    jamStart: (@Composable (PageRow.JamStart) -> Unit)?,
) {
    when (row) {
        null -> Box(Modifier.fillMaxWidth().height(PsytifySpace.PHONE_ROW_H).semantics { contentDescription = "Caricamento" })
        is PageRow.Heading -> Row(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = PsytifySpace.PHONE_GUTTER)
                .padding(top = PsytifySpace.SPACE_6, bottom = PsytifySpace.SPACE_3),
            verticalAlignment = Alignment.Bottom,
        ) {
            PsytifyText(
                row.title,
                PsytifyType.TYPE_HEADING,
                PsytifyColors.TEXT_PRIMARY,
                Modifier.weight(1f).semantics { heading() },
            )
            if (row.detail.isNotEmpty()) {
                PsytifyText(row.detail, PsytifyType.TYPE_META, PsytifyColors.TEXT_TERTIARY)
            }
        }
        is PageRow.Note -> Row(
            Modifier.fillMaxWidth().padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_3),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (row.tone == Tone.WAITING) {
                AnimatedSprite(PsytifySprites.LOADER, running = true, tint = PsytifyColors.ICON_IDLE)
                Spacer(Modifier.width(PsytifySpace.SPACE_3))
            }
            val color = if (row.tone == Tone.ERROR) PsytifyColors.TEXT_ERROR else PsytifyColors.TEXT_SECONDARY
            PsytifyText(row.text, PsytifyType.TYPE_BODY, color, maxLines = 4)
        }
        is PageRow.Track -> SwipeToQueue(enabled = row.more, onSwipe = onSwipe) {
            TrackRow(row, onTap, onLongPress = if (row.more) onMenu else null, onQueue = if (row.more) onSwipe else null)
        }
        is PageRow.Item -> Tappable(onTap, enabled = row.state != RowState.BUSY, state = row.state) {
            ArtView(row.art, PsytifySpace.COVER_S)
            Spacer(Modifier.width(PsytifySpace.SPACE_4))
            Column(Modifier.weight(1f)) {
                PsytifyText(row.title, PsytifyType.TYPE_TRACK_TITLE, PsytifyColors.TEXT_PRIMARY)
                if (row.detail.isNotEmpty()) {
                    PsytifyText(row.detail, PsytifyType.TYPE_ARTIST, PsytifyColors.TEXT_SECONDARY)
                }
            }
            RowMark(row.state)
        }
        is PageRow.Chips -> LazyRow(
            contentPadding = PaddingValues(horizontal = PsytifySpace.PHONE_GUTTER),
            horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_3),
        ) {
            itemsIndexed(row.chips, key = { _, chip -> chip.label }) { index, chip ->
                Pill(chip.label, chip.on, { onChip(index) })
            }
        }
        is PageRow.Download -> DownloadRow(row, onAction)
        is PageRow.Saved -> SwipeToQueue(enabled = true, onSwipe = onSwipe) {
            SavedRow(row, onTap, onMenu, onSwipe, onAction)
        }
        is PageRow.Person -> PersonRow(row, onTap, onAction)
        is PageRow.JamStart -> jamStart?.invoke(row)
        is PageRow.JamRoom -> JamRoomCard(row, onAction)
        is PageRow.Action -> Box(
            Modifier
                .fillMaxWidth()
                .heightIn(min = PsytifySpace.TOUCH_TARGET)
                .clickable(remember { MutableInteractionSource() }, indication = null, role = Role.Button, onClick = onTap)
                .padding(horizontal = PsytifySpace.PHONE_GUTTER),
            contentAlignment = Alignment.CenterStart,
        ) {
            PsytifyText(row.label, PsytifyType.TYPE_BUTTON, PsytifyColors.TEXT_SECONDARY)
        }
    }
}

/** A row of a track: number or cover, title and artist, length; the state replaces the number. */
@Composable
private fun TrackRow(row: PageRow.Track, onTap: () -> Unit, onLongPress: (() -> Unit)?, onQueue: (() -> Unit)?) {
    val current = row.state == RowState.PLAYING || row.state == RowState.PAUSED
    Tappable(
        onTap,
        enabled = row.state != RowState.BUSY,
        selected = current,
        onLongPress = onLongPress,
        onQueue = onQueue,
        state = row.state,
    ) {
        val number = row.number
        val art = row.art
        if (art != null) {
            Box(contentAlignment = Alignment.Center) {
                ArtView(art, PsytifySpace.COVER_S)
            }
            Spacer(Modifier.width(PsytifySpace.SPACE_4))
        } else {
            Box(Modifier.width(PsytifySpace.SPACE_7), contentAlignment = Alignment.Center) {
                if (row.state == RowState.IDLE && number != null) {
                    PsytifyText(number.toString(), PsytifyType.TYPE_NUMERIC, PsytifyColors.TEXT_TERTIARY)
                } else {
                    RowMark(row.state)
                }
            }
            Spacer(Modifier.width(PsytifySpace.SPACE_3))
        }
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (art != null && row.state != RowState.IDLE) {
                    RowMark(row.state)
                    Spacer(Modifier.width(PsytifySpace.SPACE_2))
                }
                PsytifyText(row.title, PsytifyType.TYPE_TRACK_TITLE, PsytifyColors.TEXT_PRIMARY)
            }
            if (row.subtitle.isNotEmpty()) {
                val subtitle = if (row.state == RowState.BUSY) "Apro… · ${row.subtitle}" else row.subtitle
                PsytifyText(subtitle, PsytifyType.TYPE_ARTIST, PsytifyColors.TEXT_SECONDARY)
            }
        }
        DownloadSign(row.download)
        if (row.duration.isNotEmpty()) {
            Spacer(Modifier.width(PsytifySpace.SPACE_3))
            PsytifyText(row.duration, PsytifyType.TYPE_NUMERIC, PsytifyColors.TEXT_TERTIARY)
        }
    }
}

/**
 * Where a track is in the downloads, next to its length: on the device (never lime), on its way,
 * waiting, or a problem in the error tint. Nothing, and no room taken, for the others.
 */
@Composable
private fun DownloadSign(mark: DownloadMark) {
    val (sprite, tint, label) = when (mark) {
        DownloadMark.NONE -> return
        DownloadMark.SAVED -> Triple(PsytifySprites.ICON_DOWNLOADED, PsytifyColors.ICON_ACTIVE, "Scaricato")
        DownloadMark.DOWNLOADING -> Triple(PsytifySprites.ICON_DOWNLOAD_BUSY, PsytifyColors.ICON_ACTIVE, "In download")
        DownloadMark.QUEUED -> Triple(PsytifySprites.ICON_DOWNLOAD, PsytifyColors.ICON_DISABLED, "In coda")
        DownloadMark.LOST -> Triple(PsytifySprites.ICON_DOWNLOAD, PsytifyColors.ERROR, "File mancante")
        DownloadMark.FAILED -> Triple(PsytifySprites.ICON_DOWNLOAD, PsytifyColors.ERROR, "Download non riuscito")
    }
    Spacer(Modifier.width(PsytifySpace.SPACE_3))
    AnimatedSprite(
        sprite,
        running = mark == DownloadMark.DOWNLOADING,
        modifier = Modifier.semantics { contentDescription = label },
        tint = tint,
    )
}

/** A download of the queue: cover, title, status, the ten-step bar; Annulla, or Riprova and Togli. */
@Composable
private fun DownloadRow(row: PageRow.Download, onAction: (RowAction) -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = PsytifySpace.PHONE_ROW_H)
            .padding(start = PsytifySpace.PHONE_GUTTER, end = PsytifySpace.SPACE_2),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ArtView(row.art, PsytifySpace.COVER_S)
        Spacer(Modifier.width(PsytifySpace.SPACE_4))
        Column(Modifier.weight(1f).padding(vertical = PsytifySpace.SPACE_2)) {
            PsytifyText(row.title, PsytifyType.TYPE_TRACK_TITLE, PsytifyColors.TEXT_PRIMARY)
            val color = if (row.failed) PsytifyColors.TEXT_ERROR else PsytifyColors.TEXT_SECONDARY
            PsytifyText(row.status, PsytifyType.TYPE_ARTIST, color, maxLines = 2)
            row.steps?.let { lit ->
                Spacer(Modifier.height(PsytifySpace.SPACE_2))
                StepBar(lit.toInt())
            }
        }
        if (row.failed) {
            IconButton(PsytifySprites.ICON_REPEAT, "Riprova", { onAction(RowAction.RETRY) })
            IconButton(PsytifySprites.ICON_CLOSE, "Togli dall'elenco", { onAction(RowAction.DISMISS) })
        } else {
            IconButton(PsytifySprites.ICON_CLOSE, "Annulla", { onAction(RowAction.CANCEL) })
        }
    }
}

/** The bar of ten steps of 2 art pixels: the reached ones in white (never lime), the others iron. */
@Composable
private fun StepBar(lit: Int) {
    Row(
        Modifier.semantics { contentDescription = "${lit * 10}%" },
        horizontalArrangement = Arrangement.spacedBy(ap(1)),
    ) {
        repeat(10) { step ->
            val color = if (step < lit) PsytifyColors.WARM_WHITE else PsytifyColors.IRON
            Box(Modifier.size(ap(2)).background(color))
        }
    }
}

/** A track on the device: a tap plays the saved tracks from here; «Scarica di nuovo» and «Rimuovi». */
@Composable
private fun SavedRow(
    row: PageRow.Saved,
    onTap: () -> Unit,
    onMenu: () -> Unit,
    onQueue: () -> Unit,
    onAction: (RowAction) -> Unit,
) {
    val current = row.state == RowState.PLAYING || row.state == RowState.PAUSED
    Tappable(
        onTap,
        enabled = row.state != RowState.BUSY,
        selected = current,
        onLongPress = onMenu,
        onQueue = onQueue,
        state = row.state,
    ) {
        ArtView(row.art, PsytifySpace.COVER_S)
        Spacer(Modifier.width(PsytifySpace.SPACE_4))
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (row.state != RowState.IDLE) {
                    RowMark(row.state)
                    Spacer(Modifier.width(PsytifySpace.SPACE_2))
                }
                PsytifyText(row.title, PsytifyType.TYPE_TRACK_TITLE, PsytifyColors.TEXT_PRIMARY)
            }
            val color = if (row.lost) PsytifyColors.TEXT_ERROR else PsytifyColors.TEXT_SECONDARY
            PsytifyText(row.detail, PsytifyType.TYPE_ARTIST, color)
        }
        if (row.lost) {
            IconButton(PsytifySprites.ICON_DOWNLOAD, "Scarica di nuovo", { onAction(RowAction.DOWNLOAD_AGAIN) })
        }
        IconButton(
            PsytifySprites.ICON_TRASH,
            "Rimuovi dal dispositivo",
            { onAction(RowAction.REMOVE) },
            tint = PsytifyColors.ICON_IDLE,
        )
    }
}

/** The equaliser while it plays (lime), at rest while paused, the loader while it opens. */
@Composable
private fun RowMark(state: RowState) {
    when (state) {
        RowState.PLAYING -> AnimatedSprite(PsytifySprites.EQ, running = true, tint = PsytifyColors.STATE_ON)
        RowState.PAUSED -> SpriteImage(PsytifySprites.EQ_REST, tint = PsytifyColors.STONE)
        RowState.BUSY -> AnimatedSprite(PsytifySprites.LOADER, running = true, tint = PsytifyColors.ICON_IDLE)
        RowState.IDLE -> {}
    }
}

/** What TalkBack says of a row's state after its text. */
private fun RowState.spoken(): String? = when (this) {
    RowState.PLAYING -> "in riproduzione"
    RowState.PAUSED -> "in pausa"
    RowState.BUSY -> "apro…"
    RowState.IDLE -> null
}

/**
 * A row as tall as `PHONE_ROW_H`, taller when the font scale needs it. The long-press sheet and the
 * swipe to the queue are TalkBack actions too («Altro», «Aggiungi alla coda»).
 */
@Composable
private fun Tappable(
    onTap: () -> Unit,
    enabled: Boolean,
    selected: Boolean = false,
    onLongPress: (() -> Unit)? = null,
    onQueue: (() -> Unit)? = null,
    state: RowState = RowState.IDLE,
    content: @Composable androidx.compose.foundation.layout.RowScope.() -> Unit,
) {
    val haptics = LocalHapticFeedback.current
    val spoken = state.spoken()
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = PsytifySpace.PHONE_ROW_H)
            .background(if (selected) PsytifyColors.SURFACE_SELECTED else PsytifyColors.SURFACE_CANVAS)
            .combinedClickable(
                remember { MutableInteractionSource() },
                indication = null,
                enabled = enabled,
                role = Role.Button,
                onLongClickLabel = onLongPress?.let { "Altro" },
                onLongClick = onLongPress?.let { open ->
                    {
                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                        open()
                    }
                },
                onClick = onTap,
            )
            .semantics {
                if (spoken != null) stateDescription = spoken
                if (onQueue != null) {
                    customActions = listOf(CustomAccessibilityAction("Aggiungi alla coda") { onQueue(); true })
                }
            }
            .padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_2),
        verticalAlignment = Alignment.CenterVertically,
        content = content,
    )
}

@Composable
private fun Waiting() {
    Box(
        Modifier.fillMaxWidth().padding(PsytifySpace.SPACE_7).semantics { contentDescription = "Caricamento" },
        contentAlignment = Alignment.Center,
    ) {
        AnimatedSprite(PsytifySprites.LOADER, running = true, tint = PsytifyColors.ICON_IDLE)
    }
}

@Composable
private fun Problem(message: String, onRetry: () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_6)) {
        // Said when it appears: the page did not load.
        Column(Modifier.semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite }) {
            PsytifyText("Non si è caricata", PsytifyType.TYPE_HEADING, PsytifyColors.TEXT_PRIMARY, maxLines = 3)
            Spacer(Modifier.height(PsytifySpace.SPACE_2))
            PsytifyText(message, PsytifyType.TYPE_BODY, PsytifyColors.TEXT_SECONDARY, maxLines = 3)
        }
        Spacer(Modifier.height(PsytifySpace.SPACE_5))
        SecondaryButton("Riprova", onRetry)
    }
}

@Composable
private fun Empty(message: String) {
    Column(Modifier.fillMaxWidth().padding(PsytifySpace.SPACE_7), horizontalAlignment = Alignment.CenterHorizontally) {
        SpriteImage(PsytifySprites.EMPTY_CRATE)
        Spacer(Modifier.height(PsytifySpace.SPACE_5))
        PsytifyText(message, PsytifyType.TYPE_BODY, PsytifyColors.TEXT_SECONDARY, maxLines = 3)
    }
}
