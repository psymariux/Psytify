//! Psytify core: the app's logic without an interface, shared by the frontends (GPUI on the desktop,
//! Kotlin on Android). It depends on no graphics framework: the frontends
//! start a [`Core`], send it [`Command`]s and read the [`Snapshot`]s it publishes.
//!
//! The move plan is in `docs/transformation/04-core-architecture.md`.

pub mod api;
pub mod artwork;
pub mod auth;
pub mod catalog;
pub mod command;
pub mod config;
#[cfg(not(target_os = "android"))]
pub mod discord;
mod effect;
pub mod error;
pub mod feed;
mod host;
pub mod jam;
#[cfg(target_os = "linux")]
pub mod launch;
pub mod media;
pub mod offline;
pub mod platform;
pub mod player;
pub mod shared;
pub mod snapshot;
mod state;
pub mod system_motion;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
/// The side of the music video that the frontends use directly: the audio sync rules, pure,
/// to apply to their player (the rest, which video and when, is `VideoCmd` and `VideoView`).
#[cfg(feature = "video")]
pub mod video {
    pub use crate::state::video::{
        AHEAD, Adjust, Alignment, DRIFT, FULL_HEIGHT, MAX_LEAD, MAX_UPGRADES, Measure, RESYNC_PAUSE, SETTLE,
        UPGRADE_WAIT,
    };
}

pub use command::Command;
pub use host::{AudioChoice, Core, CoreError, CoreOptions, RuntimeChoice, SystemChoice};
pub use snapshot::Snapshot;
