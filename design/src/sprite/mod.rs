//! Sprite pixel art: foglio incorporato (PNG indicizzati 1x + `sprites.json`), decodificato una sola
//! volta, ingrandimento intero nearest-neighbour e cache LRU con capacità dichiarata.
//!
//! Perché si ingrandisce su CPU: il campionatore di GPUI è sempre lineare (nessun filtro «nearest»),
//! quindi uno sprite ingrandito dalla GPU sfocherebbe. Il frontend ingrandisce di un fattore intero
//! (`scale::art_px`), dipinge la texture 1:1 su un'origine allineata al pixel fisico e il campionatore
//! diventa l'identità (`docs/transformation/03-gpui-feasibility.md`, «pixel-scaling»).
//!
//! # Memoria
//!
//! I 74 sprite 1x occupano **160 896 byte** (157,1 KiB) di RGBA decodificato, più qualche KiB di nomi e
//! indici; i PNG incorporati nel binario ne pesano circa 60 KiB. Si decodificano una volta sola
//! (`sheet()`, `OnceLock`), mai per frame. Le versioni ingrandite stanno nella [`UpscaleCache`], che
//! il frontend possiede (un fattore di scala per volta): vedi i limiti in `cache`.

mod cache;
mod names;
mod upscale;

pub use cache::{DEFAULT_CAPACITY_BYTES, DEFAULT_CAPACITY_ENTRIES, UpscaleCache};
pub use names::*;
pub use upscale::{MAX_FACTOR, Pixels, upscale_nearest, upscale_tinted};

use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;

use image::ImageFormat;
use serde::Deserialize;

use crate::tokens::space::AP_DP;

/// `sprites.json` generato da `design/tools/sprites.py`.
const MANIFEST_JSON: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pixel/export/sprites.json"));

/// Come si colora uno sprite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpriteKind {
    /// Maschera a 1 bit (icone, `eq`, `mark`, `liked_cover`): l'indice 1 va tinto con un token per stato.
    Mask,
    /// Colori fissi della palette (mascotte, illustrazioni, segnaposto, loader, icona dell'app).
    Color,
}

/// Un frame decodificato a 1x: RGBA8 non premoltiplicato, righe contigue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Quanto resta in scena, in ms (0 per gli sprite statici).
    pub duration_ms: u32,
}

/// Uno sprite: dimensioni, tipo e frame.
#[derive(Clone, Debug)]
pub struct Sprite {
    index: usize,
    name: String,
    group: String,
    kind: SpriteKind,
    frame_size: (u32, u32),
    frames: Vec<Frame>,
}

impl Sprite {
    /// Posizione nel foglio (chiave stabile per la cache).
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// `icons`, `motifs` o `mascot`.
    pub fn group(&self) -> &str {
        &self.group
    }

    pub fn kind(&self) -> SpriteKind {
        self.kind
    }

    /// Larghezza e altezza di un frame in pixel d'arte.
    pub fn frame_size(&self) -> (u32, u32) {
        self.frame_size
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn frame(&self, index: usize) -> Option<&Frame> {
        self.frames.get(index)
    }

    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// Durata di un giro completo in ms (0 se statico).
    pub fn cycle_ms(&self) -> u32 {
        self.frames.iter().map(|f| f.duration_ms).sum()
    }

    /// Frame in scena dopo `elapsed_ms` dall'inizio, in ciclo. Gli sprite statici (o con durate
    /// tutte a 0) restano sul frame 0.
    pub fn frame_index_at(&self, elapsed_ms: u64) -> usize {
        let cycle = u64::from(self.cycle_ms());
        if self.frames.len() < 2 || cycle == 0 {
            return 0;
        }
        let mut t = elapsed_ms % cycle;
        for (i, frame) in self.frames.iter().enumerate() {
            let d = u64::from(frame.duration_ms);
            if t < d {
                return i;
            }
            t -= d;
        }
        0
    }

    /// Byte di RGBA decodificato a 1x (tutti i frame).
    pub fn decoded_bytes(&self) -> usize {
        self.frames.iter().map(|f| f.rgba.len()).sum()
    }
}

/// Errore di caricamento del foglio. Con asset incorporati indica un bug di build, non di runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpriteError {
    /// `sprites.json` non si legge o non ha la forma attesa.
    Manifest(String),
    /// Lo sprite del manifest non ha un PNG incorporato (manca la riga in `names.rs`).
    MissingPng(String),
    /// Il PNG non si decodifica.
    Decode { sprite: String, reason: String },
    /// Le dimensioni del PNG, il numero di frame o le durate non tornano col manifest.
    Shape { sprite: String, reason: String },
}

impl fmt::Display for SpriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(reason) => write!(f, "sprites.json non valido: {reason}"),
            Self::MissingPng(name) => write!(f, "sprite `{name}` senza PNG incorporato"),
            Self::Decode { sprite, reason } => write!(f, "decodifica di `{sprite}` fallita: {reason}"),
            Self::Shape { sprite, reason } => write!(f, "sprite `{sprite}` incoerente: {reason}"),
        }
    }
}

impl std::error::Error for SpriteError {}

#[derive(Deserialize)]
struct Manifest {
    ap_dp: f32,
    sprites: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    group: String,
    frame_size: [u32; 2],
    frames: usize,
    durations_ms: Vec<u32>,
    palette: Vec<String>,
}

