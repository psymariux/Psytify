//! Copertine. Per i brani di Internet Archive il backend dà l'immagine pubblica dell'item: si
//! scarica una volta, il core la decodifica fuori dal thread della UI e la ritaglia al quadrato
//! (`psytify_core::artwork`), qui si tiene in memoria. Il widget riceve già un quadrato e non
//! disegna mai oltre i propri bordi: quadrata, senza raggio, senza tinta, filtrata in lineare (le copertine
//! sono il colore dell'app). Gli artisti hanno la maschera a cerchio di pixel, con i gradini sulla griglia
//! dei pixel d'arte. Finché la copertina non arriva, o se non c'è, resta un campo grigio con il disco
//! (`album_placeholder`) o, se piccola, l'icona della libreria: nessuna immagine finta, nessun colore
//! derivato dall'id.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::Instant;

use iced::widget::{container, image, stack};
use iced::{ContentFit, Element, Length};
use psytify_core::artwork::CoverPixels;
use psytify_design::sprite;
use psytify_design::tokens::size::cover::PLACEHOLDER_ART_MIN;
use uuid::Uuid;

use super::icons::{Icon, icon, sprite_image};
use super::{motion, theme};

/// Il cerchio di pixel: rende trasparenti i quadrati di un reticolo `cells` × `cells` il cui centro
/// sta fuori dal cerchio inscritto. I bordi sono netti (nessuna sfumatura) e a gradini di `side / cells`
/// pixel, così a schermo i gradini cadono sulla griglia dei pixel d'arte (`cells` = lato / 2 dp).
fn pixel_circle_mask(rgba: &mut [u8], side: u32, cells: u32) {
    let cells = cells.clamp(2, side.max(2));
    let centre = cells as f32 / 2.0;
    for y in 0..side {
        let cy = (y * cells / side) as f32 + 0.5 - centre;
        for x in 0..side {
            let cx = (x * cells / side) as f32 + 0.5 - centre;
            if cx.hypot(cy) > centre {
                let at = ((y * side + x) * 4 + 3) as usize;
                if let Some(alpha) = rgba.get_mut(at) {
                    *alpha = 0;
                }
            }
        }
    }
}

/// Copertine pronte tenute in memoria (256 KB l'una, più le tonde se servono): oltre, si scarta quella
/// disegnata meno di recente e, se torna a servire, si riscarica.
const MAX_READY: usize = 240;
/// Reticoli diversi per cui si tiene la versione tonda di una copertina: una per misura in uso.
const MAX_CIRCLES: usize = 4;

/// Copertine richieste, per URL.
#[derive(Debug, Default)]
pub struct Covers {
    states: HashMap<String, CoverState>,
    /// Orologio d'uso: ogni copertina disegnata prende il numero successivo (vedi `MAX_READY`).
    clock: Cell<u64>,
    /// Quando è arrivata l'ultima copertina: finché compare in dissolvenza servono i frame.
    last_arrival: Option<Instant>,
}

#[derive(Debug, Clone)]
enum CoverState {
    Loading,
    /// Pronta, con l'istante in cui è arrivata (da lì compare in dissolvenza).
    Ready(Ready),
    /// Non scaricabile o non valida: resta la copertina disegnata, senza riprovare.
    Failed,
}

/// Una copertina pronta in due forme: il quadrato com'è arrivato e il cerchio di pixel (artisti), che
/// si ricava dal quadrato la prima volta che serve, per ogni reticolo.
#[derive(Debug, Clone)]
struct Ready {
    square: image::Handle,
    /// Versioni tonde già fatte, per numero di celle del reticolo (si preparano mentre si disegna: la
    /// vista ha solo `&self`).
    circles: RefCell<Vec<(u32, image::Handle)>>,
    /// L'ultima volta che è stata disegnata, sull'orologio di `Covers`.
    used: Cell<u64>,
    since: Instant,
}

/// La versione tonda di una copertina quadrata, su un reticolo di `cells` celle di lato.
fn circle_of(square: &image::Handle, cells: u32) -> image::Handle {
    match square {
        image::Handle::Rgba { width, height, pixels, .. } => {
            let mut rgba = pixels.to_vec();
            pixel_circle_mask(&mut rgba, *width, cells);
            image::Handle::from_rgba(*width, *height, rgba)
        }
        other => other.clone(),
    }
}

