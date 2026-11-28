//! A track's «Altro» menu, with the entries
//! the row does not already have (play, heart and download are on the row): «Riproduci subito dopo», «Aggiungi
//! alla coda», «Avvia radio»; «Aggiungi a una playlist» (the list of playlists, with «Nuova playlist…» at the
//! top) and, in a playlist, «Togli da questa playlist»; «Vai all'artista», «Vai all'album» (if the track
//! has one), «Segui l'artista» / «Smetti di seguire» and «Copia titolo e artista»; then, as in the
//! client, the downloads ([`DownloadItems`]): «Scarica · qualità» and «Scarica in…» (the four qualities, with the
//! tick on the one from the settings) for a downloadable track that is neither already on the device nor
//! in progress, «Rimuovi dai download» for a track in the index. The download notices are given by the core.
//!
//! The menu sits above everything and `Shell` holds it; the page says only which track and where ([`TrackMenu`]).
//! Each entry becomes a [`MenuChoice`], which `Shell` turns into a command to the core and, where the earlier client gave one,
//! into a local notice ([`choice_notice`]).

use gpui::{Pixels, Point, SharedString};
use psytify_core::api::{DownloadQuality, Track};
use psytify_core::config::Playlist;
use psytify_core::snapshot::DownloadsView;
use uuid::Uuid;

use crate::notices::Notice;

/// The open menu: the track, where (the top-left corner) and, if it comes from a playlist, which one and at
/// which position.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackMenu {
    pub track: Track,
    pub at: Point<Pixels>,
    pub from_playlist: Option<(Uuid, usize)>,
}

/// Which panel of the menu is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuLevel {
    #[default]
    Main,
    /// The list of playlists (in place of the entries, in the same spot).
    Playlists,
    /// The qualities of «Scarica in…».
    Qualities,
}

/// A chosen entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuChoice {
    PlayNext,
    Enqueue,
    ShowPlaylists,
    Back,
    NewPlaylist,
    AddTo(Uuid),
    RemoveFromPlaylist,
    Radio,
    GoToArtist,
    GoToAlbum,
    ToggleFollow,
    Copy,
    /// «Scarica · qualità»: at the quality of the settings.
    Download,
    /// «Scarica in…»: opens the list of qualities.
    ShowQualities,
    DownloadAs(DownloadQuality),
    RemoveDownload,
}

/// The menu's download entries, decided on opening from the state of the downloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DownloadItems {
    /// «Scarica · qualità» and «Scarica in…».
    pub offer: bool,
    /// «Rimuovi dai download».
    pub remove: bool,
    /// The quality of the settings (in the label and with the tick in the list).
    pub quality: DownloadQuality,
}

impl DownloadItems {
    /// A track already on the device with its file in place is not downloaded again, nor one
    /// in progress; one in the index (even with a lost file) can be removed.
    pub fn of(track: &Track, downloads: &DownloadsView) -> Self {
        Self {
            offer: track.downloadable
                && !downloads.is_busy(track.id)
                && !downloads.is_offline_ready(track.id),
            remove: downloads.is_saved(track.id),
            quality: downloads.quality,
        }
    }
}

/// «Scarica · Alta»: the entry that downloads at the quality of the settings.
pub fn download_label(quality: DownloadQuality) -> String {
    format!("Scarica · {}", quality.label())
}

/// The «Scarica in…» entries: «Alta · AAC 256 kbps», one per quality, in the core's order.
pub fn quality_items() -> Vec<(DownloadQuality, SharedString)> {
    DownloadQuality::ALL
        .iter()
        .map(|quality| (*quality, SharedString::from(format!("{} · {}", quality.label(), quality.detail()))))
        .collect()
}

/// The text of «Copia titolo e artista».
pub fn copy_text(track: &Track) -> String {
    format!("{} — {}", track.title, track.artist.name)
}

/// The entry to follow the artist: label and icon according to whether you already follow them.
pub fn follow_item(following: bool) -> (&'static str, &'static str) {
    if following {
        ("Smetti di seguire", psytify_design::sprite::ICON_CHECK)
    } else {
        ("Segui l'artista", psytify_design::sprite::ICON_USER_PLUS)
    }
}

