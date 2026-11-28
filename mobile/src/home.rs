//! The Home as shelves of tiles, built from the catalogue's Home sections and the feed the library
//! derives from them (`LibraryView::feed`), in the order of the phone Home. The
//! app draws the tiles; what a tap does is decided here ([`HomeModel::activate`]), with the rules of
//! the phone Home: an album opens its page, a single plays (or pauses if it is the current
//! track), a mix plays the feed's mix, a track plays its shelf from there.

use std::sync::Arc;

use psytify_core::api::{AlbumSummary, Track};
use psytify_core::catalog::Load;
use psytify_core::command::PlaybackCmd;
use psytify_core::feed::{Feed, Release};
use psytify_core::player::PlaybackState;
use psytify_core::shared::Shared;
use psytify_core::snapshot::{CatalogView, LibraryView, PlaybackView};
use uuid::Uuid;

use crate::pages::Route;

/// The latest releases in «Aggiunti di recente».
const RELEASES: usize = 16;
/// The releases of followed artists in «Dai tuoi artisti».
const NEWS: usize = 12;
/// The last tracks listened to in «Ascoltati di recente».
const LISTENED: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ShelfKind {
    Releases,
    News,
    Mixes,
    Listened,
    Albums,
    Alphabetical,
}

/// One cover on a shelf.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Tile {
    /// Stable within the shelf: the app keys its list items with it.
    pub key: String,
    pub title: String,
    pub subtitle: String,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Shelf {
    pub kind: ShelfKind,
    pub title: String,
    pub tiles: Vec<Tile>,
}

