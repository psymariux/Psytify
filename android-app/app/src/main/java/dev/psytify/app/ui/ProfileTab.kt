package dev.psytify.app.ui

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.BasicTextField
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
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.psytify.app.core.CoreHub
import dev.psytify.app.design.AnimatedSprite
import dev.psytify.app.design.IconButton
import dev.psytify.app.design.SpriteImage
import dev.psytify.app.design.PrimaryButton
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SecondaryButton
import dev.psytify.app.design.ap
import dev.psytify.app.design.rememberNotch
import dev.psytify.app.design.style
import dev.psytify.app.rust.AccountSnapshot
import dev.psytify.app.rust.AccountState
import dev.psytify.app.rust.MenuView
import dev.psytify.app.rust.PsytifyCore
import dev.psytify.app.rust.Route
import dev.psytify.app.rust.SignInStep
import dev.psytify.app.rust.SocialMarks
import dev.psytify.app.rust.Tap
import dev.psytify.app.rust.handleChars

/**
 * The «Profilo» tab, as the account is: signed out, the sign-in form; signed in, one's own profile
 * page (built by the bridge, `Route.Profile`), or the username to choose for a new account; while the
 * saved session is restored, the loader. Above all of them: «Impostazioni», «Amici» and «Jam».
 */
@Composable
fun ProfileTab(
    core: PsytifyCore,
    onTap: (Tap) -> Unit,
    onMenu: (MenuView) -> Unit,
    onOpen: (Route) -> Unit,
    onSettings: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val account by CoreHub.account.collectAsStateWithLifecycle()
    val marks by CoreHub.marks.collectAsStateWithLifecycle()
    val now = account
    val handle = now?.handle
    val bar: @Composable () -> Unit = { ProfileBar(marks, onOpen, onSettings) }
    when {
        now != null && now.state == AccountState.SIGNED_IN && handle != null -> PageScreen(
            rememberPage(core, Route.Profile(handle)),
            onTap,
            modifier,
            onMenu = onMenu,
            top = { item(key = "title") { bar() } },
        )
        else -> Column(modifier) {
            bar()
            val rest = Modifier.weight(1f)
            when {
                now == null -> Loading(rest)
                now.state == AccountState.UNAVAILABLE -> Message(
                    "L'accesso non è configurato in questa versione dell'app.",
                    rest,
                )
                now.state == AccountState.SIGNED_IN -> ChooseHandle(core, now, rest)
                signingIn(now) -> SignInScreen(core, now, rest)
                else -> Loading(rest)
            }
        }
    }
}

/** The tab's title with «Impostazioni», then the ways into «Amici» and «Jam», with what waits there. */
@Composable
private fun ProfileBar(marks: SocialMarks, onOpen: (Route) -> Unit, onSettings: () -> Unit) {
    PageTitle("Profilo") {
        IconButton(PsytifySprites.ICON_SETTINGS, "Impostazioni", onSettings)
    }
    FlowRow(
        Modifier.padding(horizontal = PsytifySpace.PHONE_GUTTER).padding(bottom = PsytifySpace.SPACE_3),
        horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_3),
    ) {
        val requests = marks.requests.toInt()
        SecondaryButton(if (requests > 0) "Amici · $requests" else "Amici", { onOpen(Route.Friends) }, leading = {
            SpriteImage(PsytifySprites.ICON_ACCOUNT, tint = PsytifyColors.ICON_ACTIVE)
        })
        val invites = marks.invites.toInt()
        val jam = when {
            marks.live -> "Jam · in corso"
            invites == 1 -> "Jam · 1 invito"
            invites > 1 -> "Jam · $invites inviti"
            else -> "Jam"
        }
        SecondaryButton(jam, { onOpen(Route.Jam) }, leading = {
            SpriteImage(PsytifySprites.ICON_PEOPLE, tint = PsytifyColors.ICON_ACTIVE)
        })
    }
}

/** The form stays on screen while it is signed out or an attempt it started is on its way. */
private fun signingIn(account: AccountSnapshot): Boolean =
    account.state == AccountState.SIGNED_OUT ||
        account.busy ||
        account.step !is SignInStep.Form

@Composable
private fun Loading(modifier: Modifier) {
    Box(modifier.fillMaxWidth().padding(PsytifySpace.SPACE_7), contentAlignment = Alignment.TopCenter) {
        AnimatedSprite(PsytifySprites.LOADER, running = true, tint = PsytifyColors.ICON_IDLE)
    }
}

@Composable
private fun Message(text: String, modifier: Modifier) {
    Column(modifier) {
        PsytifyText(
            text,
            PsytifyType.TYPE_BODY,
            PsytifyColors.TEXT_SECONDARY,
            Modifier.padding(horizontal = PsytifySpace.PHONE_GUTTER),
            maxLines = 3,
        )
    }
}

/** A new account: the username (letters, digits and `_`) and the name shown, then «Salva». */
@Composable
private fun ChooseHandle(core: PsytifyCore, account: AccountSnapshot, modifier: Modifier) {
    var handle by rememberSaveable { mutableStateOf("") }
    var name by rememberSaveable { mutableStateOf("") }
    Column(modifier) {
        Column(
            Modifier.padding(horizontal = PsytifySpace.PHONE_GUTTER),
            verticalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_3),
        ) {
            PsytifyText(
                "Scegli il nome con cui gli amici ti trovano.",
                PsytifyType.TYPE_BODY,
                PsytifyColors.TEXT_SECONDARY,
                maxLines = 2,
            )
            Line(handle, { handle = handleChars(it) }, "Nome utente")
            Line(name, { name = it.filter { c -> !c.isISOControl() } }, "Nome visibile")
            Spacer(Modifier.height(PsytifySpace.SPACE_2))
            PrimaryButton(
                if (account.saving) "Un momento…" else "Salva",
                { core.saveProfile(handle, name.ifBlank { handle }) },
                Modifier.fillMaxWidth(),
                enabled = handle.length >= 3 && !account.saving,
                fill = true,
            )
            SecondaryButton("Esci", core::signOut, Modifier.fillMaxWidth(), fill = true)
        }
    }
}

@Composable
private fun Line(value: String, onValue: (String) -> Unit, placeholder: String) {
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
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false),
            modifier = Modifier.fillMaxWidth().semantics { contentDescription = placeholder },
        )
    }
}