/// The notice of an entry; `None` where none is given or the core gives it.
pub fn choice_notice(choice: MenuChoice, track: &Track, playlists: &[Playlist]) -> Option<Notice> {
    match choice {
        MenuChoice::PlayNext => Some(Notice::info(format!("«{}» suonerà subito dopo", track.title))),
        MenuChoice::Enqueue => Some(Notice::info(format!("«{}» aggiunto alla coda", track.title))),
        MenuChoice::AddTo(id) => {
            let playlist = playlists.iter().find(|playlist| playlist.id == id)?;
            Some(Notice::info(if playlist.contains(track.id) {
                format!("«{}» è già in «{}»", track.title, playlist.name)
            } else {
                format!("«{}» aggiunto a «{}»", track.title, playlist.name)
            }))
        }
        MenuChoice::Copy => Some(Notice::info("Copiato negli appunti")),
        // The radio is announced by the core (`PlaybackEvent::RadioStarted`), like the downloads; follow `Shell`.
        MenuChoice::ShowPlaylists
        | MenuChoice::Download
        | MenuChoice::ShowQualities
        | MenuChoice::DownloadAs(_)
        | MenuChoice::RemoveDownload
        | MenuChoice::Back
        | MenuChoice::NewPlaylist
        | MenuChoice::RemoveFromPlaylist
        | MenuChoice::Radio
        | MenuChoice::GoToArtist
        | MenuChoice::GoToAlbum
        | MenuChoice::ToggleFollow => None,
    }
}

/// The entries of the playlist list: id and name, already ready.
pub fn playlist_items(playlists: &[Playlist]) -> Vec<(Uuid, SharedString)> {
    playlists.iter().map(|playlist| (playlist.id, SharedString::from(playlist.name.clone()))).collect()
}

#[cfg(test)]
mod tests {
    use psytify_core::api::models::fixtures;

    use super::*;

    #[test]
    fn adding_says_whether_the_track_was_already_there() {
        let track = fixtures::track(1);
        let with = Playlist::new("Sera", vec![track.clone()]);
        let without = Playlist::new("Notte", Vec::new());
        let lists = [with.clone(), without.clone()];
        let notice = |id| choice_notice(MenuChoice::AddTo(id), &track, &lists).map(|n| n.text.to_string());
        assert_eq!(notice(with.id).as_deref(), Some("«Brano 1» è già in «Sera»"));
        assert_eq!(notice(without.id).as_deref(), Some("«Brano 1» aggiunto a «Notte»"));
        assert_eq!(notice(Uuid::nil()), None, "una playlist sparita non dice niente");
    }

    #[test]
    fn the_queue_choices_say_what_happens_and_the_others_are_silent() {
        let track = fixtures::track(2);
        let text = |choice| choice_notice(choice, &track, &[]).map(|n| n.text.to_string());
        assert_eq!(text(MenuChoice::PlayNext).as_deref(), Some("«Brano 2» suonerà subito dopo"));
        assert_eq!(text(MenuChoice::Enqueue).as_deref(), Some("«Brano 2» aggiunto alla coda"));
        assert_eq!(text(MenuChoice::RemoveFromPlaylist), None);
        assert_eq!(text(MenuChoice::ShowPlaylists), None);
        assert_eq!(text(MenuChoice::Copy).as_deref(), Some("Copiato negli appunti"));
        assert_eq!(text(MenuChoice::Radio), None, "la radio la annuncia il core");
    }

    #[test]
    fn the_download_items_follow_the_state_of_the_track() {
        let track = fixtures::track(4);
        let mut downloads = DownloadsView { quality: DownloadQuality::High, ..DownloadsView::default() };
        let mut free = track.clone();
        free.downloadable = true;
        assert_eq!(
            DownloadItems::of(&free, &downloads),
            DownloadItems { offer: true, remove: false, quality: DownloadQuality::High }
        );
        let mut locked = track.clone();
        locked.downloadable = false;
        assert!(
            !DownloadItems::of(&locked, &downloads).offer,
            "una licenza che non lo consente non si scarica"
        );
        downloads.saved_ids = std::sync::Arc::new([free.id].into_iter().collect());
        let saved = DownloadItems::of(&free, &downloads);
        assert!(!saved.offer && saved.remove, "già sul dispositivo: si toglie, non si riscarica");
        assert_eq!(download_label(DownloadQuality::High), "Scarica · Alta");
        let items = quality_items();
        assert_eq!(items.len(), DownloadQuality::ALL.len());
        assert_eq!(items[1].1.as_ref(), "Alta · AAC 256 kbps");
        assert_eq!(choice_notice(MenuChoice::Download, &free, &[]), None, "l'avviso lo dà il core");
    }

    #[test]
    fn copy_and_follow_read_like_the_iced_menu() {
        let track = fixtures::track(3);
        assert_eq!(copy_text(&track), format!("Brano 3 — {}", track.artist.name));
        assert_eq!(follow_item(false).0, "Segui l'artista");
        assert_eq!(follow_item(true).0, "Smetti di seguire");
    }
}
