//! Le cose che l'ascoltatore tiene per sé: "mi piace", playlist locali e il menu contestuale.
//! Il backend non ha utenti per questo, quindi tutto vive nel file di configurazione, e dal passo
//! 4.2 lo possiede il core (`core/src/state/library.rs`): qui i messaggi diventano `LibraryCmd` e
//! l'app legge le copie della sua vista (`App::playlists`, `App::liked`, …), che
//! [`App::library_published`] rinnova a ogni pubblicazione.

use std::sync::Arc;
use std::time::Instant;

use iced::Task;
use psytify_core::Command;
use psytify_core::command::{LibraryCmd, PlaybackCmd};
use psytify_core::snapshot::LibraryView;
use uuid::Uuid;

use super::{App, ContextMenu, Dialog, MenuTarget, Message, Notice, OpenDialog, Screen};
use crate::api::Track;
use crate::config::{self, MAX_PLAYLIST_NAME, MAX_PLAYLISTS};
use crate::ui::cursor;

impl App {
    /// Manda un comando al core. Il core lo applica al suo turno e pubblica la vista nuova: chi lo
    /// manda non la trova ancora in `App::liked` & co., e ciò che deve fare con il risultato (aprire
    /// la playlist appena creata, importare il brano successivo) lo fa in [`App::library_published`].
    /// Il `Task` è vuoto, tranne nei test senza core vero: lì il core è sincrono
    /// (`psytify_core::testing::LocalCore`) e la reazione alla pubblicazione torna qui.
    pub(super) fn send_core(&mut self, command: Command) -> Task<Message> {
        if let Some(core) = &self.core {
            core.send(command);
            return Task::none();
        }
        #[cfg(test)]
        if let Some(local) = &mut self.local_core
            && let Some(snapshot) = local.send(command)
        {
            return self.core_published(snapshot);
        }
        Task::none()
    }

    pub(super) fn send_library(&mut self, command: LibraryCmd) -> Task<Message> {
        self.send_core(Command::library(command))
    }

    /// Come [`App::send_library`], per i punti che non hanno un `Task` da restituire: con il core
    /// vero non ce n'è mai uno, e nei test la reazione sincrona che qui si perderebbe (aprire una
    /// playlist, importare) non riguarda questi comandi.
    pub(super) fn send_library_quiet(&mut self, command: LibraryCmd) {
        drop(self.send_library(command));
    }

    /// Copia nell'app la vista della libreria dell'ultima istantanea: gli `Arc` della vista, non
    /// gli elementi, e il tempo ascoltato (che dal passo 4.3 conta la
    /// riproduzione del core e consegna a gruppi).
    pub(super) fn copy_library(&mut self) {
        let view = Arc::clone(&self.core_snapshot.library);
        self.playlists = view.playlists.clone();
        self.liked = view.liked.clone();
        self.recent = view.recent.clone();
        self.followed = view.followed.clone();
        self.saved_albums = view.saved_albums.clone();
        self.stats = view.stats.clone();
        self.search_history = view.search_history.clone();
        self.listened_ms = view.listened_ms;
        if self.news_seen != view.news_seen {
            self.news_seen.clone_from(&view.news_seen);
        }
    }

    /// La libreria appena pubblicata dal core, al posto di `previous`: le copie dell'app si
    /// rinnovano e si fa ciò che aspettava la vista nuova.
    pub(super) fn library_published(&mut self, previous: &LibraryView) -> Task<Message> {
        let view = Arc::clone(&self.core_snapshot.library);
        self.copy_library();
        let mut tasks = Vec::new();
        // Il feed cambia solo se il core ne ha pubblicato uno nuovo: un feed azzerato da un catalogo
        // che cambia (`CatalogCmd::Changed`) non risorge per una pubblicazione qualunque.
        if !Arc::ptr_eq(&previous.feed, &view.feed) {
            self.home.feed = Arc::clone(&view.feed);
            if self.home.hero.index >= self.home.feed.hero.len() {
                self.home.hero.index = 0;
            }
        }
        // La playlist appena creata si apre quando c'è nella vista.
        if let Some(id) = self.pending_playlist
            && view.playlists.iter().any(|playlist| playlist.id == id)
        {
            self.pending_playlist = None;
            tasks.push(self.open(Screen::Playlist(id)));
        }
        Task::batch(tasks)
    }
}

