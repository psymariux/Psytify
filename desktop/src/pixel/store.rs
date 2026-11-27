//! Deposito degli sprite del desktop: possiede la [`UpscaleCache`] di `psytify-design` (pixel
//! ingranditi in RGBA, con limite di voci e di byte) e, per ogni voce, la `RenderImage` BGRA che GPUI
//! carica nell'atlante. Lo possiede l'app (un `Global`), non i singoli sprite.
//!
//! Perché due livelli: GPUI vuole BGRA e un `Arc<RenderImage>` con identità stabile (ogni nuova
//! `RenderImage` ha un id nuovo e quindi un nuovo caricamento nell'atlante). L'immagine BGRA si crea
//! una volta per voce di cache e si scarta insieme a lei; quando una voce esce dalla cache la sua
//! immagine finisce in `retired` e `drain_retired` la toglie dall'atlante al prossimo disegno.
//!
//! When the effective scale (window scale times UI zoom) changes everything is dropped: the sprites are
//! rebuilt at the new `art_px`.

use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Instant;

use gpui::{App, Global, RenderImage, Window};
use image::{Frame, RgbaImage};
use psytify_design::sprite::{self, Pixels, SpriteSheet, UpscaleCache};
use psytify_design::tokens::color::Rgba;
use smallvec::SmallVec;

/// Immagine pronta da dipingere, con la dimensione in pixel fisici.
#[derive(Clone)]
pub struct Texture {
    pub image: Arc<RenderImage>,
    pub width: u32,
    pub height: u32,
}

struct ImageEntry {
    /// Si riconosce una voce uscita dalla cache perché i suoi pixel non hanno più proprietari.
    pixels: Weak<Pixels>,
    texture: Texture,
}

/// Cache globale degli sprite ingranditi e orologio delle animazioni.
pub struct SpriteStore {
    sheet: Option<&'static SpriteSheet>,
    cache: UpscaleCache,
    images: HashMap<usize, ImageEntry>,
    retired: Vec<Arc<RenderImage>>,
    scale_bits: u32,
    started: Instant,
    /// Movimento ridotto chiesto dall'ambiente (`PSYTIFY_REDUCE_MOTION`): vale sempre.
    reduce_motion: bool,
    /// Movimento ridotto voluto dalle impostazioni (la preferenza `motion` di `config.json`) e, se la
    /// preferenza segue il sistema, da ciò che il sistema chiede (`settings_page::follow_motion`).
    reduce_motion_pref: bool,
    /// Il ridisegno programmato (uno solo efficace per volta, vedi `anim`).
    pub(super) refresh: super::anim::RefreshClock,
}

impl Global for SpriteStore {}

impl SpriteStore {
    /// Crea il deposito. Se il foglio incorporato non si decodifica (bug di build) lo segnala una volta
    /// su stderr e gli sprite non si disegnano, ma l'app continua.
    pub fn new(reduce_motion: bool) -> Self {
        let sheet = match sprite::sheet() {
            Ok(sheet) => Some(sheet),
            Err(error) => {
                eprintln!("psytify: sprite non disponibili: {error}");
                None
            }
        };
        Self {
            sheet,
            cache: UpscaleCache::new(),
            images: HashMap::new(),
            retired: Vec::new(),
            scale_bits: 0,
            started: Instant::now(),
            reduce_motion,
            reduce_motion_pref: false,
            refresh: super::anim::RefreshClock::default(),
        }
    }

    /// Installa il deposito come global dell'app.
    pub fn install(cx: &mut App, reduce_motion: bool) {
        cx.set_global(Self::new(reduce_motion));
    }

