//! Covers as GPUI textures: one global deposit, bounded, with LRU eviction.
//!
//! The core publishes only addresses (`Track::cover_url`); the frontend decides which images it needs. A
//! cover asked for the first time starts a task: `Core::artwork` fetches it from the backend and decodes it
//! on the core's runtime (an RGBA square of at most 256 px), a background thread makes two smaller versions
//! (128 and 64 px, Catmull-Rom) and the deposit keeps them as `RenderImage`. GPUI's sampler is linear and
//! 256 px at 32 or 48 dp would shimmer: the smallest version that covers the device pixels is picked. Until
//! the image is there, or if it never comes, the drawn placeholder stays (no fake image).
//!
//! A cover drawn larger than 128 device px (tiles, headers, the focus view: drawn side x zoom x scale) also
//! asks for a tier: 256 px (up to 320 device px) or 512 px. The 256 tier comes from the same answer as the small versions when it
//! is asked for before they arrive, otherwise from a request of its own; 512 always asks the core for it
//! (`Core::artwork_side`). Until a tier is ready the next smaller one is drawn.
//!
//! Bounds: [`MAX_READY`] small covers (about 80 KB each, ~10 MB), [`MAX_KNOWN`] remembered addresses
//! (failures included, never retried); tiers at most [`MAX_TIERS`] images and [`MAX_TIER_BYTES`]. An evicted
//! image goes to `retired`, and the next paint removes it from the atlas with `drop_image`.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, AppContext, AsyncApp, Entity, Global, RenderImage, Window};
use image::{Frame, RgbaImage, imageops};
use psytify_core::artwork::CoverPixels;
use smallvec::SmallVec;

use crate::core_model::CoreModel;

/// Small covers kept ready, at most.
pub const MAX_READY: usize = 128;
/// Remembered addresses, at most: beyond, the failures are forgotten (they may be asked again).
const MAX_KNOWN: usize = 1024;
/// Sides of the two small versions, in device pixels.
const SMALL: u32 = 64;
const MEDIUM: u32 = 128;
/// The tiers for covers drawn larger than [`MEDIUM`], in device pixels.
const TIER_256: u32 = 256;
const TIER_512: u32 = 512;
/// Tier images kept ready, at most: a full grid of large tiles on a 2560 px screen stays under it.
pub const MAX_TIERS: usize = 112;
/// Bytes of the tier images, at most (a 256 is 256 KiB, a 512 is 1 MiB).
pub const MAX_TIER_BYTES: usize = 28 * 1024 * 1024;
/// How far a tier may be stretched before the next one is asked for: a 256 serves up to 320 device px, so
/// grid tiles at scale 1.5 stay on 256 (a quarter of the bytes and no full-size decode each) and only
/// headers and the focus view ask for 512.
const TIER_STRETCH: f32 = 1.25;
/// Remembered tier requests (failed or ready), at most.
const MAX_TIERS_KNOWN: usize = 512;

/// A ready cover: the small and the medium version, already in BGRA as the atlas wants them.
#[derive(Clone)]
pub struct CoverArt {
    small: Arc<RenderImage>,
    medium: Arc<RenderImage>,
}

impl CoverArt {
    /// The smallest version that covers `device_px` device pixels of side.
    pub fn for_device_px(&self, device_px: f32) -> Arc<RenderImage> {
        if device_px <= SMALL as f32 { Arc::clone(&self.small) } else { Arc::clone(&self.medium) }
    }
}

/// The tier a cover drawn `device_px` device pixels wide asks for, if the small versions are not enough.
pub fn tier_for(device_px: f32) -> Option<u32> {
    if !device_px.is_finite() || device_px <= MEDIUM as f32 {
        None
    } else if device_px <= TIER_256 as f32 * TIER_STRETCH {
        Some(TIER_256)
    } else {
        Some(TIER_512)
    }
}

enum Entry {
    Loading,
    Ready { art: CoverArt, used: u64 },
    Failed,
}

/// What the deposit knows of an address.
enum Lookup {
    Ready(CoverArt),
    /// Arriving or failed: not asked again.
    Pending,
    /// Never asked.
    Unknown,
}

