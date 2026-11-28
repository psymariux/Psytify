//! The desktop's sprite store: it owns the [`UpscaleCache`] of `psytify-design` (pixels
//! upscaled in RGBA, with a limit of entries and bytes) and, for each entry, the BGRA `RenderImage` that GPUI
//! loads into the atlas. The app owns it (a `Global`), not the single sprites.
//!
//! Why two levels: GPUI wants BGRA and an `Arc<RenderImage>` with a stable identity (each new
//! `RenderImage` ha un id nuovo e quindi un nuovo caricamento nell'atlante). L'immagine BGRA si crea
//! once per cache entry and is dropped together with it; when an entry leaves the cache its
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
    /// An entry that left the cache is recognised because its pixels no longer have owners.
    pixels: Weak<Pixels>,
    texture: Texture,
}

/// Global cache of the upscaled sprites and the animation clock.
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
    /// preference follows the system, from what the system asks (`settings_page::follow_motion`).
    reduce_motion_pref: bool,
    /// The scheduled redraw (only one effective at a time, see `anim`).
    pub(super) refresh: super::anim::RefreshClock,
}

impl Global for SpriteStore {}

impl SpriteStore {
    /// Creates the store. If the embedded sheet does not decode (build bug) it reports it once
    /// on stderr and the sprites are not drawn, but the app goes on.
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

    /// Installs the store as the app's global.
    pub fn install(cx: &mut App, reduce_motion: bool) {
        cx.set_global(Self::new(reduce_motion));
    }

    pub fn sheet(&self) -> Option<&'static SpriteSheet> {
        self.sheet
    }

    /// With `reduce_motion` the sprites stay on frame 0 and no redraw is scheduled; the other
    /// animations (text scrolling, the caret blink) become cuts.
    pub fn reduce_motion(&self) -> bool {
        self.reduce_motion || self.reduce_motion_pref
    }

    /// The settings' choice. Whoever changes it redraws the windows (`App::refresh_windows`): the
    /// sprite fermi ripartono, quelli in moto si fermano al frame 0.
    pub fn set_reduce_motion_pref(&mut self, on: bool) -> bool {
        let changed = self.reduce_motion_pref != on;
        self.reduce_motion_pref = on;
        changed
    }

    /// Reduced motion is imposed by the environment: the settings switch does not remove it.
    pub fn reduce_motion_forced(&self) -> bool {
        self.reduce_motion
    }

    /// Milliseconds since start-up: the clock common to all the sprite animations.
    pub fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Entries and bytes of the pixel cache (for the gallery).
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

    /// Removes the discarded images from the atlas (to call while drawing).
    pub fn drain_retired(&mut self, window: &mut Window) {
        for image in self.retired.drain(..) {
            // The only possible error is "unknown image": there is nothing to recover.
            let _ = window.drop_image(image);
        }
    }

    /// Frame `frame` of the sprite `name` upscaled by `ap_device`, tinted if it is a mask.
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

    /// Drops the images whose cache entries have been evicted.
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

/// Non-premultiplied RGBA -> BGRA as GPUI's atlas wants it.
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
