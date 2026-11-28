//! What a frontend reads from the core: an immutable snapshot with one `Arc` per domain. A domain
//! that does not change keeps the same `Arc` from one snapshot to the next, so a frontend skips its
//! work with `Arc::ptr_eq` (or by comparing the versions, for those across an FFI).

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

/// The state of the core seen from outside, at a given instant.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Grows by one at each publish.
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

/// One version per domain: it grows when its view changes.
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
