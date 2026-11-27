//! Le copertine come texture di GPUI: un deposito globale con limite e scarto LRU.
//!
//! Il core pubblica solo gli indirizzi (`Track::cover_url`); il frontend decide quali immagini servono.
//! Una copertina chiesta per la prima volta parte in un task: `Core::artwork` la scarica dal backend e
//! la decodifica sul runtime del core (quadrato RGBA di 256 px al massimo), un thread di sfondo ne fa
//! due versioni più piccole (128 e 64 px, filtro Catmull-Rom) e il deposito le tiene come
//! `RenderImage`. Il campionatore di GPUI è lineare e 256 px a 32 o 48 dp farebbero scintillio: si
//! sceglie la versione più piccola che copre i pixel fisici richiesti. Finché l'immagine non c'è, o se
//! non arriva, resta il segnaposto disegnato (nessuna immagine finta).
//!
//! Limiti: [`MAX_READY`] copertine pronte (circa 80 KB l'una, ~10 MB in tutto) e [`MAX_KNOWN`] indirizzi
//! ricordati (compresi quelli falliti, che non si riprovano). Una copertina scartata va in `retired`, e
//! chi disegna la toglie dall'atlante con `drop_image`.
//!
//! La vista immersiva mostra la copertina fino a 640 dp: per lei sola c'è un secondo deposito, di
//! [`MAX_LARGE`] voci, con la versione intera che dà il core (256 px, come la copertina grande del client
//! Iced), chiesta solo quando la vista la disegna. Finché non arriva si disegna la versione media.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use gpui::{App, AppContext, AsyncApp, Entity, Global, RenderImage, Window};
use image::{Frame, RgbaImage, imageops};
use psytify_core::artwork::CoverPixels;
use smallvec::SmallVec;

use crate::core_model::CoreModel;

/// Copertine pronte tenute in memoria, al massimo.
pub const MAX_READY: usize = 128;
/// Indirizzi ricordati, al massimo: oltre, si dimenticano i falliti (che potrebbero riprovare).
const MAX_KNOWN: usize = 1024;
/// Lati delle due versioni, in pixel fisici.
const SMALL: u32 = 64;
const MEDIUM: u32 = 128;
/// Copertine intere tenute per la vista immersiva, al massimo (una si vede, l'altra è quella di prima).
pub const MAX_LARGE: usize = 2;

/// Una copertina pronta: la versione piccola e quella media, già in BGRA come le vuole l'atlante.
#[derive(Clone)]
pub struct CoverArt {
    small: Arc<RenderImage>,
    medium: Arc<RenderImage>,
}

impl CoverArt {
    /// La versione più piccola che copre `device_px` pixel fisici di lato.
    pub fn for_device_px(&self, device_px: f32) -> Arc<RenderImage> {
        if device_px <= SMALL as f32 { Arc::clone(&self.small) } else { Arc::clone(&self.medium) }
    }
}

enum Entry {
    Loading,
    Ready { art: CoverArt, used: u64 },
    Failed,
}

/// Cosa sa il deposito di un indirizzo.
enum Lookup {
    Ready(CoverArt),
    /// In arrivo o fallita: non si chiede di nuovo.
    Pending,
    /// Mai chiesta.
    Unknown,
}

/// Una copertina intera della vista immersiva.
enum LargeEntry {
    Loading,
    Ready(Arc<RenderImage>),
    Failed,
}

/// La logica del deposito, senza GPUI: voci, orologio d'uso, limite e scarto.
#[derive(Default)]
struct Cache {
    entries: HashMap<String, Entry>,
    /// Orologio d'uso: ogni accesso prende il numero successivo.
    clock: u64,
    /// Immagini scartate, da togliere dall'atlante al prossimo disegno.
    retired: Vec<Arc<RenderImage>>,
    /// Le copertine intere, dalla meno recente: al massimo [`MAX_LARGE`].
    large: VecDeque<(String, LargeEntry)>,
}

