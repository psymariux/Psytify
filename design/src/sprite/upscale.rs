//! Integer nearest-neighbour upscaling: each art pixel becomes a `k x k` block of pixels
//! fisici identici. Solo aritmetica intera, nessun filtro, nessun colore nuovo.

use super::Frame;
use crate::tokens::color::Rgba;

/// Maximum accepted factor (scale 8.0): above it, the factor is capped at this value.
pub const MAX_FACTOR: u32 = 16;

/// Non-premultiplied RGBA8 image, contiguous rows (`width * height * 4` bytes).
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

/// Upscales `frame` by `factor` (0 means 1; maximum [`MAX_FACTOR`]).
pub fn upscale_nearest(frame: &Frame, factor: u32) -> Pixels {
    upscale_rgba(frame, factor, |px| px)
}

/// Like [`upscale_nearest`] but first tints the mask: every non-transparent pixel takes the colour of
/// `tint`, with alpha = pixel alpha x tint alpha. Transparent pixels stay transparent.
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
