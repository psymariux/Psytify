//! Copertine. Per i brani di Internet Archive il backend dà l'immagine pubblica dell'item: si
//! scarica una volta, si decodifica fuori dal thread della UI, si ritaglia al quadrato e si tiene
//! in memoria. Il widget riceve già un quadrato e non disegna mai oltre i propri bordi. Finché non
//! arriva, o se non c'è, ogni album (o singolo) riceve un campo di colore piatto scelto dal suo id
//! e l'iniziale del titolo. Nessuna immagine finta.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::time::Instant;

use iced::widget::{container, image, stack, text};
use iced::{Color, ContentFit, Element, Length, alignment};
use uuid::Uuid;

use super::{motion, theme};

/// Toni scuri e desaturati, tutti con testo bianco oltre 4.5:1. Niente viola né indaco.
const PALETTE: [u32; 8] = [0x2f4a3a, 0x5a3e2b, 0x2b4256, 0x5b2f35, 0x4a4a24, 0x24504f, 0x5e4a22, 0x3d3d3d];
/// Lato massimo di una copertina decodificata: basta per la carta più grande su schermi 2x.
const COVER_SIDE: u32 = 256;
/// Dimensioni massime accettate prima della decodifica (niente immagini "bomba").
const MAX_SOURCE_SIDE: u32 = 4096;
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;

/// Copertina decodificata: quadrato RGBA di lato `side`.
#[derive(Clone)]
pub struct CoverPixels {
    side: u32,
    rgba: Vec<u8>,
    /// Il colore medio, per la tinta delle pagine e del pannello.
    tint: Color,
}

/// Rende trasparenti gli angoli di un quadrato RGBA di lato `side`, con un bordo sfumato di un
/// pixel. Iced 0.13 non arrotonda le immagini: lo facciamo sui pixel, una volta, alla decodifica.
/// Con `radius` uguale a metà lato il quadrato diventa un cerchio.
fn mask_corners(rgba: &mut [u8], side: u32, radius: f32) {
    let edge = side as f32;
    let r = radius.min(edge / 2.0);
    if r <= 0.0 {
        return;
    }
    for y in 0..side {
        for x in 0..side {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = if px < r {
                r - px
            } else if px > edge - r {
                px - (edge - r)
            } else {
                0.0
            };
            let dy = if py < r {
                r - py
            } else if py > edge - r {
                py - (edge - r)
            } else {
                0.0
            };
            if dx == 0.0 || dy == 0.0 {
                continue;
            }
            let coverage = (r - dx.hypot(dy) + 0.5).clamp(0.0, 1.0);
            let at = ((y * side + x) * 4 + 3) as usize;
            if let Some(alpha) = rgba.get_mut(at) {
                *alpha = (f32::from(*alpha) * coverage).round() as u8;
            }
        }
    }
}

/// Colore medio dei pixel opachi (si guarda un pixel ogni quattro: basta e costa poco).
fn average(rgba: &[u8]) -> Color {
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for pixel in rgba.as_chunks::<4>().0.iter().step_by(4) {
        r += u64::from(pixel[0]);
        g += u64::from(pixel[1]);
        b += u64::from(pixel[2]);
        n += 1;
    }
    if n == 0 {
        return theme::SURFACE;
    }
    let channel = |sum: u64| u8::try_from(sum / n).unwrap_or(u8::MAX);
    Color::from_rgb8(channel(r), channel(g), channel(b))
}

impl std::fmt::Debug for CoverPixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoverPixels").field("side", &self.side).finish_non_exhaustive()
    }
}

/// Decodifica un'immagine, ne ritaglia il quadrato centrale e lo riduce a [`COVER_SIDE`]. `None`
/// se il formato non è supportato, se l'immagine è vuota o troppo grande.
pub fn decode_square(bytes: &[u8]) -> Option<CoverPixels> {
    let mut limits = ::image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    let mut reader = ::image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let (width, height) = (decoded.width(), decoded.height());
    let side = width.min(height);
    if side == 0 {
        return None;
    }
    let square = decoded.crop_imm((width - side) / 2, (height - side) / 2, side, side);
    let target = side.min(COVER_SIDE);
    let resized = square.resize_exact(target, target, ::image::imageops::FilterType::Triangle);
    let rgba = resized.to_rgba8().into_raw();
    let tint = average(&rgba);
    Some(CoverPixels { side: target, rgba, tint })
}

