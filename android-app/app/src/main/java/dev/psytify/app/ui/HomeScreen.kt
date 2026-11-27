package dev.psytify.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SecondaryButton
import dev.psytify.app.design.SpriteImage
import dev.psytify.app.rust.HomeSnapshot
import dev.psytify.app.rust.HomeStatus
import dev.psytify.app.rust.Shelf
import dev.psytify.app.rust.Tile

/** Skeleton shelves while the catalogue part of the Home has not arrived. */
private const val SKELETON_SHELVES = 2
private const val SKELETON_TILES = 3

/**
 * The phone Home: the shelves the core built (`HomeSnapshot`), each a row of covers. A tap goes back
 * to the core as (Home version, shelf, tile): what it plays is the core's decision.
 */
@Composable
fun HomeScreen(
    home: HomeSnapshot?,
    onTile: (version: ULong, shelf: Int, tile: Int) -> Unit,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
    contentPadding: PaddingValues = PaddingValues(),
) {
    LazyColumn(modifier, contentPadding = contentPadding) {
        item(key = "header") { Header() }
        val status = home?.status ?: HomeStatus.Loading
        when (status) {
            is HomeStatus.Failed -> item(key = "error") { Problem(status.message, onRetry) }
            is HomeStatus.Empty -> item(key = "empty") { Empty(status.message) }
            else -> {}
        }
        val version = home?.version ?: 0UL
        home?.shelves.orEmpty().forEachIndexed { index, shelf ->
            item(key = "shelf:${shelf.kind}") {
                ShelfRow(shelf) { tile -> onTile(version, index, tile) }
            }
        }
        if (status == HomeStatus.Loading) {
            items(SKELETON_SHELVES) { SkeletonShelf() }
        }
        item(key = "end") { Spacer(Modifier.height(PsytifySpace.SPACE_6)) }
    }
}

@Composable
private fun Header() {
    Row(
        Modifier.fillMaxWidth().heightIn(min = PsytifySpace.PHONE_TOP_BAR_H).padding(horizontal = PsytifySpace.PHONE_GUTTER),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        SpriteImage(PsytifySprites.MARK, tint = PsytifyColors.TEXT_PRIMARY)
        Spacer(Modifier.width(PsytifySpace.SPACE_3))
        PsytifyText("Psytify", PsytifyType.TYPE_WORDMARK, PsytifyColors.TEXT_PRIMARY)
    }
    PsytifyText(
        "Home",
        PsytifyType.TYPE_PAGE_TITLE,
        PsytifyColors.TEXT_PRIMARY,
        Modifier
            .padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_3)
            .semantics { heading() },
    )
}

@Composable
private fun SectionTitle(title: String) {
    PsytifyText(
        title,
        PsytifyType.TYPE_HEADING,
        PsytifyColors.TEXT_PRIMARY,
        Modifier
            .padding(start = PsytifySpace.PHONE_GUTTER, end = PsytifySpace.PHONE_GUTTER, top = PsytifySpace.SPACE_6)
            .padding(bottom = PsytifySpace.SPACE_3)
            .semantics { heading() },
    )
}

@Composable
private fun ShelfRow(shelf: Shelf, onTile: (Int) -> Unit) {
    Column {
        SectionTitle(shelf.title)
        LazyRow(
            contentPadding = PaddingValues(horizontal = PsytifySpace.PHONE_GUTTER),
            horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_4),
        ) {
            itemsIndexed(shelf.tiles, key = { _, tile -> tile.key }) { index, tile ->
                TileView(tile) { onTile(index) }
            }
        }
    }
}

@Composable
private fun TileView(tile: Tile, onClick: () -> Unit) {
    Column(
        Modifier
            .width(PsytifySpace.PHONE_SHELF_COVER)
            .clickable(remember { MutableInteractionSource() }, indication = null, role = Role.Button, onClick = onClick),
    ) {
        Cover(tile.coverUrl, PsytifySpace.PHONE_SHELF_COVER)
        Spacer(Modifier.height(PsytifySpace.SPACE_3))
        PsytifyText(tile.title, PsytifyType.TYPE_TRACK_TITLE, PsytifyColors.TEXT_PRIMARY)
        PsytifyText(tile.subtitle, PsytifyType.TYPE_ARTIST, PsytifyColors.TEXT_SECONDARY)
    }
}

@Composable
private fun SkeletonShelf() {
    Column(Modifier.semantics(mergeDescendants = true) { contentDescription = "Caricamento" }) {
        Box(
            Modifier
                .padding(start = PsytifySpace.PHONE_GUTTER, top = PsytifySpace.SPACE_6, bottom = PsytifySpace.SPACE_3)
                .size(width = PsytifySpace.COVER_M, height = PsytifySpace.SPACE_5)
                .background(PsytifyColors.SURFACE_RAISED),
        )
        Row(
            Modifier.padding(horizontal = PsytifySpace.PHONE_GUTTER),
            horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_4),
        ) {
            repeat(SKELETON_TILES) {
                Box(Modifier.size(PsytifySpace.PHONE_SHELF_COVER).background(PsytifyColors.SURFACE_RAISED))
            }
        }
    }
}

@Composable
private fun Problem(message: String, onRetry: () -> Unit) {
    Column(
        Modifier.fillMaxWidth().padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_6),
    ) {
        Column(Modifier.semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite }) {
            PsytifyText("La Home non si è caricata", PsytifyType.TYPE_HEADING, PsytifyColors.TEXT_PRIMARY, maxLines = 3)
            Spacer(Modifier.height(PsytifySpace.SPACE_2))
            PsytifyText(message, PsytifyType.TYPE_BODY, PsytifyColors.TEXT_SECONDARY, maxLines = 3)
        }
        Spacer(Modifier.height(PsytifySpace.SPACE_5))
        SecondaryButton("Riprova", onRetry)
    }
}

@Composable
private fun Empty(message: String) {
    Column(
        Modifier.fillMaxWidth().padding(PsytifySpace.SPACE_7),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        SpriteImage(PsytifySprites.EMPTY_CRATE)
        Spacer(Modifier.height(PsytifySpace.SPACE_5))
        PsytifyText(message, PsytifyType.TYPE_BODY, PsytifyColors.TEXT_SECONDARY, maxLines = 3)
    }
}