impl Cache {
    fn lookup(&mut self, url: &str) -> Lookup {
        self.clock += 1;
        let now = self.clock;
        match self.entries.get_mut(url) {
            Some(Entry::Ready { art, used }) => {
                *used = now;
                Lookup::Ready(art.clone())
            }
            Some(Entry::Loading | Entry::Failed) => Lookup::Pending,
            None => Lookup::Unknown,
        }
    }

    fn begin(&mut self, url: &str) {
        self.entries.insert(url.to_owned(), Entry::Loading);
    }

    fn ready_len(&self) -> usize {
        self.entries.values().filter(|entry| matches!(entry, Entry::Ready { .. })).count()
    }

    fn store(&mut self, url: String, art: Option<CoverArt>) {
        self.clock += 1;
        let used = self.clock;
        let entry = match art {
            Some(art) => Entry::Ready { art, used },
            None => Entry::Failed,
        };
        self.entries.insert(url, entry);
        self.evict();
    }

    /// La copertina intera di `url`: `Ready`, `Pending` (in arrivo o fallita) o `Unknown`.
    fn large_lookup(&mut self, url: &str) -> Option<Option<Arc<RenderImage>>> {
        let ix = self.large.iter().position(|(known, _)| known == url)?;
        // Usata adesso: passa in fondo, la più recente.
        let entry = self.large.remove(ix)?;
        let image = match &entry.1 {
            LargeEntry::Ready(image) => Some(Arc::clone(image)),
            LargeEntry::Loading | LargeEntry::Failed => None,
        };
        self.large.push_back(entry);
        Some(image)
    }

    /// Comincia a chiedere la copertina intera di `url`; oltre [`MAX_LARGE`] scarta la meno recente.
    fn large_begin(&mut self, url: &str) {
        self.large.push_back((url.to_owned(), LargeEntry::Loading));
        while self.large.len() > MAX_LARGE {
            if let Some((_, LargeEntry::Ready(image))) = self.large.pop_front() {
                self.retired.push(image);
            }
        }
    }

    /// La copertina intera di `url` è arrivata (o no). Se nel frattempo è stata scartata non si tiene:
    /// non è mai entrata nell'atlante.
    fn large_store(&mut self, url: &str, image: Option<Arc<RenderImage>>) {
        if let Some((_, entry)) = self.large.iter_mut().find(|(known, _)| known == url) {
            *entry = image.map_or(LargeEntry::Failed, LargeEntry::Ready);
        }
    }

    /// Oltre [`MAX_READY`] scarta le copertine usate meno di recente; oltre [`MAX_KNOWN`] indirizzi
    /// dimentica anche i falliti.
    fn evict(&mut self) {
        while self.ready_len() > MAX_READY {
            let oldest = self
                .entries
                .iter()
                .filter_map(|(url, entry)| match entry {
                    Entry::Ready { used, .. } => Some((*used, url.clone())),
                    _ => None,
                })
                .min();
            let Some((_, url)) = oldest else { break };
            if let Some(Entry::Ready { art, .. }) = self.entries.remove(&url) {
                self.retired.push(art.small);
                self.retired.push(art.medium);
            }
        }
        if self.entries.len() > MAX_KNOWN {
            self.entries.retain(|_, entry| !matches!(entry, Entry::Failed));
        }
    }
}

/// Il deposito globale delle copertine.
pub struct Covers {
    core: Entity<CoreModel>,
    cache: Cache,
}

impl Global for Covers {}

impl Covers {
    pub fn install(cx: &mut App, core: Entity<CoreModel>) {
        cx.set_global(Self { core, cache: Cache::default() });
    }

    /// Accede al deposito se è installato (la galleria non lo ha).
    fn with<R>(cx: &mut App, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        cx.has_global::<Self>().then(|| f(cx.global_mut::<Self>()))
    }

