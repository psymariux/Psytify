//! The «Altro» sheet of a track, with the items of the GPUI menu (`desktop/src/app/track_menu.rs`):
//! play next, add to the queue, radio; add to a playlist (the list, with «Nuova playlist…» first)
//! and, on a playlist, remove from it; go to the artist or the album, follow, copy and share;
//! «Scarica · qualità», «Scarica in…» (the four qualities) and «Rimuovi dai download», decided as
//! the desktop's `DownloadItems::of`.
//!
//! The bridge keeps the open menu (one at a time) with its track; the app gets a [`MenuView`] and
//! sends back the [`MenuChoice`]. Each choice is an [`Outcome`]: the commands, the notice the
//! desktop gives, and what the app must do next.

use psytify_core::Command;
use psytify_core::Snapshot;
use psytify_core::api::DownloadQuality;
use psytify_core::api::Track;
use psytify_core::command::{DownloadsCmd, LibraryCmd, PlaybackCmd};
use psytify_core::config::FollowedArtist;
use uuid::Uuid;

use crate::dialog::Request;
use crate::downloads::Quality;
use crate::pages::{Art, Route};

/// An item of the sheet.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MenuChoice {
    PlayNext,
    Enqueue,
    Radio,
    /// Shows the playlists in place of the items (the app's own step).
    ShowPlaylists,
    NewPlaylist,
    AddTo {
        id: String,
    },
    RemoveFromPlaylist,
    GoToArtist,
    GoToAlbum,
    ToggleFollow,
    Copy,
    Share,
    /// «Scarica · qualità»: at the quality of the settings.
    Download,
    /// «Scarica in…»: shows the qualities in place of the items (the app's own step).
    ShowQualities,
    DownloadAs {
        quality: Quality,
    },
    RemoveDownload,
}

/// A quality of «Scarica in…».
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MenuQuality {
    pub quality: Quality,
    /// «Alta · AAC 256 kbps».
    pub label: String,
    /// The quality of the settings: the check.
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MenuItem {
    pub choice: MenuChoice,
    pub label: String,
    pub enabled: bool,
    /// A switch that is on (following the artist): the check icon.
    pub on: bool,
}

/// A playlist the track can go to.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MenuPlaylist {
    pub id: String,
    pub name: String,
    /// The track is already in it.
    pub has: bool,
}

/// The open sheet: the track at its top, the items, the playlists of «Aggiungi a una playlist».
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MenuView {
    /// Which menu: choices for a menu that is no longer open do nothing.
    pub id: u64,
    pub title: String,
    pub subtitle: String,
    pub art: Art,
    pub items: Vec<MenuItem>,
    pub playlists: Vec<MenuPlaylist>,
    pub qualities: Vec<MenuQuality>,
}

/// The menu the bridge keeps open.
#[derive(Debug, Clone)]
pub(crate) struct OpenMenu {
    pub(crate) id: u64,
    pub(crate) track: Track,
    /// The playlist and the position the row came from.
    pub(crate) from_playlist: Option<(Uuid, usize)>,
}

/// What the app must do after a choice, besides what the bridge sent to the core.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Next {
    Nothing,
    /// The menu stays open (the list of playlists).
    Stay,
    Open(Route),
    Copy(String),
    Share(String),
    Dialog(Request),
    /// The page at this route is gone (a deleted playlist).
    Leave(Route),
}

/// A choice or a dialog, decided: what goes to the core, what to tell, what comes next.
#[derive(Debug, Clone)]
pub(crate) struct Outcome {
    pub(crate) commands: Vec<Command>,
    pub(crate) notice: Option<(String, bool)>,
    pub(crate) next: Next,
}

impl Outcome {
    pub(crate) fn next(next: Next) -> Self {
        Self { commands: Vec::new(), notice: None, next }
    }

    pub(crate) fn send(command: Command, notice: Option<String>) -> Self {
        Self { commands: vec![command], notice: notice.map(|text| (text, false)), next: Next::Nothing }
    }
}

/// The text of «Copia titolo e artista», as on the desktop.
pub(crate) fn copy_text(track: &Track) -> String {
    format!("{} — {}", track.title, track.artist.name)
}