impl App {
    /// Il brano piace.
    pub fn is_liked(&self, track_id: Uuid) -> bool {
        self.liked.iter().any(|t| t.id == track_id)
    }

    pub(super) fn toggle_like(&mut self, track: Track) -> Task<Message> {
        let title = track.title.clone();
        let id = track.id;
        // L'esito si conosce prima del core: è l'opposto di com'è adesso.
        let now_liked = !self.is_liked(id);
        self.like_pop = Some((id, Instant::now()));
        self.notice = Some(Notice::Info(if now_liked {
            format!("«{title}» è tra i brani che ti piacciono")
        } else {
            format!("«{title}» tolto dai brani che ti piacciono")
        }));
        self.send_library(LibraryCmd::ToggleLike(track))
    }

    pub(super) fn enqueue_next(&mut self, track: Track) -> Task<Message> {
        self.notice = Some(Notice::Info(format!("«{}» suonerà subito dopo", track.title)));
        self.send_playback(PlaybackCmd::EnqueueNext(Box::new(track)))
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
            Dialog::Rename { id, name } => self.send_library(LibraryCmd::RenamePlaylist { id, name }),
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
        let id = Uuid::new_v4();
        let title = config::clean_name(name);
        self.notice = Some(Notice::Info(format!("Playlist «{title}» creata")));
        // Si apre quando il core la pubblica: finché non c'è nella vista, la pagina non la trova.
        self.pending_playlist = empty.then_some(id);
        self.send_library(LibraryCmd::CreatePlaylist { id, name: title, tracks: seed, pending: Vec::new() })
    }

    pub(super) fn add_to_playlist(&mut self, id: Uuid, track: Track) -> Task<Message> {
        let title = track.title.clone();
        let Some(playlist) = self.playlist(id) else {
            return Task::none();
        };
        let name = playlist.name.clone();
        self.notice = Some(Notice::Info(if playlist.contains(track.id) {
            format!("«{title}» è già in «{name}»")
        } else {
            format!("«{title}» aggiunto a «{name}»")
        }));
        self.send_library(LibraryCmd::AddToPlaylist { id, track })
    }

    pub(super) fn remove_from_playlist(&mut self, id: Uuid, index: usize) -> Task<Message> {
        self.send_library(LibraryCmd::RemoveFromPlaylist { id, index })
    }

    pub(super) fn delete_playlist(&mut self, id: Uuid) -> Task<Message> {
        let name = self.playlist(id).map(|p| p.name.clone());
        if let Some(name) = name {
            self.notice = Some(Notice::Info(format!("Playlist «{name}» eliminata")));
        }
        let delete = self.send_library(LibraryCmd::DeletePlaylist(id));
        let leave =
            if self.screen == Screen::Playlist(id) { self.open(Screen::Library) } else { Task::none() };
        Task::batch([delete, leave])
    }
}

impl App {
    /// I messaggi di questo dominio, smistati da `App::handle`.
    pub(super) fn handle_library(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ForgetSearch(query) => self.send_library(LibraryCmd::ForgetSearch(query)),
            Message::ToggleLike(track) => self.toggle_like(track),
            Message::ToggleFollow(artist) => self.toggle_follow(artist),
            Message::ToggleSaveAlbum(album) => self.toggle_save_album(album),
            Message::DialogSubmit => self.dialog_submit(),
            Message::AddToPlaylist(id, track) => self.add_to_playlist(id, track),
            Message::RemoveFromPlaylist(id, index) => self.remove_from_playlist(id, index),
            // `handle` manda qui solo i messaggi di questo dominio.
            _ => Task::none(),
        }
    }
}
