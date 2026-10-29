//! Il catalogo ufficiale, come in Spotify: album e playlist si aprono, si ascoltano o si salvano
//! nella libreria; la pagina di un artista mostra i suoi brani popolari e tutta la discografia.
//! L'audio non viene dal catalogo: quando si preme play il server cerca il brano giusto e lo importa.
//!
//! Salvare una playlist crea subito la playlist locale con i brani «in arrivo» (`Playlist::pending`,
//! salvati nella configurazione): il server li importa uno alla volta, e chi chiude l'app ritrova
//! l'importazione dove l'aveva lasciata. Ascoltarla senza salvarla importa subito il primo brano,
//! senza aspettare le importazioni in sottofondo, lo suona e poi accoda gli altri.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use iced::Task;
use uuid::Uuid;

use super::{App, Load, Message, Notice, Screen, into_load, seed};
use crate::api::{
    DiscoverImport, MetaArtist, MetaArtistPage, SourceItem, SourceKind, SourcePlaylist, SourcePlaylistPage,
    Track,
};
use crate::config::Playlist;
use crate::error::AppError;

/// Dopo un «troppe richieste» l'importazione riprende da sola dopo questa pausa.
const RATE_LIMIT_PAUSE: Duration = Duration::from_secs(20);
/// Copertine e foto scaricate subito per una ricerca: le prime sono quelle che si vedono.
const SEARCH_COVERS: usize = 60;

/// Una playlist esterna aperta: la sua scheda dalla ricerca e i brani, quando arrivano.
#[derive(Debug, Clone)]
pub struct OpenSourcePlaylist {
    pub playlist: SourcePlaylist,
    pub items: Load<Arc<[SourceItem]>>,
}

/// L'artista dal catalogo ufficiale: i brani più ascoltati e tutta la discografia.
#[derive(Debug, Clone)]
pub struct ArtistExtras {
    pub artist: MetaArtist,
    pub top: Arc<[SourceItem]>,
    pub albums: Arc<[SourcePlaylist]>,
}

/// Dove va un brano appena importato.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportTarget {
    /// In fondo a una playlist locale (e fuori dai suoi brani «in arrivo»).
    Playlist(Uuid),
    /// In coda, dopo il brano che sta partendo.
    Queue,
    /// Il primo brano di «Riproduci», con il suo numero: una risposta per un «Riproduci»
    /// precedente si scarta.
    Start(u64),
}

/// Le importazioni in corso: una richiesta alla volta, il server ne regge poche.
#[derive(Debug, Default)]
pub struct Imports {
    /// Brani da ascoltare di una playlist non salvata.
    queue: VecDeque<SourceItem>,
    /// Il prossimo brano della coda sostituisce quella attuale (il primo di «Riproduci»).
    start_queue: bool,
    in_flight: Option<(ImportTarget, SourceKind, String)>,
    /// Il brano premuto: si importa a parte, in parallelo a quello in sottofondo. Il resto della
    /// coda aspetta che parta.
    starting: Option<(u64, SourceItem)>,
    /// Numero dell'ultimo «Riproduci».
    plays: u64,
    /// In pausa dopo un «troppe richieste».
    paused: bool,
    /// Brani non importati per ogni playlist, per l'avviso finale.
    failed: HashMap<Uuid, usize>,
}

/// Stato delle fonti esterne fuori dalla ricerca.
#[derive(Debug, Default)]
pub struct ExternalState {
    pub open: Option<OpenSourcePlaylist>,
    /// Per artista: `Loading` finché non arrivano entrambe le ricerche.
    pub artists: HashMap<Uuid, Load<ArtistExtras>>,
    pub imports: Imports,
}

impl ExternalState {
    /// Il brano si sta importando adesso (per «Apro…» sulla sua riga).
    pub fn importing(&self, source: SourceKind, id: &str) -> bool {
        matches!(&self.imports.in_flight, Some((_, s, i)) if *s == source && i == id)
            || matches!(&self.imports.starting, Some((_, item)) if item.source == source && item.id == id)
    }

    /// Brani di «Riproduci» ancora da importare.
    pub fn queued(&self) -> usize {
        self.imports.queue.len()
    }
}