    /// La copertina di `url` se è pronta. Se non è mai stata chiesta la chiede (una volta sola) e
    /// torna `None`: all'arrivo la finestra si ridisegna da sé. Senza deposito (la galleria) è sempre
    /// `None`.
    pub fn art(url: &str, cx: &mut App) -> Option<CoverArt> {
        let core = Self::with(cx, |covers| match covers.cache.lookup(url) {
            Lookup::Ready(art) => Err(Some(art)),
            Lookup::Pending => Err(None),
            Lookup::Unknown => {
                covers.cache.begin(url);
                Ok(covers.core.clone())
            }
        })?;
        let core = match core {
            Ok(core) => core,
            Err(art) => return art,
        };
        let answer = core.read(cx).artwork(url.to_owned());
        let url = url.to_owned();
        // Qui si è nel disegno: il lavoro vero (rete, decodifica, ridimensionamento) sta altrove.
        cx.spawn(async move |cx| {
            let pixels = match answer {
                Some(answer) => answer.await.ok().flatten(),
                None => None,
            };
            let art = match pixels {
                Some(pixels) => cx.background_spawn(async move { build(&pixels) }).await,
                None => None,
            };
            arrived(cx, url, art);
        })
        .detach();
        None
    }

    /// La copertina intera di `url` (la vista immersiva), se è pronta; la prima volta la chiede al core e
    /// torna `None` (all'arrivo la finestra si ridisegna). Senza deposito è sempre `None`.
    pub fn large(url: &str, cx: &mut App) -> Option<Arc<RenderImage>> {
        let core = Self::with(cx, |covers| match covers.cache.large_lookup(url) {
            Some(image) => Err(image),
            None => {
                covers.cache.large_begin(url);
                Ok(covers.core.clone())
            }
        })?;
        let core = match core {
            Ok(core) => core,
            Err(image) => return image,
        };
        let answer = core.read(cx).artwork(url.to_owned());
        let url = url.to_owned();
        cx.spawn(async move |cx| {
            let pixels = match answer {
                Some(answer) => answer.await.ok().flatten(),
                None => None,
            };
            let image = match pixels {
                Some(pixels) => cx.background_spawn(async move { whole(&pixels) }).await,
                None => None,
            };
            cx.update(|cx| {
                Covers::with(cx, |covers| covers.cache.large_store(&url, image));
                cx.refresh_windows();
            });
        })
        .detach();
        None
    }

    /// Toglie dall'atlante le immagini scartate (da chiamare mentre si disegna).
    pub fn drain_retired(window: &mut Window, cx: &mut App) {
        let retired = Self::with(cx, |covers| std::mem::take(&mut covers.cache.retired)).unwrap_or_default();
        for image in retired {
            // L'unico errore possibile è «immagine sconosciuta»: non c'è nulla da recuperare.
            let _ = window.drop_image(image);
        }
    }
}

/// Una copertina è arrivata (o ha fallito): si tiene e la finestra si ridisegna.
fn arrived(cx: &AsyncApp, url: String, art: Option<CoverArt>) {
    cx.update(|cx| {
        Covers::with(cx, |covers| covers.cache.store(url, art));
        cx.refresh_windows();
    });
}

/// Le due versioni di una copertina decodificata. `None` se i pixel non sono un quadrato valido.
fn build(pixels: &CoverPixels) -> Option<CoverArt> {
    let source = RgbaImage::from_raw(pixels.side, pixels.side, pixels.rgba.to_vec())?;
    let medium = scaled(&source, MEDIUM);
    let small = scaled(&medium, SMALL);
    Some(CoverArt { small: texture(small)?, medium: texture(medium)? })
}

/// La copertina intera, com'è arrivata dal core. `None` se i pixel non sono un quadrato valido.
fn whole(pixels: &CoverPixels) -> Option<Arc<RenderImage>> {
    texture(RgbaImage::from_raw(pixels.side, pixels.side, pixels.rgba.to_vec())?)
}

/// Ridimensiona al quadrato di lato `side`, se l'immagine è più grande (mai un ingrandimento).
fn scaled(image: &RgbaImage, side: u32) -> RgbaImage {
    if image.width() <= side {
        return image.clone();
    }
    imageops::resize(image, side, side, imageops::FilterType::CatmullRom)
}

