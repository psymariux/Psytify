package dev.psytify.app.design

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.GenericShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.imageResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.graphics.ImageBitmap
import kotlinx.coroutines.delay

/**
 * A sprite frame at a whole multiple of the art pixel, never filtered. A mask sprite takes `tint`
 * (a colour token for its state); a colour sprite keeps its palette.
 */
@Composable
fun SpriteImage(sprite: Sprite, modifier: Modifier = Modifier, tint: Color? = null, frame: Int = 0) {
    val sheet = ImageBitmap.imageResource(sprite.res)
    val px = LocalArtPx.current
    val width = sprite.frameWidth * px
    val height = sprite.frameHeight * px
    val size = with(LocalDensity.current) { androidx.compose.ui.unit.DpSize(width.toDp(), height.toDp()) }
    val filter = remember(tint, sprite.mask) { tint?.takeIf { sprite.mask }?.let { ColorFilter.tint(it, BlendMode.SrcIn) } }
    Canvas(modifier.size(size)) {
        drawImage(
            image = sheet,
            srcOffset = IntOffset((frame % sprite.durationsMs.size) * sprite.frameWidth, 0),
            srcSize = IntSize(sprite.frameWidth, sprite.frameHeight),
            dstSize = IntSize(width, height),
            filterQuality = FilterQuality.None,
            colorFilter = filter,
        )
    }
}

/**
 * An animated sprite: its frames advance only while `running` (one timer, no per-frame work
 * otherwise) and the motion is not reduced; reduced, it rests on its first frame.
 */
@Composable
fun AnimatedSprite(sprite: Sprite, running: Boolean, modifier: Modifier = Modifier, tint: Color? = null) {
    var frame by remember(sprite) { mutableIntStateOf(0) }
    val still = LocalReduceMotion.current
    if (still) frame = 0
    LaunchedEffect(sprite, running, still) {
        if (still) return@LaunchedEffect
        if (sprite.durationsMs.size < 2) return@LaunchedEffect
        while (running) {
            delay(sprite.durationsMs[frame].toLong().coerceAtLeast(16))
            frame = (frame + 1) % sprite.durationsMs.size
        }
    }
    SpriteImage(sprite, modifier, tint, frame)
}

/** A rectangle with `notch` physical pixels cut square from each corner: the pixel "rounding". */
fun notchShape(notch: Float): Shape = GenericShape { size, _ ->
    val c = notch.coerceAtMost(minOf(size.width, size.height) / 2)
    val w = size.width
    val h = size.height
    moveTo(c, 0f)
    lineTo(w - c, 0f)
    lineTo(w - c, c)
    lineTo(w, c)
    lineTo(w, h - c)
    lineTo(w - c, h - c)
    lineTo(w - c, h)
    lineTo(c, h)
    lineTo(c, h - c)
    lineTo(0f, h - c)
    lineTo(0f, c)
    lineTo(c, c)
    close()
}

/** A notch of `ap` art pixels on this screen. */
@Composable
fun rememberNotch(ap: Int): Shape {
    val px = LocalArtPx.current
    return remember(px, ap) { notchShape((ap * px).toFloat()) }
}

/**
 * The secondary button: 1-AP notch, a 1-AP control border, and the hard ledge under it; pressed, the
 * face moves onto the ledge (`PRESS_TRANSLATE`). `selected` (a switch that is on, «Segui già») has
 * the lighter border.
 */
@Composable
fun SecondaryButton(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    selected: Boolean = false,
    enabled: Boolean = true,
    fill: Boolean = false,
    leading: (@Composable () -> Unit)? = null,
) {
    PixelButton(label, onClick, modifier, primary = false, selected, enabled, fill, leading)
}

/** The primary button: warm white face, dark label, stone ledge. One per region at most. */
@Composable
fun PrimaryButton(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    fill: Boolean = false,
) {
    PixelButton(label, onClick, modifier, primary = true, selected = false, enabled, fill, leading = null)
}

