//! Ciò che un frontend legge del core: un'istantanea immutabile con un `Arc` per dominio. Un dominio
//! che non cambia tiene lo stesso `Arc` da un'istantanea all'altra, così un frontend salta il suo
//! lavoro con `Arc::ptr_eq` (o confrontando le versioni, per chi sta oltre un FFI).

use std::sync::Arc;

pub use crate::offline::{LocalFile, Lost, LostReason, SavedTrack};
#[cfg(feature = "video")]
pub use crate::state::video::{Clip, ClipState, VideoView};
pub use crate::state::{
    catalog::{CatalogEvent, CatalogRaised, CatalogView, HomeView, LibraryListing, OpenLibrary, SearchView},
    downloads::{
        Batch, DownloadsEvent, DownloadsRaised, DownloadsView, Item, ItemState, Refusal, UpdateState,
        UpdateStatus, UpdateView,
    },
    external::{
        ArtistExtras, ExternalEvent, ExternalRaised, ExternalView, ImportLinks, ImportTarget, Importing,
        ImportsView, OpenPlaylist,
    },
    jam::{JamEvent, JamPhase, JamRaised, JamView},
    library::LibraryView,
    playback::{LyricsState, LyricsView, PlaybackEvent, PlaybackView, Raised, SleepTimer},
    settings::SettingsView,
    social::{Account, AuthView, CodeHint, MAX_FOUND, OpenProfile, SocialEvent, SocialRaised, SocialView},
};

/// Lo stato del core visto da fuori, in un certo istante.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Cresce di uno a ogni pubblicazione.
    pub version: u64,
    pub versions: DomainVersions,
    pub settings: Arc<SettingsView>,
    pub library: Arc<LibraryView>,
    pub playback: Arc<PlaybackView>,
    pub catalog: Arc<CatalogView>,
    pub external: Arc<ExternalView>,
    pub downloads: Arc<DownloadsView>,
    pub social: Arc<SocialView>,
    pub jam: Arc<JamView>,
    #[cfg(feature = "video")]
    pub video: Arc<VideoView>,
}

/// Una versione per dominio: cresce quando la sua vista cambia.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DomainVersions {
    pub settings: u64,
    pub library: u64,
    pub playback: u64,
    pub catalog: u64,
    pub external: u64,
    pub downloads: u64,
    pub social: u64,
    pub jam: u64,
    #[cfg(feature = "video")]
    pub video: u64,
}
