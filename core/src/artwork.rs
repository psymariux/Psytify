//! Decoded covers: from the downloaded bytes to a square RGBA cropped at the centre and reduced, with
//! the average colour for the tint. The black bands a source adds to square off an image that is not
//! square (Deezer: `…/1000x1000-000000-…` fills it with black) are removed before the crop, so the
//! cover fills its shape (square or circle) instead of showing two flat edges. No graphics framework type:
//! the pixels travel as `Bytes`, which the frontends' image handles take without copying them.

use std::num::NonZero;
use std::sync::LazyLock;

use bytes::Bytes;
use tokio::sync::Semaphore;

/// The default side of a decoded cover.
pub const COVER_SIDE: u32 = 256;
/// The largest side a frontend may ask for ([`fetch_side`]): a header cover at 2x and zoom.
pub const COVER_SIDE_MAX: u32 = 512;
/// Maximum size accepted before decoding (no "bomb" images).
const MAX_SOURCE_SIDE: u32 = 4096;
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;

/// A channel up to this value counts as fill black (JPEG dirties full black a little).
const BAR_MAX: u8 = 16;
/// A band counts as fill only from this fraction of the side up: a thin frame stays.
const BAR_MIN_SHARE: f32 = 0.02;
/// What remains after removing the bands must be at least this fraction of the side: an image almost
/// all black with a caption in the middle is the artwork, not fill.
const CONTENT_MIN_SHARE: f32 = 0.4;

/// Cover downloads at most at once: a page that asks for a hundred does not open a hundred
/// connections to the backend (the others wait their turn).
const MAX_FETCHES: usize = 6;

/// Decodes at most at once: one per core, as many as ran before on the runtime's workers.
/// Each keeps the whole image in memory: a page that asks for a hundred covers does not open a hundred
/// blocking threads nor add up a hundred images.
static DECODES: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(std::thread::available_parallelism().map_or(4, NonZero::get)));

/// Downloads in progress, at most [`MAX_FETCHES`] (see [`fetch`]).
static FETCHES: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(MAX_FETCHES));

/// Decoded cover: a square RGBA of side `side`.
#[derive(Clone)]
pub struct CoverPixels {
    pub side: u32,
    pub rgba: Bytes,
    /// The average colour (RGB), for the tint of pages and the panel.
    pub tint: [u8; 3],
}

impl std::fmt::Debug for CoverPixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoverPixels").field("side", &self.side).finish_non_exhaustive()
    }
}

/// Downloads the cover `url` from the backend (`/v1/cover`, never directly from the source) and decodes it.
/// `None` if the address is not allowed, if the backend does not answer or if the image is not valid: the
/// frontend stays on the drawn cover. At most [`MAX_FETCHES`] requests at once.
pub async fn fetch(api: &crate::api::ApiClient, url: &str) -> Option<CoverPixels> {
    fetch_side(api, url, COVER_SIDE).await
}

/// [`fetch`] reduced to at most `side` pixels (clamped to [`COVER_SIDE_MAX`]).
pub async fn fetch_side(api: &crate::api::ApiClient, url: &str, side: u32) -> Option<CoverPixels> {
    let bytes = {
        let _permit = FETCHES.acquire().await.ok()?;
        api.cover(url).await.ok()?
    };
    decode_side(bytes, side).await
}

/// Decodes a downloaded cover on a blocking thread of the runtime: neither the UI thread nor an
/// async worker stays stuck on the image. `None` as for [`decode_square`].
pub async fn decode(bytes: Vec<u8>) -> Option<CoverPixels> {
    decode_side(bytes, COVER_SIDE).await
}

/// [`decode`] reduced to at most `side` pixels (clamped to [`COVER_SIDE_MAX`]).
pub async fn decode_side(bytes: Vec<u8>, side: u32) -> Option<CoverPixels> {
    let _permit = DECODES.acquire().await.ok()?;
    tokio::task::spawn_blocking(move || decode_square_max(&bytes, side)).await.ok().flatten()
}

/// Decodes an image, crops its central square and reduces it to [`COVER_SIDE`]. `None`
/// if the format is not supported, if the image is empty or too large.
pub fn decode_square(bytes: &[u8]) -> Option<CoverPixels> {
    decode_square_max(bytes, COVER_SIDE)
}

/// [`decode_square`] reduced to at most `max_side` pixels (between 1 and [`COVER_SIDE_MAX`]).
pub fn decode_square_max(bytes: &[u8], max_side: u32) -> Option<CoverPixels> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    if decoded.width() == 0 || decoded.height() == 0 {
        return None;
    }
    let (left, top, width, height) = content_box(&decoded);
    let side = width.min(height);
    let square = decoded.crop_imm(left + (width - side) / 2, top + (height - side) / 2, side, side);
    let target = side.min(max_side.clamp(1, COVER_SIDE_MAX));
    let resized = square.resize_exact(target, target, image::imageops::FilterType::Triangle);
    let rgba = resized.to_rgba8().into_raw();
    // A square of side at least 1 always has a pixel: the average exists.
    let tint = average(&rgba)?;
    Some(CoverPixels { side: target, rgba: Bytes::from(rgba), tint })
}

