package dev.psytify.app.design

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.ImageShader
import androidx.compose.ui.graphics.Paint
import androidx.compose.ui.graphics.ShaderBrush
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.graphicsLayer
import kotlinx.coroutines.delay

/** Frames of the ordered-dither dissolve: one per cell of the 2 x 2 Bayer block. */
const val DISSOLVE_STEPS = 4

/** The order in which the cells of each 2 x 2 block go: (0,0), (1,1), (1,0), (0,1). */
private val BAYER = arrayOf(intArrayOf(0, 2), intArrayOf(3, 1))

/** The masks, one per step and art pixel size; a handful of tiny bitmaps for the process. */
private val masks = HashMap<Pair<Int, Int>, ImageBitmap>()

/** A 2 x 2 block of art pixels with the first `step` cells opaque (the part that is punched out). */
private fun mask(step: Int, artPx: Int): ImageBitmap = synchronized(masks) {
    masks.getOrPut(step to artPx) {
        val bitmap = ImageBitmap(2 * artPx, 2 * artPx)
        val canvas = Canvas(bitmap)
        val paint = Paint().apply { color = Color.Black }
        for (y in 0..1) for (x in 0..1) {
            if (BAYER[y][x] < step) {
                canvas.drawRect(
                    (x * artPx).toFloat(),
                    (y * artPx).toFloat(),
                    ((x + 1) * artPx).toFloat(),
                    ((y + 1) * artPx).toFloat(),
                    paint,
                )
            }
        }
        bitmap
    }
}

/**
 * Punches the first `step` cells of every 2 x 2 block of art pixels out of what this draws: 0 draws
 * it whole, [DISSOLVE_STEPS] draws nothing. Whole art pixels and no partial alpha, so a dissolve
 * stays pixel style (the guidelines' dither, as a transition).
 */
fun Modifier.ditherOut(step: Int, artPx: Int): Modifier = when {
    step <= 0 -> this
    step >= DISSOLVE_STEPS -> this.drawWithContent {}
    else -> this
        .graphicsLayer(compositingStrategy = CompositingStrategy.Offscreen)
        .drawWithCache {
            val brush = ShaderBrush(ImageShader(mask(step, artPx), TileMode.Repeated, TileMode.Repeated))
            onDrawWithContent {
                drawContent()
                drawRect(brush, Offset.Zero, Size(size.width, size.height), blendMode = BlendMode.DstOut)
            }
        }
}

/**
 * The step of a dissolve toward `gone` ([DISSOLVE_STEPS]) or back (0): one step every quarter of
 * `MOTION_BASE` (160 ms in all), only while it moves. Reduced motion cuts at once.
 */
@Composable
fun rememberDissolve(gone: Boolean): Int {
    val reduce = LocalReduceMotion.current
    var step by remember { mutableIntStateOf(if (gone) DISSOLVE_STEPS else 0) }
    LaunchedEffect(gone, reduce) {
        val goal = if (gone) DISSOLVE_STEPS else 0
        if (reduce) {
            step = goal
            return@LaunchedEffect
        }
        while (step != goal) {
            delay((PsytifySpace.MOTION_BASE_MS / DISSOLVE_STEPS).toLong())
            step += if (goal > step) 1 else -1
        }
    }
    return step
}
