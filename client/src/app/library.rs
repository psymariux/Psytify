//! Le cose che l'ascoltatore tiene per sé: "mi piace", playlist locali e il menu contestuale.
//! Il backend non ha utenti per questo, quindi tutto vive nel file di configurazione.

use std::time::Instant;

use iced::Task;
use uuid::Uuid;

use super::{App, ContextMenu, Dialog, MenuTarget, Message, Notice, OpenDialog, Screen};
use crate::api::Track;
use crate::config::{self, MAX_PLAYLIST_NAME, MAX_PLAYLISTS, Playlist};
use crate::ui::cursor;

impl App {
    /// Il brano piace.
    pub fn is_liked(&self, track_id: Uuid) -> bool {
        self.liked.iter().any(|t| t.id == track_id)
    }

    pub(super) fn toggle_like(&mut self, track: Track) -> Task<Message> {
        let title = track.title.clone();
        let id = track.id;
        let now_liked = config::toggle_like(&mut self.liked, track);
        self.like_pop = Some((id, Instant::now()));
        self.notice = Some(Notice::Info(if now_liked {
            format!("«{title}» è tra i brani che ti piacciono")
        } else {
            format!("«{title}» tolto dai brani che ti piacciono")
        }));
        self.persist()
    }

    pub(super) fn enqueue_next(&mut self, track: Track) -> Task<Message> {
        let title = track.title.clone();
        self.queue.enqueue_next(track);
        self.notice = Some(Notice::Info(format!("«{title}» suonerà subito dopo")));
        self.persist()
    }

    /// Il nome proposto per una playlist nuova: «La mia playlist n. 3».
    pub fn suggested_playlist_name(&self) -> String {
        format!("La mia playlist n. {}", self.playlists.len() + 1)
    }

    pub(super) fn open_menu(&mut self, target: MenuTarget) -> Task<Message> {
        self.menu = Some(ContextMenu {
            target,
            at: cursor::position(),
            since: Instant::now(),
            playlists: false,
            downloads: false,
            timers: false,
        });
        Task::none()
    }

    /// Esc chiude ciò che sta sopra, dal più in alto: menu, finestra di dialogo, schermo intero.
    pub(super) fn escape(&mut self) -> Task<Message> {
        if self.menu.take().is_some()
            || self.drawer.take().is_some()
            || self.dialog.take().is_some()
            || self.shortcuts.take().is_some()
        {
            return Task::none();
        }
        if self.focus {
            self.focus = false;
            self.focus_video = false;
            return Task::none();
        }
        if self.fullscreen { self.toggle_fullscreen() } else { Task::none() }
    }

    pub(super) fn dialog_input(&mut self, text: String) {
        let text: String = text.chars().filter(|c| !c.is_control()).take(MAX_PLAYLIST_NAME).collect();
        if let Some(Dialog::NewPlaylist { name, .. } | Dialog::Rename { name, .. }) =
            self.dialog.as_mut().map(|open| &mut open.dialog)
        {
            *name = text;
        }
    }

    pub(super) fn dialog_submit(&mut self) -> Task<Message> {
        let Some(OpenDialog { dialog, .. }) = self.dialog.take() else {
            return Task::none();
        };
        match dialog {
            Dialog::NewPlaylist { name, seed } => self.create_playlist(&name, seed),
            Dialog::Rename { id, name } => {
                if let Some(playlist) = self.playlists.iter_mut().find(|p| p.id == id) {
                    playlist.name = config::clean_name(&name);
                }
                self.persist()
            }
            Dialog::ConfirmDelete(id) => self.delete_playlist(id),
        }
    }

    /// Crea una playlist. Se nasce vuota (dalla barra laterale) la apre; se nasce con dei brani
    /// (dal menu di un brano o dalla coda) resta dove si è e lo dice con un avviso.
    fn create_playlist(&mut self, name: &str, seed: Vec<Track>) -> Task<Message> {
        if self.playlists.len() >= MAX_PLAYLISTS {
            self.notice = Some(Notice::Error(format!("Al massimo {MAX_PLAYLISTS} playlist")));
            return Task::none();
        }
        let empty = seed.is_empty();
        let playlist = Playlist::new(name, seed);
        let (id, title) = (playlist.id, playlist.name.clone());
        self.playlists.push(playlist);
        self.notice = Some(Notice::Info(format!("Playlist «{title}» creata")));
        let open = if empty { self.open(Screen::Playlist(id)) } else { Task::none() };
        Task::batch([open, self.persist()])
    }

    pub(super) fn add_to_playlist(&mut self, id: Uuid, track: Track) -> Task<Message> {
        let title = track.title.clone();
        let Some(playlist) = self.playlists.iter_mut().find(|p| p.id == id) else {
            return Task::none();
        };
        let name = playlist.name.clone();
        self.notice = Some(Notice::Info(if playlist.add(track) {
            format!("«{title}» aggiunto a «{name}»")
        } else {
            format!("«{title}» è già in «{name}»")
        }));
        self.persist()
    }

    pub(super) fn remove_from_playlist(&mut self, id: Uuid, index: usize) -> Task<Message> {
        if let Some(playlist) = self.playlists.iter_mut().find(|p| p.id == id) {
            playlist.remove_at(index);
        }
        self.persist()
    }

    pub(super) fn delete_playlist(&mut self, id: Uuid) -> Task<Message> {
        let name = self.playlist(id).map(|p| p.name.clone());
        self.playlists.retain(|p| p.id != id);
        if let Some(name) = name {
            self.notice = Some(Notice::Info(format!("Playlist «{name}» eliminata")));
        }
        let leave =
            if self.screen == Screen::Playlist(id) { self.open(Screen::Library) } else { Task::none() };
        Task::batch([leave, self.persist()])
    }
}
