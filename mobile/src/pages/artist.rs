//! An artist, with the sections of the GPUI page (`desktop/src/app/artist_page.rs`) in the same
//! order: the header (portrait, name, albums, singles and fans; Riproduci, Segui, Radio), then from
//! the official catalogue («Altro di …», `ArtistExtras`) *Popolari* (5, «Mostra altri» shows them all)
//! and *Discografia* (filters Tutto, Album, Singoli when there are both), then the tracks the server
//! already has, other artists, *Informazioni* and the credits.

use std::sync::Arc;

use psytify_core::Snapshot;
use psytify_core::api::{ArtistPage, SourcePlaylist, Track};
use psytify_core::catalog::{Load, portrait_url};
use psytify_core::command::{CatalogCmd, ExternalCmd, LibraryCmd, PlaybackCmd, Resource};
use psytify_core::config::FollowedArtist;
use psytify_core::snapshot::{ArtistExtras, Importing};
use psytify_core::{Command, feed};
use uuid::Uuid;

use super::album::{button, credits};
use super::search::source_detail;
use super::{
    Art, Built, Chip, Header, Mark, PageAction, PageStatus, Route, Row, RowState, SourceRef, Target, Tone, Ui,
};
use crate::downloads::DownloadMark;
use crate::format;

/// Tracks in *Popolari* until «Mostra altri».
const POPULAR: usize = 5;
/// Other artists offered at the bottom.
const RELATED: usize = 8;
const FILTERS: [&str; 3] = ["Tutto", "Album", "Singoli"];

pub(super) fn build(id: Uuid, snapshot: &Snapshot, ui: &Ui) -> Built {
    let mut page = Built::titled("Artista");
    page.action(PageAction::Retry, Target::Send(Command::Catalog(CatalogCmd::Retry(Resource::Artist(id)))));
    let artist = match &snapshot.catalog.artist {
        Some((open, Load::Ready(artist))) if *open == id => Arc::clone(artist),
        Some((open, Load::Failed(error))) if *open == id => {
            page.status = PageStatus::Failed { message: error.to_string() };
            return page;
        }
        _ => return page,
    };
    let (extras, extras_failed) = match snapshot.external.artist(id) {
        Some(Load::Ready(extras)) => (Some(extras), false),
        Some(Load::Failed(_)) => (None, true),
        _ => (None, false),
    };
    let tracks: Arc<[Track]> = Arc::from(artist.tracks.as_slice());
    let library = &snapshot.library;
    let portrait = portrait_url(&artist)
        .or_else(|| extras.and_then(|e| e.artist.image_url.as_deref()))
        .map(str::to_owned);
    let following = library.followed.iter().any(|known| known.id == id);
    let radio = feed::popular(&tracks, library.stats.as_slice(), 1).into_iter().next();
    let has_top = extras.is_some_and(|extras| !extras.top.is_empty());

    // Riproduci: the official top tracks (the server imports them one at a time); without them,
    // the server's tracks.
    let mut buttons = Vec::new();
    if has_top {
        let play = ExternalCmd::PlayArtistTop { artist: id, start: 0, video: false };
        page.action(PageAction::Play, Target::Send(Command::External(play)));
        buttons.push(button(PageAction::Play, "Riproduci", true, false));
    } else if !tracks.is_empty() {
        page.action(PageAction::Play, Target::Play(Arc::clone(&tracks), 0));
        buttons.push(button(PageAction::Play, "Riproduci", true, false));
    }
    buttons.push(button(PageAction::Follow, if following { "Segui già" } else { "Segui" }, false, following));
    let followed = FollowedArtist { id, name: artist.name.clone(), cover_url: portrait.clone() };
    let notice = if following {
        format!("Non segui più {}", artist.name)
    } else {
        format!("Segui {}: le sue nuove uscite compaiono in Novità", artist.name)
    };
    page.action(
        PageAction::Follow,
        Target::Notify(Command::library(LibraryCmd::ToggleFollow(followed)), notice),
    );
    if let Some(track) = radio {
        buttons.push(button(PageAction::Radio, "Radio", false, false));
        page.action(
            PageAction::Radio,
            Target::Send(Command::Playback(PlaybackCmd::StartRadio(Box::new(track)))),
        );
    }
    page.header = Some(Header {
        kicker: "Artista".into(),
        title: artist.name.clone(),
        subtitle: None,
        subtitle_opens: false,
        meta: meta(extras, tracks.len()),
        art: Art::Round { url: portrait },
        buttons,
        note: None,
    });
    page.status = PageStatus::Ready;

    match extras {
        Some(extras) => official(&mut page, id, extras, ui, tracks.is_empty()),
        None if extras_failed => {
            if tracks.is_empty() {
                page.note("extras", "Il catalogo ufficiale non ha risposto.".into(), Tone::Error);
            }
        }
        None => page.note("extras", "Cerco nel catalogo ufficiale…".into(), Tone::Waiting),
    }
    if !tracks.is_empty() {
        let title = if has_top { "Già nel catalogo del server" } else { "Brani" };
        page.heading("server", title, format::tracks(tracks.len()));
        super::album::track_rows(&mut page, &tracks, Some(&artist.name));
    }
    related(&mut page, &artist, snapshot);
    if !tracks.is_empty() {
        about(&mut page, &artist, &tracks, snapshot);
        credits(&mut page, &tracks);
    }
    page
}

