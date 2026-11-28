//! «Brani che ti piacciono» or a playlist of the library, as the GPUI collection page: the header with
//! the count and the length, Riproduci and Casuale, then the tracks with their covers. A tap plays
//! the collection from there, or pauses the current track. These lists can be long: the app reads
//! them a page of rows at a time.

use std::sync::Arc;

use psytify_core::Command;
use psytify_core::Snapshot;
use psytify_core::api::Track;
use psytify_core::catalog::Load;
use psytify_core::command::{PlaybackCmd, SocialCmd};
use uuid::Uuid;

use super::album::{button, download_all};
use super::{Art, Built, Header, Mark, PageAction, PageStatus, Route, Row, RowState, Target, Tone};
use crate::dialog::Request;
use crate::downloads::DownloadMark;
use crate::format;

pub(super) fn liked(snapshot: &Snapshot) -> Built {
    let tracks: Arc<[Track]> = Arc::from(&snapshot.library.liked[..]);
    let mut page = collection("Brani che ti piacciono", Art::Liked, &tracks, snapshot);
    if tracks.is_empty() {
        page.status =
            PageStatus::Empty { message: "Metti il cuore a un brano dal player: lo ritrovi qui.".into() };
    }
    page
}

pub(super) fn playlist(id: Uuid, snapshot: &Snapshot) -> Built {
    let Some(playlist) = snapshot.library.playlists.iter().find(|playlist| playlist.id == id) else {
        let mut page = Built::titled("Playlist");
        page.status = PageStatus::Empty { message: "Questa playlist non è più nella libreria.".into() };
        return page;
    };
    let tracks: Arc<[Track]> = Arc::from(playlist.tracks.as_slice());
    let cover = tracks.iter().find_map(|track| track.cover_url.clone());
    let mut page = collection(&playlist.name, Art::Cover { url: cover }, &tracks, snapshot);
    if let Some(header) = page.header.as_mut() {
        header.buttons.push(button(PageAction::Rename, "Rinomina", false, false));
        header.buttons.push(button(PageAction::Delete, "Elimina", false, false));
    }
    let (id, name) = (playlist.id, playlist.name.clone());
    page.action(PageAction::Rename, Target::Dialog(Request::Rename { id, name: name.clone() }));
    page.action(PageAction::Delete, Target::Dialog(Request::Delete { id, name }));
    if !playlist.pending.is_empty() {
        let text = format!("{} in arrivo: entrano uno alla volta.", format::tracks(playlist.pending.len()));
        page.note("pending", text, Tone::Waiting);
    }
    if tracks.is_empty() && playlist.pending.is_empty() {
        page.status = PageStatus::Empty { message: "Questa playlist è vuota.".into() };
    }
    page
}

/// A friend's public playlist (from their profile): «Playlist pubblica», the owner (who opens their
/// profile), Riproduci, Casuale and «Scarica tutto»; the core keeps one open (`SocialView::playlist`).
pub(super) fn public(handle: &str, id: Uuid, snapshot: &Snapshot) -> Built {
    let open = match &snapshot.social.playlist {
        Some(Load::Ready(open)) if open.id == id => Arc::clone(open),
        Some(Load::Failed(error)) => {
            let mut page = Built::titled("Playlist pubblica");
            let reload = SocialCmd::OpenFriendPlaylist { handle: handle.to_owned(), id };
            page.action(PageAction::Retry, Target::Send(Command::Social(reload)));
            page.status = PageStatus::Failed { message: error.to_string() };
            return page;
        }
        _ => return Built::titled("Playlist pubblica"),
    };
    let tracks = Arc::clone(&open.tracks);
    let cover = tracks.iter().find_map(|track| track.cover_url.clone());
    let mut page = collection(&open.name, Art::Cover { url: cover }, &tracks, snapshot);
    page.title = "Playlist pubblica".into();
    if let Some(header) = page.header.as_mut() {
        header.kicker = "Playlist pubblica".into();
        header.subtitle = Some(format!("di {}", open.owner.display_name));
        header.subtitle_opens = true;
    }
    let owner = Route::Profile { handle: open.owner.handle.clone() };
    page.action(PageAction::OpenArtist, Target::Open(owner));
    if tracks.is_empty() {
        page.status = PageStatus::Empty { message: "Questa playlist è vuota.".into() };
    }
    page
}

fn collection(title: &str, art: Art, tracks: &Arc<[Track]>, snapshot: &Snapshot) -> Built {
    let mut page = Built::titled("Playlist");
    let total = tracks.iter().map(|t| t.duration_ms).sum();
    let mut buttons = Vec::new();
    if !tracks.is_empty() {
        buttons.push(button(PageAction::Play, "Riproduci", true, false));
        buttons.push(button(PageAction::Shuffle, "Casuale", false, false));
        page.action(PageAction::Play, Target::Play(Arc::clone(tracks), 0));
        let shuffle = Command::Playback(PlaybackCmd::ShufflePlay(Arc::clone(tracks)));
        page.action(PageAction::Shuffle, Target::Send(shuffle));
    }
    buttons.extend(download_all(&mut page, tracks, &snapshot.downloads));
    page.header = Some(Header {
        kicker: "Playlist".into(),
        title: title.to_owned(),
        subtitle: None,
        subtitle_opens: false,
        meta: format::length(tracks.len(), total),
        art,
        buttons,
        note: None,
    });
    page.status = PageStatus::Ready;
    for (index, track) in tracks.iter().enumerate() {
        let row = Row::Track {
            key: format!("track:{index}:{}", track.id),
            number: None,
            art: Some(Art::Cover { url: track.cover_url.clone() }),
            title: track.title.clone(),
            subtitle: track.artist.name.clone(),
            duration: format::clock(track.duration_ms),
            state: RowState::Idle,
            more: false,
            download: DownloadMark::None,
        };
        page.push(row, Mark::Track(track.id), Target::Play(Arc::clone(tracks), index));
    }
    page
}
