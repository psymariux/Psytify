//! LRU cache of upscaled sprites. It belongs to the frontend (one per window/scale) and must be emptied
//! when the scale factor changes. It has two limits, both declared:
//!
//! - al massimo [`DEFAULT_CAPACITY_ENTRIES`] voci (128);
//! - at most [`DEFAULT_CAPACITY_BYTES`] bytes of pixels (8 MiB).
//!
//! The second one really counts: the mascot 32 x 32 AP at `ap_device` 8 weighs 256 KiB, so 128 entries
//! would be 32 MiB. With 8 MiB about 32 of that weight remain, well more than the sprites visible together.
//! An entry bigger than the whole budget is computed but not kept.

use std::collections::HashMap;
use std::sync::Arc;

use super::upscale::{MAX_FACTOR, Pixels, upscale_nearest, upscale_tinted};
use super::{Sprite, SpriteKind};
use crate::tokens::color::Rgba;

pub const DEFAULT_CAPACITY_ENTRIES: usize = 128;
pub const DEFAULT_CAPACITY_BYTES: usize = 8 * 1024 * 1024;

/// Key: sprite (index in the sheet), frame, factor, tint (for masks only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    sprite: usize,
    frame: usize,
    factor: u32,
    tint: Option<Rgba>,
}

struct Entry {
    pixels: Arc<Pixels>,
    last_used: u64,
}

/// LRU cache of upscaled [`Pixels`], shareable with `Arc` (the texture can outlive the entry).
pub struct UpscaleCache {
    max_entries: usize,
    max_bytes: usize,
    entries: HashMap<Key, Entry>,
    bytes: usize,
    tick: u64,
}

impl Default for UpscaleCache {
    fn default() -> Self {
        Self::new()
    }
}

impl UpscaleCache {
    /// Cache with the default limits (128 entries, 8 MiB).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY_ENTRIES, DEFAULT_CAPACITY_BYTES)
    }

    pub fn with_capacity(max_entries: usize, max_bytes: usize) -> Self {
        Self { max_entries, max_bytes, entries: HashMap::new(), bytes: 0, tick: 0 }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Bytes of pixels currently kept.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn max_entries(&self) -> usize {
        self.max_entries
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Empties the cache (call it when the scale factor changes).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// Upscaled frame at `factor`; `None` if the frame does not exist. `tint` only counts for masks
    /// (for coloured sprites it is ignored and does not enter the key). Without a tint, a mask
    /// resta del colore esportato.
    pub fn get_or_upscale(
        &mut self,
        sprite: &Sprite,
        frame: usize,
        factor: u32,
        tint: Option<Rgba>,
    ) -> Option<Arc<Pixels>> {
        let source = sprite.frame(frame)?;
        let tint = if sprite.kind() == SpriteKind::Mask { tint } else { None };
        let key = Key { sprite: sprite.index(), frame, factor: factor.clamp(1, MAX_FACTOR), tint };
        self.tick += 1;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = self.tick;
            return Some(Arc::clone(&entry.pixels));
        }
        let pixels = Arc::new(match tint {
            Some(color) => upscale_tinted(source, key.factor, color),
            None => upscale_nearest(source, key.factor),
        });
        self.insert(key, &pixels);
        Some(pixels)
    }

    fn insert(&mut self, key: Key, pixels: &Arc<Pixels>) {
        let size = pixels.byte_len();
        if size > self.max_bytes || self.max_entries == 0 {
            return;
        }
        while !self.entries.is_empty()
            && (self.entries.len() >= self.max_entries || self.bytes + size > self.max_bytes)
        {
            self.evict_oldest();
        }
        self.bytes += size;
        self.entries.insert(key, Entry { pixels: Arc::clone(pixels), last_used: self.tick });
    }

    fn evict_oldest(&mut self) {
        let oldest = self.entries.iter().min_by_key(|(_, e)| e.last_used).map(|(k, _)| *k);
        if let Some(entry) = oldest.and_then(|k| self.entries.remove(&k)) {
            self.bytes -= entry.pixels.byte_len();
        }
    }
}