/**
 * Disabled, a button keeps its shape and loses its ledge: the canvas face, stone border and text.
 * `fill` stretches the face to the width the modifier gives (a form's full-width buttons).
 */
@Composable
private fun PixelButton(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier,
    primary: Boolean,
    selected: Boolean,
    enabled: Boolean,
    fill: Boolean,
    leading: (@Composable () -> Unit)?,
) {
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val shape = rememberNotch(PsytifySpace.NOTCH_1_AP)
    val ledgeX = ap(PsytifySpace.LEDGE_X_AP)
    val ledgeY = ap(PsytifySpace.LEDGE_Y_AP)
    val ledge = if (primary) PsytifyColors.LEDGE_PRIMARY else PsytifyColors.LEDGE_SECONDARY
    Box(
        modifier
            .defaultMinSize(minHeight = PsytifySpace.TOUCH_TARGET)
            .clickable(interaction, indication = null, enabled = enabled, role = Role.Button, onClick = onClick)
            .semantics { if (selected) this.selected = true },
        contentAlignment = Alignment.Center,
    ) {
        val lit = primary && enabled
        val down = pressed && enabled
        Box(if (fill) Modifier.fillMaxWidth() else Modifier) {
            if (enabled) Box(Modifier.matchParentSize().offset(ledgeX, ledgeY).background(ledge, shape))
            val face = Modifier
                .offset(if (down) ledgeX else ap(0), if (down) ledgeY else ap(0))
                .background(if (lit) PsytifyColors.WARM_WHITE else PsytifyColors.SURFACE_CANVAS, shape)
            val bordered = if (lit) {
                face
            } else {
                val border = when {
                    !enabled -> PsytifyColors.STONE
                    selected -> PsytifyColors.CONTROL_BORDER_HOVER
                    else -> PsytifyColors.CONTROL_BORDER
                }
                face.border(ap(PsytifySpace.BORDER_W_AP), border, shape)
            }
            Row(
                (if (fill) bordered.fillMaxWidth() else bordered)
                    .defaultMinSize(minHeight = PsytifySpace.CONTROL_H_LARGE)
                    .padding(horizontal = PsytifySpace.SPACE_5),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (leading != null) {
                    leading()
                    Spacer(Modifier.width(PsytifySpace.SPACE_3))
                }
                val color = when {
                    !enabled -> PsytifyColors.TEXT_DISABLED
                    primary -> PsytifyColors.TEXT_ON_LIGHT
                    else -> PsytifyColors.TEXT_PRIMARY
                }
                PsytifyText(label, PsytifyType.TYPE_BUTTON, color)
            }
        }
    }
}

/**
 * A square icon button with a 48 dp touch target around a mask sprite. `state` is what TalkBack
 * says after the name («attiva» for shuffle on), for the buttons that are switches.
 */
@Composable
fun IconButton(
    sprite: Sprite,
    description: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    tint: Color = PsytifyColors.ICON_ACTIVE,
    state: String? = null,
) {
    Box(
        modifier
            .size(PsytifySpace.TOUCH_TARGET)
            .clickable(
                remember { MutableInteractionSource() },
                indication = null,
                role = Role.Button,
                onClick = onClick,
            )
            .semantics {
                contentDescription = description
                if (state != null) stateDescription = state
            },
        contentAlignment = Alignment.Center,
    ) {
        SpriteImage(sprite, tint = tint)
    }
}

/** A text-only button (Rifiuta, Ritira, Ignora): the label in the secondary text, a 48 dp target. */
@Composable
fun GhostButton(label: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
    Box(
        modifier
            .defaultMinSize(minHeight = PsytifySpace.TOUCH_TARGET)
            .clickable(remember { MutableInteractionSource() }, indication = null, enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = PsytifySpace.SPACE_3),
        contentAlignment = Alignment.Center,
    ) {
        PsytifyText(label, PsytifyType.TYPE_BUTTON, if (enabled) PsytifyColors.TEXT_SECONDARY else PsytifyColors.TEXT_DISABLED)
    }
}

/**
 * An on/off switch: a notched track with a square knob, on the right and lime when on (lime marks
 * a state), on the left in stone when off.
 */
