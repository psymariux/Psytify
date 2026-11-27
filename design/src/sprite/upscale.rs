//! Ingrandimento intero nearest-neighbour: ogni pixel d'arte diventa un blocco `k x k` di pixel
//! fisici identici. Solo aritmetica intera, nessun filtro, nessun colore nuovo.

use super::Frame;
use crate::tokens::color::Rgba;

/// Fattore massimo accettato (scala 8,0): oltre, il fattore si limita a questo valore.
pub const MAX_FACTOR: u32 = 16;

/// Immagine RGBA8 non premoltiplicata, righe contigue (`width * height * 4` byte).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Pixels {
    /// Byte occupati dai pixel.
    pub fn byte_len(&self) -> usize {
        self.rgba.len()
    }
}

/// Ingrandisce `frame` di `factor` (0 vale 1; massimo [`MAX_FACTOR`]).
pub fn upscale_nearest(frame: &Frame, factor: u32) -> Pixels {
    upscale_rgba(frame, factor, |px| px)
}

/// Come [`upscale_nearest`] ma tinge prima la maschera: ogni pixel non trasparente prende il colore di
/// `tint`, con alfa = alfa del pixel x alfa del tint. I pixel trasparenti restano tali.
pub fn upscale_tinted(frame: &Frame, factor: u32, tint: Rgba) -> Pixels {
    let [tr, tg, tb, ta] = tint.bytes();
    upscale_rgba(frame, factor, |px| {
        if px[3] == 0 {
            px
        } else {
            let alpha = (u16::from(px[3]) * u16::from(ta) + 127) / 255;
            [tr, tg, tb, alpha as u8]
        }
    })
}

fn upscale_rgba(frame: &Frame, factor: u32, map: impl Fn([u8; 4]) -> [u8; 4]) -> Pixels {
    let k = factor.clamp(1, MAX_FACTOR) as usize;
    let (w, h) = (frame.width as usize, frame.height as usize);
    let row_bytes = w * k * 4;
    let mut rgba = Vec::with_capacity(row_bytes * h * k);
    let mut row = Vec::with_capacity(row_bytes);
    for src_row in frame.rgba.chunks_exact(w * 4).take(h) {
        row.clear();
        for px in src_row.as_chunks::<4>().0 {
            let mapped = map(*px);
            for _ in 0..k {
                row.extend_from_slice(&mapped);
            }
        }
        for _ in 0..k {
            rgba.extend_from_slice(&row);
        }
    }
    Pixels { width: frame.width * k as u32, height: frame.height * k as u32, rgba }
}