/// *Popolari* and *Discografia*, from the official catalogue.
fn official(page: &mut Built, id: Uuid, extras: &ArtistExtras, ui: &Ui, no_tracks: bool) {
    let shown = if ui.more { extras.top.len() } else { extras.top.len().min(POPULAR) };
    if shown > 0 {
        page.heading("popular", "Popolari", String::new());
        for (index, item) in extras.top.iter().take(shown).enumerate() {
            let row = Row::Track {
                key: format!("popular:{index}:{}", item.id),
                number: None,
                art: Some(Art::Cover { url: item.cover_url.clone() }),
                title: item.title.clone(),
                subtitle: source_detail(item),
                duration: item.duration_ms.map(format::clock).unwrap_or_default(),
                state: RowState::Idle,
                more: false,
                download: DownloadMark::None,
            };
            let play = ExternalCmd::PlayArtistTop { artist: id, start: index, video: false };
            let mark = Mark::Import(Importing::Source(item.source, item.id.clone()));
            page.push(row, mark, Target::Send(Command::External(play)));
        }
        if shown < extras.top.len() {
            let row = Row::Action { key: "more".into(), label: "Mostra altri".into() };
            page.push(row, Mark::None, Target::MorePopular);
        }
    }
    let albums = extras.albums.iter().filter(|a| a.kind == "album").count();
    let singles = extras.albums.len() - albums;
    let tabs = albums > 0 && singles > 0;
    // With one kind of release the filters are not shown and do not count.
    let filter = if tabs { ui.chip.min(FILTERS.len() - 1) } else { 0 };
    if !extras.albums.is_empty() {
        page.heading("discography", "Discografia", String::new());
        if tabs {
            let chips = FILTERS
                .iter()
                .enumerate()
                .map(|(index, label)| Chip { label: (*label).into(), on: index == filter });
            page.push(Row::Chips { key: "filters".into(), chips: chips.collect() }, Mark::None, Target::None);
        }
        let keeps = |release: &SourcePlaylist| match filter {
            1 => release.kind == "album",
            2 => release.kind != "album",
            _ => true,
        };
        for (index, release) in extras.albums.iter().filter(|r| keeps(r)).enumerate() {
            let row = Row::Item {
                key: format!("release:{index}:{}", release.id),
                art: Art::Cover { url: release.cover_url.clone() },
                title: release.title.clone(),
                detail: release_subtitle(release),
                state: RowState::Idle,
            };
            page.push(row, Mark::None, Target::Open(Route::Source { source: SourceRef::of(release) }));
        }
    }
    if shown == 0 && extras.albums.is_empty() && no_tracks {
        page.note("empty", "Questo artista non ha ancora brani da ascoltare.".into(), Tone::Plain);
    }
}

/// «2024 · Album».
fn release_subtitle(release: &SourcePlaylist) -> String {
    match release.released_on.as_deref().and_then(|date| date.get(..4)) {
        Some(year) => format!("{year} · {}", release.kind_label()),
        None => release.kind_label().to_owned(),
    }
}

/// «4 album · 6 singoli ed EP · 12.400 fan» from the official catalogue; «N brani» until then.
fn meta(extras: Option<&ArtistExtras>, tracks: usize) -> String {
    let Some(extras) = extras else { return format::tracks(tracks) };
    let mut parts = Vec::new();
    let albums = extras.albums.iter().filter(|a| a.kind == "album").count();
    if albums > 0 {
        parts.push(format!("{albums} album"));
    }
    let others = extras.albums.len() - albums;
    if others > 0 {
        parts.push(format!("{others} singoli ed EP"));
    }
    if extras.artist.fans > 0 {
        parts.push(format::fans(extras.artist.fans));
    }
    if parts.is_empty() { format::tracks(tracks) } else { parts.join(" · ") }
}

/// «Altri artisti nel catalogo», from the Home feed.
fn related(page: &mut Built, artist: &ArtistPage, snapshot: &Snapshot) {
    let others: Vec<_> = snapshot
        .library
        .feed
        .artists
        .iter()
        .filter(|card| card.artist.id != artist.id)
        .take(RELATED)
        .collect();
    if others.is_empty() {
        return;
    }
    page.heading("related", "Altri artisti nel catalogo", String::new());
    for card in others {
        let row = Row::Item {
            key: format!("related:{}", card.artist.id),
            art: Art::Round { url: card.cover_url.clone() },
            title: card.artist.name.clone(),
            detail: "Artista".into(),
            state: RowState::Idle,
        };
        page.push(row, Mark::None, Target::Open(Route::artist(card.artist.id)));
    }
}

/// *Informazioni*: tracks and length, your listens, the licences.
fn about(page: &mut Built, artist: &ArtistPage, tracks: &[Track], snapshot: &Snapshot) {
    page.heading("about", "Informazioni", String::new());
    let total = tracks.iter().map(|t| t.duration_ms).sum();
    page.note("about:length", format::length(tracks.len(), total), Tone::Plain);
    let plays: u32 = snapshot
        .library
        .stats
        .iter()
        .filter(|stat| stat.track.artist.id == artist.id)
        .map(|stat| stat.plays)
        .sum();
    if plays > 0 {
        page.note("about:plays", format!("Li hai ascoltati {plays} volte"), Tone::Plain);
    }
    let licences = format::licence_summary(tracks.iter().map(|t| t.licence.label()));
    page.note("about:licences", format!("Licenze: {licences}"), Tone::Plain);
}