impl App {
    /// Apre una playlist esterna e ne carica i brani.
    pub(super) fn open_source_playlist(&mut self, playlist: SourcePlaylist) -> Task<Message> {
        let (source, id) = (playlist.source, playlist.id.clone());
        let same = self.external.open.as_ref().is_some_and(|open| {
            open.playlist.source == source && open.playlist.id == id && !open.items.needs_load()
        });
        if !same {
            self.external.open = Some(OpenSourcePlaylist { playlist, items: Load::Loading });
        }
        let navigate = self.open(Screen::SourcePlaylist);
        if same {
            return navigate;
        }
        let api = self.api();
        let load = Task::perform(async move { api?.source_playlist(source, &id).await }, move |result| {
            Message::SourcePlaylistLoaded(source, result)
        });
        Task::batch([navigate, load])
    }

    pub(super) fn source_playlist_loaded(
        &mut self,
        source: SourceKind,
        result: Result<SourcePlaylistPage, AppError>,
    ) -> Task<Message> {
        let Some(open) = self.external.open.as_mut() else { return Task::none() };
        if open.playlist.source != source {
            return Task::none();
        }
        match result {
            Ok(page) if page.id == open.playlist.id => {
                let covers: Vec<String> = page.items.iter().filter_map(|i| i.cover_url.clone()).collect();
                open.items = Load::Ready(Arc::from(page.items));
                return self.fetch_covers(covers);
            }
            Ok(_) => {}
            Err(err) => open.items = Load::Failed(err),
        }
        Task::none()
    }

    /// Salva la playlist aperta nella libreria: la playlist locale nasce subito, i brani arrivano
    /// uno alla volta.
    pub(super) fn save_source_playlist(&mut self) -> Task<Message> {
        let Some(open) = &self.external.open else { return Task::none() };
        let Load::Ready(items) = &open.items else { return Task::none() };
        if items.is_empty() || self.playlists.len() >= crate::config::MAX_PLAYLISTS {
            return Task::none();
        }
        let mut playlist = Playlist::new(&open.playlist.title, Vec::new());
        playlist.pending = items.to_vec();
        let id = playlist.id;
        let notice = Notice::Info(format!(
            "«{}» è nella libreria: i {} brani arrivano uno alla volta.",
            playlist.name,
            items.len()
        ));
        self.playlists.push(playlist);
        let shown = self.show_notice(Some(notice));
        let navigate = self.open(Screen::Playlist(id));
        Task::batch([self.persist(), shown, navigate, self.next_import()])
    }

    /// Ascolta la playlist aperta senza salvarla: il primo brano parte appena importato.
    pub(super) fn play_source_playlist(&mut self, start: usize) -> Task<Message> {
        let Some(open) = &self.external.open else { return Task::none() };
        let Load::Ready(items) = &open.items else { return Task::none() };
        let queue = items.iter().skip(start).cloned().collect();
        self.play_imported(queue)
    }

    /// Ascolta i brani popolari dell'artista da `start` in poi.
    pub(super) fn play_artist_top(&mut self, artist: Uuid, start: usize) -> Task<Message> {
        let Some(Load::Ready(extras)) = self.external.artists.get(&artist) else { return Task::none() };
        let queue = extras.top.iter().skip(start).cloned().collect();
        self.play_imported(queue)
    }

    /// Importa subito il primo brano, anche se un'altra importazione è in corso: chi ha premuto
    /// play non aspetta la fila. Gli altri entrano in coda dopo.
    fn play_imported(&mut self, mut queue: VecDeque<SourceItem>) -> Task<Message> {
        let Some(first) = queue.pop_front() else { return Task::none() };
        let imports = &mut self.external.imports;
        imports.queue = queue;
        imports.start_queue = false;
        imports.plays += 1;
        let play = imports.plays;
        imports.starting = Some((play, first.clone()));
        let api = self.api();
        let video = cfg!(feature = "video") && self.panel_video;
        Task::perform(
            async move { api?.import_source(first.source, &first.id, video).await },
            move |result| Message::SourceImported(ImportTarget::Start(play), result),
        )
    }

