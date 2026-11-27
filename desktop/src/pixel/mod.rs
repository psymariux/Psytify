// Come per `components`: parte dell'API (cornici piene, frame fissi) attende le schermate vere.
#![allow(dead_code)]

//! Pixel art sul GPUI: sprite nitidi (`PixelSprite`), cache degli sprite ingranditi (`SpriteStore`),
//! orologio delle animazioni (`anim`) e le forme a tacca di bordi, ledge e anello di focus (`shape`).

pub mod anim;
pub mod shape;
pub mod sprite;
pub mod store;

pub use sprite::PixelSprite;
pub use store::SpriteStore;