enum TierEntry {
    /// `from_base`: filled by the answer of the small versions, no request of its own.
    Loading {
        from_base: bool,
    },
    Ready {
        image: Arc<RenderImage>,
        bytes: usize,
        used: u64,
    },
    Failed,
}

/// The deposit logic, without GPUI: entries, use clock, bounds and eviction.
#[derive(Default)]
struct Cache {
    entries: HashMap<String, Entry>,
    /// Use clock: every access takes the next number.
    clock: u64,
    /// Evicted images, to drop from the atlas at the next paint.
    retired: Vec<Arc<RenderImage>>,
    tiers: HashMap<(String, u32), TierEntry>,
    tier_bytes: usize,
}

impl Cache {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    fn lookup(&mut self, url: &str) -> Lookup {
        let now = self.tick();
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
        let used = self.tick();
        let entry = match art {
            Some(art) => Entry::Ready { art, used },
            None => Entry::Failed,
        };
        self.entries.insert(url, entry);
        self.evict();
    }

    /// Beyond [`MAX_READY`] evicts the least recently used covers; beyond [`MAX_KNOWN`] addresses forgets
    /// the failures too.
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

    /// The best ready tier of `url` up to `side`, and whether `side` itself was never asked for.
    fn tier_lookup(&mut self, url: &str, side: u32) -> (Option<Arc<RenderImage>>, bool) {
        let now = self.tick();
        let mut unknown = false;
        for tier in [TIER_512, TIER_256].into_iter().filter(|tier| *tier <= side) {
            match self.tiers.get_mut(&(url.to_owned(), tier)) {
                Some(TierEntry::Ready { image, used, .. }) => {
                    *used = now;
                    return (Some(Arc::clone(image)), unknown);
                }
                Some(TierEntry::Loading { .. } | TierEntry::Failed) => {}
                None => unknown |= tier == side,
            }
        }
        (None, unknown)
    }

    fn tier_begin(&mut self, url: &str, side: u32, from_base: bool) {
        self.tiers.insert((url.to_owned(), side), TierEntry::Loading { from_base });
    }

    /// The 256 tier of `url` waits for the answer of the small versions.
    fn tier_waits_on_base(&self, url: &str) -> bool {
        matches!(self.tiers.get(&(url.to_owned(), TIER_256)), Some(TierEntry::Loading { from_base: true }))
    }

    /// A tier answer. Kept only if it is still awaited: one evicted meanwhile never entered the atlas.
    fn tier_store(&mut self, url: &str, side: u32, image: Option<Arc<RenderImage>>) {
        let key = (url.to_owned(), side);
        if !matches!(self.tiers.get(&key), Some(TierEntry::Loading { .. })) {
            return;
        }
        let used = self.tick();
        let entry = match image {
            Some(image) => {
                let size = image.size(0);
                let bytes = usize::try_from(size.width.0.max(0) * size.height.0.max(0) * 4).unwrap_or(0);
                self.tier_bytes += bytes;
                TierEntry::Ready { image, bytes, used }
            }
            None => TierEntry::Failed,
        };
        self.tiers.insert(key, entry);
        self.tier_evict();
    }

    fn tier_ready_len(&self) -> usize {
        self.tiers.values().filter(|entry| matches!(entry, TierEntry::Ready { .. })).count()
    }

    /// Beyond [`MAX_TIERS`] images or [`MAX_TIER_BYTES`] evicts the least recently used tiers; beyond
    /// [`MAX_TIERS_KNOWN`] requests forgets the failures too.
    fn tier_evict(&mut self) {
        while self.tier_ready_len() > MAX_TIERS || self.tier_bytes > MAX_TIER_BYTES {
            let oldest = self
                .tiers
                .iter()
                .filter_map(|(key, entry)| match entry {
                    TierEntry::Ready { used, .. } => Some((*used, key.clone())),
                    _ => None,
                })
                .min();
            let Some((_, key)) = oldest else { break };
            if let Some(TierEntry::Ready { image, bytes, .. }) = self.tiers.remove(&key) {
                self.tier_bytes = self.tier_bytes.saturating_sub(bytes);
                self.retired.push(image);
            }
        }
        if self.tiers.len() > MAX_TIERS_KNOWN {
            self.tiers.retain(|_, entry| !matches!(entry, TierEntry::Failed));
        }
    }
}

