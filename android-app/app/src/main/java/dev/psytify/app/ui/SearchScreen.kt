package dev.psytify.app.ui

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import dev.psytify.app.design.IconButton
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SpriteImage
import dev.psytify.app.design.ap
import dev.psytify.app.design.rememberNotch
import dev.psytify.app.design.style
import dev.psytify.app.rust.MenuView
import dev.psytify.app.rust.Tap

/**
 * Cerca: the field, then the results page the core builds. Every change of the text goes to the
 * core, which waits 300 ms after the last one before searching (the last search wins).
 */
@Composable
fun SearchScreen(
    page: OpenPage,
    query: String,
    onQuery: (String) -> Unit,
    onTap: (Tap) -> Unit,
    onMenu: (MenuView) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier) {
        PageTitle("Cerca")
        SearchField(query, onQuery)
        PageScreen(page, onTap, Modifier.weight(1f), onMenu = onMenu)
    }
}

/** The title of a tab's first screen, in the pixel type, with an optional button on the right. */
@Composable
fun PageTitle(title: String, action: (@Composable () -> Unit)? = null) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(start = PsytifySpace.PHONE_GUTTER, end = PsytifySpace.SPACE_2)
            .padding(top = PsytifySpace.PHONE_TOP_BAR_H, bottom = PsytifySpace.SPACE_4),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        PsytifyText(
            title,
            PsytifyType.TYPE_PAGE_TITLE,
            PsytifyColors.TEXT_PRIMARY,
            Modifier.weight(1f).semantics { heading() },
        )
        action?.invoke()
    }
}

/** The search field: icon, text, clear. `onSubmit` runs on the keyboard's search key. */
@Composable
fun SearchField(
    query: String,
    onQuery: (String) -> Unit,
    placeholder: String = "Artisti, brani, album",
    onSubmit: (() -> Unit)? = null,
) {
    val keyboard = LocalSoftwareKeyboardController.current
    val shape = rememberNotch(PsytifySpace.NOTCH_1_AP)
    Row(
        Modifier
            .padding(horizontal = PsytifySpace.PHONE_GUTTER)
            .fillMaxWidth()
            .heightIn(min = PsytifySpace.TOUCH_TARGET)
            .border(ap(PsytifySpace.BORDER_W_AP), PsytifyColors.CONTROL_BORDER, shape),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Spacer(Modifier.width(PsytifySpace.SPACE_4))
        SpriteImage(PsytifySprites.ICON_SEARCH, tint = PsytifyColors.ICON_IDLE)
        Spacer(Modifier.width(PsytifySpace.SPACE_3))
        Box(Modifier.weight(1f), contentAlignment = Alignment.CenterStart) {
            if (query.isEmpty()) {
                PsytifyText(placeholder, PsytifyType.TYPE_BODY, PsytifyColors.TEXT_TERTIARY, Modifier.clearAndSetSemantics {})
            }
            BasicTextField(
                value = query,
                onValueChange = onQuery,
                singleLine = true,
                textStyle = PsytifyType.TYPE_BODY.style(PsytifyColors.TEXT_PRIMARY),
                cursorBrush = SolidColor(PsytifyColors.CARET),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                keyboardActions = KeyboardActions(onSearch = {
                    keyboard?.hide()
                    onSubmit?.invoke()
                }),
                modifier = Modifier.fillMaxWidth().semantics { contentDescription = placeholder },
            )
        }
        if (query.isNotEmpty()) {
            IconButton(PsytifySprites.ICON_CLOSE, "Cancella la ricerca", { onQuery("") }, tint = PsytifyColors.ICON_IDLE)
        } else {
            Spacer(Modifier.width(PsytifySpace.SPACE_4))
        }
    }
    Spacer(Modifier.height(PsytifySpace.SPACE_3))
}
