//! I comandi che un frontend manda al core con [`Core::send`](crate::Core::send). Ogni dominio ha
//! il suo enum, dichiarato nel suo modulo dello stato: un dominio che arriva aggiunge varianti solo
//! lì, mai qui.

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

/// Un comando per il core, diviso per dominio.
#[derive(Debug, Clone)]
pub enum Command {
    Settings(SettingsCmd),
    /// Nello heap: un comando della libreria porta brani interi e farebbe pesare così ogni `Command`.
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
    /// Un comando della libreria.
    pub fn library(command: LibraryCmd) -> Self {
        Self::Library(Box::new(command))
    }
}