/// The global cover deposit.
pub struct Covers {
    core: Entity<CoreModel>,
    cache: Cache,
}

impl Global for Covers {}

impl Covers {
    pub fn install(cx: &mut App, core: Entity<CoreModel>) {
        cx.set_global(Self { core, cache: Cache::default() });
    }

    /// Reaches the deposit if it is installed (the gallery has none).
    fn with<R>(cx: &mut App, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        cx.has_global::<Self>().then(|| f(cx.global_mut::<Self>()))
    }

    /// The cover of `url` if it is ready. If it was never asked for it asks (once) and returns `None`: on
    /// arrival the window redraws by itself. Without a deposit (the gallery) always `None`.
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
        // This runs while drawing: the real work (network, decoding, resizing) happens elsewhere.
        cx.spawn(async move |cx| {
            let pixels = match answer {
                Some(answer) => answer.await.ok().flatten(),
                None => None,
            };
            // The whole image is made only for a tier that waits on this answer (set by the same draw).
            let whole = cx.update(|cx| Covers::with(cx, |covers| covers.cache.tier_waits_on_base(&url)));
            let whole = whole.unwrap_or(false);
            let built = match pixels {
                Some(pixels) => cx.background_spawn(async move { build(&pixels, whole) }).await,
                None => None,
            };
            arrived(cx, url, built);
        })
        .detach();
        None
    }

    /// The best tier of `url` for a cover drawn `device_px` device pixels wide, if one is ready; asks for
    /// the right one the first time (at most once). `None` when the small versions are enough, while
    /// nothing larger is ready, and without a deposit.
    pub fn tier(url: &str, device_px: f32, cx: &mut App) -> Option<Arc<RenderImage>> {
        let side = tier_for(device_px)?;
        let (ready, core) = Self::with(cx, |covers| {
            let (ready, unknown) = covers.cache.tier_lookup(url, side);
            if !unknown {
                return (ready, None);
            }
            // The small versions are on their way: the 256 tier rides on the same answer.
            let from_base = side == TIER_256 && matches!(covers.cache.lookup(url), Lookup::Pending);
            covers.cache.tier_begin(url, side, from_base);
            (ready, (!from_base).then(|| covers.core.clone()))
        })?;
        if let Some(core) = core {
            let answer = core.read(cx).artwork_side(url.to_owned(), side);
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
                    Covers::with(cx, |covers| covers.cache.tier_store(&url, side, image));
                    cx.refresh_windows();
                });
            })
            .detach();
        }
        ready
    }

    /// Drops the evicted images from the atlas (to call while painting).
    pub fn drain_retired(window: &mut Window, cx: &mut App) {
        let retired = Self::with(cx, |covers| std::mem::take(&mut covers.cache.retired)).unwrap_or_default();
        for image in retired {
            // The only possible error is «unknown image»: nothing to recover.
            let _ = window.drop_image(image);
        }
    }
}

/// What one answer of the core gives: the small versions, and the 256 tier when it was waited for.
struct Built {
    art: CoverArt,
    whole: Option<Arc<RenderImage>>,
}

/// A cover arrived (or failed): it is kept and the window redraws.
fn arrived(cx: &AsyncApp, url: String, built: Option<Built>) {
    cx.update(|cx| {
        Covers::with(cx, |covers| {
            if covers.cache.tier_waits_on_base(&url) {
                let whole = built.as_ref().and_then(|built| built.whole.clone());
                covers.cache.tier_store(&url, TIER_256, whole);
            }
            covers.cache.store(url, built.map(|built| built.art));
        });
        cx.refresh_windows();
    });
}

