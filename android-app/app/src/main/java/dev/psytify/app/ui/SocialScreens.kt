package dev.psytify.app.ui

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import dev.psytify.app.design.PrimaryButton
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SecondaryButton
import dev.psytify.app.design.ap
import dev.psytify.app.design.rememberNotch
import dev.psytify.app.design.style
import dev.psytify.app.rust.MenuView
import dev.psytify.app.rust.PsytifyCore
import dev.psytify.app.rust.Tap
import dev.psytify.app.rust.jamCodeChars
import dev.psytify.app.rust.jamCodeValid
import dev.psytify.app.rust.jamNameChars
import dev.psytify.app.rust.Row as PageRow

/**
 * «Amici»: the field searches people by username when submitted (an empty field clears the
 * results), then the page the core builds: people found, requests, waiting, friends.
 */
@Composable
fun FriendsScreen(
    core: PsytifyCore,
    page: OpenPage,
    onTap: (Tap) -> Unit,
    onBack: () -> Unit,
    onMenu: (MenuView) -> Unit,
    modifier: Modifier = Modifier,
) {
    var query by rememberSaveable { mutableStateOf("") }
    PageScreen(
        page,
        onTap,
        modifier,
        onBack = onBack,
        onMenu = onMenu,
        top = {
            item(key = "field") {
                SearchField(
                    query,
                    onQuery = { text ->
                        query = text.filter { !it.isISOControl() }
                        if (query.isBlank()) core.searchUsers("")
                    },
                    placeholder = "Cerca per nome utente",
                    onSubmit = { core.searchUsers(query) },
                )
            }
        },
    )
}

/** «Jam»: the page the core builds, with the card «Ascolta insieme» drawn here (its two fields). */
@Composable
fun JamScreen(
    core: PsytifyCore,
    page: OpenPage,
    onTap: (Tap) -> Unit,
    onBack: () -> Unit,
    onMenu: (MenuView) -> Unit,
    modifier: Modifier = Modifier,
) {
    PageScreen(page, onTap, modifier, onBack = onBack, onMenu = onMenu, jamStart = { row -> JamStartCard(core, row) })
}

/**
 * «Ascolta insieme»: the name to show in the room (empty: the profile's), «Crea una jam», the code
 * (six letters or digits, upper case while typed) and «Entra», lit only with a whole code; under
 * them, the error that closed the last jam and «Riprova».
 */
@Composable
private fun JamStartCard(core: PsytifyCore, row: PageRow.JamStart) {
    var name by rememberSaveable { mutableStateOf("") }
    var code by rememberSaveable { mutableStateOf("") }
    fun typed() {
        if (row.failure != null) core.jamTyped()
    }
    Column(
        Modifier.padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_4),
        verticalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_3),
    ) {
        PsytifyText("Ascolta insieme", PsytifyType.TYPE_HEADING, PsytifyColors.TEXT_PRIMARY, maxLines = 3)
        PsytifyText(
            "Crea una jam e dai il codice agli amici: chi entra sente la stessa coda, allo stesso punto.",
            PsytifyType.TYPE_BODY,
            PsytifyColors.TEXT_SECONDARY,
            maxLines = 3,
        )
        FormLine(name, { name = jamNameChars(it); typed() }, "Il tuo nome (facoltativo)", ImeAction.Done)
        PrimaryButton("Crea una jam", { core.jamCreate(name) }, Modifier.fillMaxWidth(), fill = true)
        Spacer(Modifier.height(PsytifySpace.SPACE_2))
        FormLine(
            code,
            { code = jamCodeChars(it); typed() },
            "Codice di 6 caratteri",
            ImeAction.Go,
            capitals = true,
            onSubmit = { if (jamCodeValid(code)) core.jamJoin(code, name) },
        )
        SecondaryButton("Entra", { core.jamJoin(code, name) }, Modifier.fillMaxWidth(), enabled = jamCodeValid(code), fill = true)
        row.failure?.let { failure ->
            PsytifyText(failure, PsytifyType.TYPE_BODY, PsytifyColors.TEXT_ERROR, maxLines = 3)
            if (row.retry) SecondaryButton("Riprova", { core.jamRetry(name, code) })
        }
    }
}

/** A one-line field of a form: 48 dp, the control border, the placeholder while empty. */
@Composable
fun FormLine(
    value: String,
    onValue: (String) -> Unit,
    placeholder: String,
    ime: ImeAction = ImeAction.Done,
    capitals: Boolean = false,
    onSubmit: (() -> Unit)? = null,
) {
    Box(
        Modifier
            .fillMaxWidth()
            .heightIn(min = PsytifySpace.TOUCH_TARGET)
            .border(ap(PsytifySpace.BORDER_W_AP), PsytifyColors.CONTROL_BORDER, rememberNotch(PsytifySpace.NOTCH_1_AP))
            .padding(horizontal = PsytifySpace.SPACE_4),
        contentAlignment = Alignment.CenterStart,
    ) {
        // The field itself carries the name for TalkBack; the drawn placeholder is for the eyes.
        if (value.isEmpty()) PsytifyText(placeholder, PsytifyType.TYPE_BODY, PsytifyColors.TEXT_TERTIARY, Modifier.clearAndSetSemantics {})
        BasicTextField(
            value = value,
            onValueChange = onValue,
            singleLine = true,
            textStyle = PsytifyType.TYPE_BODY.style(PsytifyColors.TEXT_PRIMARY),
            cursorBrush = SolidColor(PsytifyColors.CARET),
            keyboardOptions = KeyboardOptions(
                capitalization = if (capitals) KeyboardCapitalization.Characters else KeyboardCapitalization.None,
                autoCorrectEnabled = false,
                imeAction = ime,
            ),
            keyboardActions = KeyboardActions(onAny = { onSubmit?.invoke() }),
            modifier = Modifier.fillMaxWidth().semantics { contentDescription = placeholder },
        )
    }
}
