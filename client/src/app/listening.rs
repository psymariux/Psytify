//! Ciò che l'app sa di chi ascolta: gli artisti seguiti e gli album salvati (dal passo 4.2 stanno
//! nel core: qui i messaggi diventano comandi), il carosello in cima alla Home, gli album, i mix e
//! le radio da riprodurre. Il timer di spegnimento e la riproduzione automatica a fine coda sono
//! del core dal passo 4.3 (`psytify_core::command::PlaybackCmd`). Il feed della Home (uscite,
//! novità, mix, artisti) lo costruisce il core (`psytify_core::feed::Feed`).

use std::time::{Duration, Instant};

use iced::Task;
use uuid::Uuid;

use psytify_core::command::LibraryCmd;

use super::{App, Message, Notice, Screen};
use crate::config::{FollowedArtist, SavedAlbum};

/// Ogni quanto il carosello passa da solo all'uscita successiva.
pub const HERO_PERIOD: Duration = Duration::from_secs(8);
/// Durata del passaggio da un'uscita all'altra.
pub const HERO_SWAP: Duration = Duration::from_millis(420);

/// Il carosello della Home: quale uscita si vede, e da quando.
#[derive(Debug, Default)]
pub struct Hero {
    pub index: usize,
    pub since: Option<Instant>,
}

impl App {
    pub fn is_following(&self, artist_id: Uuid) -> bool {
        self.followed.iter().any(|a| a.id == artist_id)
    }

    pub fn has_saved(&self, album_id: Uuid) -> bool {
        self.saved_albums.iter().any(|a| a.id == album_id)
    }

    /// Uscite degli artisti seguiti arrivate dopo l'ultima visita a «Novità»: il core le ha già
    /// contate.
    pub fn unread_news(&self) -> usize {
        self.core_snapshot.library.unread_news
    }

    /// Ciò che c'è in «Novità» si considera visto.
    pub(super) fn mark_news_seen(&mut self) {
        self.send_library_quiet(LibraryCmd::MarkNewsSeen);
    }

    pub(super) fn toggle_follow(&mut self, artist: FollowedArtist) -> Task<Message> {
        let name = artist.name.clone();
        // L'esito si conosce prima del core: è l'opposto di com'è adesso.
        let now_following = !self.is_following(artist.id);
        self.notice = Some(Notice::Info(if now_following {
            format!("Segui {name}: le sue nuove uscite compaiono in Novità")
        } else {
            format!("Non segui più {name}")
        }));
        self.send_library(LibraryCmd::ToggleFollow(artist))
    }

    pub(super) fn toggle_save_album(&mut self, album: SavedAlbum) -> Task<Message> {
        let title = album.title.clone();
        let saved = !self.has_saved(album.id);
        self.notice = Some(Notice::Info(if saved {
            format!("«{title}» è nella tua libreria")
        } else {
            format!("«{title}» tolto dalla libreria")
        }));
        self.send_library(LibraryCmd::ToggleSaveAlbum(album))
    }

    /// Porta il carosello all'uscita `index` (e fa partire il passaggio).
    pub(super) fn hero_go(&mut self, index: usize) {
        let count = self.home.feed.hero.len();
        if count == 0 {
            return;
        }
        self.home.hero.index = index % count;
        self.home.hero.since = Some(Instant::now());
    }

    pub(super) fn hero_next(&mut self) {
        self.hero_go(self.home.hero.index + 1);
    }

    /// Il carosello è in passaggio da un'uscita all'altra.
    pub(super) fn hero_swapping(&self) -> bool {
        self.screen == Screen::Home && self.home.hero.since.is_some_and(|since| since.elapsed() < HERO_SWAP)
    }

    /// Il carosello gira da solo: sulla Home e se ha più di un'uscita.
    pub(super) fn hero_rotates(&self) -> bool {
        self.screen == Screen::Home && self.home.feed.hero.len() > 1
    }
}
