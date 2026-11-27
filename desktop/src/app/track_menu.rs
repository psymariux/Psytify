//! Il menu «Altro» di un brano, come quello del client Iced (`client/src/ui/context_menu.rs`), con le voci
//! che la riga non ha già (play, cuore e download stanno sulla riga): «Riproduci subito dopo», «Aggiungi
//! alla coda», «Avvia radio»; «Aggiungi a una playlist» (l'elenco delle playlist, con «Nuova playlist…» in
//! cima) e, in una playlist, «Togli da questa playlist»; «Vai all'artista», «Vai all'album» (se il brano
//! ne ha uno), «Segui l'artista» / «Smetti di seguire» e «Copia titolo e artista»; poi, come nel client
//! Iced, i download ([`DownloadItems`]): «Scarica · qualità» e «Scarica in…» (le quattro qualità, con la
//! spunta su quella delle impostazioni) per un brano scaricabile che non è già sul dispositivo né in
//! corso, «Rimuovi dai download» per un brano nell'indice. Gli avvisi dei download li dà il core.
//!
//! Il menu sta sopra tutto e lo tiene `Shell`; la pagina dice solo quale brano e dove ([`TrackMenu`]).
//! Ogni voce diventa un [`MenuChoice`], che `Shell` traduce in un comando al core e, dove il client Iced lo
//! fa, in un avviso locale ([`choice_notice`]).

use gpui::{Pixels, Point, SharedString};
use psytify_core::api::{DownloadQuality, Track};
use psytify_core::config::Playlist;
use psytify_core::snapshot::DownloadsView;
use uuid::Uuid;

use crate::notices::Notice;

/// Il menu aperto: il brano, dove (l'angolo in alto a sinistra) e, se viene da una playlist, quale e a
/// che posizione.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackMenu {
    pub track: Track,
    pub at: Point<Pixels>,
    pub from_playlist: Option<(Uuid, usize)>,
}

/// Quale pannello del menu si vede.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuLevel {
    #[default]
    Main,
    /// L'elenco delle playlist (al posto delle voci, nello stesso punto).
    Playlists,
    /// Le qualità di «Scarica in…».
    Qualities,
}

/// Una voce scelta.
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
    /// «Scarica · qualità»: alla qualità delle impostazioni.
    Download,
    /// «Scarica in…»: apre l'elenco delle qualità.
    ShowQualities,
    DownloadAs(DownloadQuality),
    RemoveDownload,
}

/// Le voci di download del menu, decise all'apertura dallo stato dei download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DownloadItems {
    /// «Scarica · qualità» e «Scarica in…».
    pub offer: bool,
    /// «Rimuovi dai download».
    pub remove: bool,
    /// La qualità delle impostazioni (nell'etichetta e con la spunta nell'elenco).
    pub quality: DownloadQuality,
}

impl DownloadItems {
    /// Come il client Iced: un brano già sul dispositivo con il file a posto non si scarica di nuovo, uno in
    /// corso nemmeno; uno nell'indice (anche con il file perso) si può togliere.
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

/// «Scarica · Alta»: la voce che scarica alla qualità delle impostazioni.
pub fn download_label(quality: DownloadQuality) -> String {
    format!("Scarica · {}", quality.label())
}

/// Le voci di «Scarica in…»: «Alta · AAC 256 kbps», una per qualità, nell'ordine del core.
pub fn quality_items() -> Vec<(DownloadQuality, SharedString)> {
    DownloadQuality::ALL
        .iter()
        .map(|quality| (*quality, SharedString::from(format!("{} · {}", quality.label(), quality.detail()))))
        .collect()
}

/// Il testo di «Copia titolo e artista», come nel client Iced.
pub fn copy_text(track: &Track) -> String {
    format!("{} — {}", track.title, track.artist.name)
}

/// La voce per seguire l'artista: etichetta e icona secondo se lo segui già.
pub fn follow_item(following: bool) -> (&'static str, &'static str) {
    if following {
        ("Smetti di seguire", psytify_design::sprite::ICON_CHECK)
    } else {
        ("Segui l'artista", psytify_design::sprite::ICON_USER_PLUS)
    }
}

/// L'avviso di una voce, come nel client Iced; `None` dove il client non ne dà o lo dà il core.
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
        // La radio la annuncia il core (`PlaybackEvent::RadioStarted`), così i download; seguire `Shell`.
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

/// Le voci dell'elenco delle playlist: id e nome, già pronti.
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
