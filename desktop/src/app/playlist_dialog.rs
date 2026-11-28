//! The playlist dialog: the name of a new playlist, rename, delete confirmation, as those of the
//! Enter confirms, Esc, «Annulla» or a click on the scrim close it
//! (the scrim belongs to `Shell`, which shows the dialog above all). The same dialog asks to confirm before
//! removing a friend («Togliere NOME dagli amici?», the owner's decision: in the old client «Amici»
//! removed with one click); the core says the outcome with its social event.
//!
//! The dialog sends the command to the core (`LibraryCmd::CreatePlaylist`, `RenamePlaylist`, `DeletePlaylist`)
//! and tells `Shell` what happened ([`DialogEvent`]): the notice to show (the core's library domain has no
//! events, and these are the same local texts), the playlist to open when
//! the core publishes it, the deleted one to remove from the history. The core cleans the name
//! (`config::clean_name`); here it is cleaned only for the notice.

use gpui::{
    AppContext, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, Styled, Window, div,
};
use psytify_core::api::Track;
use psytify_core::command::{LibraryCmd, SocialCmd};
use psytify_core::config::{self, MAX_PLAYLISTS};
use uuid::Uuid;

use crate::components::button::Button;
use crate::components::overlay::Dialog;
use crate::components::search_field::SearchField;
use crate::core_model::CoreModel;
use crate::notices::Notice;

/// Which dialog to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogRequest {
    /// A new playlist: empty (from the Libreria, and then it opens) or with tracks (from the menu of a
    /// track, and then you stay where you are).
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
    /// Removing a friend: their username and the name shown.
    Unfriend {
        handle: String,
        name: String,
    },
}

/// How the dialog ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogEvent {
    /// Closed without doing anything.
    Cancelled,
    Done {
        notice: Option<Notice>,
        /// The playlist just created: it opens when the core publishes it.
        open: Option<Uuid>,
        /// The playlist just deleted: the page and the history let it go.
        deleted: Option<Uuid>,
    },
}

/// What the dialog of a new playlist says, according to the tracks it starts with.
fn new_playlist_hint(seed: usize) -> String {
    match seed {
        0 => "Dai un nome alla playlist. Potrai aggiungere i brani dal menu «Altro» di un brano.".to_owned(),
        1 => "La playlist parte con il brano che hai scelto.".to_owned(),
        n => format!("La playlist parte con questi {n} brani."),
    }
}

/// The title of the confirmation for removing a friend.
fn unfriend_title(name: &str) -> String {
    format!("Togliere {name} dagli amici?")
}

/// The command that removes the friend, after the confirmation.
fn unfriend(handle: &str) -> SocialCmd {
    SocialCmd::FriendAction { handle: handle.to_owned(), add: false }
}

/// The proposed name for a new playlist.
pub fn suggested_name(playlists: usize) -> String {
    format!("La mia playlist n. {}", playlists + 1)
}

pub struct PlaylistDialog {
    core: Entity<CoreModel>,
    request: DialogRequest,
    /// The name field (new playlist and rename).
    field: Option<Entity<SearchField>>,
    focus: FocusHandle,
}

impl EventEmitter<DialogEvent> for PlaylistDialog {}

