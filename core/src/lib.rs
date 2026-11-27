//! Psytify core: la logica dell'app senza interfaccia, condivisa dai frontend (Iced oggi, GPUI per
//! il desktop e Kotlin su Android in seguito). Non dipende da nessun framework grafico: i frontend
//! avviano un [`Core`], gli mandano [`Command`] e leggono le [`Snapshot`] che pubblica.
//!
//! Il piano dello spostamento è in `docs/transformation/04-core-architecture.md`.

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
/// Il lato del videoclip che i frontend usano direttamente: le regole di sincronia con l'audio, pure,
/// da applicare al loro player (il resto, che videoclip e quando, è `VideoCmd` e `VideoView`).
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
