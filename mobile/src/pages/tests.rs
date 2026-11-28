//! The page models on a synchronous core (`LocalCore`): which rows, in which order, and what a tap
//! or a button sends.

use std::sync::Arc;

use psytify_core::api::models::{AlbumRef, ArtistRef};
use psytify_core::api::{
    Album, ArtistPage, MetaArtist, MetaArtistPage, MetaSearch, SearchResults, SourceItem, SourceKind,
    SourcePlaylist, Track,
};
use psytify_core::command::{CatalogCmd, Command, ExternalCmd, LibraryCmd, PlaybackCmd, Resource};
use psytify_core::config::{Config, FollowedArtist};
use psytify_core::error::AppError;
use psytify_core::testing::{LocalCore, PendingJob};
use uuid::Uuid;

use super::*;
use crate::test_support::track;

fn kinds(page: &Built) -> Vec<String> {
    page.rows
        .iter()
        .map(|row| match row {
            Row::Heading { title, .. } => format!("# {title}"),
            Row::Note { text, .. } => format!("- {text}"),
            Row::Track { title, .. } => format!("t {title}"),
            Row::Item { title, .. } => format!("i {title}"),
            Row::Chips { chips, .. } => format!("c {}", chips.len()),
            Row::Action { label, .. } => format!("a {label}"),
            Row::Download { title, status, .. } => format!("d {title} {status}"),
            Row::Saved { title, detail, .. } => format!("s {title} {detail}"),
            Row::Person { name, status, buttons, .. } => {
                let labels: Vec<&str> = buttons.iter().map(|b| b.label.as_str()).collect();
                format!("p {name} ({status}) [{}]", labels.join(", "))
            }
            Row::JamStart { failure, .. } => format!("start {}", failure.as_deref().unwrap_or("")),
            Row::JamRoom { code, members, .. } => {
                format!("room {} {}", code.as_deref().unwrap_or("…"), members.join(", "))
            }
        })
        .collect()
}

fn source(n: u32) -> SourceItem {
    SourceItem {
        source: SourceKind::Deezer,
        id: format!("{n}"),
        title: format!("Official {n}"),
        artist: "Artist".into(),
        duration_ms: Some(200_000),
        cover_url: None,
    }
}

fn release(n: u32, kind: &str) -> SourcePlaylist {
    SourcePlaylist {
        source: SourceKind::Deezer,
        id: format!("album/{n}"),
        title: format!("Release {n}"),
        owner: "Artist".into(),
        cover_url: None,
        kind: kind.into(),
        released_on: Some("2021-05-01".into()),
    }
}

/// A core that searched «low»: the three answers with `results`, `meta`, and nothing on Archive.
fn searched(results: Result<SearchResults, AppError>, meta: MetaSearch) -> LocalCore {
    let mut core = LocalCore::new(Config::default(), 1);
    core.send(Command::Catalog(CatalogCmd::SearchInput("low".into())));
    core.fire_search();
    let jobs = core.take_jobs();
    let ticket = |want: fn(&PendingJob) -> Option<u64>| jobs.iter().find_map(want).unwrap();
    let search = ticket(|job| if let PendingJob::Search { ticket, .. } = job { Some(*ticket) } else { None });
    let discover =
        ticket(|job| if let PendingJob::Discover { ticket, .. } = job { Some(*ticket) } else { None });
    let official = ticket(|job| if let PendingJob::Meta { ticket, .. } = job { Some(*ticket) } else { None });
    core.searched(search, results);
    core.meta_searched(official, Ok(meta));
    core.discovered(discover, Err(AppError::Network));
    core
}