/// Tutti gli sprite decodificati a 1x.
#[derive(Debug)]
pub struct SpriteSheet {
    sprites: Vec<Sprite>,
    by_name: HashMap<String, usize>,
}

impl SpriteSheet {
    /// Decodifica il foglio incorporato. Costa qualche millisecondo: usa [`sheet`], che lo fa una volta.
    pub fn load_embedded() -> Result<Self, SpriteError> {
        Self::from_parts(MANIFEST_JSON, names::EMBEDDED_PNGS)
    }

    fn from_parts(manifest_json: &[u8], pngs: &[(&str, &[u8])]) -> Result<Self, SpriteError> {
        let manifest: Manifest =
            serde_json::from_slice(manifest_json).map_err(|e| SpriteError::Manifest(e.to_string()))?;
        if (manifest.ap_dp - AP_DP).abs() > f32::EPSILON {
            return Err(SpriteError::Manifest(format!(
                "ap_dp {} diverso da tokens::space::AP_DP {AP_DP}",
                manifest.ap_dp
            )));
        }
        let mut sprites = Vec::with_capacity(manifest.sprites.len());
        let mut by_name = HashMap::with_capacity(manifest.sprites.len());
        for (index, entry) in manifest.sprites.into_iter().enumerate() {
            let bytes = pngs
                .iter()
                .find(|(name, _)| *name == entry.name)
                .map(|(_, bytes)| *bytes)
                .ok_or_else(|| SpriteError::MissingPng(entry.name.clone()))?;
            let sprite = decode_sprite(index, entry, bytes)?;
            if by_name.insert(sprite.name.clone(), index).is_some() {
                return Err(SpriteError::Manifest(format!("sprite `{}` duplicato", sprite.name)));
            }
            sprites.push(sprite);
        }
        Ok(Self { sprites, by_name })
    }

    pub fn len(&self) -> usize {
        self.sprites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sprites.is_empty()
    }

    /// Sprite per nome (usa le costanti: `sheet.get(sprite::ICON_PLAY)`).
    pub fn get(&self, name: &str) -> Option<&Sprite> {
        self.by_name.get(name).and_then(|&i| self.sprites.get(i))
    }

    /// Sprite per posizione.
    pub fn by_index(&self, index: usize) -> Option<&Sprite> {
        self.sprites.get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Sprite> {
        self.sprites.iter()
    }

    /// Byte totali di RGBA decodificato a 1x (160 896 con i 74 sprite attuali).
    pub fn decoded_bytes(&self) -> usize {
        self.sprites.iter().map(Sprite::decoded_bytes).sum()
    }
}

fn shape_error(sprite: &str, reason: impl Into<String>) -> SpriteError {
    SpriteError::Shape { sprite: sprite.to_owned(), reason: reason.into() }
}

fn decode_sprite(index: usize, entry: Entry, png: &[u8]) -> Result<Sprite, SpriteError> {
    let [fw, fh] = entry.frame_size;
    if entry.frames == 0 || fw == 0 || fh == 0 {
        return Err(shape_error(&entry.name, "frame vuoto"));
    }
    if entry.durations_ms.len() != entry.frames {
        return Err(shape_error(&entry.name, "durate e frame in numero diverso"));
    }
    let image = image::load_from_memory_with_format(png, ImageFormat::Png)
        .map_err(|e| SpriteError::Decode { sprite: entry.name.clone(), reason: e.to_string() })?
        .into_rgba8();
    let frame_count = entry.frames as u64;
    if u64::from(image.width()) != u64::from(fw) * frame_count || image.height() != fh {
        return Err(shape_error(
            &entry.name,
            format!("PNG {}x{}, atteso {}x{fh}", image.width(), image.height(), u64::from(fw) * frame_count),
        ));
    }
    // I frame stanno affiancati in orizzontale: si copia riga per riga la loro finestra.
    let (strip_w, fw_us, fh_us) = (image.width() as usize, fw as usize, fh as usize);
    let strip = image.as_raw();
    let mut frames = Vec::with_capacity(entry.frames);
    for (i, duration_ms) in entry.durations_ms.iter().enumerate() {
        let mut rgba = Vec::with_capacity(fw_us * fh_us * 4);
        for row in 0..fh_us {
            let start = (row * strip_w + i * fw_us) * 4;
            let slice = strip
                .get(start..start + fw_us * 4)
                .ok_or_else(|| shape_error(&entry.name, "riga fuori dal PNG"))?;
            rgba.extend_from_slice(slice);
        }
        frames.push(Frame { width: fw, height: fh, rgba, duration_ms: *duration_ms });
    }
    let kind = if entry.palette.len() == 2 && entry.palette[1] == "tint" {
        SpriteKind::Mask
    } else {
        SpriteKind::Color
    };
    Ok(Sprite { index, name: entry.name, group: entry.group, kind, frame_size: (fw, fh), frames })
}

static SHEET: OnceLock<Result<SpriteSheet, SpriteError>> = OnceLock::new();

/// Il foglio incorporato, decodificato alla prima chiamata e poi condiviso. L'errore (asset
/// corrotti: bug di build) è sempre lo stesso a ogni chiamata.
pub fn sheet() -> Result<&'static SpriteSheet, &'static SpriteError> {
    SHEET.get_or_init(SpriteSheet::load_embedded).as_ref()
}

#[cfg(test)]
mod tests;