/// Lato della griglia di colori da cui nasce lo sfondo sfocato, e lato dello sfondo.
const MINI_SIDE: u32 = 8;
const BACKDROP_SIDE: u32 = 128;
/// Sfondi tenuti pronti: il brano in corso e i pochi appena passati.
const BACKDROPS: usize = 4;
/// Copertine pronte tenute in memoria (256 KB l'una, più la tonda se serve): oltre, si scarta quella
/// disegnata meno di recente e, se torna a servire, si riscarica.
const MAX_READY: usize = 240;

/// Copertine richieste, per URL, e gli sfondi sfocati già preparati.
#[derive(Debug, Default)]
pub struct Covers {
    states: HashMap<String, CoverState>,
    /// Si preparano mentre si disegna (la vista ha solo `&self`): per questo una `RefCell`.
    backdrops: RefCell<Vec<(String, [image::Handle; 2])>>,
    /// Orologio d'uso: ogni copertina disegnata prende il numero successivo (vedi `MAX_READY`).
    clock: Cell<u64>,
    /// Quando è arrivata l'ultima copertina: finché compare in dissolvenza servono i frame.
    last_arrival: Option<Instant>,
}

#[derive(Debug, Clone)]
enum CoverState {
    Loading,
    /// Pronta, con l'istante in cui è arrivata (da lì compare in dissolvenza) e il colore medio.
    Ready(Ready),
    /// Non scaricabile o non valida: resta la copertina disegnata, senza riprovare.
    Failed,
}

/// Una copertina pronta in due forme: a quadrato con gli angoli arrotondati (album) e a cerchio
/// (artisti). Il cerchio si ricava dal quadrato la prima volta che serve: gli angoli arrotondati
/// stanno tutti fuori dal cerchio inscritto, quindi il risultato è lo stesso.
#[derive(Debug, Clone)]
struct Ready {
    square: image::Handle,
    circle: OnceCell<image::Handle>,
    /// L'ultima volta che è stata disegnata, sull'orologio di `Covers`.
    used: Cell<u64>,
    since: Instant,
    tint: Color,
    /// La copertina ridotta a `MINI_SIDE` × `MINI_SIDE` colori: la materia dello sfondo sfocato.
    mini: Vec<u8>,
}

/// La versione tonda di una copertina quadrata già arrotondata (vedi `Ready`).
fn circle_of(square: &image::Handle) -> image::Handle {
    match square {
        image::Handle::Rgba { width, height, pixels, .. } => {
            let mut rgba = pixels.to_vec();
            mask_corners(&mut rgba, *width, *width as f32 / 2.0);
            image::Handle::from_rgba(*width, *height, rgba)
        }
        other => other.clone(),
    }
}

/// Riduce un quadrato RGBA di lato `side` a `MINI_SIDE` × `MINI_SIDE` colori medi.
fn shrink(rgba: &[u8], side: u32) -> Vec<u8> {
    let n = MINI_SIDE as usize;
    let side = side as usize;
    let mut sums = vec![[0u64; 4]; n * n];
    for y in 0..side {
        for x in 0..side {
            let cell = (y * n / side.max(1)) * n + x * n / side.max(1);
            let at = (y * side + x) * 4;
            if let (Some(sum), Some(pixel)) = (sums.get_mut(cell), rgba.get(at..at + 3)) {
                sum[0] += u64::from(pixel[0]);
                sum[1] += u64::from(pixel[1]);
                sum[2] += u64::from(pixel[2]);
                sum[3] += 1;
            }
        }
    }
    sums.iter()
        .flat_map(|[r, g, b, count]| {
            let count = (*count).max(1);
            let channel = |sum: u64| u8::try_from(sum / count).unwrap_or(u8::MAX);
            [channel(*r), channel(*g), channel(*b), u8::MAX]
        })
        .collect()
}