/// Il reticolo di un cerchio di `size` dp: una cella ogni 2 dp (un AP), tra 8 e 96.
fn circle_cells(size: f32) -> u32 {
    ((size / theme::AP).round() as u32).clamp(8, 96)
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
                let now = Instant::now();
                self.last_arrival = Some(now);
                // I pixel arrivano dal core come `Bytes` di un solo proprietario: tornano `Bytes`
                // nell'`image::Handle` senza copia. La tinta che il core calcola qui non serve.
                CoverState::Ready(Ready {
                    square: image::Handle::from_rgba(side, side, pixels.rgba),
                    circles: RefCell::new(Vec::new()),
                    used: Cell::new(self.tick()),
                    since: now,
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

    /// L'immagine da disegnare: il quadrato, o il cerchio di pixel con `round` dato il lato in dp.
    fn handle(&self, url: Option<&str>, round: Option<f32>) -> Option<(image::Handle, Instant)> {
        match self.states.get(url?)? {
            CoverState::Ready(ready) => {
                ready.used.set(self.tick());
                let handle = match round {
                    None => ready.square.clone(),
                    Some(size) => {
                        let cells = circle_cells(size);
                        let mut circles = ready.circles.borrow_mut();
                        match circles.iter().find(|(known, _)| *known == cells) {
                            Some((_, handle)) => handle.clone(),
                            None => {
                                let handle = circle_of(&ready.square, cells);
                                circles.insert(0, (cells, handle.clone()));
                                circles.truncate(MAX_CIRCLES);
                                handle
                            }
                        }
                    }
                };
                Some((handle, ready.since))
            }
            CoverState::Loading | CoverState::Failed => None,
        }
    }

    /// Una copertina arrivata da poco sta ancora comparendo.
    pub fn is_fading(&self) -> bool {
        self.last_arrival.is_some_and(|since| since.elapsed() < theme::MOTION_BASE)
    }

    /// Lato e pixel RGBA della copertina già arrivata, per chi la mostra fuori dall'app (la
    /// schermata di blocco del telefono): il core li riceve così. I `Bytes` si condividono, non si copiano.
    pub fn artwork(&self, url: &str) -> Option<(u32, iced::advanced::image::Bytes)> {
        match self.states.get(url)? {
            CoverState::Ready(ready) => match &ready.square {
                image::Handle::Rgba { width, pixels, .. } => Some((*width, pixels.clone())),
                _ => None,
            },
            CoverState::Loading | CoverState::Failed => None,
        }
    }
}

/// Che cosa mostrare come copertina: l'immagine se c'è, altrimenti il segnaposto. L'`id` e il titolo
/// restano per chi chiama (non colorano più nulla).
#[derive(Debug, Clone, Copy)]
pub struct Art<'a> {
    pub id: Uuid,
    /// Non colora più il segnaposto: resta perché i chiamanti lo passano ovunque.
    #[allow(dead_code)]
    pub title: &'a str,
    pub url: Option<&'a str>,
}

impl<'a> Art<'a> {
    /// Solo il segnaposto (playlist locali, risultati senza immagine).
    pub fn drawn(id: Uuid, title: &'a str) -> Self {
        Self { id, title, url: None }
    }
}

/// Copertina quadrata, com'è: senza raggio e senza tinta.
pub fn cover<'a, M: 'a>(covers: &Covers, art: Art<'_>, size: f32) -> Element<'a, M> {
    shaped(covers, art, size, false)
}

/// Copertina tonda: per gli artisti.
pub fn cover_round<'a, M: 'a>(covers: &Covers, art: Art<'_>, size: f32) -> Element<'a, M> {
    shaped(covers, art, size, true)
}

fn shaped<'a, M: 'a>(covers: &Covers, art: Art<'_>, size: f32, round: bool) -> Element<'a, M> {
    let Some((handle, since)) = covers.handle(art.url, round.then_some(size)) else {
        return drawn(size, round);
    };
    // Già quadrata (o a cerchio di pixel): riempie il widget senza uscire dai bordi.
    let picture =
        image(handle).width(Length::Fixed(size)).height(Length::Fixed(size)).content_fit(ContentFit::Fill);
    let shown = motion::appear(since, theme::MOTION_BASE);
    if shown >= 1.0 {
        return picture.into();
    }
    // Compare sopra il segnaposto, senza passare dal nero.
    stack![drawn(size, round), picture.opacity(shown)].into()
}

/// Il quadrato (o cerchio di pixel) di un colore solo: il grigio del segnaposto, a griglia.
static FIELDS: LazyLock<Mutex<HashMap<u32, image::Handle>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Un cerchio di pixel pieno, `cells` × `cells` pixel di un colore, che Iced ingrandisce con il filtro
/// `Nearest`: i gradini restano netti a ogni misura.
fn circle_field(cells: u32) -> image::Handle {
    let mut fields = FIELDS.lock().unwrap_or_else(PoisonError::into_inner);
    fields
        .entry(cells)
        .or_insert_with(|| {
            let [r, g, b, _] = theme::rgba_of(theme::RAISED).bytes();
            let mut rgba: Vec<u8> = (0..cells * cells).flat_map(|_| [r, g, b, u8::MAX]).collect();
            pixel_circle_mask(&mut rgba, cells, cells);
            image::Handle::from_rgba(cells, cells, rgba)
        })
        .clone()
}

/// Il segnaposto: campo grigio. Da 64 dp in su porta il disco (`album_placeholder`, 24 AP) al centro;
/// più piccolo, l'icona della libreria in grigio spento. Tondo (artisti): il cerchio di pixel con
/// l'icona dell'account.
fn drawn<'a, M: 'a>(size: f32, round: bool) -> Element<'a, M> {
    let (art, glyph) =
        if round { (None, Icon::Account) } else { (Some(sprite::ALBUM_PLACEHOLDER), Icon::Library) };
    let mark: Element<'a, M> = match art {
        Some(name) if size >= PLACEHOLDER_ART_MIN => sprite_image(name, 0, None, 1),
        _ => icon(glyph, 0.0, theme::DISABLED),
    };
    let centred = container(mark).center(Length::Fixed(size));
    if round {
        let field = image(circle_field(circle_cells(size)))
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .content_fit(ContentFit::Fill)
            .filter_method(image::FilterMethod::Nearest);
        return stack![field, centred].into();
    }
    centred.width(Length::Fixed(size)).height(Length::Fixed(size)).style(theme::cover_field).into()
}

#[cfg(test)]
mod tests {
    use psytify_core::artwork::decode_square;

    use super::*;

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
    fn the_pixel_circle_has_hard_edges_and_steps_on_the_grid() {
        let mut square = vec![255u8; 32 * 32 * 4];
        pixel_circle_mask(&mut square, 32, 16);
        let alpha = |x: usize, y: usize| square[(y * 32 + x) * 4 + 3];
        assert_eq!((alpha(0, 0), alpha(31, 31), alpha(16, 16), alpha(16, 1)), (0, 0, 255, 255));
        // Nessuna sfumatura: ogni pixel è o pieno o trasparente.
        assert!(square.chunks(4).all(|p| p[3] == 0 || p[3] == 255));
        // Due pixel della stessa cella (2 x 2 pixel per cella) hanno lo stesso esito.
        assert_eq!(alpha(0, 5), alpha(1, 5));
    }

    #[test]
    fn the_least_recently_drawn_covers_are_dropped_first() {
        let mut covers = Covers::default();
        let pixels = || decode_square(&png(4, 4));
        for i in 0..MAX_READY {
            covers.loaded(format!("u{i}"), pixels());
        }
        // La prima torna sullo schermo: a uscire è la seconda.
        assert!(covers.handle(Some("u0"), None).is_some());
        covers.loaded("new".into(), pixels());
        assert!(covers.handle(Some("u0"), None).is_some());
        assert!(covers.handle(Some("u1"), None).is_none());
        assert_eq!(covers.request(["u1"]), vec!["u1".to_owned()]);
    }

    #[test]
    fn the_round_cover_is_a_pixel_circle_of_the_square_one() {
        let mut covers = Covers::default();
        covers.loaded("u".into(), decode_square(&png(64, 64)));
        let Some((image::Handle::Rgba { pixels, .. }, _)) = covers.handle(Some("u"), Some(96.0)) else {
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
        assert!(covers.handle(Some("a"), None).is_none());
        assert!(covers.request(["a"]).is_empty(), "a failed cover is not retried");
    }
}
