package dev.psytify.app.design

import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.selection.LocalTextSelectionColors
import androidx.compose.foundation.text.selection.TextSelectionColors
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.platform.ViewConfiguration
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import dev.psytify.app.R
import dev.psytify.app.rust.artPx

/** Physical pixels per art pixel on this screen (`PsytifyCore.artPx`, the design crate's rule). */
val LocalArtPx = staticCompositionLocalOf { 2 }

/**
 * «Riduci il movimento» as the core decided it (`reduceMotion`): sprites stay on their first frame
 * and panels appear without sliding.
 */
val LocalReduceMotion = compositionLocalOf { false }

object PsytifyFonts {
    val ui = FontFamily(
        Font(R.font.inter_regular, FontWeight.W400),
        Font(R.font.inter_medium, FontWeight.W500),
        Font(R.font.inter_semibold, FontWeight.W600),
    )
    val pixel = FontFamily(Font(R.font.departuremono_regular, FontWeight.W400))
}

@Composable
fun PsytifyTheme(content: @Composable () -> Unit) {
    val density = LocalDensity.current.density
    // 2 only if the native library is missing, and then the app shows the start error anyway.
    val px = remember(density) { runCatching { artPx(density).toInt() }.getOrDefault(2) }
    // Long-press opens the context sheet after the design's time, not the platform's 400 ms.
    val base = LocalViewConfiguration.current
    val config = remember(base) {
        object : ViewConfiguration by base {
            override val longPressTimeoutMillis: Long = PsytifySpace.LONG_PRESS_MS.toLong()
        }
    }
    // Selected text in the strong line's grey, the caret's white handles: no platform blue.
    val selection = remember { TextSelectionColors(handleColor = PsytifyColors.CARET, backgroundColor = PsytifyColors.IRON) }
    CompositionLocalProvider(
        LocalArtPx provides px,
        LocalViewConfiguration provides config,
        LocalTextSelectionColors provides selection,
        content = content,
    )
}

/** `n` art pixels as dp: a whole number of physical pixels. */
@Composable
fun ap(n: Int): Dp = with(LocalDensity.current) { (n * LocalArtPx.current).toDp() }

/**
 * The text style of a role. The pixel type is sized in art pixels (one font pixel = one AP), so it
 * stays on the pixel grid at every density; Inter follows the role in sp.
 */
@Composable
fun TypeRole.style(color: Color): TextStyle {
    val density = LocalDensity.current
    val features = listOfNotNull("tnum".takeIf { tabular }, "zero".takeIf { slashedZero }).joinToString(",")
    val (fontSize, lineHeight) = when (family) {
        TypeFamily.PIXEL -> {
            val px = LocalArtPx.current
            with(density) {
                (size / PsytifySpace.AP_DP * px).toSp() to (lineHeight / PsytifySpace.AP_DP * px).toSp()
            }
        }
        TypeFamily.UI -> size.sp to lineHeight.sp
    }
    return TextStyle(
        color = color,
        fontFamily = if (family == TypeFamily.PIXEL) PsytifyFonts.pixel else PsytifyFonts.ui,
        fontWeight = FontWeight(weight),
        fontSize = fontSize,
        lineHeight = lineHeight,
        letterSpacing = (tracking / size).em,
        fontFeatureSettings = features.ifEmpty { null },
    )
}

/** Text in a type role, on one line unless `maxLines` says otherwise, with an ellipsis. */
@Composable
fun PsytifyText(
    text: String,
    role: TypeRole,
    color: Color,
    modifier: Modifier = Modifier,
    maxLines: Int = 1,
    align: TextAlign = TextAlign.Unspecified,
) {
    BasicText(
        text = if (role.case == TypeCase.UPPER) text.uppercase() else text,
        style = role.style(color).copy(textAlign = align),
        modifier = modifier,
        maxLines = maxLines,
        overflow = TextOverflow.Ellipsis,
    )
}