/// The two small versions of a decoded cover, and with `whole` the whole one for the 256 tier when the
/// source is larger than the medium version. `None` if the pixels are not a valid square.
fn build(pixels: &CoverPixels, whole: bool) -> Option<Built> {
    let source = RgbaImage::from_raw(pixels.side, pixels.side, pixels.rgba.to_vec())?;
    let medium = scaled(&source, MEDIUM);
    let small = scaled(&medium, SMALL);
    let art = CoverArt { small: texture(small)?, medium: texture(medium)? };
    let whole = if whole && source.width() > MEDIUM { texture(source) } else { None };
    Some(Built { art, whole })
}

/// The whole cover, as it came from the core. `None` if the pixels are not a valid square.
fn whole(pixels: &CoverPixels) -> Option<Arc<RenderImage>> {
    texture(RgbaImage::from_raw(pixels.side, pixels.side, pixels.rgba.to_vec())?)
}

/// Resizes to a square of side `side`, if the image is larger (never enlarges).
fn scaled(image: &RgbaImage, side: u32) -> RgbaImage {
    if image.width() <= side {
        return image.clone();
    }
    imageops::resize(image, side, side, imageops::FilterType::CatmullRom)
}

/// RGBA -> BGRA as GPUI's atlas wants it.
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

    fn side_of(image: &Arc<RenderImage>) -> i32 {
        image.size(0).width.0
    }

    #[test]
    fn a_cover_gets_two_smaller_versions_and_the_right_one_is_picked() {
        let built = build(&pixels(256), true).expect("a square cover");
        let art = built.art;
        assert_eq!(side_of(&art.small), 64);
        assert_eq!(side_of(&art.medium), 128);
        assert_eq!(side_of(&art.for_device_px(48.0)), 64);
        assert_eq!(side_of(&art.for_device_px(64.0)), 64);
        assert_eq!(side_of(&art.for_device_px(72.0)), 128);
        assert_eq!(built.whole.as_ref().map(side_of), Some(256));
    }

    #[test]
    fn a_small_source_is_never_enlarged() {
        let built = build(&pixels(40), true).expect("a small cover");
        assert_eq!(side_of(&built.art.medium), 40);
        assert_eq!(side_of(&built.art.small), 40);
        assert!(built.whole.is_none(), "no tier from a source the medium version already holds");
    }

    #[test]
    fn pixels_that_do_not_fit_a_square_are_refused() {
        let broken = CoverPixels { side: 10, rgba: vec![0u8; 12].into(), tint: [0, 0, 0] };
        assert!(build(&broken, true).is_none());
    }

    #[test]
    fn the_tier_follows_the_drawn_device_pixels() {
        assert_eq!(tier_for(40.0), None);
        assert_eq!(tier_for(128.0), None);
        assert_eq!(tier_for(168.0), Some(256), "a 168 dp tile at 1x");
        assert_eq!(tier_for(256.0), Some(256));
        assert_eq!(tier_for(200.0 * 1.5), Some(256), "a stretched grid tile at scale 1.5");
        assert_eq!(tier_for(232.0 * 1.5), Some(512), "the album header at zoom 1.5");
        assert_eq!(tier_for(f32::NAN), None);
    }

    #[test]
    fn the_cache_keeps_at_most_max_ready_covers_and_retires_the_least_recent() {
        let art = build(&pixels(64), false).expect("cover").art;
        let mut cache = Cache::default();
        for n in 0..MAX_READY {
            cache.store(format!("u{n}"), Some(art.clone()));
        }
        // `u0` is the oldest, but it is used: the oldest becomes `u1`.
        assert!(matches!(cache.lookup("u0"), Lookup::Ready(_)));
        for n in 0..5 {
            cache.store(format!("new{n}"), Some(art.clone()));
        }
        assert_eq!(cache.ready_len(), MAX_READY);
        assert!(matches!(cache.lookup("u0"), Lookup::Ready(_)));
        assert!(matches!(cache.lookup("u1"), Lookup::Unknown), "evicted");
        assert!(matches!(cache.lookup("new4"), Lookup::Ready(_)));
        assert_eq!(cache.retired.len(), 10, "two images per evicted cover");
    }

    #[test]
    fn tiers_are_bounded_by_bytes_and_the_least_recent_is_retired() {
        let big = whole(&pixels(512)).expect("cover");
        let mut cache = Cache::default();
        let fits = MAX_TIER_BYTES / (512 * 512 * 4);
        for n in 0..fits {
            let url = format!("u{n}");
            cache.tier_begin(&url, TIER_512, false);
            cache.tier_store(&url, TIER_512, Some(Arc::clone(&big)));
        }
        assert_eq!(cache.tier_bytes, MAX_TIER_BYTES);
        // `u0` used now: the next one in evicts `u1`.
        assert!(cache.tier_lookup("u0", TIER_512).0.is_some());
        cache.tier_begin("new", TIER_512, false);
        cache.tier_store("new", TIER_512, Some(Arc::clone(&big)));
        assert!(cache.tier_bytes <= MAX_TIER_BYTES);
        assert!(cache.tier_lookup("u1", TIER_512).0.is_none(), "evicted");
        assert!(cache.tier_lookup("u0", TIER_512).0.is_some());
        assert_eq!(cache.retired.len(), 1);
    }

    #[test]
    fn tiers_are_bounded_by_count_too() {
        let mid = whole(&pixels(256)).expect("cover");
        let mut cache = Cache::default();
        for n in 0..=MAX_TIERS {
            let url = format!("u{n}");
            cache.tier_begin(&url, TIER_256, false);
            cache.tier_store(&url, TIER_256, Some(Arc::clone(&mid)));
        }
        assert_eq!(cache.tier_ready_len(), MAX_TIERS);
        assert!(cache.tier_lookup("u0", TIER_256).0.is_none(), "the oldest went");
    }

    #[test]
    fn a_smaller_ready_tier_stands_in_and_a_tier_is_asked_once() {
        let mid = whole(&pixels(256)).expect("cover");
        let mut cache = Cache::default();
        assert!(cache.tier_lookup("a", TIER_512).1, "never asked");
        cache.tier_begin("a", TIER_256, false);
        cache.tier_store("a", TIER_256, Some(mid));
        cache.tier_begin("a", TIER_512, false);
        let (ready, ask) = cache.tier_lookup("a", TIER_512);
        assert_eq!(ready.as_ref().map(side_of), Some(256), "the 256 while the 512 loads");
        assert!(!ask, "already asked");
        cache.tier_store("a", TIER_512, None);
        assert!(!cache.tier_lookup("a", TIER_512).1, "a failure is not asked again");
    }

    #[test]
    fn the_256_tier_rides_on_the_small_versions_and_a_late_answer_is_dropped() {
        let mut cache = Cache::default();
        cache.begin("a");
        cache.tier_begin("a", TIER_256, true);
        assert!(cache.tier_waits_on_base("a"));
        let built = build(&pixels(256), true).expect("cover");
        cache.tier_store("a", TIER_256, built.whole);
        assert!(!cache.tier_waits_on_base("a"));
        assert!(cache.tier_lookup("a", TIER_256).0.is_some());
        // An answer for a request nobody waits for is not kept.
        cache.tier_store("b", TIER_512, whole(&pixels(512)));
        assert!(cache.tier_lookup("b", TIER_512).0.is_none());
        assert_eq!(cache.tier_bytes, 256 * 256 * 4);
    }

    #[test]
    fn a_failed_or_pending_address_is_not_asked_again_and_failures_are_forgotten_in_bulk() {
        let mut cache = Cache::default();
        assert!(matches!(cache.lookup("a"), Lookup::Unknown));
        cache.begin("a");
        assert!(matches!(cache.lookup("a"), Lookup::Pending));
        cache.store("a".into(), None);
        assert!(matches!(cache.lookup("a"), Lookup::Pending), "failed: not retried");
        for n in 0..=MAX_KNOWN {
            cache.store(format!("bad{n}"), None);
        }
        assert!(cache.entries.len() <= MAX_KNOWN, "failures are forgotten beyond the bound");
    }
}
