//! An album of the server, as the GPUI page (`desktop/src/app/album_page.rs`): the header with the
//! cover, «Album · 2024», the title, the artist (it opens), tracks, length and `Lossless` if every
//! track is; Riproduci, Casuale, save; then the tracks and the credits of the licences that ask for
//! them. The core keeps the album (`CatalogView::album`, an LRU of 24 pages).

use std::sync::Arc;

use psytify_core::Command;
use psytify_core::Snapshot;
use psytify_core::api::{Album, Track};
use psytify_core::catalog::Load;
use psytify_core::command::{CatalogCmd, DownloadsCmd, LibraryCmd, PlaybackCmd, Resource};
use psytify_core::config::SavedAlbum;
use psytify_core::snapshot::DownloadsView;
use uuid::Uuid;

use super::{
    Art, Built, Header, HeaderButton, Mark, PageAction, PageStatus, Route, Row, RowState, Target, Tone,
};
use crate::downloads::{DownloadMark, Marks};
use crate::format;

pub(super) fn build(id: Uuid, snapshot: &Snapshot) -> Built {
    let mut page = Built::titled("Album");
    page.action(PageAction::Retry, Target::Send(Command::Catalog(CatalogCmd::Retry(Resource::Album(id)))));
    let album = match &snapshot.catalog.album {
        Some((open, Load::Ready(album))) if *open == id => Arc::clone(album),
        Some((open, Load::Failed(error))) if *open == id => {
            page.status = PageStatus::Failed { message: error.to_string() };
            return page;
        }
        _ => return page,
    };
    let tracks: Arc<[Track]> = Arc::from(album.tracks.as_slice());
    let saved = snapshot.library.saved_albums.iter().any(|known| known.id == id);
    let mut head = header(&album, &tracks, saved);
    head.buttons.extend(download_all(&mut page, &tracks, &snapshot.downloads));
    page.header = Some(head);
    if !tracks.is_empty() {
        page.action(PageAction::Play, Target::Play(Arc::clone(&tracks), 0));
        let shuffle = Command::Playback(PlaybackCmd::ShufflePlay(Arc::clone(&tracks)));
        page.action(PageAction::Shuffle, Target::Send(shuffle));
    }
    let entry = SavedAlbum {
        id,
        title: album.title.clone(),
        artist: album.artist.clone(),
        cover_url: album.cover_url.clone(),
    };
    let notice = if saved {
        format!("«{}» tolto dalla libreria", album.title)
    } else {
        format!("«{}» è nella tua libreria", album.title)
    };
    page.action(
        PageAction::Save,
        Target::Notify(Command::library(LibraryCmd::ToggleSaveAlbum(entry)), notice),
    );
    page.action(PageAction::OpenArtist, Target::Open(Route::artist(album.artist.id)));

    page.status = if tracks.is_empty() {
        PageStatus::Empty { message: "Questo album non ha ancora brani pubblicati.".into() }
    } else {
        PageStatus::Ready
    };
    track_rows(&mut page, &tracks, Some(&album.artist.name));
    credits(&mut page, &tracks);
    page
}

/// Rows for `tracks`, numbered; a tap plays the list from there. The artist is under the title only
/// where it is not `artist` (a compilation).
pub(super) fn track_rows(page: &mut Built, tracks: &Arc<[Track]>, artist: Option<&str>) {
    for (index, track) in tracks.iter().enumerate() {
        let number = track.track_no.unwrap_or(u32::try_from(index + 1).unwrap_or(u32::MAX));
        let subtitle = match artist {
            Some(name) if name == track.artist.name => String::new(),
            _ => track.artist.name.clone(),
        };
        let row = Row::Track {
            key: format!("track:{index}:{}", track.id),
            number: Some(number),
            art: None,
            title: track.title.clone(),
            subtitle,
            duration: format::clock(track.duration_ms),
            state: RowState::Idle,
            more: false,
            download: DownloadMark::None,
        };
        page.push(row, Mark::Track(track.id), Target::Play(Arc::clone(tracks), index));
    }
}

/// «Crediti»: «Titolo — CC BY · Titolare» for the licences that ask for the attribution.
pub(super) fn credits(page: &mut Built, tracks: &[Track]) {
    let credited: Vec<&Track> = tracks.iter().filter(|t| t.licence.requires_attribution()).collect();
    if credited.is_empty() {
        return;
    }
    page.heading("credits", "Crediti", String::new());
    for track in credited {
        let text = format!("{} — {} · {}", track.title, track.licence.label(), track.licence.attribution);
        page.note(&format!("credit:{}", track.id), text, Tone::Plain);
    }
}

fn header(album: &Album, tracks: &[Track], saved: bool) -> Header {
    let kicker = match album.released_on.as_deref().and_then(|date| date.get(..4)) {
        Some(year) => format!("Album · {year}"),
        None => "Album".to_owned(),
    };
    let total = tracks.iter().map(|t| t.duration_ms).sum();
    let mut meta = format::length(tracks.len(), total);
    if quality_lossless(tracks) {
        meta.push_str(" · Lossless");
    }
    let mut buttons = Vec::new();
    if !tracks.is_empty() {
        buttons.push(button(PageAction::Play, "Riproduci", true, false));
        buttons.push(button(PageAction::Shuffle, "Casuale", false, false));
    }
    buttons.push(button(PageAction::Save, if saved { "Salvato" } else { "Salva" }, false, saved));
    Header {
        kicker,
        title: album.title.clone(),
        subtitle: Some(album.artist.name.clone()),
        subtitle_opens: true,
        meta,
        art: Art::Cover { url: album.cover_url.clone() },
        buttons,
        note: None,
    }
}

/// «Scarica tutto» for `tracks`, as the Iced phone layout: there whenever the list has tracks (if
/// none can be kept, the core says why); «Scarica i brani disponibili» when only some can; the count
/// of the batch while this list's tracks download.
pub(super) fn download_all(
    page: &mut Built,
    tracks: &Arc<[Track]>,
    view: &DownloadsView,
) -> Option<HeaderButton> {
    if tracks.is_empty() {
        return None;
    }
    let marks = Marks::of(view);
    let running = tracks.iter().any(|track| marks.busy(track.id));
    let downloadable = tracks.iter().filter(|track| track.downloadable).count();
    let label = match view.batch_progress() {
        Some((done, total)) if running => format!("{done}/{total}"),
        _ if downloadable == tracks.len() => "Scarica tutto".to_owned(),
        _ => "Scarica i brani disponibili".to_owned(),
    };
    let all = DownloadsCmd::DownloadAll(Arc::clone(tracks));
    page.action(PageAction::DownloadAll, Target::Send(Command::Downloads(all)));
    Some(button(PageAction::DownloadAll, &label, false, running))
}

pub(super) fn button(action: PageAction, label: &str, primary: bool, on: bool) -> HeaderButton {
    HeaderButton { action, label: label.to_owned(), primary, on, enabled: true }
}

/// Every track in a format without loss (FLAC, WAV): a quality, not a state.
fn quality_lossless(tracks: &[Track]) -> bool {
    let lossless = |t: &Track| matches!(t.audio_format.as_str(), "flac" | "wav" | "alac" | "aiff");
    !tracks.is_empty() && tracks.iter().all(lossless)
}