fn following(track: &Track, snapshot: &Snapshot) -> bool {
    snapshot.library.followed.iter().any(|known| known.id == track.artist.id)
}

pub(crate) fn view(menu: &OpenMenu, snapshot: &Snapshot) -> MenuView {
    let track = &menu.track;
    let item = |choice, label: &str| MenuItem { choice, label: label.to_owned(), enabled: true, on: false };
    let mut items = vec![
        item(MenuChoice::PlayNext, "Riproduci subito dopo"),
        item(MenuChoice::Enqueue, "Aggiungi alla coda"),
        item(MenuChoice::Radio, "Avvia radio"),
        item(MenuChoice::ShowPlaylists, "Aggiungi a una playlist"),
    ];
    if menu.from_playlist.is_some() {
        items.push(item(MenuChoice::RemoveFromPlaylist, "Togli da questa playlist"));
    }
    items.push(item(MenuChoice::GoToArtist, "Vai all'artista"));
    if track.album.is_some() {
        items.push(item(MenuChoice::GoToAlbum, "Vai all'album"));
    }
    let follows = following(track, snapshot);
    let label = if follows { "Smetti di seguire" } else { "Segui l'artista" };
    items.push(MenuItem { on: follows, ..item(MenuChoice::ToggleFollow, label) });
    items.push(item(MenuChoice::Copy, "Copia titolo e artista"));
    items.push(item(MenuChoice::Share, "Condividi"));
    // The download items of the desktop, decided the same way: a track whose file is in place or
    // that is already on its way is not downloaded again; one in the index can be removed.
    let downloads = &snapshot.downloads;
    if track.downloadable && !downloads.is_busy(track.id) && !downloads.is_offline_ready(track.id) {
        let label = format!("Scarica · {}", downloads.quality.label());
        items.push(item(MenuChoice::Download, &label));
        items.push(item(MenuChoice::ShowQualities, "Scarica in…"));
    }
    if downloads.is_saved(track.id) {
        items.push(item(MenuChoice::RemoveDownload, "Rimuovi dai download"));
    }
    let qualities = DownloadQuality::ALL
        .iter()
        .map(|quality| MenuQuality {
            quality: Quality::of(*quality),
            label: format!("{} · {}", quality.label(), quality.detail()),
            on: *quality == downloads.quality,
        })
        .collect();
    let playlists = snapshot
        .library
        .playlists
        .iter()
        .map(|playlist| MenuPlaylist {
            id: playlist.id.to_string(),
            name: playlist.name.clone(),
            has: playlist.contains(track.id),
        })
        .collect();
    MenuView {
        id: menu.id,
        title: track.title.clone(),
        subtitle: track.artist.name.clone(),
        art: Art::Cover { url: track.cover_url.clone() },
        items,
        playlists,
        qualities,
    }
}

