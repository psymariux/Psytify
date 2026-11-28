//! Cerca: the results of `CatalogView::search` in one list, in the order of the phone layout and the
//! GPUI page (`desktop/src/app/search_page.rs`): the official catalogue (artists, which the core
//! brings to the server and then opens; tracks, which the server imports when played; albums and
//! playlists, which open the source page), what the server already has (tracks, artists, albums)
//! and Internet Archive. The field sends `CatalogCmd::SearchInput`: the 300 ms wait and «the last
//! search wins» are the core's. With nothing searched, the past searches.

use std::sync::Arc;

use psytify_core::Command;
use psytify_core::Snapshot;
use psytify_core::api::{
    AlbumSummary, ArtistSummary, DiscoverItem, MetaArtist, SourceItem, SourceKind, SourcePlaylist, Track,
};
use psytify_core::catalog::Load;
use psytify_core::command::{CatalogCmd, ExternalCmd};
use psytify_core::error::AppError;
use psytify_core::snapshot::Importing;

use super::{Art, Built, Mark, PageStatus, Route, Row, RowState, SourceRef, Target, Tone};
use crate::downloads::DownloadMark;
use crate::format;

/// Official catalogue tracks shown: the others are found by refining the search.
const OFFICIAL_TRACKS: usize = 12;
/// Artists, albums and playlists shown per section.
const CARDS: usize = 6;

pub(super) fn build(snapshot: &Snapshot) -> Built {
    let search = &snapshot.catalog.search;
    let mut page = Built::titled("Cerca");
    match &search.results {
        Load::Idle => {
            history(&mut page, &snapshot.library.search_history);
            if page.rows.is_empty() {
                page.status = PageStatus::Empty {
                    message: "Cerca artisti, album e brani: nel catalogo ufficiale, nel server e su Internet Archive.".into(),
                };
            } else {
                page.status = PageStatus::Ready;
            }
            return page;
        }
        Load::Loading => return page,
        Load::Ready(_) | Load::Failed(_) => {}
    }

    // The official catalogue first: it is where one discovers. Its artists wait for its tracks.
    if !search.sources.is_loading() {
        let artists: Vec<&MetaArtist> = search.artists.iter().take(CARDS).collect();
        cards(
            &mut page,
            "official-artists",
            "Artisti",
            artists_count(artists.len()),
            artists,
            official_artist,
        );
    }
    let waiting = "Cerco nel catalogo ufficiale…";
    section(
        &mut page,
        "official",
        "Catalogo ufficiale",
        waiting,
        &search.sources,
        OFFICIAL_TRACKS,
        source_row,
    );
    let lists = search.playlists.ready().map_or(&[][..], |lists| &lists[..]);
    let albums: Vec<&SourcePlaylist> = lists.iter().filter(|l| l.kind != "playlist").take(CARDS).collect();
    let playlists: Vec<&SourcePlaylist> = lists.iter().filter(|l| l.kind == "playlist").take(CARDS).collect();
    cards(&mut page, "official-albums", "Album", format!("{} album", albums.len()), albums, release);
    cards(
        &mut page,
        "official-playlists",
        "Playlist",
        format!("{} playlist", playlists.len()),
        playlists,
        release,
    );

    if let Load::Ready(results) = &search.results {
        let tracks: Arc<[Track]> = Arc::from(results.tracks.as_slice());
        if !tracks.is_empty() {
            page.heading("server", "Già nel catalogo", format::tracks(tracks.len()));
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
                page.push(row, Mark::Track(track.id), Target::Play(Arc::clone(&tracks), index));
            }
        }
        let artists: Vec<&ArtistSummary> = results.artists.iter().take(CARDS).collect();
        cards(
            &mut page,
            "server-artists",
            "Artisti nel catalogo",
            artists_count(artists.len()),
            artists,
            server_artist,
        );
        let albums: Vec<&AlbumSummary> = results.albums.iter().take(CARDS).collect();
        cards(
            &mut page,
            "server-albums",
            "Album nel catalogo",
            format!("{} album", albums.len()),
            albums,
            server_album,
        );
    }
    let waiting = "Cerco su Internet Archive…";
    section(&mut page, "archive", "Da Internet Archive", waiting, &search.discover, usize::MAX, archive_row);

    page.status = match &search.results {
        Load::Failed(error) if page.rows.is_empty() => PageStatus::Failed { message: error.to_string() },
        _ if page.rows.is_empty() => PageStatus::Empty {
            message: format!(
                "Nessun risultato per «{}». Controlla l'ortografia o prova con meno parole.",
                search.submitted
            ),
        },
        _ => PageStatus::Ready,
    };
    page
}

fn artists_count(count: usize) -> String {
    if count == 1 { "1 artista".to_owned() } else { format!("{count} artisti") }
}

fn history(page: &mut Built, history: &[String]) {
    if history.is_empty() {
        return;
    }
    page.heading("history", "Ricerche recenti", String::new());
    for query in history {
        let row = Row::Item {
            key: format!("history:{query}"),
            art: Art::Search,
            title: query.clone(),
            detail: String::new(),
            state: RowState::Idle,
        };
        page.push(row, Mark::None, Target::Search(query.clone()));
    }
}

/// A section that comes from an external search: at most `limit` rows, or its state while it is
/// not there.
fn section<T>(
    page: &mut Built,
    key: &str,
    title: &str,
    waiting: &str,
    load: &Load<Arc<[T]>>,
    limit: usize,
    row: fn(&mut Built, usize, &T),
) {
    match load {
        Load::Idle => {}
        Load::Ready(items) if items.is_empty() => {}
        Load::Loading => {
            page.heading(key, title, String::new());
            page.note(key, waiting.to_owned(), Tone::Waiting);
        }
        Load::Failed(error) => {
            page.heading(key, title, String::new());
            page.note(key, problem(error), Tone::Error);
        }
        Load::Ready(items) => {
            page.heading(key, title, format::tracks(items.len().min(limit)));
            for (index, item) in items.iter().take(limit).enumerate() {
                row(page, index, item);
            }
        }
    }
}

