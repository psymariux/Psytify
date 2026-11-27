package dev.psytify.app.ui

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import dev.psytify.app.design.PrimaryButton
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SecondaryButton
import dev.psytify.app.design.ap
import dev.psytify.app.design.rememberNotch
import dev.psytify.app.design.style
import dev.psytify.app.rust.DialogView

/**
 * A playlist dialog as the bridge built it (new, rename, delete), anchored to the bottom and full
 * width as the phone dialogs are. The name field starts with the proposed name selected; the
 * keyboard's «done» confirms. The core cleans the name.
 */
@Composable
fun PlaylistDialog(dialog: DialogView, onConfirm: (String?) -> Unit, onCancel: () -> Unit) {
    BottomSheet(dialog.title, onCancel) {
        Column(Modifier.fillMaxWidth().padding(horizontal = PsytifySpace.PHONE_GUTTER)) {
            PsytifyText(
                dialog.title,
                PsytifyType.TYPE_HEADING,
                PsytifyColors.TEXT_PRIMARY,
                Modifier.semantics { heading() },
                maxLines = 2,
            )
            dialog.hint?.let { hint ->
                Spacer(Modifier.height(PsytifySpace.SPACE_2))
                PsytifyText(hint, PsytifyType.TYPE_BODY, PsytifyColors.TEXT_SECONDARY, maxLines = 3)
            }
            var text by remember(dialog.id) {
                val first = dialog.field.orEmpty()
                mutableStateOf(TextFieldValue(first, TextRange(0, first.length)))
            }
            val confirm = { onConfirm(dialog.field?.let { text.text }) }
            if (dialog.field != null) {
                Spacer(Modifier.height(PsytifySpace.SPACE_4))
                val focus = remember { FocusRequester() }
                LaunchedEffect(dialog.id) { focus.requestFocus() }
                Box(
                    Modifier
                        .fillMaxWidth()
                        .heightIn(min = PsytifySpace.TOUCH_TARGET)
                        .border(ap(PsytifySpace.BORDER_W_AP), PsytifyColors.CONTROL_BORDER_HOVER, rememberNotch(PsytifySpace.NOTCH_1_AP))
                        .padding(horizontal = PsytifySpace.SPACE_4),
                    contentAlignment = Alignment.CenterStart,
                ) {
                    BasicTextField(
                        value = text,
                        onValueChange = { text = it },
                        singleLine = true,
                        textStyle = PsytifyType.TYPE_BODY.style(PsytifyColors.TEXT_PRIMARY),
                        cursorBrush = SolidColor(PsytifyColors.CARET),
                        keyboardOptions = KeyboardOptions(
                            capitalization = KeyboardCapitalization.Sentences,
                            imeAction = ImeAction.Done,
                        ),
                        keyboardActions = KeyboardActions(onDone = { confirm() }),
                        modifier = Modifier.fillMaxWidth().focusRequester(focus).semantics { contentDescription = "Nome della playlist" },
                    )
                }
            }
            Spacer(Modifier.height(PsytifySpace.SPACE_5))
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_3, Alignment.End),
            ) {
                SecondaryButton("Annulla", onCancel)
                PrimaryButton(dialog.confirm, confirm)
            }
            Spacer(Modifier.height(PsytifySpace.SPACE_5))
        }
    }
}