    /// Avvia la prossima importazione, se non ce n'è già una: prima i brani da ascoltare, poi le
    /// playlist salvate.
    pub(super) fn next_import(&mut self) -> Task<Message> {
        let imports = &self.external.imports;
        if imports.in_flight.is_some() || imports.starting.is_some() || imports.paused {
            return Task::none();
        }
        let next = match imports.queue.front() {
            Some(item) => Some((ImportTarget::Queue, item.clone())),
            None => self
                .playlists
                .iter()
                .find_map(|p| p.pending.first().map(|item| (ImportTarget::Playlist(p.id), item.clone()))),
        };
        let Some((target, item)) = next else { return Task::none() };
        self.external.imports.in_flight = Some((target, item.source, item.id.clone()));
        let api = self.api();
        let (source, id) = (item.source, item.id);
        // I brani da ascoltare arrivano con il videoclip, se il pannello lo mostra.
        let video = cfg!(feature = "video") && self.panel_video && target == ImportTarget::Queue;
        Task::perform(async move { api?.import_source(source, &id, video).await }, move |result| {
            Message::SourceImported(target, result)
        })
    }

    /// Un brano importato (o no): va dove doveva, poi si passa al successivo.
    pub(super) fn source_imported(
        &mut self,
        target: ImportTarget,
        result: Result<DiscoverImport, AppError>,
    ) -> Task<Message> {
        if let ImportTarget::Start(play) = target {
            return self.started(play, result);
        }
        let Some((_, source, id)) = self.external.imports.in_flight.take() else { return Task::none() };
        if let Err(AppError::Api { code, .. }) = &result
            && code == "rate_limited"
        {
            // Il brano resta il primo da importare: si riprova dopo la pausa.
            self.external.imports.paused = true;
            return Task::perform(tokio::time::sleep(RATE_LIMIT_PAUSE), |()| Message::ImportResume);
        }
        let track = result.ok().and_then(|imported| imported.tracks.into_iter().next());
        let done = match target {
            ImportTarget::Queue => self.imported_to_queue(source, &id, track),
            ImportTarget::Playlist(playlist) => self.imported_to_playlist(playlist, source, &id, track),
            ImportTarget::Start(_) => Task::none(),
        };
        Task::batch([done, self.next_import()])
    }

    /// Il brano premuto è arrivato: parte, e la fila riprende con il resto della coda.
    fn started(&mut self, play: u64, result: Result<DiscoverImport, AppError>) -> Task<Message> {
        let imports = &mut self.external.imports;
        // Un «Riproduci» più recente ha preso il suo posto.
        if !imports.starting.as_ref().is_some_and(|(current, _)| *current == play) {
            return Task::none();
        }
        let Some((_, item)) = imports.starting.take() else { return Task::none() };
        if let Err(AppError::Api { code, .. }) = &result
            && code == "rate_limited"
        {
            // Torna il primo della coda: parte quando la fila riprende, dopo la pausa.
            imports.queue.push_front(item);
            imports.start_queue = true;
            imports.paused = true;
            return Task::perform(tokio::time::sleep(RATE_LIMIT_PAUSE), |()| Message::ImportResume);
        }
        let Some(track) = result.ok().and_then(|imported| imported.tracks.into_iter().next()) else {
            // Non disponibile: parte il prossimo che arriva.
            imports.start_queue = true;
            return self.next_import();
        };
        let covers = self.fetch_covers(track.cover_url.clone().into_iter().collect());
        self.queue.replace(vec![track], 0, seed());
        self.catalogue_changed();
        Task::batch([self.start_current(), self.persist(), covers, self.next_import()])
    }

    pub(super) fn import_resume(&mut self) -> Task<Message> {
        self.external.imports.paused = false;
        self.next_import()
    }

    fn imported_to_queue(&mut self, source: SourceKind, id: &str, track: Option<Track>) -> Task<Message> {
        let imports = &mut self.external.imports;
        // «Riproduci» premuto di nuovo nel frattempo: la coda è un'altra.
        if !imports.queue.front().is_some_and(|item| item.source == source && item.id == id) {
            return Task::none();
        }
        imports.queue.pop_front();
        let Some(track) = track else { return Task::none() };
        let covers = self.fetch_covers(track.cover_url.clone().into_iter().collect());
        if std::mem::take(&mut self.external.imports.start_queue) {
            self.queue.replace(vec![track], 0, seed());
            self.catalogue_changed();
            return Task::batch([self.start_current(), self.persist(), covers]);
        }
        self.queue.enqueue(track);
        Task::batch([self.persist(), covers])
    }

