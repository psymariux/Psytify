// As for `components`: part of the API (full frames, fixed frames) waits for the real screens.
#![allow(dead_code)]

//! Pixel art on GPUI: crisp sprites (`PixelSprite`), cache of the upscaled sprites (`SpriteStore`),
//! the animation clock (`anim`) and the notched shapes of borders, ledges and the focus ring (`shape`).

pub mod anim;
pub mod shape;
pub mod sprite;
pub mod store;

pub use sprite::PixelSprite;
pub use store::SpriteStore;
