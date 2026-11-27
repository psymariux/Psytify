//! I pezzi comuni alle pagine che elencano brani del catalogo (raccolte, album, artista): i testi di una
//! riga già pronti, lo stato del download di un brano, l'etichetta di «Scarica tutto» e il conteggio del
//! lotto, il brano in corso. Funzioni pure sulle viste del core.

use gpui::{ClickEvent, Context, EventEmitter, SharedString};
use psytify_core::api::Track;
use psytify_core::command::DownloadsCmd;
use psytify_core::snapshot::{DownloadsView, ItemState, PlaybackView};
use uuid::Uuid;

use super::{Page, PageEvent, play_state};
use crate::components::indicators::{DownloadState, PlayState};
use crate::components::track_row::TrackRow;
use crate::format;

/// I testi di una riga, pronti: la vista non formatta né alloca stringhe a ogni disegno.
#[derive(Clone, Debug)]
pub struct RowText {
    pub title: SharedString,
    pub artist: SharedString,
    pub album: SharedString,
    pub duration: SharedString,
    pub cover_url: Option<SharedString>,
}

impl RowText {
    pub fn of(track: &Track) -> Self {
        Self {
            title: track.title.clone().into(),
            artist: track.artist.name.clone().into(),
            album: track
                .album
                .as_ref()
                .map_or_else(|| SharedString::from("Singolo"), |a| a.title.clone().into()),
            duration: format::clock_ms(track.duration_ms).into(),
            cover_url: track.cover_url.clone().map(SharedString::from),
        }
    }
}

/// Lo stato del download di un brano, come la cella del client Iced: prima la coda (in coda, in corso,
/// fallito: anche un brano già sul dispositivo che si sta riscaricando), poi l'indice (sul dispositivo, o
/// il file perso: da scaricare di nuovo), poi se si può scaricare.
pub fn download_state(downloads: &DownloadsView, track: &Track) -> DownloadState {
    if let Some(item) = downloads.item(track.id) {
        return match item.state {
            ItemState::Queued => DownloadState::Queued,
            ItemState::Failed(_) => DownloadState::Error,
            _ => DownloadState::Downloading(item.percent().map_or(0.0, |p| f32::from(p) / 100.0)),
        };
    }
    if downloads.saved_ids.contains(&track.id) {
        return if downloads.lost_reason(track.id).is_some() {
            DownloadState::Lost
        } else {
            DownloadState::Done
        };
    }
    if track.downloadable { DownloadState::Available } else { DownloadState::NotDownloadable }
}

/// Il comando del clic sulla cella del download: «Riprova» per un download fallito (con la qualità che
/// aveva, come nel client Iced), altrimenti «Scarica» (anche un file perso, che il core sostituisce).
pub fn download_command(downloads: &DownloadsView, track: &Track) -> DownloadsCmd {
    match download_state(downloads, track) {
        DownloadState::Error => DownloadsCmd::Retry(track.id),
        _ => DownloadsCmd::Download(Box::new(track.clone())),
    }
}

/// «Scarica tutto» o «Scarica i brani disponibili»; `None` se nessuno si può scaricare.
pub fn download_all_label(tracks: &[Track]) -> Option<&'static str> {
    let downloadable = tracks.iter().filter(|t| t.downloadable).count();
    match downloadable {
        0 => None,
        n if n == tracks.len() => Some("Scarica tutto"),
        _ => Some("Scarica i brani disponibili"),
    }
}

/// Il conteggio di «Scarica tutto» mentre il lotto scarica («3/10»).
pub fn batch_label(view: &DownloadsView) -> Option<SharedString> {
    view.batch_progress().map(|(done, total)| format!("{done}/{total}").into())
}

/// Il brano in corso e come mostrarlo (equalizzatore lime solo mentre suona).
pub fn now_playing(view: &PlaybackView) -> Option<(Uuid, PlayState)> {
    play_state(view).and_then(|state| view.queue.current().map(|track| (track.id, state)))
}

