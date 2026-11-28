//! The playlist dialogs, as on the desktop (`desktop/src/app/playlist_dialog.rs`): the name of a new
//! playlist (empty, from Libreria, or with a track, from the «Altro» sheet), rename, and the
//! confirmation before deleting. The bridge keeps the open dialog (one at a time, with the tracks
//! it starts with); the app draws a [`DialogView`] and sends back the text. The core cleans the name.
//! The same dialog confirms removing a friend («Togliere NOME dagli amici?»).

use psytify_core::Command;
use psytify_core::Snapshot;
use psytify_core::api::Track;
use psytify_core::command::{LibraryCmd, SocialCmd};
use psytify_core::config::{self, MAX_PLAYLISTS};
use uuid::Uuid;

use crate::menu::{Next, Outcome};
use crate::pages::Route;

/// A dialog as the bridge keeps it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Request {
    /// From Libreria (empty: it opens once made) or from a track's menu (it starts with it).
    NewPlaylist {
        seed: Vec<Track>,
    },
    Rename {
        id: Uuid,
        name: String,
    },
    Delete {
        id: Uuid,
        name: String,
    },
    /// Removing a friend, from their profile (the owner's choice: a confirmation, as on the desktop).
    Unfriend {
        handle: String,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DialogView {
    /// Which dialog: an answer to one that is no longer open does nothing.
    pub id: u64,
    pub title: String,
    pub hint: Option<String>,
    /// The name field and its first text, for a new playlist and a rename.
    pub field: Option<String>,
    pub confirm: String,
    /// The confirmation deletes something: the button says so.
    pub destructive: bool,
}

/// The name proposed for a new playlist.
fn suggested_name(playlists: usize) -> String {
    format!("La mia playlist n. {}", playlists + 1)
}

fn new_playlist_hint(seed: usize) -> String {
    match seed {
        0 => "Dai un nome alla playlist. Potrai aggiungere i brani dal menu «Altro» di un brano.".to_owned(),
        1 => "La playlist parte con il brano che hai scelto.".to_owned(),
        n => format!("La playlist parte con questi {n} brani."),
    }
}

pub(crate) fn view(id: u64, request: &Request, snapshot: &Snapshot) -> DialogView {
    match request {
        Request::NewPlaylist { seed } => DialogView {
            id,
            title: "Nuova playlist".into(),
            hint: Some(new_playlist_hint(seed.len())),
            field: Some(suggested_name(snapshot.library.playlists.len())),
            confirm: "Crea".into(),
            destructive: false,
        },
        Request::Rename { name, .. } => DialogView {
            id,
            title: "Rinomina la playlist".into(),
            hint: None,
            field: Some(name.clone()),
            confirm: "Salva".into(),
            destructive: false,
        },
        Request::Delete { name, .. } => DialogView {
            id,
            title: format!("Eliminare «{name}»?"),
            hint: Some("La playlist esce dalla libreria. I brani restano nel catalogo.".into()),
            field: None,
            confirm: "Elimina".into(),
            destructive: true,
        },
        Request::Unfriend { name, .. } => DialogView {
            id,
            title: format!("Togliere {name} dagli amici?"),
            hint: Some(
                "Non vedrete più cosa ascolta l'altro. Potete tornare amici con una nuova richiesta.".into(),
            ),
            field: None,
            confirm: "Togli".into(),
            destructive: true,
        },
    }
}

/// The dialog confirmed with `text` (the field's text, if it has one).
pub(crate) fn confirm(request: Request, text: Option<String>, snapshot: &Snapshot) -> Outcome {
    let typed = text.unwrap_or_default();
    match request {
        Request::NewPlaylist { seed } => {
            if snapshot.library.playlists.len() >= MAX_PLAYLISTS {
                let mut out = Outcome::next(Next::Nothing);
                out.notice = Some((format!("Al massimo {MAX_PLAYLISTS} playlist"), true));
                return out;
            }
            let id = Uuid::new_v4();
            let name = config::clean_name(&typed);
            let notice = format!("Playlist «{name}» creata");
            // From Libreria the new playlist opens; from a track's menu the person stays where they are.
            let next = if seed.is_empty() {
                Next::Open(Route::Playlist { id: id.to_string() })
            } else {
                Next::Nothing
            };
            let create = LibraryCmd::CreatePlaylist { id, name, tracks: seed, pending: Vec::new() };
            Outcome { commands: vec![Command::library(create)], notice: Some((notice, false)), next }
        }
        Request::Rename { id, .. } => {
            Outcome::send(Command::library(LibraryCmd::RenamePlaylist { id, name: typed }), None)
        }
        Request::Delete { id, name } => Outcome {
            commands: vec![Command::library(LibraryCmd::DeletePlaylist(id))],
            notice: Some((format!("Playlist «{name}» eliminata"), false)),
            next: Next::Leave(Route::Playlist { id: id.to_string() }),
        },
        // The core tells the outcome with its own social event.
        Request::Unfriend { handle, .. } => {
            Outcome::send(Command::Social(SocialCmd::FriendAction { handle, add: false }), None)
        }
    }
}

#[cfg(test)]
mod tests {
    use psytify_core::config::Config;
    use psytify_core::testing::LocalCore;

    use super::*;
    use crate::test_support::track;

    #[test]
    fn a_new_playlist_from_libreria_opens_and_one_from_a_menu_starts_with_its_track() {
        let core = LocalCore::new(Config::default(), 1);
        let snapshot = core.snapshot();
        let empty = Request::NewPlaylist { seed: Vec::new() };
        let shown = view(3, &empty, &snapshot);
        assert_eq!(shown.field.as_deref(), Some("La mia playlist n. 1"));
        assert!(!shown.destructive);

        let out = confirm(empty, Some("  Sera  ".into()), &snapshot);
        let [Command::Library(cmd)] = &out.commands[..] else { panic!("{:?}", out.commands) };
        let LibraryCmd::CreatePlaylist { id, name, tracks, .. } = &**cmd else { panic!("{cmd:?}") };
        assert_eq!((name.as_str(), tracks.len()), ("Sera", 0));
        assert_eq!(out.next, Next::Open(Route::Playlist { id: id.to_string() }));
        assert_eq!(out.notice, Some(("Playlist «Sera» creata".into(), false)));

        let seeded = Request::NewPlaylist { seed: vec![track(1)] };
        assert_eq!(
            view(4, &seeded, &snapshot).hint.as_deref(),
            Some("La playlist parte con il brano che hai scelto.")
        );
        assert_eq!(confirm(seeded, Some("Notte".into()), &snapshot).next, Next::Nothing);
    }

    #[test]
    fn rename_and_delete_send_their_commands_and_delete_leaves_the_page() {
        let core = LocalCore::new(Config::default(), 1);
        let snapshot = core.snapshot();
        let id = Uuid::from_u128(4);
        let rename = Request::Rename { id, name: "Sera".into() };
        assert_eq!(view(1, &rename, &snapshot).field.as_deref(), Some("Sera"));
        let out = confirm(rename, Some("Notte".into()), &snapshot);
        assert!(matches!(
            &out.commands[..],
            [Command::Library(cmd)] if matches!(&**cmd, LibraryCmd::RenamePlaylist { name, .. } if name == "Notte")
        ));
        let delete = Request::Delete { id, name: "Sera".into() };
        let shown = view(2, &delete, &snapshot);
        assert!(shown.destructive && shown.field.is_none());
        assert_eq!(shown.title, "Eliminare «Sera»?");
        let out = confirm(delete, None, &snapshot);
        assert_eq!(out.next, Next::Leave(Route::Playlist { id: id.to_string() }));
        assert_eq!(out.notice.map(|n| n.0).as_deref(), Some("Playlist «Sera» eliminata"));
    }
}
