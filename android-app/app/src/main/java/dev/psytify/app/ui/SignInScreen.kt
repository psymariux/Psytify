package dev.psytify.app.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading

import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import dev.psytify.app.R
import dev.psytify.app.design.AnimatedSprite
import dev.psytify.app.design.Pill
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
import dev.psytify.app.rust.PsytifyCore
import dev.psytify.app.rust.SignInMode
import dev.psytify.app.rust.SignInProvider
import dev.psytify.app.rust.SignInStep
import dev.psytify.app.rust.canSubmitSignIn
import dev.psytify.app.rust.codeChars
import dev.psytify.app.rust.handleChars

/** The column of the form, as the desktop's `SIGN_IN_W`. */
private val FORM_W = 360.dp

private val SignInProvider.label: String
    get() = when (this) {
        SignInProvider.GOOGLE -> "Google"
        SignInProvider.GIT_HUB -> "GitHub"
        SignInProvider.DISCORD -> "Discord"
    }

private val SignInProvider.mark: Int
    get() = when (this) {
        SignInProvider.GOOGLE -> R.drawable.brand_google
        SignInProvider.GIT_HUB -> R.drawable.brand_github
        SignInProvider.DISCORD -> R.drawable.brand_discord
    }

/**
 * «Accedi a Psytify», as the desktop page and the Iced client: Nastro over the wordmark and a line
 * that follows the step; the tabs Accedi / Crea un account; the fields; GitHub, Google and Discord
 * with their marks (the one vector exception); the error under the button. The email code and the
 * wait for the browser take the place of the fields. No lime: the art direction wants none here.
 *
 * The password and the code live only in their masked fields, never in saved state: on submit the
 * text is taken, the field emptied, and the text goes to the core, which wraps it in `Secret`.
 */
@Composable
fun SignInScreen(core: PsytifyCore, account: AccountSnapshot, modifier: Modifier = Modifier) {
    var mode by rememberSaveable { mutableStateOf(SignInMode.SIGN_IN) }
    var identifier by rememberSaveable { mutableStateOf("") }
    var email by rememberSaveable { mutableStateOf("") }
    var username by rememberSaveable { mutableStateOf("") }
    // Not saveable: a secret does not go into the activity's saved state.
    var password by remember { mutableStateOf("") }
    var code by remember { mutableStateOf("") }
    val step = account.step
    val ready = canSubmitSignIn(mode, account, identifier, email, password.length.toUInt(), code.length.toUInt())
    // Typing again takes the shown error away, as on the desktop.
    fun typed() {
        if (account.error != null) core.dismissSignInError()
    }
    fun submit() {
        if (!ready) return
        when {
            step is SignInStep.Code -> {
                val taken = code
                code = ""
                core.submitCode(taken)
            }
            mode == SignInMode.SIGN_IN -> {
                val taken = password
                password = ""
                core.signIn(identifier, taken)
            }
            else -> {
                val taken = password
                password = ""
                core.signUp(email, username, taken)
            }
        }
    }

    Box(modifier.fillMaxWidth().verticalScroll(rememberScrollState()).imePadding(), contentAlignment = Alignment.TopCenter) {
        Column(
            Modifier
                .widthIn(max = FORM_W)
                .fillMaxWidth()
                .padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_6),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            AnimatedSprite(PsytifySprites.NASTRO_IDLE, running = true)
            Spacer(Modifier.height(PsytifySpace.SPACE_4))
            PsytifyText(
                "Psytify",
                PsytifyType.TYPE_PAGE_TITLE,
                PsytifyColors.TEXT_PRIMARY,
                Modifier.semantics { heading() },
            )
            Spacer(Modifier.height(PsytifySpace.SPACE_2))
            val line = when {
                step is SignInStep.Code -> "Controlla la posta"
                step is SignInStep.Browser -> "Ti aspettiamo nel browser"
                mode == SignInMode.SIGN_IN -> "Accedi per ascoltare con i tuoi amici"
                else -> "Crea il tuo account"
            }
            PsytifyText(line, PsytifyType.TYPE_BODY, PsytifyColors.TEXT_SECONDARY, maxLines = 3, align = TextAlign.Center)
            Spacer(Modifier.height(PsytifySpace.SPACE_6))

            when (step) {
                is SignInStep.Browser -> Waiting(step.provider, onCancel = core::cancelSignIn)
                is SignInStep.Code -> {
                    PsytifyText(
                        "Abbiamo mandato un codice di 6 cifre a ${step.sentTo}.",
                        PsytifyType.TYPE_BODY,
                        PsytifyColors.TEXT_SECONDARY,
                        Modifier.fillMaxWidth(),
                        maxLines = 3,
                    )
                    Spacer(Modifier.height(PsytifySpace.SPACE_4))
                    Field(
                        code,
                        { text -> code = codeChars(text); typed() },
                        "Codice",
                        masked = true,
                        keyboard = KeyboardType.NumberPassword,
                        ime = ImeAction.Done,
                        onIme = ::submit,
                    )
                    Submit("Conferma", ready, account, ::submit)
                    Spacer(Modifier.height(PsytifySpace.SPACE_3))
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_3)) {
                        SecondaryButton("Indietro", core::cancelSignIn, Modifier.weight(1f), enabled = !account.busy, fill = true)
                        SecondaryButton("Nuovo codice", core::resendCode, Modifier.weight(1f), enabled = !account.busy, fill = true)
                    }
                }
                is SignInStep.Form -> {
                    // A row that wraps: at large font sizes the two tabs do not fit side by side.
                    FlowRow(
                        Modifier.fillMaxWidth().selectableGroup(),
                        horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_2),
                    ) {
                        Pill("Accedi", mode == SignInMode.SIGN_IN, { mode = SignInMode.SIGN_IN; typed() })
                        Pill("Crea un account", mode == SignInMode.SIGN_UP, { mode = SignInMode.SIGN_UP; typed() })
                    }
                    Spacer(Modifier.height(PsytifySpace.SPACE_4))
                    if (mode == SignInMode.SIGN_IN) {
                        Field(
                            identifier,
                            { text -> identifier = text.filter { !it.isISOControl() }; typed() },
                            "Email o nome utente",
                            keyboard = KeyboardType.Email,
                        )
                        Spacer(Modifier.height(PsytifySpace.SPACE_3))
                        Field(
                            password,
                            { text -> password = text; typed() },
                            "Password",
                            masked = true,
                            keyboard = KeyboardType.Password,
                            ime = ImeAction.Done,
                            onIme = ::submit,
                        )
                        Submit("Accedi", ready, account, ::submit)
                    } else {
                        Field(
                            email,
                            { text -> email = text.filter { !it.isWhitespace() }; typed() },
                            "Email",
                            keyboard = KeyboardType.Email,
                        )
                        Spacer(Modifier.height(PsytifySpace.SPACE_3))
                        Field(
                            username,
                            { text -> username = handleChars(text); typed() },
                            "Nome utente (facoltativo)",
                        )
                        Spacer(Modifier.height(PsytifySpace.SPACE_3))
                        Field(
                            password,
                            { text -> password = text; typed() },
                            "Password (almeno 8 caratteri)",
                            masked = true,
                            keyboard = KeyboardType.Password,
                            ime = ImeAction.Done,
                            onIme = ::submit,
                        )
                        Submit("Crea l'account", ready, account, ::submit)
                    }
                    Spacer(Modifier.height(PsytifySpace.SPACE_6))
                    PsytifyText("oppure", PsytifyType.TYPE_META, PsytifyColors.TEXT_TERTIARY)
                    Spacer(Modifier.height(PsytifySpace.SPACE_4))
                    listOf(SignInProvider.GIT_HUB, SignInProvider.GOOGLE, SignInProvider.DISCORD).forEach { provider ->
                        SecondaryButton(
                            "Continua con ${provider.label}",
                            { core.signInWith(provider) },
                            Modifier.fillMaxWidth(),
                            enabled = !account.busy,
                            fill = true,
                            leading = {
                                Image(
                                    painterResource(provider.mark),
                                    contentDescription = null,
                                    modifier = Modifier.size(PsytifySpace.SPACE_6),
                                    colorFilter = ColorFilter.tint(
                                        if (account.busy) PsytifyColors.ICON_DISABLED else PsytifyColors.ICON_ACTIVE,
                                    ),
                                )
                            },
                        )
                        Spacer(Modifier.height(PsytifySpace.SPACE_3))
                    }
                }
            }
        }
    }
}