/// Il nome dell'artista e il titolo dell'album della riga aprono le loro pagine (un singolo non ha album).
pub fn with_links<T: EventEmitter<PageEvent>>(row: TrackRow, track: &Track, cx: &mut Context<T>) -> TrackRow {
    let artist = track.artist.id;
    let row = row.on_artist(
        cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Artist(artist)))),
    );
    match track.album.as_ref().map(|album| album.id) {
        Some(album) => row.on_album(
            cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Album(album)))),
        ),
        None => row,
    }
}

/// Doppio clic o Invio: il gesto che suona una riga (un clic solo non fa niente).
pub fn plays(event: &ClickEvent) -> bool {
    event.click_count() >= 2 || matches!(event, ClickEvent::Keyboard(_))
}

#[cfg(test)]
mod tests {
    use psytify_core::api::models::fixtures;

    use super::*;

    #[test]
    fn download_all_says_whether_every_track_can_be_kept() {
        let mut open = fixtures::track(1);
        open.downloadable = true;
        let closed = fixtures::track(2);
        assert_eq!(download_all_label(&[open.clone()]), Some("Scarica tutto"));
        assert_eq!(download_all_label(&[open, closed.clone()]), Some("Scarica i brani disponibili"));
        assert_eq!(download_all_label(&[closed]), None);
        assert_eq!(download_all_label(&[]), None);
    }

    #[test]
    fn the_download_cell_follows_the_queue_then_the_index() {
        use std::sync::Arc;

        use psytify_core::api::DownloadQuality;
        use psytify_core::offline::{Lost, SavedTrack};
        use psytify_core::snapshot::{Item, LostReason};

        let mut open = fixtures::track(1);
        open.downloadable = true;
        let closed = fixtures::track(2);
        let mut view = DownloadsView::default();
        assert_eq!(download_state(&view, &open), DownloadState::Available);
        assert_eq!(download_state(&view, &closed), DownloadState::NotDownloadable);
        assert!(matches!(download_command(&view, &open), DownloadsCmd::Download(_)));

        // Sul dispositivo; poi il file sparisce: si scarica di nuovo.
        let saved = SavedTrack {
            track: open.clone(),
            path: "/dati/Download/a.flac".into(),
            format: "flac".into(),
            quality: Some(DownloadQuality::Lossless),
            size: 10,
            sha256: String::new(),
        };
        view.saved = vec![saved].into();
        view.saved_ids = Arc::new([open.id].into_iter().collect());
        assert_eq!(download_state(&view, &open), DownloadState::Done);
        view.lost = vec![Lost { id: open.id, reason: LostReason::Missing }].into();
        assert_eq!(download_state(&view, &open), DownloadState::Lost);
        assert!(matches!(download_command(&view, &open), DownloadsCmd::Download(_)));

        // Mentre si riscarica conta la coda; un download fallito si riprova.
        let item = |state| Item {
            track: Arc::new(open.clone()),
            quality: DownloadQuality::High,
            state,
            batched: false,
        };
        view.items = vec![item(ItemState::Queued)].into();
        assert_eq!(download_state(&view, &open), DownloadState::Queued);
        view.items = vec![item(ItemState::Downloading { received: 5, total: Some(10) })].into();
        assert_eq!(download_state(&view, &open), DownloadState::Downloading(0.5));
        view.items = vec![item(ItemState::Failed(psytify_core::error::AppError::Network))].into();
        assert_eq!(download_state(&view, &open), DownloadState::Error);
        assert!(matches!(download_command(&view, &open), DownloadsCmd::Retry(id) if id == open.id));
    }

    #[test]
    fn a_track_without_an_album_is_a_single() {
        let mut track = fixtures::track(3);
        track.album = None;
        assert_eq!(RowText::of(&track).album, SharedString::from("Singolo"));
    }
}
