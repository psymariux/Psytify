//! Design tokens as plain constants. Values and names follow `docs/design/PSYTIFY_DESIGN_TOKENS.md`:
//! they change together with that document. Lengths in dp (`f32`), durations in ms (`u32`), colours `Rgba`.
//! Frontends use the semantic aliases (`color::TEXT_PRIMARY`), never the raw greys.

pub mod color;
pub mod interaction;
pub mod motion;
pub mod shape;
pub mod size;
pub mod space;
pub mod type_;
pub mod z;
pub mod zoom;

pub use color::Rgba;
