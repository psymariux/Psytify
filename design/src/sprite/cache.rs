//! Cache LRU degli sprite ingranditi. Appartiene al frontend (una per finestra/scala) e va svuotata
//! quando cambia il fattore di scala. Ha due limiti, entrambi dichiarati:
//!
//! - al massimo [`DEFAULT_CAPACITY_ENTRIES`] voci (128);
//! - al massimo [`DEFAULT_CAPACITY_BYTES`] byte di pixel (8 MiB).
//!
//! Il secondo conta davvero: la mascotte 32 x 32 AP a `ap_device` 8 pesa 256 KiB, quindi 128 voci
//! sarebbero 32 MiB. Con 8 MiB ne restano ~32 di quel peso, ben più degli sprite visibili insieme.
//! Una voce più grande dell'intero budget si calcola ma non si conserva.

use std::collections::HashMap;
use std::sync::Arc;

use super::upscale::{MAX_FACTOR, Pixels, upscale_nearest, upscale_tinted};
use super::{Sprite, SpriteKind};
use crate::tokens::color::Rgba;

pub const DEFAULT_CAPACITY_ENTRIES: usize = 128;
pub const DEFAULT_CAPACITY_BYTES: usize = 8 * 1024 * 1024;

/// Chiave: sprite (indice nel foglio), frame, fattore, tinta (solo per le maschere).
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

/// Cache LRU di [`Pixels`] ingranditi, condivisibili con `Arc` (la texture può vivere oltre la voce).
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
    /// Cache con i limiti predefiniti (128 voci, 8 MiB).
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

    /// Byte di pixel attualmente conservati.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn max_entries(&self) -> usize {
        self.max_entries
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Svuota la cache (da chiamare al cambio del fattore di scala).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// Frame ingrandito di `factor`; `None` se il frame non esiste. `tint` vale solo per le maschere
    /// (per gli sprite a colori è ignorato e non entra nella chiave). Senza tinta, una maschera
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
