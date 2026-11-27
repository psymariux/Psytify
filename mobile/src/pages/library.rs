//! Libreria, as in the Iced phone layout (`client/src/ui/phone/library.rs`): the filters, then
//! «Brani che ti piacciono», «Scaricati», the playlists, the saved
//! albums and the followed artists. Everything is local: nothing to load.

use psytify_core::Snapshot;

use super::{Art, Built, Chip, Mark, PageAction, PageStatus, Route, Row, RowState, Target, Tone};
use crate::dialog::Request;
use crate::format;

/// The filters, in the order of the Iced client.
const CHIPS: [&str; 4] = ["Tutto", "Playlist", "Album", "Artisti"];
const ALL: usize = 0;
const PLAYLISTS: usize = 1;
const ALBUMS: usize = 2;
const ARTISTS: usize = 3;

pub(super) fn build(snapshot: &Snapshot, ui: &super::Ui) -> Built {
    let library = &snapshot.library;
    let filter = ui.chip.min(CHIPS.len() - 1);
    let mut page = Built::titled("Libreria");
    page.status = PageStatus::Ready;
    page.action(PageAction::NewPlaylist, Target::Dialog(Request::NewPlaylist { seed: Vec::new() }));
    let chips = CHIPS
        .iter()
        .enumerate()
        .map(|(index, label)| Chip { label: (*label).to_owned(), on: index == filter });
    page.push(Row::Chips { key: "chips".into(), chips: chips.collect() }, Mark::None, Target::None);

    let shows = |kind: usize| filter == ALL || filter == kind;
    let item = |page: &mut Built, key: String, art: Art, title: &str, detail: String, target: Target| {
        let row = Row::Item { key, art, title: title.to_owned(), detail, state: RowState::Idle };
        page.push(row, Mark::None, target);
    };
    if shows(PLAYLISTS) {
        let detail = format!("Playlist · {}", format::tracks(library.liked.len()));
        item(
            &mut page,
            "liked".into(),
            Art::Liked,
            "Brani che ti piacciono",
            detail,
            Target::Open(Route::Liked),
        );
    }
    if filter == ALL {
        let downloads = &snapshot.downloads;
        let detail = if downloads.saved.is_empty() {
            "Si ascoltano anche senza rete".to_owned()
        } else {
            format!("{} · {}", format::tracks(downloads.saved.len()), format::size(downloads.used_bytes))
        };
        let target = Target::Open(Route::Downloads);
        item(&mut page, "downloads".into(), Art::Downloads, "Scaricati", detail, target);
    }
    if shows(PLAYLISTS) {
        for playlist in library.playlists.iter() {
            let cover = playlist.tracks.iter().find_map(|track| track.cover_url.clone());
            let mut detail = format!("Playlist · {}", format::tracks(playlist.tracks.len()));
            if !playlist.pending.is_empty() {
                detail.push_str(&format!(" · {} in arrivo", playlist.pending.len()));
            }
            let target = Target::Open(Route::Playlist { id: playlist.id.to_string() });
            item(
                &mut page,
                format!("playlist:{}", playlist.id),
                Art::Cover { url: cover },
                &playlist.name,
                detail,
                target,
            );
        }
    }
    if shows(ALBUMS) {
        for album in library.saved_albums.iter() {
            let detail = format!("Album · {}", album.artist.name);
            let art = Art::Cover { url: album.cover_url.clone() };
            item(
                &mut page,
                format!("album:{}", album.id),
                art,
                &album.title,
                detail,
                Target::Open(Route::album(album.id)),
            );
        }
    }
    if shows(ARTISTS) {
        for artist in library.followed.iter() {
            let art = Art::Round { url: artist.cover_url.clone() };
            let target = Target::Open(Route::artist(artist.id));
            item(&mut page, format!("artist:{}", artist.id), art, &artist.name, "Artista".into(), target);
        }
    }
    let hint = match filter {
        PLAYLISTS if library.playlists.is_empty() => {
            Some("Le playlist salvate da un album del catalogo ufficiale o da YouTube compaiono qui.")
        }
        ALBUMS if library.saved_albums.is_empty() => Some("Salva un album dalla sua pagina: lo ritrovi qui."),
        ARTISTS if library.followed.is_empty() => {
            Some("Segui un artista dalla sua pagina: lo ritrovi qui, e le sue uscite in Novità.")
        }
        _ => None,
    };
    if let Some(hint) = hint {
        page.note("hint", hint.to_owned(), Tone::Plain);
    }
    page
}