impl PlaylistDialog {
    pub fn new(
        core: Entity<CoreModel>,
        request: DialogRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial = match &request {
            DialogRequest::NewPlaylist { .. } => {
                Some(suggested_name(core.read(cx).snapshot().library.playlists.len()))
            }
            DialogRequest::Rename { name, .. } => Some(name.clone()),
            DialogRequest::Delete { .. } | DialogRequest::Unfriend { .. } => None,
        };
        let this = cx.entity().downgrade();
        let field = initial.map(|text| {
            let submit = this.clone();
            let escape = this.clone();
            cx.new(|cx| {
                SearchField::new("playlist-name", "Nome della playlist", cx)
                    .plain()
                    .with_text(text)
                    .select_all_text()
                    .on_submit(move |text, _, cx| {
                        // The field is being updated: the text comes from it, it is not read back.
                        let text = text.to_owned();
                        submit.update(cx, |dialog, cx| dialog.confirm(Some(text), cx)).ok();
                    })
                    .on_escape(move |_, cx| {
                        escape.update(cx, |dialog, cx| dialog.cancel(cx)).ok();
                    })
            })
        });
        let focus = cx.focus_handle();
        match &field {
            Some(field) => {
                let handle = field.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
            None => window.focus(&focus, cx),
        }
        Self { core, request, field, focus }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(DialogEvent::Cancelled);
    }

    /// Enter or the main button: the command to the core and the outcome to `Shell`.
    /// `typed` is the text when the field itself sends it (Enter); from the button it is read from the field.
    fn confirm(&mut self, typed: Option<String>, cx: &mut Context<Self>) {
        let typed = typed
            .or_else(|| self.field.as_ref().map(|field| field.read(cx).text().to_owned()))
            .unwrap_or_default();
        let snapshot = self.core.read(cx).snapshot();
        let event = match &self.request {
            DialogRequest::NewPlaylist { seed } => {
                if snapshot.library.playlists.len() >= MAX_PLAYLISTS {
                    DialogEvent::Done {
                        notice: Some(Notice::error(format!("Al massimo {MAX_PLAYLISTS} playlist"))),
                        open: None,
                        deleted: None,
                    }
                } else {
                    let id = Uuid::new_v4();
                    let name = config::clean_name(&typed);
                    let notice = Notice::info(format!("Playlist «{name}» creata"));
                    let open = seed.is_empty().then_some(id);
                    self.core.read(cx).library(LibraryCmd::CreatePlaylist {
                        id,
                        name,
                        tracks: seed.clone(),
                        pending: Vec::new(),
                    });
                    DialogEvent::Done { notice: Some(notice), open, deleted: None }
                }
            }
            DialogRequest::Rename { id, .. } => {
                self.core.read(cx).library(LibraryCmd::RenamePlaylist { id: *id, name: typed });
                DialogEvent::Done { notice: None, open: None, deleted: None }
            }
            DialogRequest::Delete { id, name } => {
                self.core.read(cx).library(LibraryCmd::DeletePlaylist(*id));
                DialogEvent::Done {
                    notice: Some(Notice::info(format!("Playlist «{name}» eliminata"))),
                    open: None,
                    deleted: Some(*id),
                }
            }
            DialogRequest::Unfriend { handle, .. } => {
                self.core.read(cx).social(unfriend(handle));
                DialogEvent::Done { notice: None, open: None, deleted: None }
            }
        };
        cx.emit(event);
    }

    /// Esc and Enter when the dialog has no field (the confirmations).
    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.cancel(cx),
            "enter" => self.confirm(None, cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Focusable for PlaylistDialog {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PlaylistDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cancel = Button::new("dialog-cancel", "Annulla")
            .secondary()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)));
        let ok = |label: &'static str| {
            Button::new("dialog-ok", label)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.confirm(None, cx)))
        };
        let dialog = match &self.request {
            DialogRequest::NewPlaylist { seed } => {
                Dialog::new("Nuova playlist", new_playlist_hint(seed.len()))
                    .button(cancel)
                    .button(ok("Crea").primary())
            }
            DialogRequest::Rename { .. } => {
                Dialog::new("Rinomina la playlist", "Il nuovo nome della playlist.")
                    .button(cancel)
                    .button(ok("Salva").primary())
            }
            DialogRequest::Delete { name, .. } => Dialog::new(
                "Eliminare la playlist?",
                SharedString::from(format!("«{name}» sparirà dalla libreria. I brani restano nel catalogo.")),
            )
            .button(cancel)
            .button(ok("Elimina").destructive()),
            DialogRequest::Unfriend { handle, name } => Dialog::new(
                unfriend_title(name),
                SharedString::from(format!(
                    "Non vedrete più cosa ascolta l'altro. Potrai mandare di nuovo una richiesta a @{handle}."
                )),
            )
            .button(cancel)
            .button(ok("Togli").destructive()),
        };
        let dialog = match &self.field {
            Some(field) => dialog.content(field.clone()),
            None => dialog,
        };
        div()
            .id("playlist-dialog")
            .relative()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            // A click on the dialog does not reach the scrim behind it (which closes).
            .occlude()
            .child(dialog)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_playlist_is_offered_the_next_number() {
        assert_eq!(suggested_name(0), "La mia playlist n. 1");
        assert_eq!(suggested_name(4), "La mia playlist n. 5");
    }

    #[test]
    fn the_hint_says_what_the_new_playlist_starts_with() {
        assert!(new_playlist_hint(0).starts_with("Dai un nome"));
        assert_eq!(new_playlist_hint(1), "La playlist parte con il brano che hai scelto.");
        assert_eq!(new_playlist_hint(3), "La playlist parte con questi 3 brani.");
    }

    #[test]
    fn unfriending_asks_first_and_then_removes_the_friend() {
        assert_eq!(unfriend_title("Luca Bianchi"), "Togliere Luca Bianchi dagli amici?");
        assert!(matches!(
            unfriend("luca"),
            SocialCmd::FriendAction { handle, add: false } if handle == "luca"
        ));
    }
}
