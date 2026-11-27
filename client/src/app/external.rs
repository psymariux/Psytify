//! Il catalogo ufficiale, come in Spotify: album e playlist si aprono, si ascoltano o si salvano
//! nella libreria; la pagina di un artista mostra i suoi brani popolari e tutta la discografia.
//! L'audio non viene dal catalogo: quando si preme play il server cerca il brano giusto e lo importa.
//!
//! Dal passo 4.5 lo possiede il core (`core/src/state/external.rs`): la playlist aperta, le schede
//! degli artisti, le importazioni uno alla volta (con la pausa dopo un «troppe richieste») e il play
//! di un risultato della ricerca. Qui i messaggi diventano `ExternalCmd`, e l'app legge la copia della
//! sua vista (`App::external`), che [`App::external_published`] rinnova a ogni pubblicazione:
//! le copertine nuove si chiedono e gli eventi diventano gli avvisi di sempre.

use std::sync::Arc;

use iced::Task;
use psytify_core::Command;
use psytify_core::command::ExternalCmd;
use psytify_core::snapshot::{ExternalEvent, ExternalView};
use uuid::Uuid;

use super::{App, Load, Message, Notice, Screen};

/// Copertine e foto scaricate subito per le schede degli artisti: le prime sono quelle che si vedono.
const EXTRAS_COVERS: usize = 120;
/// Copertine e foto scaricate subito per una ricerca: le prime sono quelle che si vedono.
const SEARCH_COVERS: usize = 60;

/// Lo stesso dato: i due `Arc` sono la stessa allocazione (o lo stato non ne ha, ed è uguale).
fn same_load<T>(a: Option<&Load<T>>, b: Option<&Load<T>>, same: impl Fn(&T, &T) -> bool) -> bool {
    match (a, b) {
        (Some(Load::Ready(a)), Some(Load::Ready(b))) => same(a, b),
        (Some(Load::Idle), Some(Load::Idle)) | (Some(Load::Loading), Some(Load::Loading)) => true,
        (Some(Load::Failed(a)), Some(Load::Failed(b))) => a == b,
        (None, None) => true,
        _ => false,
    }
}

impl App {
    /// Un comando alle fonti esterne del core. Il core lo applica al suo turno e pubblica la vista
    /// nuova: chi lo manda non la trova ancora in `App::external`, e ciò che deve fare con il
    /// risultato (le copertine, un avviso) lo fa in [`App::external_published`].
    pub(super) fn send_external(&mut self, command: ExternalCmd) -> Task<Message> {
        self.send_core(Command::External(command))
    }

    /// Copia nell'app la vista delle fonti esterne dell'ultima istantanea, senza chiedere niente: lo
    /// stato iniziale. Gli eventi già nella vista non si rimostrano.
    pub(super) fn copy_external(&mut self) {
        let view = Arc::clone(&self.core_snapshot.external);
        self.external_event = view.events.back().map_or(0, |raised| raised.seq);
        self.external = view;
    }

    /// Se i brani si chiedono con il videoclip: lo vuole il pannello, se questa build lo mostra.
    fn wants_clips(&self) -> bool {
        cfg!(feature = "video") && self.panel_video
    }

