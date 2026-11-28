//! Psytify design: the design tokens (colours `[u8; 4]`, spaces, typography, layout, motion) and the
//! sprite sheets with the integer nearest-neighbour upscaler, as plain data without a graphics framework.
//! Each frontend converts them to its own types, so the Kotlin app (Android) and GPUI (desktop)
//! share the same pixel art identity.
//!
//! - [`tokens`]: `pub const` values (palette, spaces, shapes, types, layout, motion, z levels).
//!   Source of the values: `docs/design/PSYTIFY_DESIGN_TOKENS.md`.
//! - [`scale`]: device pixels per pixel art pixel (`art_px`) and grid alignment.
//! - [`sprite`]: embedded sprite sheet (decoded once), integer upscaling and an LRU cache
//!   with a declared capacity.
//! - [`contrast`]: relative luminance and WCAG 2.x contrast ratio.
//! - [`brand`]: the monochrome SVG marks of the access providers (the only vector exception).
//!
//! No GPUI or Kotlin type enters here; the full pipeline is in
//! `docs/design/PSYTIFY_PIXEL_ART_GUIDELINES.md` (section 10).

pub mod brand;
pub mod contrast;
pub mod scale;
pub mod sprite;
pub mod tokens;