/// How the catalogue part of the Home is doing. Shelves that do not need the server (what was
/// listened to) are there in every state.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HomeStatus {
    Loading,
    Ready,
    /// The server answered with an empty catalogue.
    Empty {
        message: String,
    },
    /// «Riprova» sends `HomeAction::Retry`.
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HomeSnapshot {
    /// Grows with every rebuild; `PsytifyCore::activate_home` takes it back to ignore stale taps.
    pub version: u64,
    pub status: HomeStatus,
    pub shelves: Vec<Shelf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HomeAction {
    /// The Home is shown: load it if it is not there, rebuild the feed if the listens changed.
    Ensure,
    /// «Riprova»: reload from the network.
    Retry,
}

/// What a tap on a tile does.
#[derive(Debug, Clone)]
pub(crate) enum HomeTap {
    Play(PlaybackCmd),
    Open(Route),
}

/// What a tile plays.
#[derive(Debug, Clone)]
enum Target {
    Release(Release),
    Album(Uuid),
    Mix(usize),
    Listened(usize),
    Alphabetical(usize),
}

/// The shelves as built, kept by the bridge to resolve a tap.
#[derive(Debug, Default)]
pub(crate) struct HomeModel {
    version: u64,
    shelves: Vec<Vec<Target>>,
    listened: Shared<Track>,
    alphabetical: Option<Arc<[Track]>>,
}

impl HomeModel {
    /// The Home for these views. `version` is the bridge's counter, not a core version.
    pub(crate) fn build(version: u64, catalog: &CatalogView, library: &LibraryView) -> (HomeSnapshot, Self) {
        let mut shelves = Vec::new();
        let mut model = Self { version, listened: library.recent.clone(), ..Self::default() };
        let feed: &Feed = &library.feed;
        let mut push = |kind: ShelfKind, title: &str, tiles: Vec<(Tile, Target)>| {
            if tiles.is_empty() {
                return;
            }
            let (tiles, targets): (Vec<Tile>, Vec<Target>) = tiles.into_iter().unzip();
            shelves.push(Shelf { kind, title: title.to_owned(), tiles });
            model.shelves.push(targets);
        };

        push(ShelfKind::Releases, "Aggiunti di recente", releases(&feed.releases, RELEASES));
        push(ShelfKind::News, "Dai tuoi artisti", releases(&feed.news, NEWS));
        let mixes = feed.mixes.iter().enumerate().map(|(index, mix)| {
            let tile = Tile {
                key: format!("mix:{index}"),
                title: mix.title.clone(),
                subtitle: mix.subtitle.clone(),
                cover_url: mix.tracks.iter().find_map(|track| track.cover_url.clone()),
            };
            (tile, Target::Mix(index))
        });
        push(ShelfKind::Mixes, "Mix per te", mixes.collect());
        let listened = library.recent.iter().take(LISTENED).enumerate();
        push(
            ShelfKind::Listened,
            "Ascoltati di recente",
            listened.map(|(index, track)| (track_tile(track), Target::Listened(index))).collect(),
        );
        if let Load::Ready(albums) = &catalog.home.albums {
            push(ShelfKind::Albums, "Album", albums.iter().map(album_tile).collect());
        }
        if let Load::Ready(tracks) = &catalog.home.alphabetical {
            model.alphabetical = Some(Arc::clone(tracks));
            let tiles = tracks.iter().enumerate();
            push(
                ShelfKind::Alphabetical,
                "Dalla A alla Z",
                tiles.map(|(index, track)| (track_tile(track), Target::Alphabetical(index))).collect(),
            );
        }

        let status = match &catalog.home.recent {
            Load::Idle | Load::Loading => HomeStatus::Loading,
            Load::Failed(err) => HomeStatus::Failed { message: err.to_string() },
            Load::Ready(tracks) if tracks.is_empty() => {
                HomeStatus::Empty { message: "Il catalogo non ha ancora brani pubblicati.".into() }
            }
            Load::Ready(_) => HomeStatus::Ready,
        };
        (HomeSnapshot { version, status, shelves }, model)
    }

    /// What a tap on tile `tile` of shelf `shelf` does, built for Home `version`. `None` for a tap
    /// on a Home that has been rebuilt since, or on a tile that is not there.
    pub(crate) fn activate(
        &self,
        version: u64,
        shelf: usize,
        tile: usize,
        playback: &PlaybackView,
    ) -> Option<HomeTap> {
        if version != self.version {
            return None;
        }
        let current = playback.queue.current().filter(|_| !matches!(playback.state, PlaybackState::Stopped));
        let command = match self.shelves.get(shelf)?.get(tile)? {
            Target::Release(release) if release.is_album => {
                return Some(HomeTap::Open(Route::album(release.id)));
            }
            Target::Release(release) => {
                let single = release.tracks.first()?;
                play_in(Arc::from([single.clone()]), 0, current)?
            }
            Target::Album(id) => return Some(HomeTap::Open(Route::album(*id))),
            Target::Mix(index) => PlaybackCmd::PlayMix(*index),
            Target::Listened(index) => {
                let list: Arc<[Track]> = Arc::from(&self.listened[..self.listened.len().min(LISTENED)]);
                play_in(list, *index, current)?
            }
            Target::Alphabetical(index) => play_in(Arc::clone(self.alphabetical.as_ref()?), *index, current)?,
        };
        Some(HomeTap::Play(command))
    }
}

/// Plays `list` from `start`, or pauses and resumes if that track is the current one.
fn play_in(list: Arc<[Track]>, start: usize, current: Option<&Track>) -> Option<PlaybackCmd> {
    let chosen = list.get(start)?;
    if current.is_some_and(|track| track.id == chosen.id) {
        return Some(PlaybackCmd::Toggle);
    }
    Some(PlaybackCmd::Play { tracks: list, start })
}

fn releases(releases: &[Release], limit: usize) -> Vec<(Tile, Target)> {
    releases
        .iter()
        .take(limit)
        .map(|release| {
            let tile = Tile {
                key: format!("release:{}", release.id),
                title: release.title.clone(),
                subtitle: format!("{} · {}", release.kind_label(), release.artist.name),
                cover_url: release.cover_url.clone(),
            };
            (tile, Target::Release(release.clone()))
        })
        .collect()
}

fn track_tile(track: &Track) -> Tile {
    Tile {
        key: format!("track:{}", track.id),
        title: track.title.clone(),
        subtitle: track.artist.name.clone(),
        cover_url: track.cover_url.clone(),
    }
}

fn album_tile(album: &AlbumSummary) -> (Tile, Target) {
    let tile = Tile {
        key: format!("album:{}", album.id),
        title: album.title.clone(),
        subtitle: album.artist.name.clone(),
        cover_url: album.cover_url.clone(),
    };
    (tile, Target::Album(album.id))
}

#[cfg(test)]
mod tests {
    use psytify_core::api::Page;
    use psytify_core::api::models::{AlbumRef, ArtistRef};
    use psytify_core::command::{CatalogCmd, Command, Resource};
    use psytify_core::config::Config;
    use psytify_core::error::AppError;
    use psytify_core::player::Queue;
    use psytify_core::testing::{LocalCore, Section};

    use super::*;
    use crate::test_support::track;

    fn page<T>(items: Vec<T>) -> Page<T> {
        Page { items, limit: 50, offset: 0, next_offset: None }
    }

    fn album_track(n: u128, album: u128) -> Track {
        let mut track = track(n);
        track.album = Some(AlbumRef { id: Uuid::from_u128(album), title: format!("Album {album}") });
        track
    }

    fn album(n: u128) -> AlbumSummary {
        AlbumSummary {
            id: Uuid::from_u128(n),
            title: format!("Album {n}"),
            released_on: None,
            artist: ArtistRef { id: Uuid::from_u128(1), name: "Artist".into() },
            track_count: 3,
            cover_url: None,
        }
    }

    /// A core whose Home has loaded: two tracks of one album and a single, two albums, A to Z.
    fn loaded() -> LocalCore {
        let mut core = LocalCore::new(Config::default(), 1);
        core.send(Command::Catalog(CatalogCmd::Ensure(Resource::Home)));
        core.home_loaded(Section::Recent, Ok(page(vec![album_track(1, 50), album_track(2, 50), track(3)])));
        core.home_loaded(Section::Alphabetical, Ok(page(vec![track(3), album_track(1, 50)])));
        core.home_albums_loaded(Ok(page(vec![album(50), album(60)])));
        core
    }

    fn build(core: &LocalCore) -> (HomeSnapshot, HomeModel) {
        let snapshot = core.snapshot();
        HomeModel::build(1, &snapshot.catalog, &snapshot.library)
    }

    fn playing(tracks: Vec<Track>, start: usize) -> PlaybackView {
        let mut queue = Queue::default();
        queue.replace(tracks, start, 1);
        PlaybackView { state: PlaybackState::Playing, queue: Arc::new(queue), ..PlaybackView::default() }
    }

    #[test]
    fn before_the_server_answers_the_home_is_loading_with_no_shelves() {
        let mut core = LocalCore::new(Config::default(), 1);
        core.send(Command::Catalog(CatalogCmd::Ensure(Resource::Home)));
        let (home, _) = build(&core);
        assert_eq!(home.status, HomeStatus::Loading);
        assert!(home.shelves.is_empty());
    }

    #[test]
    fn a_failure_is_shown_with_its_message() {
        let mut core = LocalCore::new(Config::default(), 1);
        core.send(Command::Catalog(CatalogCmd::Ensure(Resource::Home)));
        core.home_loaded(Section::Recent, Err(AppError::Network));
        let (home, _) = build(&core);
        assert_eq!(home.status, HomeStatus::Failed { message: AppError::Network.to_string() });
    }

    #[test]
    fn an_empty_catalogue_says_so() {
        let mut core = LocalCore::new(Config::default(), 1);
        core.send(Command::Catalog(CatalogCmd::Ensure(Resource::Home)));
        core.home_loaded(Section::Recent, Ok(page(Vec::new())));
        let (home, _) = build(&core);
        assert!(matches!(home.status, HomeStatus::Empty { .. }));
    }

    #[test]
    fn a_loaded_home_has_the_phone_shelves_in_order() {
        let (home, _) = build(&loaded());
        assert_eq!(home.status, HomeStatus::Ready);
        let kinds: Vec<ShelfKind> =
            home.shelves.iter().map(|shelf| shelf.kind).filter(|k| *k != ShelfKind::Mixes).collect();
        assert_eq!(kinds, [ShelfKind::Releases, ShelfKind::Albums, ShelfKind::Alphabetical]);
        let releases = &home.shelves[0];
        assert_eq!(releases.title, "Aggiunti di recente");
        // The two tracks of album 50 are one release; the single is another.
        assert_eq!(releases.tiles.len(), 2);
        assert!(releases.tiles.iter().any(|tile| tile.subtitle.starts_with("Album · ")));
        assert!(releases.tiles.iter().any(|tile| tile.subtitle.starts_with("Singolo · ")));
        let keys: std::collections::HashSet<&str> = releases.tiles.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys.len(), releases.tiles.len(), "keys are unique within a shelf");
    }

    fn played(tap: Option<HomeTap>) -> Option<PlaybackCmd> {
        match tap {
            Some(HomeTap::Play(command)) => Some(command),
            _ => None,
        }
    }

    #[test]
    fn an_album_opens_its_page_and_a_single_plays_or_pauses() {
        let (home, model) = build(&loaded());
        let idle = PlaybackView::default();
        let album_at =
            home.shelves[0].tiles.iter().position(|t| t.subtitle.starts_with("Album")).unwrap_or(9);
        let single_at =
            home.shelves[0].tiles.iter().position(|t| t.subtitle.starts_with("Singolo")).unwrap_or(9);
        assert!(
            matches!(model.activate(1, 0, album_at, &idle), Some(HomeTap::Open(route)) if route == Route::album(Uuid::from_u128(50)))
        );
        match played(model.activate(1, 0, single_at, &idle)) {
            Some(PlaybackCmd::Play { tracks, start: 0 }) => assert_eq!(tracks[0].id, Uuid::from_u128(3)),
            other => panic!("unexpected {other:?}"),
        }
        let single = playing(vec![track(3)], 0);
        assert!(matches!(played(model.activate(1, 0, single_at, &single)), Some(PlaybackCmd::Toggle)));
        let albums = home.shelves.iter().position(|s| s.kind == ShelfKind::Albums).unwrap_or(9);
        assert!(matches!(model.activate(1, albums, 1, &idle), Some(HomeTap::Open(Route::Album { .. }))));
    }

    #[test]
    fn an_a_to_z_tile_plays_the_whole_shelf_from_it() {
        let (home, model) = build(&loaded());
        let shelf = home.shelves.iter().position(|s| s.kind == ShelfKind::Alphabetical).unwrap_or(9);
        match played(model.activate(1, shelf, 1, &PlaybackView::default())) {
            Some(PlaybackCmd::Play { tracks, start: 1 }) => assert_eq!(tracks.len(), 2),
            other => panic!("unexpected {other:?}"),
        }
        let current = playing(vec![album_track(1, 50)], 0);
        assert!(matches!(played(model.activate(1, shelf, 1, &current)), Some(PlaybackCmd::Toggle)));
    }

    #[test]
    fn stale_or_missing_taps_do_nothing() {
        let (_, model) = build(&loaded());
        let idle = PlaybackView::default();
        assert!(model.activate(2, 0, 0, &idle).is_none(), "a Home rebuilt since");
        assert!(model.activate(1, 40, 0, &idle).is_none());
        assert!(model.activate(1, 0, 40, &idle).is_none());
    }

    #[test]
    fn listened_tracks_have_their_shelf_without_the_server() {
        let mut core = LocalCore::new(Config::default(), 1);
        core.send(Command::library(psytify_core::command::LibraryCmd::RememberListen(track(7))));
        let (home, model) = build(&core);
        let shelf = home.shelves.iter().position(|s| s.kind == ShelfKind::Listened);
        assert_eq!(shelf.map(|at| home.shelves[at].tiles.len()), Some(1));
        let at = shelf.unwrap_or(9);
        assert!(matches!(
            played(model.activate(1, at, 0, &PlaybackView::default())),
            Some(PlaybackCmd::Play { .. })
        ));
    }
}