    /// Le fonti esterne appena pubblicate dal core, al posto di `previous`: la copia dell'app si
    /// rinnova, le copertine nuove si chiedono e gli eventi diventano avvisi, in ordine.
    pub(super) fn external_published(&mut self, previous: &ExternalView) -> Task<Message> {
        let view = Arc::clone(&self.core_snapshot.external);
        self.external = Arc::clone(&view);
        let mut urls: Vec<String> = Vec::new();

        // La playlist aperta: le copertine dei suoi brani, quando arrivano.
        let items = |view: &ExternalView| view.open.as_ref().map(|open| open.items.clone());
        let (before, now) = (items(previous), items(&view));
        if !same_load(before.as_ref(), now.as_ref(), Arc::ptr_eq)
            && let Some(Load::Ready(items)) = &now
        {
            urls.extend(items.iter().filter_map(|i| i.cover_url.clone()));
        }

        // Le schede degli artisti nuove o aggiornate: foto, copertine della discografia e dei brani.
        if !Arc::ptr_eq(&previous.artists, &view.artists) {
            for (id, load) in view.artists.iter() {
                let unchanged = same_load(previous.artists.get(id), Some(load), |a, b| {
                    Arc::ptr_eq(&a.top, &b.top) && Arc::ptr_eq(&a.albums, &b.albums)
                });
                let Load::Ready(extras) = load else { continue };
                if unchanged {
                    continue;
                }
                urls.extend(extras.artist.image_url.clone());
                urls.extend(extras.albums.iter().filter_map(|a| a.cover_url.clone()));
                urls.extend(extras.top.iter().filter_map(|t| t.cover_url.clone()));
            }
            urls.truncate(EXTRAS_COVERS);
        }

        // Ciò che il core ha voluto far sapere, in ordine.
        let shown = self.external_event;
        for raised in view.events.iter().filter(|raised| raised.seq > shown) {
            self.external_event = raised.seq;
            if let ExternalEvent::Imported(tracks) = &raised.event {
                urls.extend(tracks.iter().filter_map(|t| t.cover_url.clone()));
            }
            if let Some(message) = raised.event.message() {
                self.notice = Some(if raised.event.is_error() {
                    Notice::Error(message)
                } else {
                    Notice::Info(message)
                });
            }
        }
        self.fetch_covers(urls)
    }

    /// Copertine e foto dei risultati della ricerca nel catalogo ufficiale.
    pub(super) fn search_covers(&self) -> Vec<String> {
        let search = &self.search;
        let artists = search.artists.iter().filter_map(|a| a.image_url.clone());
        let lists = match &search.playlists {
            Load::Ready(lists) => lists.iter().filter_map(|p| p.cover_url.clone()).collect(),
            _ => Vec::new(),
        };
        let tracks = match &search.sources {
            Load::Ready(tracks) => tracks.iter().filter_map(|t| t.cover_url.clone()).collect(),
            _ => Vec::new(),
        };
        let mut urls: Vec<String> = artists.chain(lists).chain(tracks).collect();
        urls.truncate(SEARCH_COVERS);
        urls
    }
}

impl App {
    /// I messaggi di questo dominio, smistati da `App::handle`.
    pub(super) fn handle_external(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenSourcePlaylist(playlist) => {
                let navigate = self.open(Screen::SourcePlaylist);
                let open = self.send_external(ExternalCmd::OpenPlaylist(playlist));
                Task::batch([navigate, open])
            }
            Message::SaveSourcePlaylist => {
                // La playlist si apre quando il core la pubblica (`App::library_published`); se il
                // core non la crea (vuota, libreria piena) l'identificatore non compare mai.
                let id = Uuid::new_v4();
                self.pending_playlist = Some(id);
                self.send_external(ExternalCmd::SavePlaylist { id })
            }
            Message::PlaySourcePlaylist(start) => {
                let video = self.wants_clips();
                self.send_external(ExternalCmd::PlayPlaylist { start, video })
            }
            Message::PlayArtistTop(artist, start) => {
                let video = self.wants_clips();
                self.send_external(ExternalCmd::PlayArtistTop { artist, start, video })
            }
            Message::PlayDiscovered(identifier) => {
                self.send_external(ExternalCmd::PlayArchive { identifier })
            }
            Message::PlaySource(source, id) => {
                // Il brano sta per suonare: se il pannello mostra i videoclip, il server scarica
                // anche quello.
                let video = self.wants_clips();
                self.send_external(ExternalCmd::PlaySource { source, id, video })
            }
            // `handle` manda qui solo i messaggi di questo dominio.
            _ => Task::none(),
        }
    }
}