/// Ingrandisce la griglia di colori a `BACKDROP_SIDE` con un'interpolazione morbida (smoothstep):
/// ne esce la copertina come sfocata, senza gli spigoli dell'interpolazione lineare. Con
/// `flipped` la griglia è ruotata di mezzo giro: la seconda immagine con cui lo sfondo si muove.
fn backdrop_pixels(mini: &[u8], flipped: bool) -> Vec<u8> {
    let n = MINI_SIDE as usize;
    let out = BACKDROP_SIDE as usize;
    let cell = |x: usize, y: usize, c: usize| -> f32 {
        let (x, y) = if flipped { (n - 1 - x, n - 1 - y) } else { (x, y) };
        f32::from(mini.get((y * n + x) * 4 + c).copied().unwrap_or(0))
    };
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let mut rgba = Vec::with_capacity(out * out * 4);
    for y in 0..out {
        let v = ((y as f32 + 0.5) / out as f32 * n as f32 - 0.5).clamp(0.0, (n - 1) as f32);
        let (y0, ty) = (v.floor() as usize, smooth(v.fract()));
        let y1 = (y0 + 1).min(n - 1);
        for x in 0..out {
            let u = ((x as f32 + 0.5) / out as f32 * n as f32 - 0.5).clamp(0.0, (n - 1) as f32);
            let (x0, tx) = (u.floor() as usize, smooth(u.fract()));
            let x1 = (x0 + 1).min(n - 1);
            for c in 0..3 {
                let top = cell(x0, y0, c) * (1.0 - tx) + cell(x1, y0, c) * tx;
                let bottom = cell(x0, y1, c) * (1.0 - tx) + cell(x1, y1, c) * tx;
                rgba.push((top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8);
            }
            rgba.push(u8::MAX);
        }
    }
    rgba
}

impl Covers {
    /// Segna come in caricamento gli URL mai visti e li restituisce, da scaricare.
    pub fn request<'a>(&mut self, urls: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let mut wanted = Vec::new();
        for url in urls {
            if !self.states.contains_key(url) {
                self.states.insert(url.to_owned(), CoverState::Loading);
                wanted.push(url.to_owned());
            }
        }
        wanted
    }

    pub fn loaded(&mut self, url: String, pixels: Option<CoverPixels>) {
        let state = match pixels {
            Some(pixels) => {
                let side = pixels.side;
                let mini = shrink(&pixels.rgba, side);
                let mut square = pixels.rgba;
                mask_corners(&mut square, side, side as f32 * theme::COVER_CORNER);
                let now = Instant::now();
                self.last_arrival = Some(now);
                CoverState::Ready(Ready {
                    square: image::Handle::from_rgba(side, side, square),
                    circle: OnceCell::new(),
                    used: Cell::new(self.tick()),
                    since: now,
                    tint: pixels.tint,
                    mini,
                })
            }
            None => CoverState::Failed,
        };
        self.states.insert(url, state);
        self.evict();
    }

    /// Il numero successivo dell'orologio d'uso.
    fn tick(&self) -> u64 {
        let next = self.clock.get() + 1;
        self.clock.set(next);
        next
    }

    /// Oltre `MAX_READY` copertine pronte scarta quelle disegnate meno di recente. Una copertina
    /// scartata torna «mai vista»: se una pagina la chiede di nuovo, si riscarica.
    fn evict(&mut self) {
        let ready = self.states.values().filter(|state| matches!(state, CoverState::Ready(_))).count();
        let Some(excess) = ready.checked_sub(MAX_READY).filter(|excess| *excess > 0) else {
            return;
        };
        let mut by_use: Vec<(u64, String)> = self
            .states
            .iter()
            .filter_map(|(url, state)| match state {
                CoverState::Ready(ready) => Some((ready.used.get(), url.clone())),
                CoverState::Loading | CoverState::Failed => None,
            })
            .collect();
        by_use.sort_unstable();
        for (_, url) in by_use.into_iter().take(excess) {
            self.states.remove(&url);
        }
    }

    fn handle(&self, url: Option<&str>, round: bool) -> Option<(image::Handle, Instant)> {
        match self.states.get(url?)? {
            CoverState::Ready(ready) => {
                ready.used.set(self.tick());
                let handle =
                    if round { ready.circle.get_or_init(|| circle_of(&ready.square)) } else { &ready.square };
                Some((handle.clone(), ready.since))
            }
            CoverState::Loading | CoverState::Failed => None,
        }
    }

    /// Una copertina arrivata da poco sta ancora comparendo.
    pub fn is_fading(&self) -> bool {
        self.last_arrival.is_some_and(|since| since.elapsed() < theme::MOTION_BASE)
    }

    /// Il colore medio di una copertina già arrivata.
    fn tint(&self, url: Option<&str>) -> Option<Color> {
        match self.states.get(url?)? {
            CoverState::Ready(ready) => Some(ready.tint),
            CoverState::Loading | CoverState::Failed => None,
        }
    }

    /// Le due immagini dello sfondo sfocato di una copertina già arrivata, preparate la prima volta
    /// che servono.
    fn backdrop_handles(&self, url: &str) -> Option<[image::Handle; 2]> {
        let mut cache = self.backdrops.borrow_mut();
        if let Some((_, handles)) = cache.iter().find(|(known, _)| known == url) {
            return Some(handles.clone());
        }
        let CoverState::Ready(ready) = self.states.get(url)? else { return None };
        let side = BACKDROP_SIDE;
        let handles = [
            image::Handle::from_rgba(side, side, backdrop_pixels(&ready.mini, false)),
            image::Handle::from_rgba(side, side, backdrop_pixels(&ready.mini, true)),
        ];
        cache.insert(0, (url.to_owned(), handles.clone()));
        cache.truncate(BACKDROPS);
        Some(handles)
    }

    /// Il colore con cui tingere una pagina o un pannello: quello della copertina se c'è, altrimenti
    /// il colore piatto dell'album.
    pub fn tint_of(&self, art: Art<'_>) -> Color {
        self.tint(art.url).unwrap_or_else(|| color_for(art.id))
    }
}

