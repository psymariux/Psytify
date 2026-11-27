//! Copertine decodificate: dai byte scaricati a un quadrato RGBA ritagliato al centro e ridotto, con
//! il colore medio per la tinta. Le fasce nere che una fonte aggiunge per fare quadrata un'immagine che
//! non lo è (Deezer: `…/1000x1000-000000-…` riempie di nero) si tolgono prima del ritaglio, così la
//! copertina riempie la sua forma (quadrato o cerchio) invece di mostrare due bordi piatti. Nessun tipo di framework grafico: i pixel viaggiano come `Bytes`,
//! che Iced (`image::Handle::from_rgba`) prende senza copiarli.

use std::num::NonZero;
use std::sync::LazyLock;

use bytes::Bytes;
use tokio::sync::Semaphore;

/// Lato massimo di una copertina decodificata: basta per la carta più grande su schermi 2x.
pub const COVER_SIDE: u32 = 256;
/// Dimensioni massime accettate prima della decodifica (niente immagini "bomba").
const MAX_SOURCE_SIDE: u32 = 4096;
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;

/// Un canale fino a questo valore conta come nero di riempimento (il JPEG sporca un poco il nero pieno).
const BAR_MAX: u8 = 16;
/// Una fascia conta come riempimento solo da questa frazione del lato in su: una cornice sottile resta.
const BAR_MIN_SHARE: f32 = 0.02;
/// Quel che resta dopo aver tolto le fasce deve essere almeno questa frazione del lato: un'immagine quasi
/// tutta nera con una scritta nel mezzo è il disegno, non un riempimento.
const CONTENT_MIN_SHARE: f32 = 0.4;

/// Scaricamenti di copertine insieme al massimo: una pagina che ne chiede cento non apre cento
/// connessioni verso il backend (le altre aspettano il loro turno).
const MAX_FETCHES: usize = 6;

/// Decodifiche insieme al massimo: una per core, quante ne giravano prima sui worker del runtime.
/// Ognuna tiene in memoria l'immagine intera: una pagina che chiede cento copertine non apre cento
/// thread bloccanti né somma cento immagini.
static DECODES: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(std::thread::available_parallelism().map_or(4, NonZero::get)));

/// Scaricamenti in corso, al massimo [`MAX_FETCHES`] (vedi [`fetch`]).
static FETCHES: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(MAX_FETCHES));

/// Copertina decodificata: quadrato RGBA di lato `side`.
#[derive(Clone)]
pub struct CoverPixels {
    pub side: u32,
    pub rgba: Bytes,
    /// Il colore medio (RGB), per la tinta delle pagine e del pannello.
    pub tint: [u8; 3],
}

impl std::fmt::Debug for CoverPixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoverPixels").field("side", &self.side).finish_non_exhaustive()
    }
}

/// Scarica la copertina `url` dal backend (`/v1/cover`, mai direttamente dalla fonte) e la decodifica.
/// `None` se l'indirizzo non è ammesso, se il backend non risponde o se l'immagine non è valida: il
/// frontend resta sulla copertina disegnata. Al più [`MAX_FETCHES`] richieste insieme.
pub async fn fetch(api: &crate::api::ApiClient, url: &str) -> Option<CoverPixels> {
    let bytes = {
        let _permit = FETCHES.acquire().await.ok()?;
        api.cover(url).await.ok()?
    };
    decode(bytes).await
}

/// Decodifica una copertina scaricata su un thread bloccante del runtime: né il thread della UI né
/// un worker asincrono restano fermi sull'immagine. `None` come [`decode_square`].
pub async fn decode(bytes: Vec<u8>) -> Option<CoverPixels> {
    let _permit = DECODES.acquire().await.ok()?;
    tokio::task::spawn_blocking(move || decode_square(&bytes)).await.ok().flatten()
}

/// Decodifica un'immagine, ne ritaglia il quadrato centrale e lo riduce a [`COVER_SIDE`]. `None`
/// se il formato non è supportato, se l'immagine è vuota o troppo grande.
pub fn decode_square(bytes: &[u8]) -> Option<CoverPixels> {
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
    let target = side.min(COVER_SIDE);
    let resized = square.resize_exact(target, target, image::imageops::FilterType::Triangle);
    let rgba = resized.to_rgba8().into_raw();
    // Un quadrato di lato almeno 1 ha sempre un pixel: la media c'è.
    let tint = average(&rgba)?;
    Some(CoverPixels { side: target, rgba: Bytes::from(rgba), tint })
}

/// Il riquadro dell'immagine senza le fasce nere di riempimento: `(x, y, larghezza, altezza)`. Si tolgono
/// solo fasce su un asse (sopra e sotto, o ai lati), lunghe almeno [`BAR_MIN_SHARE`] del lato, e solo se
/// resta almeno [`CONTENT_MIN_SHARE`]: una cornice nera su tutti e quattro i lati, o un'immagine quasi tutta
/// nera, è il disegno e resta com'è.
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

/// Colore medio dei pixel (si guarda un pixel ogni quattro: basta e costa poco). `None` senza pixel.
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

    /// Un'immagine `width` x `height` nera con un riquadro grigio chiaro `(x, y, w, h)`.
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
        // Come Deezer: 200 x 200 con una foto 200 x 110 dentro e il nero sopra e sotto (non al centro).
        let pixels = decode_square(&framed(200, 200, (0, 20, 200, 110))).unwrap();
        assert_eq!(pixels.side, 110);
        assert_eq!(dark_pixels(&pixels), 0, "niente fasce nere dentro la forma");
        // Lo stesso ai lati.
        let pixels = decode_square(&framed(200, 200, (40, 0, 120, 200))).unwrap();
        assert_eq!((pixels.side, dark_pixels(&pixels)), (120, 0));
    }

    #[test]
    fn a_black_frame_or_a_mostly_black_cover_is_the_artwork() {
        // Nero su tutti e quattro i lati: è il disegno, si tiene intero.
        let pixels = decode_square(&framed(200, 200, (60, 80, 80, 40))).unwrap();
        assert_eq!(pixels.side, 200);
        // Una striscia nel mezzo di una copertina nera: troppo poco per essere la foto.
        let pixels = decode_square(&framed(200, 200, (0, 90, 200, 20))).unwrap();
        assert_eq!(pixels.side, 200);
        // Un filo nero di un pixel non è un riempimento.
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
