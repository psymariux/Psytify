//! Token del design come costanti semplici. Valori e nomi seguono `docs/design/PSYTIFY_DESIGN_TOKENS.md`:
//! cambiano insieme a quel documento. Lunghezze in dp (`f32`), durate in ms (`u32`), colori `Rgba`.
//! I frontend usano gli alias semantici (`color::TEXT_PRIMARY`), mai i grigi grezzi.

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
