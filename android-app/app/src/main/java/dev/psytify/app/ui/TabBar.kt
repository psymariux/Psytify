package dev.psytify.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.Sprite
import dev.psytify.app.design.SpriteImage
import dev.psytify.app.design.ap

/** The five tabs of the phone, in the order of the Iced phone layout (`client/src/ui/phone/mod.rs`). */
enum class Tab(val label: String, val idle: Sprite, val active: Sprite) {
    HOME("Home", PsytifySprites.ICON_HOME, PsytifySprites.ICON_HOME_FILL),
    SEARCH("Cerca", PsytifySprites.ICON_SEARCH, PsytifySprites.ICON_SEARCH_FILL),
    LIBRARY("Libreria", PsytifySprites.ICON_LIBRARY, PsytifySprites.ICON_LIBRARY_FILL),
    PROFILE("Profilo", PsytifySprites.ICON_ACCOUNT, PsytifySprites.ICON_ACCOUNT),
    NEWS("Novità", PsytifySprites.ICON_BELL, PsytifySprites.ICON_BELL_FILL),
}

/**
 * The tab bar (`docs/design/PSYTIFY_COMPONENT_GUIDELINES.md`, «Phone tab»): the chosen tab has the
 * filled icon, the primary text and the 2 AP lime marker, 12 AP wide, centred on its top edge; the
 * others the secondary text. It grows with the font scale instead of clipping its labels.
 */
@Composable
fun TabBar(current: Tab, onSelect: (Tab) -> Unit, modifier: Modifier = Modifier) {
    Row(
        modifier
            .fillMaxWidth()
            .heightIn(min = PsytifySpace.PHONE_TAB_BAR_H)
            .background(PsytifyColors.SURFACE_CHROME)
            .selectableGroup(),
        horizontalArrangement = Arrangement.SpaceEvenly,
    ) {
        Tab.entries.forEach { tab ->
            val on = tab == current
            val tint = if (on) PsytifyColors.TEXT_PRIMARY else PsytifyColors.TEXT_SECONDARY
            Column(
                Modifier
                    .weight(1f)
                    .heightIn(min = PsytifySpace.PHONE_TAB_BAR_H)
                    .selectable(on, remember { MutableInteractionSource() }, indication = null, role = Role.Tab) {
                        onSelect(tab)
                    },
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Box(
                    Modifier
                        .width(ap(12))
                        .height(ap(2))
                        .background(if (on) PsytifyColors.STATE_ON else PsytifyColors.SURFACE_CHROME),
                )
                Column(
                    Modifier.heightIn(min = PsytifySpace.PHONE_TAB_BAR_H - ap(2)).padding(vertical = PsytifySpace.SPACE_1),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    SpriteImage(if (on) tab.active else tab.idle, tint = tint)
                    PsytifyText(tab.label, PsytifyType.TYPE_NAV, tint)
                }
            }
        }
    }
}