#[test]
fn search_sections_follow_the_order_of_the_other_frontends() {
    let results = SearchResults { tracks: vec![track(1), track(2)], ..SearchResults::default() };
    let meta = MetaSearch {
        tracks: (1..=14).map(source).collect(),
        artists: vec![MetaArtist { id: 9, name: "Low".into(), image_url: None, fans: 12_400 }],
        lists: vec![release(1, "album"), release(2, "playlist")],
    };
    let core = searched(Ok(results), meta);
    let page = search::build(&core.snapshot());
    assert_eq!(page.status, PageStatus::Ready);
    let rows = kinds(&page);
    let heading = |title: &str| rows.iter().position(|row| row == &format!("# {title}")).unwrap();
    let order =
        ["Artisti", "Catalogo ufficiale", "Album", "Playlist", "Già nel catalogo", "Da Internet Archive"];
    let positions: Vec<usize> = order.iter().map(|title| heading(title)).collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{rows:#?}");
    let official = rows.iter().filter(|row| row.starts_with("t Official")).count();
    assert_eq!(official, 12, "the official tracks stop at twelve");
    assert_eq!(rows.last().map(String::as_str), Some(&*format!("- {}", AppError::Network)));

    // An official track imports and plays with one command; a server track plays the results.
    let at = rows.iter().position(|row| row == "t Official 1").unwrap();
    assert!(matches!(
        &page.targets[at],
        Target::Send(Command::External(ExternalCmd::PlaySource { source: SourceKind::Deezer, id, video: false })) if id == "1"
    ));
    assert_eq!(
        page.marks[at],
        Mark::Import(psytify_core::snapshot::Importing::Source(SourceKind::Deezer, "1".into()))
    );
    let at = rows.iter().position(|row| row == "t Track 2").unwrap();
    assert!(matches!(&page.targets[at], Target::Play(list, 1) if list.len() == 2));
    let at = rows.iter().position(|row| row == "i Low").unwrap();
    assert!(matches!(&page.targets[at], Target::Send(Command::Catalog(CatalogCmd::OpenMetaArtist(9)))));
    let at = rows.iter().position(|row| row == "i Release 1").unwrap();
    assert!(matches!(&page.targets[at], Target::Open(Route::Source { source }) if source.id == "album/1"));
}

#[test]
fn search_states_without_rows() {
    let idle = LocalCore::new(Config::default(), 1);
    assert!(matches!(search::build(&idle.snapshot()).status, PageStatus::Empty { .. }));

    let mut loading = LocalCore::new(Config::default(), 1);
    loading.send(Command::Catalog(CatalogCmd::Search("low".into())));
    assert_eq!(search::build(&loading.snapshot()).status, PageStatus::Loading);

    let failed =
        searched(Err(AppError::Network), MetaSearch { tracks: vec![], artists: vec![], lists: vec![] });
    let page = search::build(&failed.snapshot());
    // Archive failed too: its note is a row, so the page shows it rather than a bare error.
    assert_eq!(page.status, PageStatus::Ready);
    let empty =
        searched(Ok(SearchResults::default()), MetaSearch { tracks: vec![], artists: vec![], lists: vec![] });
    let page = search::build(&empty.snapshot());
    assert!(kinds(&page).iter().any(|row| row == "# Da Internet Archive"));
}

#[test]
fn past_searches_are_rows_that_search_again() {
    let mut core = LocalCore::new(Config::default(), 1);
    core.send(Command::library(LibraryCmd::RememberSearch("daft punk".into())));
    let page = search::build(&core.snapshot());
    assert_eq!(kinds(&page), ["# Ricerche recenti", "i daft punk"]);
    assert!(matches!(&page.targets[1], Target::Search(query) if query == "daft punk"));
}

fn album_track(n: u128, album: u128) -> Track {
    let mut track = track(n);
    track.album = Some(AlbumRef { id: Uuid::from_u128(album), title: "Album".into() });
    track.artist = ArtistRef { id: Uuid::from_u128(500), name: "Artist".into() };
    track
}

fn album(id: u128) -> Album {
    Album {
        id: Uuid::from_u128(id),
        title: "Album".into(),
        released_on: Some("2024-01-01".into()),
        artist: ArtistRef { id: Uuid::from_u128(500), name: "Artist".into() },
        cover_url: None,
        tracks: vec![album_track(1, id), album_track(2, id)],
    }
}

#[test]
fn an_album_page_loads_shows_its_tracks_and_saves() {
    let id = Uuid::from_u128(50);
    let mut core = LocalCore::new(Config::default(), 1);
    core.send(Command::Catalog(CatalogCmd::Ensure(Resource::Album(id))));
    assert_eq!(album::build(id, &core.snapshot()).status, PageStatus::Loading);
    core.catalog_album(id, Ok(album(50)));
    let page = album::build(id, &core.snapshot());
    assert_eq!(page.status, PageStatus::Ready);
    let header = page.header.clone().unwrap();
    assert_eq!(
        (header.kicker.as_str(), header.meta.as_str()),
        ("Album · 2024", "2 brani · 6 min · Lossless")
    );
    assert_eq!(
        header.buttons.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
        ["Riproduci", "Casuale", "Salva", "Scarica i brani disponibili"]
    );
    assert_eq!(kinds(&page), ["t Track 1", "t Track 2"]);
    assert!(matches!(&page.rows[0], Row::Track { number: Some(1), subtitle, .. } if subtitle.is_empty()));
    let save = page.actions.iter().find(|(action, _)| *action == PageAction::Save).map(|(_, target)| target);
    let Some(Target::Notify(Command::Library(command), _)) = save else { panic!("{save:?}") };
    core.send(Command::Library(command.clone()));
    let saved = album::build(id, &core.snapshot());
    assert!(saved.header.unwrap().buttons.iter().any(|b| b.label == "Salvato" && b.on));
    let open = saved.actions.iter().find(|(action, _)| *action == PageAction::OpenArtist).map(|(_, t)| t);
    assert!(matches!(open, Some(Target::Open(route)) if *route == Route::artist(Uuid::from_u128(500))));

    // Another album open in the core: this page waits for its own.
    assert_eq!(album::build(Uuid::from_u128(51), &core.snapshot()).status, PageStatus::Loading);
    let failed = Uuid::from_u128(52);
    core.send(Command::Catalog(CatalogCmd::Ensure(Resource::Album(failed))));
    core.catalog_album(failed, Err(AppError::Network));
    assert_eq!(
        album::build(failed, &core.snapshot()).status,
        PageStatus::Failed { message: AppError::Network.to_string() }
    );
}

#[test]
fn an_artist_page_has_popular_discography_and_server_tracks() {
    let id = Uuid::from_u128(500);
    let mut core = LocalCore::new(Config::default(), 1);
    core.send(Command::Catalog(CatalogCmd::Ensure(Resource::Artist(id))));
    let artist = ArtistPage {
        id,
        name: "Artist".into(),
        image_url: None,
        albums: vec![],
        tracks: vec![album_track(1, 50)],
    };
    core.catalog_artist(id, false, Ok(artist));
    let page = artist::build(id, &core.snapshot(), &Ui::default());
    assert_eq!(kinds(&page)[0], "- Cerco nel catalogo ufficiale…", "«Altro di …» is on its way");
    let extras = MetaArtistPage {
        artist: MetaArtist { id: 9, name: "Artist".into(), image_url: None, fans: 2_000 },
        top: (1..=7).map(source).collect(),
        albums: vec![release(1, "album"), release(2, "single")],
    };
    core.artist_extras_loaded(id, Ok(extras));
    let page = artist::build(id, &core.snapshot(), &Ui::default());
    let rows = kinds(&page);
    assert_eq!(
        &rows[..7],
        [
            "# Popolari",
            "t Official 1",
            "t Official 2",
            "t Official 3",
            "t Official 4",
            "t Official 5",
            "a Mostra altri"
        ]
    );
    assert_eq!(&rows[7..11], ["# Discografia", "c 3", "i Release 1", "i Release 2"]);
    assert!(rows.contains(&"# Già nel catalogo del server".to_owned()));
    assert!(rows.contains(&"# Informazioni".to_owned()));
    let header = page.header.clone().unwrap();
    assert_eq!(header.meta, "1 album · 1 singoli ed EP · 2.000 fan");
    let play = page.actions.iter().find(|(action, _)| *action == PageAction::Play).map(|(_, t)| t);
    assert!(matches!(
        play,
        Some(Target::Send(Command::External(ExternalCmd::PlayArtistTop { start: 0, .. })))
    ));

    let more = artist::build(id, &core.snapshot(), &Ui { more: true, ..Ui::default() });
    assert_eq!(kinds(&more).iter().filter(|row| row.starts_with("t Official")).count(), 7);
    let singles = artist::build(id, &core.snapshot(), &Ui { chip: 2, ..Ui::default() });
    assert!(
        kinds(&singles).contains(&"i Release 2".to_owned())
            && !kinds(&singles).contains(&"i Release 1".to_owned())
    );

    core.send(Command::library(LibraryCmd::ToggleFollow(FollowedArtist {
        id,
        name: "Artist".into(),
        cover_url: None,
    })));
    let followed = artist::build(id, &core.snapshot(), &Ui::default());
    assert!(followed.header.unwrap().buttons.iter().any(|b| b.label == "Segui già" && b.on));
}

#[test]
fn the_library_lists_liked_downloads_playlists_albums_and_artists_with_filters() {
    let mut core = LocalCore::new(Config::default(), 1);
    core.send(Command::library(LibraryCmd::ToggleLike(track(1))));
    let playlist = Uuid::from_u128(77);
    core.send(Command::library(LibraryCmd::CreatePlaylist {
        id: playlist,
        name: "Notte".into(),
        tracks: vec![track(2)],
        pending: vec![],
    }));
    let artist = FollowedArtist { id: Uuid::from_u128(500), name: "Artist".into(), cover_url: None };
    core.send(Command::library(LibraryCmd::ToggleFollow(artist)));
    let page = library::build(&core.snapshot(), &Ui::default());
    assert_eq!(kinds(&page), ["c 4", "i Brani che ti piacciono", "i Scaricati", "i Notte", "i Artist"]);
    assert!(matches!(&page.targets[1], Target::Open(Route::Liked)));
    assert!(matches!(&page.targets[2], Target::Open(Route::Downloads)));
    assert!(matches!(&page.targets[3], Target::Open(Route::Playlist { id }) if *id == playlist.to_string()));
    let albums = library::build(&core.snapshot(), &Ui { chip: 2, ..Ui::default() });
    assert_eq!(kinds(&albums), ["c 4", "- Salva un album dalla sua pagina: lo ritrovi qui."]);
}

#[test]
fn the_liked_page_plays_from_the_tapped_track() {
    let mut core = LocalCore::new(Config::default(), 1);
    core.send(Command::library(LibraryCmd::ToggleLike(track(1))));
    core.send(Command::library(LibraryCmd::ToggleLike(track(2))));
    let page = collection::liked(&core.snapshot());
    assert_eq!(kinds(&page), ["t Track 2", "t Track 1"]);
    assert!(matches!(&page.targets[1], Target::Play(list, 1) if list.len() == 2));
    let empty = collection::liked(&LocalCore::new(Config::default(), 1).snapshot());
    assert!(matches!(empty.status, PageStatus::Empty { .. }));
    let gone = collection::playlist(Uuid::from_u128(1), &core.snapshot());
    assert!(matches!(gone.status, PageStatus::Empty { .. }));
}

#[test]
fn a_source_page_opens_its_playlist_and_plays_from_a_track() {
    let playlist = release(1, "album");
    let mut core = LocalCore::new(Config::default(), 1);
    let page = Page::Source(playlist.clone());
    let ensure = page.ensure(&core.snapshot());
    assert!(matches!(ensure.as_slice(), [Command::External(ExternalCmd::OpenPlaylist(p))] if *p == playlist));
    core.send(ensure.into_iter().next().unwrap());
    assert!(page.ensure(&core.snapshot()).is_empty(), "already open: nothing to ask");
    let built = source::build(&playlist, &core.snapshot());
    assert_eq!(built.status, PageStatus::Loading);
    let ticket = core
        .take_jobs()
        .into_iter()
        .find_map(
            |job| if let PendingJob::ExternalPlaylist { ticket, .. } = job { Some(ticket) } else { None },
        )
        .unwrap();
    let items = psytify_core::api::SourcePlaylistPage {
        source: SourceKind::Deezer,
        id: playlist.id.clone(),
        title: playlist.title.clone(),
        owner: playlist.owner.clone(),
        items: vec![source(1), source(2)],
    };
    core.playlist_loaded(ticket, SourceKind::Deezer, Ok(items));
    let built = source::build(&playlist, &core.snapshot());
    assert_eq!(built.status, PageStatus::Ready);
    assert_eq!(kinds(&built), ["t Official 1", "t Official 2"]);
    assert!(matches!(
        &built.targets[1],
        Target::Send(Command::External(ExternalCmd::PlayPlaylist { start: 1, .. }))
    ));
    let header = built.header.unwrap();
    assert_eq!((header.kicker.as_str(), header.meta.as_str()), ("Album", "Artist · 2 brani · 7 min"));
}

#[test]
fn row_states_follow_the_current_track_without_rebuilding() {
    let mut core = LocalCore::new(Config::default(), 1);
    core.send(Command::library(LibraryCmd::ToggleLike(track(1))));
    let snapshot = core.snapshot();
    let mut model = Model::new(Page::Liked);
    assert!(model.take(&snapshot));
    assert!(!model.take(&snapshot), "the same snapshot: nothing new");
    assert!(matches!(model.row(0), Some(Row::Track { state: RowState::Idle, .. })));
    core.send(Command::Playback(PlaybackCmd::Play { tracks: Arc::from([track(1)]), start: 0 }));
    assert!(model.take(&core.snapshot()), "the track loads");
    assert!(matches!(model.row(0), Some(Row::Track { state: RowState::Busy, .. })));
    assert_eq!(model.view().total, 1);
}

#[test]
fn an_invalid_route_is_a_failed_page() {
    let core = LocalCore::new(Config::default(), 1);
    let page = build(
        &Page::of(&Route::Album { id: "nope".into() }),
        &core.snapshot(),
        &Ui::default(),
        &mut downloads::Memo::default(),
    );
    assert!(matches!(page.status, PageStatus::Failed { .. }));
}

fn model(page: Page) -> Model {
    Model::new(page)
}

/// An official row has no track until it is imported; then it is that server track: the menu, the
/// swipe and the equaliser while it plays.
#[test]
fn an_official_row_becomes_its_imported_track() {
    let meta = MetaSearch { tracks: vec![source(1)], ..MetaSearch::default() };
    let mut core = searched(Ok(SearchResults::default()), meta);
    let mut page = model(Page::Search);
    assert!(page.take(&core.snapshot()));
    let at = kinds(&page.built).iter().position(|row| row == "t Official 1").unwrap();
    assert!(matches!(page.row(at), Some(Row::Track { more: false, state: RowState::Idle, .. })));
    assert!(page.track_at(at).is_none());

    core.send(Command::External(ExternalCmd::PlaySource {
        source: SourceKind::Deezer,
        id: "1".into(),
        video: false,
    }));
    let jobs = core.take_jobs();
    let target = jobs.iter().find_map(|job| match job {
        PendingJob::Import { target, .. } => Some(*target),
        _ => None,
    });
    core.imported(
        target.unwrap(),
        Ok(psytify_core::api::DiscoverImport { album: None, tracks: vec![track(41)] }),
    );
    assert!(page.take(&core.snapshot()));
    assert!(matches!(page.row(at), Some(Row::Track { more: true, state: RowState::Busy, .. })), "it loads");
    assert_eq!(page.track_at(at).map(|(track, from)| (track.id, from)), Some((Uuid::from_u128(41), None)));
}

#[test]
fn a_playlist_row_knows_its_place_for_the_menu() {
    let mut core = LocalCore::new(Config::default(), 1);
    let id = Uuid::from_u128(77);
    core.send(Command::library(LibraryCmd::CreatePlaylist {
        id,
        name: "Sera".into(),
        tracks: vec![track(1), track(2)],
        pending: Vec::new(),
    }));
    let mut page = model(Page::Playlist(id));
    page.take(&core.snapshot());
    assert_eq!(page.track_at(1).map(|(track, from)| (track.id, from)), Some((track(2).id, Some((id, 1)))));
    let header = page.built.header.clone().unwrap();
    let labels: Vec<&str> = header.buttons.iter().map(|button| button.label.as_str()).collect();
    assert_eq!(labels, ["Riproduci", "Casuale", "Scarica i brani disponibili", "Rinomina", "Elimina"]);
    assert!(matches!(
        page.built.actions.iter().find(|(action, _)| *action == PageAction::Delete),
        Some((_, Target::Dialog(Request::Delete { name, .. }))) if name == "Sera"
    ));
    let library = build(&Page::Library, &core.snapshot(), &Ui::default(), &mut downloads::Memo::default());
    assert!(library.actions.iter().any(|(action, _)| *action == PageAction::NewPlaylist));
}

fn free(n: u128) -> Track {
    let mut track = track(n);
    track.downloadable = true;
    track
}

fn delivered(n: u128) -> psytify_core::api::Downloaded {
    psytify_core::api::Downloaded {
        path: std::env::temp_dir().join(format!("psytify-mobile-{n}.flac")),
        size: 2_000_000,
        sha256: String::new(),
        format: "flac".into(),
        delivered: Some(psytify_core::api::DownloadQuality::Lossless),
        asked: psytify_core::api::DownloadQuality::Lossless,
    }
}

#[test]
fn downloads_page_shows_the_queue_then_the_device_with_their_buttons() {
    use psytify_core::command::DownloadsCmd;

    let mut core = LocalCore::new(Config::default(), 1);
    let mut page = model(Page::Downloads);
    page.take(&core.snapshot());
    assert!(matches!(page.built.status, PageStatus::Empty { .. }));
    assert!(page.built.header.as_ref().is_some_and(|header| header.buttons.is_empty()));

    core.send(Command::Downloads(DownloadsCmd::Download(Box::new(free(1)))));
    core.send(Command::Downloads(DownloadsCmd::Download(Box::new(free(2)))));
    assert!(page.take(&core.snapshot()));
    let rows = kinds(&page.built);
    assert_eq!(rows[0], "# In corso");
    assert!(rows[1].starts_with("d Track 1 ") && rows[2].starts_with("d Track 2 "));
    assert_eq!(rows[3], "- Niente sul dispositivo, per ora: i brani arrivano qui appena scaricati.");
    let buttons = |page: &Model, at: usize| -> Vec<RowAction> {
        page.built.row_actions.iter().filter(|(row, ..)| *row == at).map(|(_, action, _)| *action).collect()
    };
    assert_eq!(buttons(&page, 1), [RowAction::Cancel]);
    assert!(matches!(
        page.built.row_actions.iter().find(|(row, ..)| *row == 1),
        Some((_, _, Target::Send(Command::Downloads(DownloadsCmd::Cancel(id))))) if *id == Uuid::from_u128(1)
    ));

    core.downloaded(Uuid::from_u128(1), Ok(delivered(1)));
    core.downloaded(Uuid::from_u128(2), Err(AppError::Network));
    assert!(page.take(&core.snapshot()));
    let rows = kinds(&page.built);
    assert_eq!(rows.len(), 4);
    assert_eq!((rows[0].as_str(), rows[2].as_str()), ("# In corso", "# Sul dispositivo"));
    assert_eq!(rows[3], "s Track 1 Artist 1 · FLAC · 2,0 MB");
    assert!(kinds(&page.built)[1].starts_with("d Track 2 Non riuscito."));
    assert_eq!(buttons(&page, 1), [RowAction::Retry, RowAction::Dismiss]);
    assert_eq!(buttons(&page, 3), [RowAction::Remove]);
    assert!(matches!(page.built.targets[3], Target::Play(ref list, 0) if list.len() == 1));
    let header = page.built.header.clone().unwrap();
    assert_eq!(header.meta, "1 brano · 2,0 MB");
    let labels: Vec<&str> = header.buttons.iter().map(|button| button.label.as_str()).collect();
    assert_eq!(labels, ["Riproduci", "Casuale", "Controlla i file"]);
    assert!(page.track_at(3).is_some(), "a saved track has its «Altro» sheet");
}

#[test]
fn track_rows_carry_the_download_mark_and_lists_offer_download_all() {
    use psytify_core::command::DownloadsCmd;

    use crate::downloads::DownloadMark;

    let mut core = LocalCore::new(Config::default(), 1);
    core.send(Command::library(LibraryCmd::ToggleLike(free(1))));
    core.send(Command::library(LibraryCmd::ToggleLike(free(2))));
    let mut page = model(Page::Liked);
    page.take(&core.snapshot());
    let marks = |page: &Model| -> Vec<DownloadMark> {
        (0..2)
            .map(|at| match page.row(at) {
                Some(Row::Track { download, .. }) => download,
                _ => DownloadMark::None,
            })
            .collect()
    };
    assert_eq!(marks(&page), [DownloadMark::None, DownloadMark::None]);
    let header = page.built.header.clone().unwrap();
    assert!(header.buttons.iter().any(|button| button.label == "Scarica tutto"));

    // «Scarica tutto»: two at a time, the count on the button while this list downloads.
    let tap = page.built.actions.iter().find(|(action, _)| *action == PageAction::DownloadAll);
    let Some((_, Target::Send(all))) = tap.cloned() else { panic!("no download all") };
    core.send(all);
    assert!(page.take(&core.snapshot()));
    assert_eq!(marks(&page), [DownloadMark::Downloading, DownloadMark::Downloading]);
    let header = page.built.header.clone().unwrap();
    assert!(header.buttons.iter().any(|button| button.label == "0/2" && button.on));

    core.downloaded(Uuid::from_u128(1), Ok(delivered(1)));
    core.send(Command::Downloads(DownloadsCmd::Cancel(Uuid::from_u128(2))));
    assert!(page.take(&core.snapshot()));
    // The newest like first: Track 2 (cancelled), then Track 1 (saved).
    assert_eq!(marks(&page), [DownloadMark::None, DownloadMark::Saved]);
}

mod social {
    use psytify_core::api::Me;
    use psytify_core::api::social::{
        Friend, Friends, JamInvite, MyProfile, NowPlaying, PlaylistSummary, Profile, PublicPlaylist,
        Relation, UserPage,
    };
    use psytify_core::auth::{ClerkApp, Session};
    use psytify_core::command::JamCmd;
    use psytify_core::jam::{Event, Member};

    use super::*;
    use crate::dialog::Request;
    use crate::pages::jam::{Attempt, accept, failure, retry};

    fn person(handle: &str) -> Profile {
        Profile { handle: handle.into(), display_name: handle.to_uppercase(), avatar_url: None }
    }

    fn signed_in() -> LocalCore {
        let mut config = Config::default();
        config.core.clerk_issuer = "https://clerk.example.test".into();
        let mut core = LocalCore::new(config, 1);
        let profile = MyProfile {
            handle: "ada".into(),
            display_name: "Ada".into(),
            avatar_url: None,
            share_activity: true,
        };
        let me = Me { user_id: "user_1".into(), admin: false, profile: Some(profile) };
        let app = ClerkApp { issuer: "https://clerk.example.test".into(), port: 0, session_file: None };
        core.signed_in(Session::for_tests(app), me);
        core
    }

    fn friends() -> Friends {
        let now = NowPlaying { track: track(1), position_ms: 0, paused: false };
        Friends {
            friends: vec![
                Friend { profile: person("luca"), now_playing: Some(now) },
                Friend { profile: person("sara"), now_playing: None },
            ],
            incoming: vec![person("giulia")],
            outgoing: vec![person("paolo")],
        }
    }

    #[test]
    fn friends_need_a_profile_and_then_list_requests_waiting_and_friends() {
        let signed_out = LocalCore::new(Config::default(), 1);
        assert!(matches!(friends::build(&signed_out.snapshot()).status, PageStatus::Empty { .. }));

        let mut core = signed_in();
        assert_eq!(kinds(&friends::build(&core.snapshot())), ["- Carico gli amici…"]);
        core.friends_loaded(Ok(friends()));
        core.users_found(Ok(vec![person("luca"), person("lucia"), person("ada")]));
        let page = friends::build(&core.snapshot());
        assert_eq!(
            kinds(&page),
            [
                "# Persone",
                "p LUCA (@luca) [Amici]",
                "p LUCIA (@lucia) [Aggiungi]",
                "p ADA (@ada) []",
                "# Richieste",
                "p GIULIA (@giulia) [Accetta, Rifiuta]",
                "# In attesa",
                "p PAOLO (Richiesta inviata) [Ritira]",
                "# I tuoi amici",
                "p LUCA (Sta ascoltando Track 1 · Artist 1) []",
                "p SARA (@sara) []",
            ]
        );
        // The listening friend has the live dot; a tap opens the profile.
        assert!(matches!(&page.rows[9], Row::Person { live: true, .. }));
        assert!(matches!(&page.targets[9], Target::Open(Route::Profile { handle }) if handle == "luca"));
        let accept = page.row_actions.iter().find(|(row, action, _)| *row == 5 && *action == RowAction::Add);
        assert!(matches!(
            accept,
            Some((_, _, Target::Send(Command::Social(SocialCmd::FriendAction { handle, add: true })))) if handle == "giulia"
        ));
        assert!(page.row_actions.iter().all(|(row, _, _)| *row != 1), "«Amici» is a state, not a button");

        // While a request for Giulia is on its way her buttons are off.
        core.send(Command::Social(SocialCmd::FriendAction { handle: "giulia".into(), add: true }));
        let page = friends::build(&core.snapshot());
        assert!(matches!(&page.rows[5], Row::Person { buttons, .. } if buttons.iter().all(|b| !b.enabled)));
        assert!(page.row_actions.iter().all(|(row, _, _)| *row != 5));
    }

    #[test]
    fn nobody_and_no_search_is_the_empty_page() {
        let mut core = signed_in();
        core.friends_loaded(Ok(Friends::default()));
        assert!(matches!(friends::build(&core.snapshot()).status, PageStatus::Empty { .. }));
        assert_eq!(friends::search_query("  @luca "), "luca");
    }

    fn user(relation: Relation) -> UserPage {
        UserPage {
            profile: person("luca"),
            relation,
            friend_count: 2,
            mutual_friends: vec![person("sara")],
            playlists: vec![PlaylistSummary { id: Uuid::from_u128(9), name: "Sera".into(), track_count: 3 }],
            activity: None,
        }
    }

    #[test]
    fn a_profile_has_its_relation_button_playlists_and_mutual_friends() {
        let mut core = signed_in();
        core.send(Command::Social(SocialCmd::OpenProfile("luca".into())));
        core.user_opened("luca", Ok(user(Relation::Friend)));
        let page = profile::build("luca", &core.snapshot());
        let buttons: Vec<(String, bool)> =
            page.header.iter().flat_map(|h| h.buttons.iter().map(|b| (b.label.clone(), b.on))).collect();
        assert_eq!(buttons, [("Amici".to_owned(), true)]);
        // Removing a friend asks first.
        let relation = page.actions.iter().find(|(action, _)| *action == PageAction::Relation);
        assert!(matches!(
            relation,
            Some((_, Target::Dialog(Request::Unfriend { handle, name }))) if handle == "luca" && name == "LUCA"
        ));
        assert_eq!(
            kinds(&page),
            [
                "- L'attività è per gli amici: quando lo siete vedi cosa ascolta, gli ascolti recenti e gli artisti preferiti.",
                "# Playlist pubbliche",
                "i Sera",
                "# Amici in comune",
                "p SARA (@sara) []"
            ]
        );
        let opens = page.targets.get(2).cloned();
        assert!(
            matches!(opens, Some(Target::Open(Route::FriendPlaylist { handle, .. })) if handle == "luca")
        );

        core.send(Command::Social(SocialCmd::ReloadProfile));
        core.user_opened("luca", Ok(user(Relation::None)));
        let page = profile::build("luca", &core.snapshot());
        let relation = page.actions.iter().find(|(action, _)| *action == PageAction::Relation);
        assert!(matches!(
            relation,
            Some((_, Target::Send(Command::Social(SocialCmd::FriendAction { add: true, .. }))))
        ));
        let snapshot = core.snapshot();
        let shown = crate::dialog::view(
            1,
            &Request::Unfriend { handle: "luca".into(), name: "Luca".into() },
            &snapshot,
        );
        assert_eq!((shown.title.as_str(), shown.destructive), ("Togliere Luca dagli amici?", true));
    }

    #[test]
    fn a_public_playlist_opens_as_a_collection_with_its_owner() {
        let mut core = signed_in();
        let id = Uuid::from_u128(9);
        core.send(Command::Social(SocialCmd::OpenFriendPlaylist { handle: "luca".into(), id }));
        assert_eq!(collection::public("luca", id, &core.snapshot()).status, PageStatus::Loading);
        let tracks: Arc<[Track]> = Arc::from(vec![track(1), track(2)]);
        core.friend_playlist_loaded(Ok(Arc::new(PublicPlaylist {
            id,
            name: "Sera".into(),
            owner: person("luca"),
            tracks,
        })));
        let page = collection::public("luca", id, &core.snapshot());
        let header = page.header.clone().unwrap_or_else(|| panic!("no header"));
        assert_eq!(
            (header.kicker.as_str(), header.subtitle.as_deref()),
            ("Playlist pubblica", Some("di LUCA"))
        );
        assert!(header.subtitle_opens);
        assert_eq!(kinds(&page), ["t Track 1", "t Track 2"]);
        let owner = page.actions.iter().find(|(action, _)| *action == PageAction::OpenArtist);
        assert!(matches!(owner, Some((_, Target::Open(Route::Profile { handle }))) if handle == "luca"));
    }

    #[test]
    fn the_jam_page_goes_from_the_invites_to_the_room_and_back_with_the_error() {
        let mut core = signed_in();
        core.friends_loaded(Ok(friends()));
        core.invites_loaded(Ok(vec![JamInvite {
            code: "K7Q2MX".into(),
            from_handle: "luca".into(),
            from_name: "Luca".into(),
        }]));
        let page = jam::build(&core.snapshot(), &Ui::default());
        assert_eq!(
            kinds(&page),
            ["# Inviti", "p Luca (@luca ti invita in una jam) [Entra, Ignora]", "start "]
        );
        assert!(page.row_actions.iter().any(|(row, action, target)| *row == 1
            && *action == RowAction::Join
            && matches!(target, Target::JoinInvite(code) if code == "K7Q2MX")));
        assert!(accept("K7Q2MX", true).is_err(), "already in a jam");
        assert_eq!(accept("K7Q2MX", false).map(|c| c.len()), Ok(2));

        core.send(Command::Jam(JamCmd::Create { name: String::new() }));
        assert_eq!(kinds(&jam::build(&core.snapshot(), &Ui::default())), ["room … "]);
        let members = vec![Member { id: Uuid::from_u128(1), name: "Ada".into() }];
        core.jam_event(Event::Joined {
            code: "K7Q2MX".into(),
            you: Uuid::from_u128(1),
            members,
            state: None,
        });
        let ui = Ui { chosen: vec!["sara".into()], ..Ui::default() };
        let page = jam::build(&core.snapshot(), &ui);
        assert_eq!(
            kinds(&page),
            [
                "room K7Q2MX Ada",
                "# Persone",
                "p Ada (Tu) []",
                "# Invita amici",
                "p LUCA (Sta ascoltando Track 1 · Artist 1) [Scegli]",
                "p SARA (@sara) [Scelto]",
                "a Invia l'invito",
            ]
        );
        assert!(page.row_actions.iter().any(|(_, action, target)| *action == RowAction::Copy
            && matches!(target, Target::Copy(code) if code == "K7Q2MX")));
        assert!(matches!(page.targets.last(), Some(Target::Invite)));

        core.jam_event(Event::Closed("La jam si è chiusa".into()));
        let page = jam::build(&core.snapshot(), &Ui::default());
        assert!(
            matches!(page.rows.last(), Some(Row::JamStart { failure: Some(text), retry: true, .. }) if text == "La jam si è chiusa")
        );
        let seq = failure(&core.snapshot().jam, 0).map(|(seq, _, _)| seq).unwrap_or_default();
        let dismissed = Ui { jam_dismissed: seq, ..Ui::default() };
        assert!(matches!(
            jam::build(&core.snapshot(), &dismissed).rows.last(),
            Some(Row::JamStart { failure: None, .. })
        ));
        assert!(
            matches!(retry(None, " Ada ", "K7Q2MX"), (Attempt::Join, JamCmd::Join { name, .. }) if name == "Ada")
        );
        assert!(matches!(retry(None, "", "K7Q"), (Attempt::Create, JamCmd::Create { .. })));
        assert_eq!(jam::jam_code_chars("k7q-2mx9"), "K7Q2MX");
        assert!(jam::jam_code_valid("K7Q2MX") && !jam::jam_code_valid("K7Q"));
    }
}