    pub fn sheet(&self) -> Option<&'static SpriteSheet> {
        self.sheet
    }

    /// Con `reduce_motion` gli sprite restano sul frame 0 e non si programmano ridisegni; le altre
    /// animazioni (lo scorrimento del testo, il lampeggio del cursore) diventano tagli.
    pub fn reduce_motion(&self) -> bool {
        self.reduce_motion || self.reduce_motion_pref
    }

    /// La scelta delle impostazioni. Chi la cambia ridisegna le finestre (`App::refresh_windows`): gli
    /// sprite fermi ripartono, quelli in moto si fermano al frame 0.
    pub fn set_reduce_motion_pref(&mut self, on: bool) -> bool {
        let changed = self.reduce_motion_pref != on;
        self.reduce_motion_pref = on;
        changed
    }

    /// Il movimento ridotto è imposto dall'ambiente: l'interruttore delle impostazioni non lo toglie.
    pub fn reduce_motion_forced(&self) -> bool {
        self.reduce_motion
    }

    /// Millisecondi dall'avvio: l'orologio comune a tutte le animazioni degli sprite.
    pub fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Voci e byte della cache dei pixel (per la galleria).
    pub fn cache_stats(&self) -> (usize, usize) {
        (self.cache.len(), self.cache.bytes())
    }

    /// Drops everything if the effective scale (window scale times zoom) changed since the last paint.
    pub fn ensure_scale(&mut self, scale: f32) {
        let bits = scale.to_bits();
        if bits != self.scale_bits {
            self.scale_bits = bits;
            self.cache.clear();
            self.retired.extend(self.images.drain().map(|(_, entry)| entry.texture.image));
        }
    }

    /// Toglie dall'atlante le immagini scartate (da chiamare mentre si disegna).
    pub fn drain_retired(&mut self, window: &mut Window) {
        for image in self.retired.drain(..) {
            // L'errore possibile è solo «immagine sconosciuta»: non c'è nulla da recuperare.
            let _ = window.drop_image(image);
        }
    }

    /// Frame `frame` dello sprite `name` ingrandito di `ap_device`, tinto se è una maschera.
    pub fn texture(
        &mut self,
        name: &str,
        frame: usize,
        ap_device: u32,
        tint: Option<Rgba>,
    ) -> Option<Texture> {
        let sprite = self.sheet?.get(name)?;
        let pixels = self.cache.get_or_upscale(sprite, frame, ap_device, tint)?;
        let key = Arc::as_ptr(&pixels) as usize;
        if let Some(entry) = self.images.get(&key)
            && entry.pixels.strong_count() > 0
        {
            return Some(entry.texture.clone());
        }
        self.prune();
        let texture = to_texture(&pixels)?;
        self.images.insert(key, ImageEntry { pixels: Arc::downgrade(&pixels), texture: texture.clone() });
        Some(texture)
    }

    /// Scarta le immagini le cui voci di cache sono state sfrattate.
    fn prune(&mut self) {
        let dead: Vec<usize> = self
            .images
            .iter()
            .filter(|(_, entry)| entry.pixels.strong_count() == 0)
            .map(|(key, _)| *key)
            .collect();
        for key in dead {
            if let Some(entry) = self.images.remove(&key) {
                self.retired.push(entry.texture.image);
            }
        }
    }
}

/// RGBA non premoltiplicato -> BGRA come lo vuole l'atlante di GPUI.
fn to_texture(pixels: &Pixels) -> Option<Texture> {
    let mut bgra = pixels.rgba.clone();
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let buffer = RgbaImage::from_raw(pixels.width, pixels.height, bgra)?;
    let image = Arc::new(RenderImage::new(SmallVec::from_elem(Frame::new(buffer), 1)));
    Some(Texture { image, width: pixels.width, height: pixels.height })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_key_returns_the_same_texture_and_a_new_scale_rebuilds() {
        let mut store = SpriteStore::new(false);
        let a = store.texture(sprite::ICON_PLAY, 0, 2, None);
        let b = store.texture(sprite::ICON_PLAY, 0, 2, None);
        let (Some(a), Some(b)) = (a, b) else { return };
        assert!(Arc::ptr_eq(&a.image, &b.image));
        assert_eq!((a.width, a.height), (24, 24));
        let (entries, bytes) = store.cache_stats();
        assert_eq!(entries, 1);
        assert_eq!(bytes, 24 * 24 * 4);
        store.ensure_scale(1.5);
        assert_eq!(store.cache_stats(), (0, 0));
        let c = store.texture(sprite::ICON_PLAY, 0, 3, None);
        assert_eq!(c.map(|t| t.width), Some(36));
    }

    #[test]
    fn upscaled_masks_are_tinted_and_stay_two_colour() {
        let mut store = SpriteStore::new(false);
        let tint = psytify_design::tokens::color::ICON_ACTIVE;
        let Some(sheet) = store.sheet() else { return };
        let Some(sprite) = sheet.get(sprite::ICON_PLAY) else { return };
        let Some(pixels) = store.cache.get_or_upscale(sprite, 0, 3, Some(tint)) else { return };
        let mut colors: Vec<[u8; 4]> = pixels.rgba.as_chunks::<4>().0.to_vec();
        colors.sort_unstable();
        colors.dedup();
        assert_eq!(colors.len(), 2, "trasparente e tinta, nessun colore intermedio");
    }
}