/// What `choice` does to the track of `menu`.
pub(crate) fn choose(menu: &OpenMenu, choice: &MenuChoice, snapshot: &Snapshot) -> Outcome {
    let track = &menu.track;
    let boxed = || Box::new(track.clone());
    match choice {
        MenuChoice::ShowPlaylists | MenuChoice::ShowQualities => Outcome::next(Next::Stay),
        MenuChoice::PlayNext => Outcome::send(
            Command::Playback(PlaybackCmd::EnqueueNext(boxed())),
            Some(format!("«{}» suonerà subito dopo", track.title)),
        ),
        MenuChoice::Enqueue => Outcome::send(
            Command::Playback(PlaybackCmd::Enqueue(boxed())),
            Some(format!("«{}» aggiunto alla coda", track.title)),
        ),
        // The core announces the radio (`PlaybackEvent::RadioStarted`).
        MenuChoice::Radio => Outcome::send(Command::Playback(PlaybackCmd::StartRadio(boxed())), None),
        MenuChoice::NewPlaylist => {
            Outcome::next(Next::Dialog(Request::NewPlaylist { seed: vec![track.clone()] }))
        }
        MenuChoice::AddTo { id } => {
            let Some(playlist) = Uuid::parse_str(id)
                .ok()
                .and_then(|id| snapshot.library.playlists.iter().find(|playlist| playlist.id == id))
            else {
                return Outcome::next(Next::Nothing);
            };
            let notice = if playlist.contains(track.id) {
                format!("«{}» è già in «{}»", track.title, playlist.name)
            } else {
                format!("«{}» aggiunto a «{}»", track.title, playlist.name)
            };
            let add = LibraryCmd::AddToPlaylist { id: playlist.id, track: track.clone() };
            Outcome::send(Command::library(add), Some(notice))
        }
        MenuChoice::RemoveFromPlaylist => match menu.from_playlist {
            Some((id, index)) => {
                Outcome::send(Command::library(LibraryCmd::RemoveFromPlaylist { id, index }), None)
            }
            None => Outcome::next(Next::Nothing),
        },
        MenuChoice::GoToArtist => Outcome::next(Next::Open(Route::artist(track.artist.id))),
        MenuChoice::GoToAlbum => match &track.album {
            Some(album) => Outcome::next(Next::Open(Route::album(album.id))),
            None => Outcome::next(Next::Nothing),
        },
        MenuChoice::ToggleFollow => {
            let artist = FollowedArtist {
                id: track.artist.id,
                name: track.artist.name.clone(),
                cover_url: track.cover_url.clone(),
            };
            let notice = if following(track, snapshot) {
                format!("Non segui più {}", artist.name)
            } else {
                format!("Segui {}: le sue nuove uscite compaiono in Novità", artist.name)
            };
            Outcome::send(Command::library(LibraryCmd::ToggleFollow(artist)), Some(notice))
        }
        MenuChoice::Copy => Outcome::next(Next::Copy(copy_text(track))),
        MenuChoice::Share => Outcome::next(Next::Share(copy_text(track))),
        // The core tells how it went (saved, refused, failed).
        MenuChoice::Download => Outcome::send(Command::Downloads(DownloadsCmd::Download(boxed())), None),
        MenuChoice::DownloadAs { quality } => {
            Outcome::send(Command::Downloads(DownloadsCmd::DownloadAs(boxed(), quality.core())), None)
        }
        MenuChoice::RemoveDownload => Outcome::send(Command::Downloads(DownloadsCmd::Remove(track.id)), None),
    }
}

#[cfg(test)]
mod tests {
    use psytify_core::api::models::AlbumRef;
    use psytify_core::config::{Config, Playlist};
    use psytify_core::testing::LocalCore;

    use super::*;
    use crate::test_support::track;

    fn menu(track: Track, from_playlist: Option<(Uuid, usize)>) -> OpenMenu {
        OpenMenu { id: 1, track, from_playlist }
    }

    fn labels(view: &MenuView) -> Vec<&str> {
        view.items.iter().map(|item| item.label.as_str()).collect()
    }

    #[test]
    fn the_items_follow_the_track_and_where_it_came_from() {
        let core = LocalCore::new(Config::default(), 1);
        let plain = view(&menu(track(1), None), &core.snapshot());
        assert_eq!(
            labels(&plain),
            [
                "Riproduci subito dopo",
                "Aggiungi alla coda",
                "Avvia radio",
                "Aggiungi a una playlist",
                "Vai all'artista",
                "Segui l'artista",
                "Copia titolo e artista",
                "Condividi",
            ]
        );
        assert!(plain.items.iter().all(|item| item.enabled && !item.on));

        let mut full = track(2);
        full.album = Some(AlbumRef { id: Uuid::from_u128(9), title: "Album".into() });
        full.downloadable = true;
        let rich = view(&menu(full, Some((Uuid::from_u128(5), 0))), &core.snapshot());
        let found = labels(&rich);
        assert!(found.contains(&"Togli da questa playlist") && found.contains(&"Vai all'album"));
        let download = rich.items.iter().find(|item| item.choice == MenuChoice::Download).unwrap();
        assert!(download.enabled && download.label.starts_with("Scarica · "));
        assert!(found.contains(&"Scarica in…") && !found.contains(&"Rimuovi dai download"));
        assert_eq!(rich.qualities.len(), 4);
        assert_eq!(rich.qualities.iter().filter(|quality| quality.on).count(), 1);
        assert_eq!(rich.qualities[1].label, "Alta · AAC 256 kbps");
    }