@Composable
fun PixelSwitch(on: Boolean, onChange: (Boolean) -> Unit, description: String, modifier: Modifier = Modifier) {
    val shape = rememberNotch(PsytifySpace.NOTCH_1_AP)
    Box(
        modifier
            .size(PsytifySpace.TOUCH_TARGET)
            // A real switch for TalkBack: «on» or «off» after the name, double tap toggles.
            .toggleable(on, remember { MutableInteractionSource() }, indication = null, role = Role.Switch, onValueChange = onChange)
            .semantics { contentDescription = description },
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier
                .size(ap(18), ap(10))
                .border(ap(PsytifySpace.BORDER_W_AP), if (on) PsytifyColors.LINE_STRONG else PsytifyColors.CONTROL_BORDER, shape)
                .padding(ap(2)),
            contentAlignment = if (on) Alignment.CenterEnd else Alignment.CenterStart,
        ) {
            Box(Modifier.size(ap(6)).background(if (on) PsytifyColors.STATE_ON else PsytifyColors.STONE))
        }
    }
}

/**
 * A filter or a switch on one line: the chosen one has the selected surface and the strong line.
 * `role` is a tab for filters and panels, a radio button for one choice among a setting's.
 */
@Composable
fun Pill(label: String, on: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier, role: Role = Role.Tab) {
    val shape = rememberNotch(PsytifySpace.NOTCH_1_AP)
    Box(
        modifier
            .defaultMinSize(minHeight = PsytifySpace.TOUCH_TARGET)
            .selectable(on, remember { MutableInteractionSource() }, indication = null, role = role, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier
                .background(if (on) PsytifyColors.SURFACE_SELECTED else PsytifyColors.SURFACE_CANVAS, shape)
                .border(ap(PsytifySpace.BORDER_W_AP), if (on) PsytifyColors.LINE_STRONG else PsytifyColors.CONTROL_BORDER, shape)
                .padding(horizontal = PsytifySpace.SPACE_5, vertical = PsytifySpace.SPACE_2),
        ) {
            PsytifyText(label, PsytifyType.TYPE_BUTTON, if (on) PsytifyColors.TEXT_PRIMARY else PsytifyColors.TEXT_SECONDARY)
        }
    }
}

/**
 * A circle drawn in whole art pixels (`px` physical pixels each): each row of art pixels is as wide
 * as the circle at its middle, so the edge steps like a sprite.
 */
fun pixelCircle(px: Float): Shape = GenericShape { size, _ ->
    val side = minOf(size.width, size.height)
    val rows = (side / px).toInt().coerceAtLeast(1)
    val r = rows / 2f
    for (row in 0 until rows) {
        val dy = row + 0.5f - r
        val half = kotlin.math.round(kotlin.math.sqrt((r * r - dy * dy).coerceAtLeast(0f)))
        if (half <= 0f) continue
        val top = row * px
        addRect(androidx.compose.ui.geometry.Rect((r - half) * px, top, (r + half) * px, top + px))
    }
}

/** The pixel circle on this screen. */
@Composable
fun rememberPixelCircle(): Shape {
    val px = LocalArtPx.current
    return remember(px) { pixelCircle(px.toFloat()) }
}

/** A rectangle with the two top corners cut by `notch` physical pixels: bottom sheets and dialogs. */
fun topNotchShape(notch: Float): Shape = GenericShape { size, _ ->
    val c = notch.coerceAtMost(minOf(size.width, size.height) / 2)
    val w = size.width
    val h = size.height
    moveTo(c, 0f)
    lineTo(w - c, 0f)
    lineTo(w - c, c)
    lineTo(w, c)
    lineTo(w, h)
    lineTo(0f, h)
    lineTo(0f, c)
    lineTo(c, c)
    close()
}

/** The top notch of `ap` art pixels on this screen. */
@Composable
fun rememberTopNotch(ap: Int): Shape {
    val px = LocalArtPx.current
    return remember(px, ap) { topNotchShape((ap * px).toFloat()) }
}