/// The image's box without the black fill bands: `(x, y, width, height)`. Bands are removed
/// only on one axis (top and bottom, or the sides), at least [`BAR_MIN_SHARE`] of the side long, and only if
/// at least [`CONTENT_MIN_SHARE`] remains: a black frame on all four sides, or an image almost all
/// black, is the artwork and stays as it is.
fn content_box(image: &image::DynamicImage) -> (u32, u32, u32, u32) {
    use image::GenericImageView;
    let (width, height) = image.dimensions();
    let dark = |x: u32, y: u32| image.get_pixel(x, y).0[..3].iter().all(|channel| *channel <= BAR_MAX);
    let dark_row = |y: u32| (0..width).all(|x| dark(x, y));
    let dark_column = |x: u32| (0..height).all(|y| dark(x, y));
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    let whole = (0, 0, width, height);
    let top = count((0..height).take_while(|y| dark_row(*y)).count());
    if top == height {
        return whole;
    }
    let bottom = count((0..height).rev().take_while(|y| dark_row(*y)).count());
    let left = count((0..width).take_while(|x| dark_column(*x)).count());
    let right = count((0..width).rev().take_while(|x| dark_column(*x)).count());
    let bars = |a: u32, b: u32, side: u32| a + b >= ((side as f32 * BAR_MIN_SHARE).ceil() as u32).max(1);
    let fits = |kept: u32, side: u32| kept as f32 >= side as f32 * CONTENT_MIN_SHARE;
    match (bars(top, bottom, height), bars(left, right, width)) {
        (true, false) if fits(height - top - bottom, height) => (0, top, width, height - top - bottom),
        (false, true) if fits(width - left - right, width) => (left, 0, width - left - right, height),
        _ => whole,
    }
}

/// Average colour of the pixels (one pixel in four is looked at: enough, and cheap). `None` without pixels.
fn average(rgba: &[u8]) -> Option<[u8; 3]> {
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for pixel in rgba.as_chunks::<4>().0.iter().step_by(4) {
        r += u64::from(pixel[0]);
        g += u64::from(pixel[1]);
        b += u64::from(pixel[2]);
        n += 1;
    }
    if n == 0 {
        return None;
    }
    let channel = |sum: u64| u8::try_from(sum / n).unwrap_or(u8::MAX);
    Some([channel(r), channel(g), channel(b)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::new_rgb8(width, height)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn wide_images_become_centred_squares() {
        let pixels = decode_square(&png(400, 100)).unwrap();
        assert_eq!((pixels.side, pixels.rgba.len()), (100, 100 * 100 * 4));
    }

    /// A `width` x `height` black image with a light grey box `(x, y, w, h)`.
    fn framed(width: u32, height: u32, content: (u32, u32, u32, u32)) -> Vec<u8> {
        let (cx, cy, cw, ch) = content;
        let image = image::RgbImage::from_fn(width, height, |x, y| {
            let inside = x >= cx && x < cx + cw && y >= cy && y < cy + ch;
            image::Rgb(if inside { [200, 190, 180] } else { [3, 2, 4] })
        });
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn dark_pixels(pixels: &CoverPixels) -> usize {
        pixels.rgba.as_chunks::<4>().0.iter().filter(|p| p[..3].iter().all(|c| *c <= BAR_MAX)).count()
    }

    #[test]
    fn black_bars_that_make_a_wide_image_square_are_cropped_away() {
        // As Deezer: 200 x 200 with a 200 x 110 photo inside and black above and below (not centred).
        let pixels = decode_square(&framed(200, 200, (0, 20, 200, 110))).unwrap();
        assert_eq!(pixels.side, 110);
        assert_eq!(dark_pixels(&pixels), 0, "niente fasce nere dentro la forma");
        // The same on the sides.
        let pixels = decode_square(&framed(200, 200, (40, 0, 120, 200))).unwrap();
        assert_eq!((pixels.side, dark_pixels(&pixels)), (120, 0));
    }

    #[test]
    fn a_black_frame_or_a_mostly_black_cover_is_the_artwork() {
        // Black on all four sides: it is the artwork, kept whole.
        let pixels = decode_square(&framed(200, 200, (60, 80, 80, 40))).unwrap();
        assert_eq!(pixels.side, 200);
        // A strip in the middle of a black cover: too little to be the photo.
        let pixels = decode_square(&framed(200, 200, (0, 90, 200, 20))).unwrap();
        assert_eq!(pixels.side, 200);
        // A one-pixel black line is not fill.
        let pixels = decode_square(&framed(200, 200, (0, 1, 200, 199))).unwrap();
        assert_eq!(pixels.side, 200);
    }

    #[test]
    fn the_average_of_a_flat_image_is_its_colour() {
        let flat: Vec<u8> = std::iter::repeat_n([200u8, 100, 50, 255], 64).flatten().collect();
        assert_eq!(average(&flat), Some([200, 100, 50]));
        assert_eq!(average(&[]), None);
    }

    #[test]
    fn large_images_are_scaled_down() {
        assert_eq!(decode_square(&png(1000, 800)).unwrap().side, COVER_SIDE);
        assert_eq!(decode_square_max(&png(1000, 800), 512).unwrap().side, 512);
        assert_eq!(decode_square_max(&png(1000, 800), 4096).unwrap().side, COVER_SIDE_MAX, "clamped");
        assert_eq!(decode_square_max(&png(300, 300), 512).unwrap().side, 300, "never enlarged");
        assert_eq!(decode_square_max(&png(300, 300), 0).unwrap().side, 1);
    }

    #[test]
    fn non_images_and_oversized_images_are_refused() {
        assert!(decode_square(b"not an image").is_none());
        assert!(decode_square(&png(MAX_SOURCE_SIDE + 1, 10)).is_none());
    }

    #[tokio::test]
    async fn decoding_off_the_async_threads_gives_the_same_pixels() {
        let bytes = png(64, 32);
        let direct = decode_square(&bytes).unwrap();
        let blocking = decode(bytes).await.unwrap();
        assert_eq!((blocking.side, blocking.tint, &blocking.rgba), (direct.side, direct.tint, &direct.rgba));
    }
}