    #[test]
    fn download_choices_become_download_commands() {
        let core = LocalCore::new(Config::default(), 1);
        let mut free = track(3);
        free.downloadable = true;
        let open = menu(free, None);
        let out = choose(&open, &MenuChoice::Download, &core.snapshot());
        assert!(matches!(out.commands[..], [Command::Downloads(DownloadsCmd::Download(_))]));
        let lossless = MenuChoice::DownloadAs { quality: Quality::Lossless };
        let out = choose(&open, &lossless, &core.snapshot());
        assert!(matches!(
            out.commands[..],
            [Command::Downloads(DownloadsCmd::DownloadAs(_, DownloadQuality::Lossless))]
        ));
        assert_eq!(choose(&open, &MenuChoice::ShowQualities, &core.snapshot()).next, Next::Stay);
        let out = choose(&open, &MenuChoice::RemoveDownload, &core.snapshot());
        assert!(
            matches!(out.commands[..], [Command::Downloads(DownloadsCmd::Remove(id))] if id == open.track.id)
        );
    }

    #[test]
    fn choices_become_the_desktop_commands_and_notices() {
        let mut core = LocalCore::new(Config::default(), 1);
        let sera = Playlist::new("Sera", vec![track(1)]);
        let notte = Playlist::new("Notte", Vec::new());
        for playlist in [&sera, &notte] {
            core.send(Command::library(LibraryCmd::CreatePlaylist {
                id: playlist.id,
                name: playlist.name.clone(),
                tracks: playlist.tracks.clone(),
                pending: Vec::new(),
            }));
        }
        let snapshot = core.snapshot();
        let open = menu(track(1), Some((sera.id, 0)));
        let shown = view(&open, &snapshot);
        let has: Vec<bool> = shown.playlists.iter().map(|playlist| playlist.has).collect();
        assert_eq!(has, [true, false]);

        let out = choose(&open, &MenuChoice::Enqueue, &snapshot);
        assert!(matches!(out.commands[..], [Command::Playback(PlaybackCmd::Enqueue(_))]));
        assert_eq!(out.notice, Some(("«Track 1» aggiunto alla coda".into(), false)));

        let into = |playlist: &Playlist| MenuChoice::AddTo { id: playlist.id.to_string() };
        let out = choose(&open, &into(&sera), &snapshot);
        assert_eq!(out.notice.map(|n| n.0).as_deref(), Some("«Track 1» è già in «Sera»"));
        let out = choose(&open, &into(&notte), &snapshot);
        assert_eq!(out.notice.map(|n| n.0).as_deref(), Some("«Track 1» aggiunto a «Notte»"));
        let gone = choose(&open, &MenuChoice::AddTo { id: Uuid::nil().to_string() }, &snapshot);
        assert!(gone.commands.is_empty() && gone.next == Next::Nothing);

        let out = choose(&open, &MenuChoice::RemoveFromPlaylist, &snapshot);
        assert!(matches!(
            &out.commands[..],
            [Command::Library(cmd)] if matches!(**cmd, LibraryCmd::RemoveFromPlaylist { index: 0, .. })
        ));
        assert_eq!(choose(&open, &MenuChoice::ShowPlaylists, &snapshot).next, Next::Stay);
        assert_eq!(
            choose(&open, &MenuChoice::GoToArtist, &snapshot).next,
            Next::Open(Route::artist(Uuid::from_u128(1001)))
        );
        assert_eq!(choose(&open, &MenuChoice::GoToAlbum, &snapshot).next, Next::Nothing, "no album");
        assert_eq!(choose(&open, &MenuChoice::Copy, &snapshot).next, Next::Copy("Track 1 — Artist 1".into()));
        assert!(matches!(
            choose(&open, &MenuChoice::NewPlaylist, &snapshot).next,
            Next::Dialog(Request::NewPlaylist { seed }) if seed.len() == 1
        ));
        let follow = choose(&open, &MenuChoice::ToggleFollow, &snapshot);
        assert!(follow.notice.is_some_and(|(text, _)| text.starts_with("Segui Artist 1")));
    }
}