    fn imported_to_playlist(
        &mut self,
        playlist_id: Uuid,
        source: SourceKind,
        id: &str,
        track: Option<Track>,
    ) -> Task<Message> {
        let Some(playlist) = self.playlists.iter_mut().find(|p| p.id == playlist_id) else {
            return Task::none();
        };
        let Some(at) = playlist.pending.iter().position(|item| item.source == source && item.id == id) else {
            return Task::none();
        };
        playlist.pending.remove(at);
        let url = track.as_ref().and_then(|t| t.cover_url.clone());
        match track {
            Some(track) => {
                playlist.add(track);
            }
            None => *self.external.imports.failed.entry(playlist_id).or_default() += 1,
        }
        let finished = playlist.pending.is_empty().then(|| (playlist.name.clone(), playlist.tracks.len()));
        let covers = self.fetch_covers(url.into_iter().collect());
        let Some((name, count)) = finished else {
            return Task::batch([self.persist(), covers]);
        };
        let failed = self.external.imports.failed.remove(&playlist_id).unwrap_or(0);
        let notice = if failed == 0 {
            Notice::Info(format!("«{name}» è pronta: {count} brani."))
        } else {
            Notice::Info(format!("«{name}» è pronta: {count} brani, {failed} non disponibili."))
        };
        self.catalogue_changed();
        let shown = self.show_notice(Some(notice));
        Task::batch([self.persist(), covers, shown])
    }

    /// Il catalogo è cambiato: Home e Libreria si ricaricano alla prossima visita.
    fn catalogue_changed(&mut self) {
        self.home = super::HomeState::default();
        self.library.tracks = Load::Idle;
        self.library.albums = Load::Idle;
    }

    /// Carica dal catalogo ufficiale i brani popolari e la discografia dell'artista, una volta per
    /// sessione.
    pub(super) fn load_artist_extras(&mut self, id: Uuid, name: String) -> Task<Message> {
        if self.external.artists.get(&id).is_some_and(|load| !load.needs_load()) {
            return Task::none();
        }
        self.external.artists.insert(id, Load::Loading);
        let api = self.api();
        Task::perform(async move { api?.meta_artist_named(&name).await }, move |result| {
            Message::ArtistExtrasLoaded(id, result)
        })
    }

    pub(super) fn artist_extras_loaded(
        &mut self,
        id: Uuid,
        result: Result<MetaArtistPage, AppError>,
    ) -> Task<Message> {
        let extras = result.map(|page| ArtistExtras {
            artist: page.artist,
            top: Arc::from(page.top),
            albums: Arc::from(page.albums),
        });
        let covers = match &extras {
            Ok(extras) => {
                let mut urls: Vec<String> = extras.artist.image_url.clone().into_iter().collect();
                urls.extend(extras.albums.iter().filter_map(|a| a.cover_url.clone()));
                urls.extend(extras.top.iter().filter_map(|t| t.cover_url.clone()));
                urls.truncate(SEARCH_COVERS * 2);
                urls
            }
            Err(_) => Vec::new(),
        };
        // Un artista seguito senza foto prende quella del catalogo ufficiale.
        let photo = extras.as_ref().ok().and_then(|extras| extras.artist.image_url.clone());
        let mut changed = false;
        if let Some(photo) = photo {
            for followed in self.followed.iter_mut().filter(|f| f.id == id && f.cover_url.is_none()) {
                followed.cover_url = Some(photo.clone());
                changed = true;
            }
        }
        self.external.artists.insert(id, into_load(extras));
        let save = if changed { self.persist() } else { Task::none() };
        Task::batch([self.fetch_covers(covers), save])
    }

    /// Le informazioni sull'artista del brano che suona (scheda del pannello) e le foto degli
    /// artisti seguiti che non ne hanno una: una richiesta per artista per sessione.
    pub(super) fn load_artist_photos(&mut self) -> Task<Message> {
        let mut wanted: Vec<(Uuid, String)> =
            self.queue.current().map(|t| (t.artist.id, t.artist.name.clone())).into_iter().collect();
        wanted.extend(self.followed.iter().filter(|f| f.cover_url.is_none()).map(|f| (f.id, f.name.clone())));
        Task::batch(
            wanted.into_iter().map(|(id, name)| self.load_artist_extras(id, name)).collect::<Vec<_>>(),
        )
    }

    /// Porta l'artista del catalogo nel server e ne apre la pagina.
    pub(super) fn open_meta_artist(&mut self, id: u64) -> Task<Message> {
        let api = self.api();
        Task::perform(async move { api?.open_meta_artist(id).await }, Message::MetaArtistOpened)
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