fn problem(error: &AppError) -> String {
    error.to_string()
}

/// A section of artists, albums or playlists, if it has any.
fn cards<T>(
    page: &mut Built,
    key: &str,
    title: &str,
    count: String,
    items: Vec<&T>,
    row: fn(&mut Built, usize, &T),
) {
    if items.is_empty() {
        return;
    }
    page.heading(key, title, count);
    for (index, item) in items.into_iter().enumerate() {
        row(page, index, item);
    }
}

fn item(page: &mut Built, key: String, art: Art, title: &str, detail: String, target: Target) {
    let row = Row::Item { key, art, title: title.to_owned(), detail, state: RowState::Idle };
    page.push(row, Mark::None, target);
}

/// «Artista · 12.400 fan». A tap brings the artist to the server; the page opens when it is there
/// (`CatalogEvent::MetaArtistOpened`).
fn official_artist(page: &mut Built, index: usize, artist: &MetaArtist) {
    let detail = if artist.fans > 0 {
        format!("Artista · {}", format::fans(artist.fans))
    } else {
        "Artista".to_owned()
    };
    let target = Target::Send(Command::Catalog(CatalogCmd::OpenMetaArtist(artist.id)));
    let art = Art::Round { url: artist.image_url.clone() };
    item(page, format!("meta-artist:{index}:{}", artist.id), art, &artist.name, detail, target);
}

/// «Album · Autore · 2021»: opens the source page.
fn release(page: &mut Built, index: usize, release: &SourcePlaylist) {
    let target = Target::Open(Route::Source { source: SourceRef::of(release) });
    let art = Art::Cover { url: release.cover_url.clone() };
    let key = format!("release:{index}:{}", release.id);
    item(page, key, art, &release.title, release_detail(release), target);
}

pub(super) fn release_detail(release: &SourcePlaylist) -> String {
    let mut detail = vec![release.kind_label(), release.owner.as_str()];
    if let Some(year) = release.year() {
        detail.push(year);
    }
    detail.retain(|part| !part.is_empty());
    detail.join(" · ")
}

/// «Artista · 12 brani · 2 album».
fn server_artist(page: &mut Built, index: usize, artist: &ArtistSummary) {
    let mut detail = vec!["Artista".to_owned(), format::tracks(artist.track_count as usize)];
    if artist.album_count > 0 {
        detail.push(format!("{} album", artist.album_count));
    }
    let art = Art::Round { url: artist.cover_url.clone() };
    let target = Target::Open(Route::artist(artist.id));
    item(page, format!("artist:{index}:{}", artist.id), art, &artist.name, detail.join(" · "), target);
}

/// «Album · Artista · 10 brani».
fn server_album(page: &mut Built, index: usize, album: &AlbumSummary) {
    let detail = format!("Album · {} · {}", album.artist.name, format::tracks(album.track_count as usize));
    let art = Art::Cover { url: album.cover_url.clone() };
    let target = Target::Open(Route::album(album.id));
    item(page, format!("album:{index}:{}", album.id), art, &album.title, detail, target);
}

/// «Artista» for the official catalogue, «YouTube · Artista» for the other sources.
pub(super) fn source_detail(item: &SourceItem) -> String {
    if item.source == SourceKind::Deezer {
        item.artist.clone()
    } else {
        format!("{} · {}", item.source.label(), item.artist)
    }
}

/// A track of the official catalogue (or of another source): a tap imports and plays it, with one
/// command; while the server imports it the row is busy.
fn source_row(page: &mut Built, index: usize, item: &SourceItem) {
    let row = Row::Track {
        key: format!("source:{index}:{}", item.id),
        number: None,
        art: Some(Art::Cover { url: item.cover_url.clone() }),
        title: item.title.clone(),
        subtitle: source_detail(item),
        duration: item.duration_ms.map(format::clock).unwrap_or_default(),
        state: RowState::Idle,
        more: false,
        download: DownloadMark::None,
    };
    let command = ExternalCmd::PlaySource { source: item.source, id: item.id.clone(), video: false };
    page.push(
        row,
        Mark::Import(Importing::Source(item.source, item.id.clone())),
        Target::Send(Command::External(command)),
    );
}

/// «Autore · CC BY · 2019 · Lossless».
fn archive_row(page: &mut Built, index: usize, item: &DiscoverItem) {
    let mut detail: Vec<&str> = Vec::new();
    if let Some(creator) = &item.creator {
        detail.push(creator);
    }
    detail.push(format::licence_label(&item.licence));
    if let Some(year) = &item.year {
        detail.push(year);
    }
    if item.lossless {
        detail.push("Lossless");
    }
    let row = Row::Track {
        key: format!("archive:{index}:{}", item.identifier),
        number: None,
        art: Some(Art::Cover { url: item.cover_url.clone() }),
        title: item.title.clone(),
        subtitle: detail.join(" · "),
        duration: String::new(),
        state: RowState::Idle,
        more: false,
        download: DownloadMark::None,
    };
    let command = ExternalCmd::PlayArchive { identifier: item.identifier.clone() };
    page.push(
        row,
        Mark::Import(Importing::Archive(item.identifier.clone())),
        Target::Send(Command::External(command)),
    );
}