/// Che cosa mostrare come copertina: l'immagine se c'è, altrimenti colore e iniziale da `id` e
/// `title`.
#[derive(Debug, Clone, Copy)]
pub struct Art<'a> {
    pub id: Uuid,
    pub title: &'a str,
    pub url: Option<&'a str>,
}

impl<'a> Art<'a> {
    /// Solo colore e iniziale (playlist locali, risultati senza immagine).
    pub fn drawn(id: Uuid, title: &'a str) -> Self {
        Self { id, title, url: None }
    }
}

pub fn color_for(id: Uuid) -> Color {
    let index = usize::try_from(id.as_u128() % PALETTE.len() as u128).unwrap_or(0);
    let rgb = PALETTE[index];
    Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Prima lettera o cifra del titolo, in maiuscolo.
pub fn initial(title: &str) -> String {
    title
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "·".into())
}

/// La copertina sfocata a tutta l'area, come sfondo delle viste a tutto schermo: due immagini (la
/// seconda ruotata) che si mescolano secondo `phase`, da 0 a 1, così i colori si muovono piano.
/// `None` finché la copertina non è arrivata.
pub fn backdrop<'a, M: 'a>(covers: &Covers, art: Art<'_>, phase: f32) -> Option<Element<'a, M>> {
    let [first, second] = covers.backdrop_handles(art.url?)?;
    let layer = |handle: image::Handle| {
        image(handle)
            .width(Length::Fill)
            .height(Length::Fill)
            .content_fit(ContentFit::Cover)
            .filter_method(image::FilterMethod::Linear)
    };
    Some(stack![layer(first), layer(second).opacity(phase.clamp(0.0, 1.0))].into())
}

/// Copertina quadrata con gli angoli arrotondati.
pub fn cover<'a, M: 'a>(covers: &Covers, art: Art<'_>, size: f32) -> Element<'a, M> {
    shaped(covers, art, size, false)
}

/// Copertina tonda: per gli artisti.
pub fn cover_round<'a, M: 'a>(covers: &Covers, art: Art<'_>, size: f32) -> Element<'a, M> {
    shaped(covers, art, size, true)
}

fn shaped<'a, M: 'a>(covers: &Covers, art: Art<'_>, size: f32, round: bool) -> Element<'a, M> {
    let Some((handle, since)) = covers.handle(art.url, round) else {
        return drawn(art, size, round);
    };
    // Già quadrata e con gli angoli trasparenti: riempie il widget senza uscire dai bordi.
    let picture =
        image(handle).width(Length::Fixed(size)).height(Length::Fixed(size)).content_fit(ContentFit::Fill);
    let shown = motion::appear(since, theme::MOTION_BASE);
    if shown >= 1.0 {
        return picture.into();
    }
    // Compare sopra la copertina disegnata, senza passare dal nero.
    stack![drawn(art, size, round), picture.opacity(shown)].into()
}

/// Colore piatto dall'id e iniziale del titolo (al centro se tonda, in basso a sinistra se no).
fn drawn<'a, M: 'a>(art: Art<'_>, size: f32, round: bool) -> Element<'a, M> {
    let letter =
        text(initial(art.title)).size((size * 0.42).round()).font(theme::FONT_SEMIBOLD).color(theme::TEXT);
    let radius = if round { size / 2.0 } else { size * theme::COVER_CORNER };
    let frame = container(letter).width(Length::Fixed(size)).height(Length::Fixed(size));
    let frame = if round {
        frame.center(Length::Fixed(size))
    } else {
        frame
            .padding((size * 0.1).round())
            .align_x(alignment::Horizontal::Left)
            .align_y(alignment::Vertical::Bottom)
    };
    frame.style(theme::cover(color_for(art.id), radius)).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_id_always_gets_same_colour() {
        let id = Uuid::from_u128(77);
        assert_eq!(color_for(id), color_for(id));
    }

    #[test]
    fn initial_skips_punctuation() {
        assert_eq!(initial("(live) àlba"), "L");
    }

    #[test]
    fn initial_of_empty_title_is_a_dot() {
        assert_eq!(initial(""), "·");
    }

    #[test]
    fn each_cover_is_requested_once() {
        let mut covers = Covers::default();
        assert_eq!(covers.request(["a", "b", "a"]), vec!["a".to_owned(), "b".to_owned()]);
        assert!(covers.request(["a"]).is_empty());
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        ::image::DynamicImage::new_rgb8(width, height)
            .write_to(&mut std::io::Cursor::new(&mut bytes), ::image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn wide_images_become_centred_squares() {
        let pixels = decode_square(&png(400, 100)).unwrap();
        assert_eq!((pixels.side, pixels.rgba.len()), (100, 100 * 100 * 4));
    }

    #[test]
    fn corners_become_transparent_and_the_centre_stays() {
        let mut square = vec![255u8; 32 * 32 * 4];
        mask_corners(&mut square, 32, 8.0);
        let alpha = |x: usize, y: usize| square[(y * 32 + x) * 4 + 3];
        assert_eq!((alpha(0, 0), alpha(31, 31), alpha(16, 16), alpha(16, 0)), (0, 0, 255, 255));
    }

    #[test]
    fn a_full_radius_makes_a_circle() {
        let mut square = vec![255u8; 32 * 32 * 4];
        mask_corners(&mut square, 32, 16.0);
        let alpha = |x: usize, y: usize| square[(y * 32 + x) * 4 + 3];
        assert_eq!((alpha(1, 1), alpha(16, 16), alpha(16, 1)), (0, 255, 255));
    }

    #[test]
    fn the_average_of_a_flat_image_is_its_colour() {
        let flat: Vec<u8> = std::iter::repeat_n([200u8, 100, 50, 255], 64).flatten().collect();
        assert_eq!(average(&flat), Color::from_rgb8(200, 100, 50));
        assert_eq!(average(&[]), theme::SURFACE);
    }

    #[test]
    fn a_cover_shrinks_to_a_grid_of_its_colours() {
        let half: Vec<u8> = (0..16 * 16)
            .flat_map(|i| if i % 16 < 8 { [255u8, 0, 0, 255] } else { [0, 0, 255, 255] })
            .collect();
        let mini = shrink(&half, 16);
        assert_eq!(mini.len(), (MINI_SIDE * MINI_SIDE * 4) as usize);
        assert_eq!(
            (&mini[0..4], &mini[mini.len() - 4..]),
            (&[255u8, 0, 0, 255][..], &[0u8, 0, 255, 255][..])
        );
    }

    #[test]
    fn the_backdrop_is_smooth_and_opaque() {
        let mini = shrink(&vec![90u8; 256 * 256 * 4], 256);
        let pixels = backdrop_pixels(&mini, true);
        assert_eq!(pixels.len(), (BACKDROP_SIDE * BACKDROP_SIDE * 4) as usize);
        assert!(pixels.chunks(4).all(|p| p == [90, 90, 90, 255]));
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

    #[test]
    fn the_least_recently_drawn_covers_are_dropped_first() {
        let mut covers = Covers::default();
        let pixels = || decode_square(&png(4, 4));
        for i in 0..MAX_READY {
            covers.loaded(format!("u{i}"), pixels());
        }
        // La prima torna sullo schermo: a uscire è la seconda.
        assert!(covers.handle(Some("u0"), false).is_some());
        covers.loaded("new".into(), pixels());
        assert!(covers.handle(Some("u0"), false).is_some());
        assert!(covers.handle(Some("u1"), false).is_none());
        assert_eq!(covers.request(["u1"]), vec!["u1".to_owned()]);
    }

    #[test]
    fn the_round_cover_is_a_circle_of_the_square_one() {
        let mut covers = Covers::default();
        covers.loaded("u".into(), decode_square(&png(64, 64)));
        let Some((image::Handle::Rgba { pixels, .. }, _)) = covers.handle(Some("u"), true) else {
            panic!("niente copertina tonda");
        };
        // Il pixel a un ottavo del lato, sulla diagonale, sta fuori dal cerchio.
        assert_eq!(pixels[(8 * 64 + 8) * 4 + 3], 0);
        assert_eq!(pixels[(32 * 64 + 32) * 4 + 3], u8::MAX);
    }

    #[test]
    fn failed_cover_falls_back_to_the_drawn_one() {
        let mut covers = Covers::default();
        covers.request(["a"]);
        covers.loaded("a".into(), None);
        assert!(covers.handle(Some("a"), false).is_none());
        assert!(covers.request(["a"]).is_empty(), "a failed cover is not retried");
    }
}