/** The primary button and, under it, the error of the last attempt. */
@Composable
private fun Submit(label: String, ready: Boolean, account: AccountSnapshot, onClick: () -> Unit) {
    Spacer(Modifier.height(PsytifySpace.SPACE_5))
    PrimaryButton(if (account.busy) "Un momento…" else label, onClick, Modifier.fillMaxWidth(), enabled = ready, fill = true)
    account.error?.let { error ->
        Spacer(Modifier.height(PsytifySpace.SPACE_3))
        PsytifyText(error, PsytifyType.TYPE_BODY, PsytifyColors.TEXT_ERROR, Modifier.fillMaxWidth(), maxLines = 4)
    }
}

/** The browser is open on the provider: the loader, what to do, «Annulla». */
@Composable
private fun Waiting(provider: SignInProvider, onCancel: () -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        AnimatedSprite(PsytifySprites.LOADER, running = true, tint = PsytifyColors.ICON_IDLE)
        Spacer(Modifier.size(PsytifySpace.SPACE_3))
        PsytifyText(
            "Continua su ${provider.label} nel browser, poi torna qui.",
            PsytifyType.TYPE_BODY,
            PsytifyColors.TEXT_PRIMARY,
            maxLines = 3,
        )
    }
    Spacer(Modifier.height(PsytifySpace.SPACE_6))
    SecondaryButton("Annulla", onCancel, Modifier.fillMaxWidth(), fill = true)
}

/** A 40 dp field of the form; a masked one shows dots and never suggests or remembers. */
@Composable
private fun Field(
    value: String,
    onValue: (String) -> Unit,
    placeholder: String,
    masked: Boolean = false,
    keyboard: KeyboardType = KeyboardType.Text,
    ime: ImeAction = ImeAction.Next,
    onIme: () -> Unit = {},
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
            visualTransformation = if (masked) PasswordVisualTransformation() else VisualTransformation.None,
            keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.None,
                autoCorrectEnabled = false,
                keyboardType = keyboard,
                imeAction = ime,
            ),
            keyboardActions = KeyboardActions(onDone = { onIme() }),
            modifier = Modifier.fillMaxWidth().semantics { contentDescription = placeholder },
        )
    }
}
