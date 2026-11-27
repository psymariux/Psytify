package dev.psytify.app.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SpriteImage

/** A tab whose screen comes in a later stage of the migration: its title and Nastro waiting. */
@Composable
fun Placeholder(title: String, modifier: Modifier = Modifier, contentPadding: PaddingValues = PaddingValues()) {
    Column(modifier.fillMaxSize().padding(contentPadding)) {
        PsytifyText(
            title,
            PsytifyType.TYPE_PAGE_TITLE,
            PsytifyColors.TEXT_PRIMARY,
            Modifier
                .padding(horizontal = PsytifySpace.PHONE_GUTTER)
                .padding(top = PsytifySpace.PHONE_TOP_BAR_H, bottom = PsytifySpace.SPACE_3)
                .semantics { heading() },
        )
        Column(
            Modifier.weight(1f).padding(PsytifySpace.SPACE_7),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            SpriteImage(PsytifySprites.NASTRO_IDLE)
            Spacer(Modifier.height(PsytifySpace.SPACE_5))
            PsytifyText("Questa schermata arriva presto.", PsytifyType.TYPE_BODY, PsytifyColors.TEXT_SECONDARY, maxLines = 3)
        }
    }
}
