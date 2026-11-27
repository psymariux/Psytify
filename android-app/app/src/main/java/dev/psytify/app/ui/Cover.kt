package dev.psytify.app.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.Dp
import dev.psytify.app.core.Covers
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.SpriteImage

/** A square cover: the image when the core has it, the drawn placeholder until then (or without one). */
@Composable
fun Cover(url: String?, size: Dp, modifier: Modifier = Modifier) {
    var image by remember(url) { mutableStateOf<ImageBitmap?>(url?.let(Covers::cached)) }
    LaunchedEffect(url) {
        if (url != null && image == null) image = Covers.get(url)
    }
    Box(modifier.size(size).background(PsytifyColors.SURFACE_RAISED), contentAlignment = Alignment.Center) {
        val shown = image
        if (shown != null) {
            Image(shown, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.size(size))
        } else {
            SpriteImage(PsytifySprites.ALBUM_PLACEHOLDER)
        }
    }
}