/// RGBA -> BGRA come lo vuole l'atlante di GPUI.
fn texture(mut image: RgbaImage) -> Option<Arc<RenderImage>> {
    for pixel in image.as_mut().as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(SmallVec::from_elem(Frame::new(image), 1))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(side: u32) -> CoverPixels {
        CoverPixels { side, rgba: vec![200u8; (side * side * 4) as usize].into(), tint: [200, 200, 200] }
    }

    #[test]
    fn a_cover_gets_two_smaller_versions_and_the_right_one_is_picked() {
        let art = build(&pixels(256)).expect("a square cover");
        let size = |image: &Arc<RenderImage>| {
            let size = image.size(0);
            (size.width.0, size.height.0)
        };
        assert_eq!(size(&art.small), (64, 64));
        assert_eq!(size(&art.medium), (128, 128));
        assert_eq!(size(&art.for_device_px(48.0)), (64, 64));
        assert_eq!(size(&art.for_device_px(64.0)), (64, 64));
        assert_eq!(size(&art.for_device_px(72.0)), (128, 128));
    }

    #[test]
    fn a_small_source_is_never_enlarged() {
        let art = build(&pixels(40)).expect("a small cover");
        assert_eq!(art.medium.size(0).width.0, 40);
        assert_eq!(art.small.size(0).width.0, 40);
    }

    #[test]
    fn pixels_that_do_not_fit_a_square_are_refused() {
        let broken = CoverPixels { side: 10, rgba: vec![0u8; 12].into(), tint: [0, 0, 0] };
        assert!(build(&broken).is_none());
    }

    #[test]
    fn the_cache_keeps_at_most_max_ready_covers_and_retires_the_least_recent() {
        let art = build(&pixels(64)).expect("cover");
        let mut cache = Cache::default();
        for n in 0..MAX_READY {
            cache.store(format!("u{n}"), Some(art.clone()));
        }
        // `u0` è la più vecchia, ma la si usa: la più vecchia diventa `u1`.
        assert!(matches!(cache.lookup("u0"), Lookup::Ready(_)));
        for n in 0..5 {
            cache.store(format!("new{n}"), Some(art.clone()));
        }
        assert_eq!(cache.ready_len(), MAX_READY);
        assert!(matches!(cache.lookup("u0"), Lookup::Ready(_)));
        assert!(matches!(cache.lookup("u1"), Lookup::Unknown), "scartata");
        assert!(matches!(cache.lookup("new4"), Lookup::Ready(_)));
        assert_eq!(cache.retired.len(), 10, "due immagini per copertina scartata");
    }

    #[test]
    fn the_large_covers_are_at_most_two_and_the_least_recent_is_retired() {
        let image = whole(&pixels(256)).expect("cover");
        assert_eq!(image.size(0).width.0, 256, "intera, non ridotta");
        let mut cache = Cache::default();
        assert!(cache.large_lookup("a").is_none());
        cache.large_begin("a");
        assert_eq!(cache.large_lookup("a").map(|image| image.is_some()), Some(false), "in arrivo");
        cache.large_store("a", Some(Arc::clone(&image)));
        cache.large_begin("b");
        cache.large_store("b", Some(Arc::clone(&image)));
        // `a` usata adesso: la meno recente diventa `b`.
        assert!(matches!(cache.large_lookup("a"), Some(Some(_))));
        cache.large_begin("c");
        assert!(cache.large_lookup("b").is_none(), "scartata");
        assert_eq!(cache.retired.len(), 1);
        assert!(matches!(cache.large_lookup("a"), Some(Some(_))));
        // Una risposta per una voce già scartata non si tiene.
        cache.large_store("b", Some(image));
        assert!(cache.large_lookup("b").is_none());
    }

    #[test]
    fn a_failed_or_pending_address_is_not_asked_again_and_failures_are_forgotten_in_bulk() {
        let mut cache = Cache::default();
        assert!(matches!(cache.lookup("a"), Lookup::Unknown));
        cache.begin("a");
        assert!(matches!(cache.lookup("a"), Lookup::Pending));
        cache.store("a".into(), None);
        assert!(matches!(cache.lookup("a"), Lookup::Pending), "fallita: non si riprova");
        for n in 0..=MAX_KNOWN {
            cache.store(format!("bad{n}"), None);
        }
        assert!(cache.entries.len() <= MAX_KNOWN, "i falliti si dimenticano oltre il limite");
    }
}
