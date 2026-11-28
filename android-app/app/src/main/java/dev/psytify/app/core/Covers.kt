package dev.psytify.app.core

import android.graphics.Bitmap
import android.util.LruCache
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import java.nio.ByteBuffer
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async

/**
 * The textures of the covers. The core downloads (from the backend, at most six at a time) and
 * decodes them into squares of RGBA pixels; this cache only keeps the bitmaps, as the desktop's
 * `covers.rs` does. Bounded: [CAPACITY] covers of at most 256 x 256 (16 MB), least recently used out.
 */
object Covers {
    private const val CAPACITY = 64

    private val ready = LruCache<String, ImageBitmap>(CAPACITY)
    /** Addresses that gave no image: not asked again in this session (bounded too). */
    private val missing = LruCache<String, Unit>(4 * CAPACITY)
    private val pending = HashMap<String, Deferred<ImageBitmap?>>()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    fun cached(url: String): ImageBitmap? = ready.get(url)

    suspend fun get(url: String): ImageBitmap? {
        ready.get(url)?.let { return it }
        if (missing.get(url) != null) return null
        val job = synchronized(pending) { pending.getOrPut(url) { scope.async { fetch(url) } } }
        return try {
            job.await()
        } finally {
            synchronized(pending) { pending.remove(url) }
        }
    }

    private suspend fun fetch(url: String): ImageBitmap? {
        val cover = CoreHub.core?.artwork(url)
        val side = cover?.side?.toInt() ?: 0
        if (cover == null || side <= 0 || cover.rgba.size != side * side * 4) {
            missing.put(url, Unit)
            return null
        }
        // ARGB_8888 keeps its pixels as R, G, B, A bytes: the core's layout. Covers are opaque, so
        // the premultiplied format changes nothing.
        val bitmap = Bitmap.createBitmap(side, side, Bitmap.Config.ARGB_8888)
        bitmap.copyPixelsFromBuffer(ByteBuffer.wrap(cover.rgba))
        val image = bitmap.asImageBitmap()
        ready.put(url, image)
        return image
    }
}
