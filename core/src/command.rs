//! The commands a frontend sends to the core with [`Core::send`](crate::Core::send). Each domain has
//! its own enum, declared in its module of the state: a new domain adds variants only there,
//! never here.

#[cfg(feature = "video")]
pub use crate::state::video::VideoCmd;
pub use crate::state::{
    catalog::{CatalogCmd, Resource},
    downloads::{DownloadsCmd, UpdateCheck},
    external::ExternalCmd,
    jam::JamCmd,
    library::LibraryCmd,
    playback::{Align, PlaybackCmd, SleepChoice},
    settings::SettingsCmd,
    social::SocialCmd,
    system::Artwork,
};

/// A command for the core, split by domain.
#[derive(Debug, Clone)]
pub enum Command {
    Settings(SettingsCmd),
    /// On the heap: a library command carries whole tracks, and would otherwise make every `Command` heavy.
    Library(Box<LibraryCmd>),
    Playback(PlaybackCmd),
    Catalog(CatalogCmd),
    External(ExternalCmd),
    Downloads(DownloadsCmd),
    Social(SocialCmd),
    Jam(JamCmd),
    #[cfg(feature = "video")]
    Video(VideoCmd),
}

impl Command {
    /// A library command.
    pub fn library(command: LibraryCmd) -> Self {
        Self::Library(Box::new(command))
    }
}
