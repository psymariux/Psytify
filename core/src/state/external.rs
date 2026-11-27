//! Fonti esterne (passo 4.5): le playlist di YouTube e gli album di Bandcamp si aprono, si ascoltano
//! e si salvano; il catalogo ufficiale (Deezer) dà a ogni artista i brani popolari e la discografia,
//! e un brano trovato dalla ricerca si importa e si suona con un comando solo. L'audio non viene
//! dalla fonte: quando si preme play il server lo trova e lo porta nel catalogo, e il core suona
//! quello che il server gli restituisce.
//!
//! Salvare una playlist crea subito la playlist locale con i brani «in arrivo» (`Playlist::pending`,
//! salvati nella configurazione dalla libreria): il dominio li importa uno alla volta, e chi chiude
//! l'app ritrova l'importazione dove l'aveva lasciata (riparte dal primo avvio del core). Ascoltare
//! una playlist senza salvarla importa subito il primo brano, senza aspettare le importazioni in
//! sottofondo, lo suona e poi accoda gli altri. Dopo un «troppe richieste» la coda aspetta venti
//! secondi, con un timer del core, e riprende dallo stesso brano.
//!
//! Il passo successivo di una playlist salvata non aspetta che il frontend rilegga la vista: il
//! dominio legge la libreria nel passaggio che il core fa dopo ogni ingresso (`settle`, poi `pace`),
//! quando il brano importato è già al suo posto nella playlist. Come gli altri domini, non chiama mai
//! la libreria, la riproduzione o il catalogo: chiede (un brano importato, un `Play`, «il catalogo è
//! cambiato») con tre serbatoi che il passaggio svuota nello stesso giro, così il brano, la coda e la
//! vista sono un'unica istantanea.
//!
//! Cambiare vista solo se qualcosa cambia davvero: un comando uguale al precedente, o una risposta
//! superata (un'apertura, una scelta o un'importazione più recenti), non pubblica niente.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::api::{
    ApiClient, ArtistPage, DiscoverImport, MetaArtist, MetaArtistPage, SourceItem, SourceKind,
    SourcePlaylist, SourcePlaylistPage, Track,
};
use crate::catalog::Load;
use crate::config::{self, MAX_PLAYLISTS};
use crate::effect::{self, Effect, TimerKey};
use crate::error::AppError;
use crate::state::catalog::{CatalogCmd, CatalogView};
use crate::state::library::{LibraryCmd, LibraryView};
use crate::state::playback::{PlaybackCmd, PlaybackView};

/// Dopo un «troppe richieste» l'importazione riprende da sola dopo questa pausa.
pub const RATE_LIMIT_PAUSE: Duration = Duration::from_secs(20);
/// Schede «Altro di …» tenute in memoria: oltre, esce la meno usata.
pub const ARTIST_EXTRAS: usize = 64;
/// Eventi tenuti nella vista: un frontend che legge a ogni pubblicazione non ne perde.
const EVENTS: usize = 32;
/// Artisti seguiti già chiesti in questa sessione: se la lista cresce oltre, si ricomincia.
const ASKED: usize = 512;
/// Imports remembered with the server track they became: the oldest go first.
pub const LINKS: usize = 128;

/// I comandi del dominio.
#[derive(Debug, Clone)]
pub enum ExternalCmd {
    /// Apre una playlist o un album esterno e ne carica i brani. Se è già aperto e pronto non fa
    /// niente.
    OpenPlaylist(SourcePlaylist),
    /// «Riprova»: ricarica i brani della playlist aperta.
    RetryPlaylist,
    /// Salva la playlist aperta nella libreria come playlist `id` (lo sceglie chi chiede, che la apre
    /// quando compare nella vista): i brani arrivano uno alla volta. Con la libreria piena, o se la
    /// playlist non è pronta, non fa niente.
    SavePlaylist { id: Uuid },
    /// Ascolta la playlist aperta da `start`: il primo brano parte appena importato. `video` chiede
    /// anche il videoclip, per chi lo mostra.
    PlayPlaylist { start: usize, video: bool },
    /// Ascolta i brani popolari dell'artista, da `start` in poi.
    PlayArtistTop { artist: Uuid, start: usize, video: bool },
    /// Importa un risultato della ricerca (del catalogo ufficiale o di una fonte esterna) e lo suona.
    /// Una richiesta alla volta: finché una è in corso le altre di questo tipo si ignorano.
    PlaySource { source: SourceKind, id: String, video: bool },
    /// Importa un risultato di Internet Archive e lo suona. Come [`ExternalCmd::PlaySource`].
    PlayArchive { identifier: String },
}

/// Una playlist esterna aperta: la sua scheda dalla ricerca e i brani, quando arrivano.
#[derive(Debug, Clone)]
pub struct OpenPlaylist {
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

/// Che cosa si sta importando: un item di Archive (per identificatore) o un brano di una fonte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Importing {
    Archive(String),
    Source(SourceKind, String),
}

/// Le importazioni in corso, per «Apro…» sulle righe e per la riga «in arrivo».
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportsView {
    /// Brani di «Riproduci» ancora da importare.
    pub queued: usize,
    /// Il brano premuto nella ricerca: se c'è, le altre scelte di questo tipo aspettano.
    pub play: Option<Importing>,
    /// Il primo brano di «Riproduci»: si importa a parte, subito.
    pub start: Option<Importing>,
    /// L'importazione in sottofondo, una alla volta: la coda di «Riproduci» o una playlist salvata.
    pub step: Option<Importing>,
    /// In pausa dopo un «troppe richieste».
    pub paused: bool,
}

/// The server track each external item became in this session, newest last, at most [`LINKS`]: a
/// row of the official catalogue knows from it that the current track is its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportLinks(VecDeque<(Importing, Track)>);

impl ImportLinks {
    /// The server track that `key` became, if it was imported in this session.
    pub fn track(&self, key: &Importing) -> Option<&Track> {
        self.0.iter().rev().find(|(known, _)| known == key).map(|(_, track)| track)
    }

    fn put(&mut self, key: Importing, track: &Track) {
        self.0.retain(|(known, _)| *known != key);
        self.0.push_back((key, track.clone()));
        while self.0.len() > LINKS {
            self.0.pop_front();
        }
    }
}

/// Qualcosa che il frontend deve far sapere o fare.
#[derive(Debug, Clone, PartialEq)]
pub enum ExternalEvent {
    /// Dei brani sono entrati nel catalogo, in coda o in una playlist: le loro copertine si possono
    /// scaricare.
    Imported(Arc<[Track]>),
    /// Una playlist è nella libreria: i suoi `count` brani arrivano uno alla volta.
    PlaylistSaved { name: String, count: usize },
    /// Una playlist salvata ha finito di importare: `count` brani, `failed` non disponibili.
    PlaylistReady { name: String, count: usize, failed: usize },
    /// Il brano premuto non si è potuto importare.
    PlayFailed(AppError),
    /// Il server ha portato l'item nel catalogo, ma senza brani da ascoltare.
    NothingToPlay,
}

impl ExternalEvent {
    /// L'avviso da mostrare, se l'evento ne ha uno (`Imported` no: porta solo dati).
    pub fn message(&self) -> Option<String> {
        match self {
            Self::Imported(_) => None,
            Self::PlaylistSaved { name, count } => Some(saved_message(name, *count)),
            Self::PlaylistReady { name, count, failed } => Some(ready_message(name, *count, *failed)),
            Self::PlayFailed(err) => Some(err.to_string()),
            Self::NothingToPlay => Some("Questo contenuto non ha brani da ascoltare.".to_owned()),
        }
    }

    /// L'avviso è un errore, non un'informazione.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::PlayFailed(_) | Self::NothingToPlay)
    }
}

/// «1 brano», «2 brani».
fn tracks_label(count: usize) -> String {
    if count == 1 { "1 brano".to_owned() } else { format!("{count} brani") }
}

/// L'avviso di una playlist salvata: il numero di brani concorda («il brano arriva», «i 3 brani
/// arrivano»).
pub fn saved_message(name: &str, count: usize) -> String {
    if count == 1 {
        format!("«{name}» è nella libreria: il brano arriva a breve.")
    } else {
        format!("«{name}» è nella libreria: i {count} brani arrivano uno alla volta.")
    }
}

/// L'avviso di fine importazione, con i numeri che concordano: «1 brano», «1 non disponibile».
pub fn ready_message(name: &str, count: usize, failed: usize) -> String {
    let tracks = tracks_label(count);
    match failed {
        0 => format!("«{name}» è pronta: {tracks}."),
        1 => format!("«{name}» è pronta: {tracks}, 1 non disponibile."),
        n => format!("«{name}» è pronta: {tracks}, {n} non disponibili."),
    }
}

/// Un [`ExternalEvent`] con il suo numero d'ordine, che cresce a ogni evento.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalRaised {
    pub seq: u64,
    pub event: ExternalEvent,
}

/// Ciò che i frontend vedono del dominio.
#[derive(Debug, Clone, Default)]
pub struct ExternalView {
    /// La playlist esterna aperta e il suo caricamento.
    pub open: Option<OpenPlaylist>,
    /// Per artista: `Loading` finché non arrivano entrambe le ricerche. Le più vecchie escono oltre
    /// [`ARTIST_EXTRAS`].
    pub artists: Arc<HashMap<Uuid, Load<ArtistExtras>>>,
    pub imports: ImportsView,
    /// What the imports of this session became on the server.
    pub links: Arc<ImportLinks>,
    /// Gli ultimi eventi, dal più vecchio: un frontend mostra quelli con `seq` nuovo, in ordine.
    pub events: Arc<VecDeque<ExternalRaised>>,
}

impl ExternalView {
    /// Il brano si sta importando adesso (per «Apro…» sulla sua riga).
    pub fn importing(&self, source: SourceKind, id: &str) -> bool {
        let is = |slot: &Option<Importing>| matches!(slot, Some(Importing::Source(s, i)) if *s == source && i == id);
        is(&self.imports.play) || is(&self.imports.start) || is(&self.imports.step)
    }

    /// L'item di Archive si sta importando adesso.
    pub fn importing_archive(&self, identifier: &str) -> bool {
        matches!(&self.imports.play, Some(Importing::Archive(i)) if i == identifier)
    }

    /// Brani di «Riproduci» ancora da importare.
    pub fn queued(&self) -> usize {
        self.imports.queued
    }

    /// La scheda «Altro di …» dell'artista, se è stata chiesta.
    pub fn artist(&self, id: Uuid) -> Option<&Load<ArtistExtras>> {
        self.artists.get(&id)
    }
}

/// I timer del dominio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {
    /// Scade [`RATE_LIMIT_PAUSE`] dopo un «troppe richieste»: l'importazione riprende.
    Resume,
}

/// Dove va un brano importato. Una risposta per una scelta superata si scarta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportTarget {
    /// In fondo a una playlist locale (e fuori dai suoi brani «in arrivo»).
    Playlist(Uuid),
    /// In coda, dopo il brano che sta partendo.
    Queue,
    /// Il primo brano di «Riproduci», con il numero della scelta.
    Start(u64),
    /// Il brano premuto nella ricerca, con il numero della scelta.
    Click(u64),
}

/// I lavori asincroni del dominio (rete), che l'host esegue sul runtime.
#[derive(Debug)]
pub(crate) enum Job {
    /// I brani di una playlist esterna; `ticket` è il numero dell'apertura.
    Playlist { api: Result<ApiClient, AppError>, ticket: u64, source: SourceKind, id: String },
    /// Porta un brano (o un item di Archive) nel catalogo.
    Import { api: Result<ApiClient, AppError>, target: ImportTarget, key: Importing, video: bool },
    /// I brani popolari e la discografia di un artista dal catalogo ufficiale.
    Artist { api: Result<ApiClient, AppError>, id: Uuid, name: String },
}

/// L'esito di un [`Job`], che torna al dominio.
#[derive(Debug)]
pub(crate) enum Done {
    Playlist { ticket: u64, source: SourceKind, result: Result<Box<SourcePlaylistPage>, AppError> },
    Imported { target: ImportTarget, result: Result<Box<DiscoverImport>, AppError> },
    Artist { id: Uuid, result: Result<Box<MetaArtistPage>, AppError> },
}

/// L'esito di un lavoro andato perso.
fn lost<T>() -> Result<T, AppError> {
    Err(AppError::Network)
}

/// Il primo brano che il server ha importato.
fn first_track(result: Result<Box<DiscoverImport>, AppError>) -> Option<Track> {
    result.ok().and_then(|imported| imported.tracks.into_iter().next())
}

/// L'importazione si è fermata per le troppe richieste.
fn rate_limited(result: &Result<Box<DiscoverImport>, AppError>) -> bool {
    matches!(result, Err(AppError::Api { code, .. }) if code == "rate_limited")
}

/// Un brano di una playlist salvata, importato (o no): aspetta il passaggio, che legge la libreria
/// per sapere se la playlist ha finito.
#[derive(Debug)]
struct Landed {
    playlist: Uuid,
    source: SourceKind,
    id: String,
    track: Option<Track>,
}

/// Una playlist da salvare: il passaggio la crea se la libreria ha posto.
#[derive(Debug)]
struct Save {
    id: Uuid,
    name: String,
    items: Arc<[SourceItem]>,
}

/// Le importazioni: una richiesta alla volta (il server ne regge poche).
#[derive(Debug, Default)]
struct Imports {
    /// Brani da ascoltare di una playlist non salvata.
    queue: VecDeque<SourceItem>,
    /// Il prossimo brano della coda sostituisce quella attuale (il primo di «Riproduci» che non si è
    /// potuto importare).
    start_queue: bool,
    /// Le importazioni della coda portano il videoclip.
    video: bool,
    /// L'importazione in sottofondo.
    in_flight: Option<(ImportTarget, SourceKind, String)>,
    /// Il primo brano di «Riproduci», con il numero della scelta.
    starting: Option<(u64, SourceItem)>,
    /// Il brano premuto nella ricerca, con il numero della scelta.
    clicking: Option<(u64, Importing)>,
    /// Il numero dell'ultima scelta («Riproduci» o un brano premuto): l'ultima vince.
    picks: u64,
    /// In pausa dopo un «troppe richieste».
    paused: bool,
    /// Brani non importati per ogni playlist, per l'avviso finale.
    failed: HashMap<Uuid, usize>,
}

/// Lo stato del dominio: la vista pubblicata e ciò che resta dentro il core.
#[derive(Debug)]
pub(crate) struct External {
    pub(crate) view: Arc<ExternalView>,
    api: Result<ApiClient, AppError>,
    server: Option<String>,
    /// Il numero dell'ultima apertura di una playlist: le risposte di una precedente si scartano.
    ticket: u64,
    /// L'ordine d'uso delle schede degli artisti, la meno usata in testa.
    recency: VecDeque<Uuid>,
    /// Gli artisti seguiti di cui si è già cercata la foto in questa sessione.
    asked: HashSet<Uuid>,
    imports: Imports,
    landed: Vec<Landed>,
    saves: Vec<Save>,
    /// Il numero dell'ultimo evento.
    events: u64,
    /// La pagina dell'artista aperta che ha già avviato la sua scheda.
    seen_artist: Option<(Uuid, Arc<ArtistPage>)>,
    /// L'ultimo avvio di brano visto: ne parte uno nuovo, e si cerca la foto del suo artista.
    seen_started: u64,
    /// Il core è appena partito: le foto degli artisti seguiti si cercano alla prima occasione.
    startup: bool,
    /// Ciò che il dominio chiede alle altre, in attesa del passaggio.
    library: Vec<LibraryCmd>,
    playback: Vec<PlaybackCmd>,
    catalog: Vec<CatalogCmd>,
}

impl Default for External {
    fn default() -> Self {
        Self::new(None)
    }
}

impl External {
    /// Il dominio con il backend scelto nelle impostazioni.
    pub(crate) fn new(server: Option<String>) -> Self {
        Self {
            view: Arc::default(),
            api: ApiClient::new(&config::api_url(server.as_deref())),
            server,
            ticket: 0,
            recency: VecDeque::new(),
            asked: HashSet::new(),
            imports: Imports::default(),
            landed: Vec::new(),
            saves: Vec::new(),
            events: 0,
            seen_artist: None,
            seen_started: 0,
            startup: false,
            library: Vec::new(),
            playback: Vec::new(),
            catalog: Vec::new(),
        }
    }

    fn view_mut(&mut self) -> &mut ExternalView {
        Arc::make_mut(&mut self.view)
    }

    /// Remembers that `key` became `track` on the server.
    fn link(&mut self, key: Importing, track: &Track) {
        Arc::make_mut(&mut self.view_mut().links).put(key, track);
    }

    fn api(&self) -> Result<ApiClient, AppError> {
        self.api.clone()
    }

    fn run(effects: &mut Vec<Effect>, job: Job) {
        effects.push(Effect::Run(effect::Job::External(job)));
    }

    // ---------- Passaggio fra i domini ----------

    /// Ciò che il dominio chiede alla libreria, e lo svuota.
    pub(crate) fn take_library(&mut self) -> Vec<LibraryCmd> {
        std::mem::take(&mut self.library)
    }

    /// Ciò che il dominio chiede alla riproduzione, e lo svuota.
    pub(crate) fn take_playback(&mut self) -> Vec<PlaybackCmd> {
        std::mem::take(&mut self.playback)
    }

    /// Ciò che il dominio chiede al catalogo, e lo svuota.
    pub(crate) fn take_catalog(&mut self) -> Vec<CatalogCmd> {
        std::mem::take(&mut self.catalog)
    }

    /// Il core è appena partito: le foto degli artisti si cercano alla prima occasione.
    pub(crate) fn start(&mut self) {
        self.startup = true;
    }

    /// Il backend scelto nelle impostazioni è cambiato: il client HTTP si rifà e le schede degli
    /// artisti, che sono di un altro server, si buttano.
    pub(crate) fn set_server(&mut self, server: &Option<String>) {
        if self.server == *server {
            return;
        }
        self.server.clone_from(server);
        self.api = ApiClient::new(&config::api_url(server.as_deref()));
        self.recency.clear();
        self.asked.clear();
        self.seen_artist = None;
        if !self.view.artists.is_empty() {
            self.view_mut().artists = Arc::default();
        }
    }

    /// Il passaggio dopo ogni ingresso, prima che le altre vedano ciò che il dominio chiede: salva le
    /// playlist chieste, sistema i brani arrivati in una playlist (che hanno bisogno della libreria
    /// per dire se ha finito) e avvia le schede degli artisti che servono.
    pub(crate) fn settle(
        &mut self,
        library: &LibraryView,
        catalog: &CatalogView,
        playback: &PlaybackView,
        effects: &mut Vec<Effect>,
    ) {
        self.create_playlists(library);
        self.land(library);
        // La pagina dell'artista aperta: la sua scheda «Altro di …» si chiede una volta, e di nuovo se
        // la pagina torna (una pagina nuova, o la stessa dopo essere stata altrove).
        match &catalog.artist {
            Some((id, Load::Ready(page))) => {
                let known =
                    matches!(&self.seen_artist, Some((seen, last)) if seen == id && Arc::ptr_eq(last, page));
                if !known {
                    self.seen_artist = Some((*id, Arc::clone(page)));
                    self.ensure_artist(*id, page.name.clone(), effects);
                }
            }
            _ => self.seen_artist = None,
        }
        // Un altro brano suona, o il core è appena partito: l'artista in corso e gli artisti seguiti
        // senza foto prendono la loro dal catalogo ufficiale.
        let started = playback.started != self.seen_started;
        self.seen_started = playback.started;
        if started || std::mem::take(&mut self.startup) {
            self.photos(library, playback, effects);
        }
    }

    /// Crea le playlist salvate dalla playlist aperta, se la libreria ha ancora posto.
    fn create_playlists(&mut self, library: &LibraryView) {
        let mut created = 0;
        for Save { id, name, items } in std::mem::take(&mut self.saves) {
            if library.playlists.len() + created >= MAX_PLAYLISTS {
                continue;
            }
            created += 1;
            let count = items.len();
            self.library.push(LibraryCmd::CreatePlaylist {
                id,
                name: name.clone(),
                tracks: Vec::new(),
                pending: items.to_vec(),
            });
            self.raise(ExternalEvent::PlaylistSaved { name, count });
        }
    }

    /// I brani arrivati per una playlist salvata vanno alla libreria; se era l'ultimo, la playlist è
    /// pronta e il catalogo è cambiato.
    fn land(&mut self, library: &LibraryView) {
        for Landed { playlist, source, id, track } in std::mem::take(&mut self.landed) {
            let waiting = |item: &SourceItem| item.source == source && item.id == id;
            // La playlist può essere sparita, o il brano non essere più da importare: niente da fare.
            let found = library.playlists.iter().find(|p| p.id == playlist);
            let Some(found) = found.filter(|p| p.pending.iter().any(waiting)) else {
                self.imports.failed.remove(&playlist);
                continue;
            };
            // Com'è la playlist dopo il comando: serve per l'avviso di fine importazione.
            let added = track.as_ref().is_some_and(|t| !found.contains(t.id));
            let finished = found.pending.iter().all(waiting);
            let summary = finished.then(|| (found.name.clone(), found.tracks.len() + usize::from(added)));
            if track.is_none() {
                *self.imports.failed.entry(playlist).or_default() += 1;
            }
            if let Some(track) = &track {
                self.link(Importing::Source(source, id.clone()), track);
                self.raise(ExternalEvent::Imported(Arc::from(vec![track.clone()])));
            }
            self.library.push(LibraryCmd::ImportedToPlaylist { id: playlist, source, item: id, track });
            if let Some((name, count)) = summary {
                let failed = self.imports.failed.remove(&playlist).unwrap_or(0);
                self.catalog.push(CatalogCmd::Changed);
                self.raise(ExternalEvent::PlaylistReady { name, count, failed });
            }
        }
    }

    /// L'artista del brano in corso e quelli seguiti senza foto: la scheda dal catalogo ufficiale
    /// dice anche la foto, che va all'artista seguito. Un artista seguito si chiede una volta per
    /// sessione.
    fn photos(&mut self, library: &LibraryView, playback: &PlaybackView, effects: &mut Vec<Effect>) {
        if let Some(track) = playback.queue.current() {
            self.ensure_artist(track.artist.id, track.artist.name.clone(), effects);
        }
        let wanted: Vec<(Uuid, String)> = library
            .followed
            .iter()
            .filter(|f| f.cover_url.is_none() && !self.asked.contains(&f.id))
            .map(|f| (f.id, f.name.clone()))
            .collect();
        if self.asked.len() + wanted.len() > ASKED {
            self.asked.clear();
        }
        for (id, name) in wanted {
            self.asked.insert(id);
            self.ensure_artist(id, name, effects);
        }
    }

    /// Avvia, se serve, la prossima importazione: prima i brani da ascoltare, poi le playlist
    /// salvate. Una alla volta, e mai in pausa. Il passaggio la chiama dopo aver dato alla libreria
    /// ciò che il dominio aveva chiesto, così non rilegge un brano appena importato.
    pub(crate) fn pace(&mut self, library: &LibraryView, effects: &mut Vec<Effect>) {
        let imports = &self.imports;
        if imports.in_flight.is_some() || imports.starting.is_some() || imports.paused {
            return;
        }
        let next = match imports.queue.front() {
            Some(item) => Some((ImportTarget::Queue, item.clone())),
            None => library
                .playlists
                .iter()
                .find_map(|p| p.pending.first().map(|item| (ImportTarget::Playlist(p.id), item.clone()))),
        };
        let Some((target, item)) = next else { return };
        self.imports.in_flight = Some((target, item.source, item.id.clone()));
        // I brani da ascoltare arrivano con il videoclip, se chi ascolta lo mostra.
        let video = self.imports.video && target == ImportTarget::Queue;
        let key = Importing::Source(item.source, item.id);
        Self::run(effects, Job::Import { api: self.api(), target, key, video });
        self.publish_imports();
    }

    // ---------- Comandi ----------

    pub(crate) fn apply(&mut self, command: ExternalCmd, _now: Instant, effects: &mut Vec<Effect>) {
        match command {
            ExternalCmd::OpenPlaylist(playlist) => self.open_playlist(playlist, false, effects),
            ExternalCmd::RetryPlaylist => {
                if let Some(open) = &self.view.open {
                    let playlist = open.playlist.clone();
                    self.open_playlist(playlist, true, effects);
                }
            }
            ExternalCmd::SavePlaylist { id } => {
                let Some(open) = &self.view.open else { return };
                let Load::Ready(items) = &open.items else { return };
                if items.is_empty() {
                    return;
                }
                let name = config::clean_name(&open.playlist.title);
                self.saves.push(Save { id, name, items: Arc::clone(items) });
            }
            ExternalCmd::PlayPlaylist { start, video } => {
                let Some(Load::Ready(items)) = self.view.open.as_ref().map(|open| &open.items) else {
                    return;
                };
                let queue = items.iter().skip(start).cloned().collect();
                self.play_imported(queue, video, effects);
            }
            ExternalCmd::PlayArtistTop { artist, start, video } => {
                let Some(Load::Ready(extras)) = self.view.artists.get(&artist) else { return };
                let queue = extras.top.iter().skip(start).cloned().collect();
                self.play_imported(queue, video, effects);
            }
            ExternalCmd::PlaySource { source, id, video } => {
                self.play_one(Importing::Source(source, id), video, effects);
            }
            ExternalCmd::PlayArchive { identifier } => {
                self.play_one(Importing::Archive(identifier), false, effects);
            }
        }
    }

    pub(crate) fn fire(&mut self, timer: Timer, _now: Instant, _effects: &mut Vec<Effect>) {
        match timer {
            // Il passaggio che segue riavvia l'importazione dallo stesso brano.
            Timer::Resume => {
                self.imports.paused = false;
                self.publish_imports();
            }
        }
    }

    // ---------- Playlist aperta ----------

    fn open_playlist(&mut self, playlist: SourcePlaylist, force: bool, effects: &mut Vec<Effect>) {
        let same = !force
            && self.view.open.as_ref().is_some_and(|open| {
                open.playlist.source == playlist.source
                    && open.playlist.id == playlist.id
                    && !open.items.needs_load()
            });
        if same {
            return;
        }
        self.ticket += 1;
        let (source, id) = (playlist.source, playlist.id.clone());
        self.view_mut().open = Some(OpenPlaylist { playlist, items: Load::Loading });
        Self::run(effects, Job::Playlist { api: self.api(), ticket: self.ticket, source, id });
    }

    // ---------- Importazioni ----------

    /// Importa subito il primo brano, anche se un'altra importazione è in corso: chi ha premuto play
    /// non aspetta la fila. Gli altri entrano in coda dopo, uno alla volta.
    fn play_imported(&mut self, mut queue: VecDeque<SourceItem>, video: bool, effects: &mut Vec<Effect>) {
        let Some(first) = queue.pop_front() else { return };
        let imports = &mut self.imports;
        imports.queue = queue;
        imports.start_queue = false;
        imports.video = video;
        imports.picks += 1;
        let play = imports.picks;
        imports.starting = Some((play, first.clone()));
        let key = Importing::Source(first.source, first.id);
        Self::run(effects, Job::Import { api: self.api(), target: ImportTarget::Start(play), key, video });
        self.publish_imports();
    }

    /// Importa un brano premuto nella ricerca e lo suona: uno alla volta.
    fn play_one(&mut self, key: Importing, video: bool, effects: &mut Vec<Effect>) {
        if self.imports.clicking.is_some() {
            return;
        }
        self.imports.picks += 1;
        let pick = self.imports.picks;
        self.imports.clicking = Some((pick, key.clone()));
        Self::run(effects, Job::Import { api: self.api(), target: ImportTarget::Click(pick), key, video });
        self.publish_imports();
    }

    /// L'importazione si ferma per [`RATE_LIMIT_PAUSE`]: il brano resta il primo da importare.
    fn pause(&mut self, effects: &mut Vec<Effect>) {
        self.imports.paused = true;
        effects.push(Effect::Timer { key: TimerKey::External(Timer::Resume), after: RATE_LIMIT_PAUSE });
    }

    fn imported(
        &mut self,
        target: ImportTarget,
        result: Result<Box<DiscoverImport>, AppError>,
        effects: &mut Vec<Effect>,
    ) {
        match target {
            ImportTarget::Click(pick) => self.clicked(pick, result),
            ImportTarget::Start(play) => self.started(play, result, effects),
            ImportTarget::Queue | ImportTarget::Playlist(_) => {
                // Una risposta che non è quella in corso (la coda è stata cambiata) si ignora.
                if !matches!(&self.imports.in_flight, Some((current, ..)) if *current == target) {
                    return;
                }
                let Some((_, source, id)) = self.imports.in_flight.take() else { return };
                if rate_limited(&result) {
                    self.pause(effects);
                    return;
                }
                let track = first_track(result);
                match target {
                    ImportTarget::Playlist(playlist) => {
                        // La playlist cambia nella libreria, e il passaggio dice se ha finito.
                        self.landed.push(Landed { playlist, source, id, track });
                    }
                    _ => self.imported_to_queue(source, &id, track),
                }
            }
        }
    }

    /// Il brano premuto nella ricerca è arrivato: parte, con tutti i brani che l'item ha portato.
    fn clicked(&mut self, pick: u64, result: Result<Box<DiscoverImport>, AppError>) {
        let key = match &self.imports.clicking {
            Some((current, _)) if *current == pick => self.imports.clicking.take().map(|(_, key)| key),
            _ => None,
        };
        if let (Some(key), Ok(imported)) = (key, &result)
            && let Some(first) = imported.tracks.first()
        {
            self.link(key, first);
        }
        // Chi ascolta ha scelto altro nel frattempo: l'ultima scelta vince.
        if pick != self.imports.picks {
            return;
        }
        match result {
            Ok(imported) if !imported.tracks.is_empty() => {
                let tracks: Arc<[Track]> = Arc::from(imported.tracks);
                self.raise(ExternalEvent::Imported(Arc::clone(&tracks)));
                // Il catalogo è cambiato: Home e Libreria si ricaricano alla prossima visita.
                self.catalog.push(CatalogCmd::Changed);
                self.playback.push(PlaybackCmd::Play { tracks, start: 0 });
            }
            Ok(_) => self.raise(ExternalEvent::NothingToPlay),
            Err(err) => self.raise(ExternalEvent::PlayFailed(err)),
        }
    }

    /// Il primo brano di «Riproduci» è arrivato: parte, e la fila riprende con il resto della coda.
    fn started(
        &mut self,
        play: u64,
        result: Result<Box<DiscoverImport>, AppError>,
        effects: &mut Vec<Effect>,
    ) {
        // Una scelta più recente ha preso il suo posto.
        if !matches!(&self.imports.starting, Some((current, _)) if *current == play) {
            return;
        }
        let Some((_, item)) = self.imports.starting.take() else { return };
        if rate_limited(&result) {
            // Torna il primo della coda: parte quando la fila riprende, dopo la pausa.
            self.imports.queue.push_front(item);
            self.imports.start_queue = true;
            self.pause(effects);
            return;
        }
        let Some(track) = first_track(result) else {
            // Non disponibile: parte il prossimo che arriva.
            self.imports.start_queue = true;
            return;
        };
        self.link(Importing::Source(item.source, item.id), &track);
        self.raise(ExternalEvent::Imported(Arc::from(vec![track.clone()])));
        self.catalog.push(CatalogCmd::Changed);
        self.playback.push(PlaybackCmd::Play { tracks: Arc::from(vec![track]), start: 0 });
    }

    /// Un brano della coda di «Riproduci» è arrivato: in coda, o a sostituirla se il primo non c'era.
    fn imported_to_queue(&mut self, source: SourceKind, id: &str, track: Option<Track>) {
        let imports = &mut self.imports;
        // «Riproduci» premuto di nuovo nel frattempo: la coda è un'altra.
        if !imports.queue.front().is_some_and(|item| item.source == source && item.id == id) {
            return;
        }
        imports.queue.pop_front();
        let Some(track) = track else { return };
        self.link(Importing::Source(source, id.to_owned()), &track);
        let start = std::mem::take(&mut self.imports.start_queue);
        self.raise(ExternalEvent::Imported(Arc::from(vec![track.clone()])));
        if start {
            self.catalog.push(CatalogCmd::Changed);
            self.playback.push(PlaybackCmd::Play { tracks: Arc::from(vec![track]), start: 0 });
        } else {
            self.playback.push(PlaybackCmd::Enqueue(Box::new(track)));
        }
    }

    /// Rifà la parte della vista che riguarda le importazioni, se è cambiata.
    fn publish_imports(&mut self) {
        let imports = &self.imports;
        let next = ImportsView {
            queued: imports.queue.len(),
            play: imports.clicking.as_ref().map(|(_, key)| key.clone()),
            start: imports.starting.as_ref().map(|(_, item)| Importing::Source(item.source, item.id.clone())),
            step: imports.in_flight.as_ref().map(|(_, source, id)| Importing::Source(*source, id.clone())),
            paused: imports.paused,
        };
        if self.view.imports != next {
            self.view_mut().imports = next;
        }
    }

    // ---------- Schede degli artisti ----------

    /// Porta la scheda in fondo all'elenco d'uso; restituisce le schede che escono per fare posto.
    fn touch(&mut self, id: Uuid) -> Vec<Uuid> {
        self.recency.retain(|seen| *seen != id);
        self.recency.push_back(id);
        let mut evicted = Vec::new();
        while self.recency.len() > ARTIST_EXTRAS {
            evicted.extend(self.recency.pop_front());
        }
        evicted
    }

    /// Mette la scheda nella vista e fa uscire le più vecchie oltre il limite.
    fn put_artist(&mut self, id: Uuid, load: Load<ArtistExtras>) {
        let evicted = self.touch(id);
        let artists = Arc::make_mut(&mut self.view_mut().artists);
        artists.insert(id, load);
        for old in evicted {
            artists.remove(&old);
        }
    }

    /// La scheda «Altro di …» dell'artista, una volta per sessione: se è già pronta o in arrivo non fa
    /// niente (un errore si riprova alla prossima occasione).
    fn ensure_artist(&mut self, id: Uuid, name: String, effects: &mut Vec<Effect>) {
        if matches!(self.view.artists.get(&id), Some(load) if !load.needs_load()) {
            self.touch(id);
            return;
        }
        self.put_artist(id, Load::Loading);
        Self::run(effects, Job::Artist { api: self.api(), id, name });
    }

    fn artist_arrived(&mut self, id: Uuid, result: Result<Box<MetaArtistPage>, AppError>) {
        let extras = result.map(|page| ArtistExtras {
            artist: page.artist,
            top: Arc::from(page.top),
            albums: Arc::from(page.albums),
        });
        // Un artista seguito senza foto prende quella del catalogo ufficiale.
        if let Ok(extras) = &extras
            && let Some(photo) = &extras.artist.image_url
        {
            self.library.push(LibraryCmd::SetFollowedCover {
                id,
                cover_url: Some(photo.clone()),
                only_if_missing: true,
            });
        }
        self.put_artist(id, Load::of(extras));
    }

    fn raise(&mut self, event: ExternalEvent) {
        self.events += 1;
        let raised = ExternalRaised { seq: self.events, event };
        let events = Arc::make_mut(&mut self.view_mut().events);
        events.push_back(raised);
        while events.len() > EVENTS {
            events.pop_front();
        }
    }

    // ---------- Risposte ----------

    pub(crate) fn done(&mut self, done: Done, _now: Instant, effects: &mut Vec<Effect>) {
        match done {
            Done::Playlist { ticket, source, result } => {
                // Una risposta per un'apertura superata, o per un'altra fonte, si ignora.
                if ticket != self.ticket {
                    return;
                }
                let Some(open) = self.view.open.as_ref().filter(|open| open.playlist.source == source) else {
                    return;
                };
                let items = match result {
                    Ok(page) if page.id == open.playlist.id => Load::Ready(Arc::from(page.items)),
                    Ok(_) => return,
                    Err(err) => Load::Failed(err),
                };
                if let Some(open) = &mut self.view_mut().open {
                    open.items = items;
                }
            }
            Done::Imported { target, result } => {
                self.imported(target, result, effects);
                self.publish_imports();
            }
            Done::Artist { id, result } => self.artist_arrived(id, result),
        }
    }
}

impl Job {
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Playlist { api, ticket, source, id } => {
                let result = match api {
                    Ok(api) => api.source_playlist(source, &id).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Playlist { ticket, source, result }
            }
            Self::Import { api, target, key, video } => {
                let result = match (api, &key) {
                    (Ok(api), Importing::Archive(identifier)) => api.import_item(identifier).await,
                    (Ok(api), Importing::Source(source, id)) => api.import_source(*source, id, video).await,
                    (Err(err), _) => Err(err),
                };
                Done::Imported { target, result: result.map(Box::new) }
            }
            Self::Artist { api, id, name } => {
                let result = match api {
                    Ok(api) => api.meta_artist_named(&name).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Artist { id, result }
            }
        }
    }

    /// Se il lavoro va perso, l'esito è un errore di rete: il dominio non resta ad aspettare (una
    /// playlist non resta «in caricamento» per sempre, e la fila delle importazioni va avanti).
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Playlist { ticket, source, .. } => {
                Done::Playlist { ticket: *ticket, source: *source, result: lost() }
            }
            Self::Import { target, .. } => Done::Imported { target: *target, result: lost() },
            Self::Artist { id, .. } => Done::Artist { id: *id, result: lost() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures::track;
    use crate::command::Command;
    use crate::config::{Config, Playlist};
    use crate::snapshot::Snapshot;
    use crate::state::catalog::CatalogView;
    use crate::state::{CoreState, Input};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn item(id: &str) -> SourceItem {
        SourceItem {
            source: SourceKind::Deezer,
            id: id.into(),
            title: format!("Brano {id}"),
            artist: "Artista".into(),
            duration_ms: Some(180_000),
            cover_url: None,
        }
    }

    fn evening() -> SourcePlaylist {
        SourcePlaylist {
            source: SourceKind::Deezer,
            id: "playlist/1".into(),
            title: "Sera".into(),
            owner: "Psytify".into(),
            cover_url: None,
            kind: "playlist".into(),
            released_on: None,
        }
    }

    fn page(playlist: &SourcePlaylist, items: Vec<SourceItem>) -> Result<Box<SourcePlaylistPage>, AppError> {
        Ok(Box::new(SourcePlaylistPage {
            source: playlist.source,
            id: playlist.id.clone(),
            title: playlist.title.clone(),
            owner: playlist.owner.clone(),
            items,
        }))
    }

    fn imported(tracks: Vec<Track>) -> Result<Box<DiscoverImport>, AppError> {
        Ok(Box::new(DiscoverImport { album: None, tracks }))
    }

    fn one(n: u128) -> Result<Box<DiscoverImport>, AppError> {
        imported(vec![track(n)])
    }

    fn missing() -> Result<Box<DiscoverImport>, AppError> {
        Err(AppError::Api { status: 404, code: "not_found".into(), message: "Non trovato".into() })
    }

    fn limited() -> Result<Box<DiscoverImport>, AppError> {
        Err(AppError::Api { status: 429, code: "rate_limited".into(), message: "Troppe richieste".into() })
    }

    /// Lo stato di tutto il core con un orologio finto che si sposta a mano: il dominio gira con gli
    /// altri, come nel core vero, perché il passo successivo di una playlist dipende dalla libreria.
    struct Rig {
        state: CoreState,
        now: Instant,
    }

    impl Rig {
        fn new() -> Self {
            Self { state: CoreState::default(), now: Instant::now() }
        }

        fn with(config: Config) -> Self {
            Self { state: CoreState::new(config, None, 1), now: Instant::now() }
        }

        fn input(&mut self, input: Input) -> Vec<Effect> {
            self.state.apply(input, self.now)
        }

        fn cmd(&mut self, command: ExternalCmd) -> Vec<Effect> {
            self.input(Input::Command(Command::External(command)))
        }

        fn library(&mut self, command: LibraryCmd) -> Vec<Effect> {
            self.input(Input::Command(Command::library(command)))
        }

        fn done(&mut self, done: Done) -> Vec<Effect> {
            self.input(Input::Done(effect::Done::External(done)))
        }

        /// Un'importazione finita; `target` è quello del lavoro chiesto.
        fn imported(
            &mut self,
            target: ImportTarget,
            result: Result<Box<DiscoverImport>, AppError>,
        ) -> Vec<Effect> {
            self.done(Done::Imported { target, result })
        }

        fn fire(&mut self) -> Vec<Effect> {
            self.input(Input::Timer(TimerKey::External(Timer::Resume)))
        }

        fn wait(&mut self, by: Duration) {
            self.now += by;
        }

        fn view(&self) -> Arc<ExternalView> {
            Arc::clone(&self.state.external.view)
        }

        /// Gli eventi, dal più vecchio.
        fn events(&self) -> Vec<ExternalEvent> {
            self.view().events.iter().map(|raised| raised.event.clone()).collect()
        }

        /// I brani nella coda di ascolto, in ordine.
        fn queue(&self) -> Vec<Uuid> {
            self.state.playback.view.queue.tracks_in_order().into_iter().map(|t| t.id).collect()
        }

        /// Chi ascolta fa partire questi brani (la coda è la loro).
        fn play(&mut self, tracks: Vec<Track>) {
            self.input(Input::Command(Command::Playback(PlaybackCmd::Play {
                tracks: tracks.into(),
                start: 0,
            })));
        }

        fn playlist(&self, id: Uuid) -> Option<Playlist> {
            self.state.library.view.playlists.iter().find(|p| p.id == id).cloned()
        }

        /// Apre la playlist «Sera» con questi brani.
        fn open(&mut self, items: Vec<SourceItem>) {
            let playlist = evening();
            self.cmd(ExternalCmd::OpenPlaylist(playlist.clone()));
            let ticket = self.state.external.ticket;
            self.done(Done::Playlist { ticket, source: playlist.source, result: page(&playlist, items) });
        }

        /// Apre e salva la playlist «Sera»: restituisce la playlist e gli effetti del salvataggio.
        fn save(&mut self, items: Vec<SourceItem>) -> (Uuid, Vec<Effect>) {
            self.open(items);
            let playlist = id(100);
            let effects = self.cmd(ExternalCmd::SavePlaylist { id: playlist });
            (playlist, effects)
        }
    }

    fn jobs(effects: &[Effect]) -> Vec<&Job> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Run(effect::Job::External(job)) => Some(job),
                _ => None,
            })
            .collect()
    }

    /// Le importazioni chieste: dove vanno, che cosa, con il videoclip o no.
    fn imports(effects: &[Effect]) -> Vec<(ImportTarget, Importing, bool)> {
        jobs(effects)
            .into_iter()
            .filter_map(|job| match job {
                Job::Import { target, key, video, .. } => Some((*target, key.clone(), *video)),
                _ => None,
            })
            .collect()
    }

    fn deezer(id: &str) -> Importing {
        Importing::Source(SourceKind::Deezer, id.into())
    }

    fn timers(effects: &[Effect]) -> Vec<Duration> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Timer { key: TimerKey::External(Timer::Resume), after } => Some(*after),
                _ => None,
            })
            .collect()
    }

    // ---------- Importazioni di una playlist salvata ----------

    /// `imports_run_one_at_a_time`, e la correzione di BACKLOG 8: l'avviso finale concorda in numero.
    #[test]
    fn imports_run_one_at_a_time() {
        let mut rig = Rig::new();
        let (playlist, effects) = rig.save(vec![item("a"), item("b"), item("c")]);
        let target = ImportTarget::Playlist(playlist);
        assert_eq!(imports(&effects), [(target, deezer("a"), false)]);
        assert_eq!(
            rig.events().last().and_then(ExternalEvent::message),
            Some("«Sera» è nella libreria: i 3 brani arrivano uno alla volta.".into())
        );
        assert!(rig.view().importing(SourceKind::Deezer, "a"));
        assert_eq!(rig.playlist(playlist).map(|p| p.pending.len()), Some(3));

        // Un altro ingresso, qualunque, non avvia una seconda richiesta.
        assert!(imports(&rig.library(LibraryCmd::MarkNewsSeen)).is_empty(), "una richiesta alla volta");

        assert_eq!(imports(&rig.imported(target, one(1))), [(target, deezer("b"), false)]);
        assert!(
            rig.view().importing(SourceKind::Deezer, "b") && !rig.view().importing(SourceKind::Deezer, "a")
        );
        assert_eq!(
            imports(&rig.imported(target, missing())),
            [(target, deezer("c"), false)],
            "un brano non trovato non ferma gli altri"
        );
        assert!(imports(&rig.imported(target, one(3))).is_empty());
        assert_eq!(rig.view().imports, ImportsView::default());

        let saved = rig.playlist(playlist).expect("la playlist c'è ancora");
        assert_eq!(saved.tracks.iter().map(|t| t.id).collect::<Vec<_>>(), [id(1), id(3)]);
        assert!(saved.pending.is_empty());
        let ready = ExternalEvent::PlaylistReady { name: "Sera".into(), count: 2, failed: 1 };
        assert!(rig.events().contains(&ready), "{:?}", rig.events());
        assert_eq!(ready.message(), Some("«Sera» è pronta: 2 brani, 1 non disponibile.".into()));
    }

    /// Il passo 4.2 lo faceva aspettare la vista pubblicata; ora la playlist cambia nello stesso giro
    /// in cui arriva il brano, e il prossimo parte senza che nessun frontend rilegga niente: mai due
    /// volte lo stesso brano, mai uno saltato.
    #[test]
    fn the_next_import_follows_in_the_same_turn_and_never_repeats() {
        let mut rig = Rig::new();
        let (playlist, _) = rig.save(vec![item("a"), item("b")]);
        let target = ImportTarget::Playlist(playlist);
        let effects = rig.imported(target, one(1));
        assert_eq!(imports(&effects), [(target, deezer("b"), false)], "b, e solo b");
        let waiting = rig.playlist(playlist).expect("la playlist c'è");
        assert_eq!(waiting.pending.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!(waiting.tracks.iter().map(|t| t.id).collect::<Vec<_>>(), [id(1)]);
    }

    #[test]
    fn a_rate_limit_pauses_then_resumes() {
        let mut rig = Rig::new();
        let (playlist, _) = rig.save(vec![item("a"), item("b")]);
        let target = ImportTarget::Playlist(playlist);

        let effects = rig.imported(target, limited());
        assert!(imports(&effects).is_empty());
        assert_eq!(timers(&effects), [RATE_LIMIT_PAUSE], "venti secondi, con un timer del core");
        assert!(rig.view().imports.paused && rig.view().imports.step.is_none());
        assert!(imports(&rig.library(LibraryCmd::MarkNewsSeen)).is_empty(), "in pausa non parte nulla");
        assert_eq!(rig.playlist(playlist).map(|p| p.pending.len()), Some(2), "il brano resta da importare");

        // Dopo venti secondi si riprende, dallo stesso brano.
        rig.wait(RATE_LIMIT_PAUSE);
        let effects = rig.fire();
        assert_eq!(imports(&effects), [(target, deezer("a"), false)]);
        assert!(!rig.view().imports.paused);
    }

    /// La playlist è sparita mentre un brano si importava: il suo arrivo non fa nulla e la fila passa
    /// alle altre playlist.
    #[test]
    fn a_playlist_deleted_while_importing_is_skipped() {
        let mut rig = Rig::new();
        let (first, _) = rig.save(vec![item("a")]);
        rig.cmd(ExternalCmd::SavePlaylist { id: id(101) });
        assert!(imports(&rig.library(LibraryCmd::DeletePlaylist(first))).is_empty(), "a è ancora in corso");
        let effects = rig.imported(ImportTarget::Playlist(first), one(1));
        assert_eq!(imports(&effects), [(ImportTarget::Playlist(id(101)), deezer("a"), false)]);
        assert!(rig.playlist(first).is_none());
        assert!(
            !rig.events().iter().any(|e| matches!(e, ExternalEvent::PlaylistReady { .. })),
            "la playlist sparita non è «pronta»"
        );
    }

    /// All'avvio le playlist con brani ancora da importare riprendono da dove erano.
    #[test]
    fn the_first_turn_resumes_a_saved_playlist_where_it_stopped() {
        let mut playlist = Playlist::new("Sera", Vec::new());
        playlist.pending = vec![item("b"), item("c")];
        let mut config = Config::default();
        config.core.playlists = vec![playlist.clone()].into();
        let mut rig = Rig::with(config);
        let effects = rig.input(Input::Start);
        assert_eq!(imports(&effects), [(ImportTarget::Playlist(playlist.id), deezer("b"), false)]);
    }

    // ---------- Ascoltare una playlist ----------

    #[test]
    fn listening_imports_the_first_track_at_once_and_queues_the_rest_one_by_one() {
        let mut rig = Rig::new();
        rig.open(vec![item("a"), item("b"), item("c")]);
        let effects = rig.cmd(ExternalCmd::PlayPlaylist { start: 0, video: true });
        let start = ImportTarget::Start(1);
        assert_eq!(imports(&effects), [(start, deezer("a"), true)], "il primo parte da solo");
        assert_eq!(rig.view().imports.queued, 2);
        assert!(rig.view().importing(SourceKind::Deezer, "a"));
        let home = rig.state.catalog.home_epoch();

        // Il primo arriva: suona, il catalogo è cambiato, e la fila riprende con il videoclip.
        let effects = rig.imported(start, one(1));
        assert_eq!(rig.queue(), [id(1)]);
        assert_eq!(rig.state.catalog.home_epoch(), home + 1);
        assert_eq!(imports(&effects), [(ImportTarget::Queue, deezer("b"), true)]);
        // Gli altri si accodano, in ordine, uno alla volta.
        let effects = rig.imported(ImportTarget::Queue, one(2));
        assert_eq!(rig.queue(), [id(1), id(2)]);
        assert_eq!(imports(&effects), [(ImportTarget::Queue, deezer("c"), true)]);
        assert!(imports(&rig.imported(ImportTarget::Queue, one(3))).is_empty());
        assert_eq!(rig.queue(), [id(1), id(2), id(3)]);
        assert_eq!(rig.view().imports, ImportsView::default());
        let covers =
            rig.view().events.iter().filter(|r| matches!(r.event, ExternalEvent::Imported(_))).count();
        assert_eq!(covers, 3, "ogni brano importato dice che la sua copertina può servire");
    }

    #[test]
    fn listening_from_the_middle_skips_what_comes_before() {
        let mut rig = Rig::new();
        rig.open(vec![item("a"), item("b"), item("c")]);
        let effects = rig.cmd(ExternalCmd::PlayPlaylist { start: 1, video: false });
        assert_eq!(imports(&effects), [(ImportTarget::Start(1), deezer("b"), false)]);
        assert_eq!(rig.view().imports.queued, 1);
    }

    /// Se il primo non è disponibile parte il primo che arriva dopo di lui.
    #[test]
    fn an_unavailable_first_track_hands_the_start_to_the_next_one() {
        let mut rig = Rig::new();
        rig.open(vec![item("a"), item("b"), item("c")]);
        rig.cmd(ExternalCmd::PlayPlaylist { start: 0, video: false });
        let effects = rig.imported(ImportTarget::Start(1), missing());
        assert!(rig.queue().is_empty(), "niente suona ancora");
        assert_eq!(imports(&effects), [(ImportTarget::Queue, deezer("b"), false)]);
        let effects = rig.imported(ImportTarget::Queue, one(2));
        assert_eq!(rig.queue(), [id(2)], "b prende il posto del primo");
        assert_eq!(imports(&effects), [(ImportTarget::Queue, deezer("c"), false)]);
        rig.imported(ImportTarget::Queue, one(3));
        assert_eq!(rig.queue(), [id(2), id(3)], "c si accoda a b");
    }

    #[test]
    fn a_rate_limited_first_track_waits_and_still_starts_the_queue() {
        let mut rig = Rig::new();
        rig.open(vec![item("a"), item("b")]);
        rig.cmd(ExternalCmd::PlayPlaylist { start: 0, video: false });
        let effects = rig.imported(ImportTarget::Start(1), limited());
        assert!(imports(&effects).is_empty());
        assert_eq!(timers(&effects), [RATE_LIMIT_PAUSE]);
        assert_eq!(rig.view().imports.queued, 2, "il primo è tornato in testa");
        rig.wait(RATE_LIMIT_PAUSE);
        assert_eq!(imports(&rig.fire()), [(ImportTarget::Queue, deezer("a"), false)]);
        rig.imported(ImportTarget::Queue, one(1));
        assert_eq!(rig.queue(), [id(1)], "il primo, arrivato dopo la pausa, suona");
    }

    /// Un «Riproduci» più recente prende il posto del precedente: la risposta tardiva non suona.
    #[test]
    fn the_last_pick_wins_over_a_late_answer() {
        let mut rig = Rig::new();
        rig.open(vec![item("a"), item("b")]);
        rig.cmd(ExternalCmd::PlayPlaylist { start: 0, video: false });
        let effects = rig.cmd(ExternalCmd::PlayPlaylist { start: 1, video: false });
        assert_eq!(imports(&effects), [(ImportTarget::Start(2), deezer("b"), false)]);
        rig.imported(ImportTarget::Start(1), one(1));
        assert!(rig.queue().is_empty(), "la scelta superata non suona");
        rig.imported(ImportTarget::Start(2), one(2));
        assert_eq!(rig.queue(), [id(2)]);
    }

    #[test]
    fn the_top_tracks_of_an_artist_play_like_a_playlist() {
        let mut rig = Rig::new();
        assert!(
            imports(&rig.cmd(ExternalCmd::PlayArtistTop { artist: id(5), start: 0, video: false }))
                .is_empty()
        );
        let top = vec![item("t1"), item("t2")];
        rig.state.external.view_mut().artists = Arc::new(HashMap::from([(
            id(5),
            Load::Ready(ArtistExtras {
                artist: MetaArtist { id: 9, name: "Marea".into(), image_url: None, fans: 1 },
                top: Arc::from(top),
                albums: Arc::default(),
            }),
        )]));
        let effects = rig.cmd(ExternalCmd::PlayArtistTop { artist: id(5), start: 1, video: false });
        assert_eq!(imports(&effects), [(ImportTarget::Start(1), deezer("t2"), false)]);
    }

    // ---------- Suonare un risultato della ricerca ----------

    #[test]
    fn a_pressed_result_is_imported_and_played_with_all_its_tracks() {
        let mut rig = Rig::new();
        let effects =
            rig.cmd(ExternalCmd::PlaySource { source: SourceKind::Deezer, id: "7".into(), video: true });
        let click = ImportTarget::Click(1);
        assert_eq!(imports(&effects), [(click, deezer("7"), true)]);
        assert!(rig.view().importing(SourceKind::Deezer, "7"));
        // Finché uno è in corso, un altro play (o un item di Archive) aspetta.
        let effects =
            rig.cmd(ExternalCmd::PlaySource { source: SourceKind::Deezer, id: "8".into(), video: false });
        assert!(imports(&effects).is_empty());
        assert!(imports(&rig.cmd(ExternalCmd::PlayArchive { identifier: "item".into() })).is_empty());
        let home = rig.state.catalog.home_epoch();

        rig.imported(click, imported(vec![track(1), track(2)]));
        assert_eq!(rig.queue(), [id(1), id(2)], "suona tutto ciò che l'item ha portato");
        assert_eq!(rig.state.playback.view.queue.current().map(|t| t.id), Some(id(1)));
        assert_eq!(rig.state.catalog.home_epoch(), home + 1, "il catalogo è cambiato");
        assert!(matches!(rig.events().last(), Some(ExternalEvent::Imported(tracks)) if tracks.len() == 2));
        assert_eq!(rig.view().imports, ImportsView::default());
    }

    /// A row of the catalogue finds the server track it became, whichever way it was imported.
    #[test]
    fn every_import_remembers_the_server_track_it_became() {
        let mut rig = Rig::new();
        rig.cmd(ExternalCmd::PlaySource { source: SourceKind::Deezer, id: "7".into(), video: false });
        rig.imported(ImportTarget::Click(1), one(70));
        assert_eq!(rig.view().links.track(&deezer("7")).map(|t| t.id), Some(id(70)));

        rig.open(vec![item("a"), item("b")]);
        rig.cmd(ExternalCmd::PlayPlaylist { start: 0, video: false });
        rig.imported(ImportTarget::Start(2), one(1));
        rig.imported(ImportTarget::Queue, missing());
        assert_eq!(rig.view().links.track(&deezer("a")).map(|t| t.id), Some(id(1)));
        assert_eq!(rig.view().links.track(&deezer("b")), None, "not available: no link");

        rig.cmd(ExternalCmd::PlayArchive { identifier: "marea".into() });
        rig.imported(ImportTarget::Click(3), one(9));
        assert_eq!(rig.view().links.track(&Importing::Archive("marea".into())).map(|t| t.id), Some(id(9)));

        let mut links = ImportLinks::default();
        for n in 0..=LINKS {
            links.put(deezer(&n.to_string()), &track(n as u128));
        }
        assert_eq!(links.track(&deezer("0")), None, "the oldest goes past the limit");
        assert!(links.track(&deezer(&LINKS.to_string())).is_some());
    }

    #[test]
    fn an_archive_item_is_imported_by_identifier() {
        let mut rig = Rig::new();
        let effects = rig.cmd(ExternalCmd::PlayArchive { identifier: "marea-live".into() });
        assert_eq!(
            imports(&effects),
            [(ImportTarget::Click(1), Importing::Archive("marea-live".into()), false)]
        );
        assert!(rig.view().importing_archive("marea-live") && !rig.view().importing_archive("altro"));
        assert!(!rig.view().importing(SourceKind::Deezer, "marea-live"), "un item di Archive non è un brano");
    }

    #[test]
    fn a_failed_or_empty_import_is_an_event_and_frees_the_next_press() {
        let mut rig = Rig::new();
        rig.cmd(ExternalCmd::PlaySource { source: SourceKind::Deezer, id: "7".into(), video: false });
        rig.imported(ImportTarget::Click(1), missing());
        assert!(matches!(rig.events().last(), Some(ExternalEvent::PlayFailed(AppError::Api { .. }))));
        assert!(rig.queue().is_empty());
        assert_eq!(rig.view().imports, ImportsView::default());

        rig.cmd(ExternalCmd::PlayArchive { identifier: "vuoto".into() });
        rig.imported(ImportTarget::Click(2), imported(Vec::new()));
        let last = rig.events().last().cloned();
        assert_eq!(last, Some(ExternalEvent::NothingToPlay));
        assert_eq!(
            last.and_then(|event| event.message()),
            Some("Questo contenuto non ha brani da ascoltare.".into())
        );
        assert!(rig.queue().is_empty());
        assert!(
            !imports(&rig.cmd(ExternalCmd::PlaySource {
                source: SourceKind::Deezer,
                id: "9".into(),
                video: false
            }))
            .is_empty(),
            "dopo la risposta si può premere di nuovo"
        );
    }

    /// Chi preme altro mentre un brano si importa non se lo ritrova sopra: l'ultima scelta vince, e
    /// «Apro…» sparisce comunque.
    #[test]
    fn a_pressed_result_loses_to_a_later_pick() {
        let mut rig = Rig::new();
        rig.open(vec![item("a")]);
        rig.cmd(ExternalCmd::PlaySource { source: SourceKind::Deezer, id: "7".into(), video: false });
        rig.cmd(ExternalCmd::PlayPlaylist { start: 0, video: false });
        rig.imported(ImportTarget::Click(1), one(7));
        assert!(rig.queue().is_empty(), "la risposta superata non suona");
        assert!(rig.view().imports.play.is_none());
        rig.imported(ImportTarget::Start(2), one(1));
        assert_eq!(rig.queue(), [id(1)]);
    }

    // ---------- Playlist aperta ----------

    #[test]
    fn opening_loads_once_and_late_answers_are_dropped() {
        let mut rig = Rig::new();
        let first = evening();
        let effects = rig.cmd(ExternalCmd::OpenPlaylist(first.clone()));
        let asked: Vec<_> = jobs(&effects)
            .into_iter()
            .map(|job| match job {
                Job::Playlist { ticket, source, id, .. } => (*ticket, *source, id.clone()),
                other => panic!("not a playlist job: {other:?}"),
            })
            .collect();
        assert_eq!(asked, [(1, SourceKind::Deezer, "playlist/1".to_owned())]);
        assert!(matches!(rig.view().open.as_ref().map(|o| &o.items), Some(Load::Loading)));
        assert!(jobs(&rig.cmd(ExternalCmd::OpenPlaylist(first.clone()))).is_empty(), "già in arrivo");

        // Un'altra playlist prende il posto: la risposta della prima non conta.
        let second = SourcePlaylist { id: "album/2".into(), title: "Mattino".into(), ..evening() };
        rig.cmd(ExternalCmd::OpenPlaylist(second.clone()));
        rig.done(Done::Playlist {
            ticket: 1,
            source: SourceKind::Deezer,
            result: page(&first, vec![item("a")]),
        });
        assert!(matches!(rig.view().open.as_ref().map(|o| &o.items), Some(Load::Loading)));
        rig.done(Done::Playlist {
            ticket: 2,
            source: SourceKind::Deezer,
            result: page(&second, vec![item("x")]),
        });
        let open = rig.view().open.clone().expect("aperta");
        assert_eq!(open.playlist.title, "Mattino");
        assert_eq!(open.items.ready().map(|items| items.len()), Some(1));

        // Pronta: riaprirla non fa niente.
        assert!(jobs(&rig.cmd(ExternalCmd::OpenPlaylist(second))).is_empty());
    }

    #[test]
    fn a_failed_playlist_retries_and_a_page_for_another_playlist_is_ignored() {
        let mut rig = Rig::new();
        let playlist = evening();
        rig.cmd(ExternalCmd::OpenPlaylist(playlist.clone()));
        let other = SourcePlaylist { id: "playlist/2".into(), ..evening() };
        rig.done(Done::Playlist { ticket: 1, source: SourceKind::Deezer, result: page(&other, Vec::new()) });
        assert!(matches!(rig.view().open.as_ref().map(|o| &o.items), Some(Load::Loading)), "id diverso");

        rig.done(Done::Playlist { ticket: 1, source: SourceKind::Deezer, result: Err(AppError::Timeout) });
        assert!(matches!(rig.view().open.as_ref().map(|o| &o.items), Some(Load::Failed(AppError::Timeout))));
        // «Riprova»: di nuovo in caricamento, con un numero nuovo; la risposta vecchia non conta.
        let effects = rig.cmd(ExternalCmd::RetryPlaylist);
        assert!(matches!(jobs(&effects).as_slice(), [Job::Playlist { ticket: 2, .. }]));
        rig.done(Done::Playlist { ticket: 1, source: SourceKind::Deezer, result: Err(AppError::Network) });
        assert!(matches!(rig.view().open.as_ref().map(|o| &o.items), Some(Load::Loading)));
        rig.done(Done::Playlist {
            ticket: 2,
            source: SourceKind::Deezer,
            result: page(&playlist, vec![item("a")]),
        });
        assert!(rig.view().open.as_ref().is_some_and(|o| o.items.ready().is_some()));
    }

    /// Salvare: la playlist nasce subito, con tutti i brani «in arrivo», e un avviso lo dice. Non
    /// serve nulla se non c'è una playlist pronta, se è vuota o se la libreria è piena.
    #[test]
    fn saving_creates_the_playlist_with_every_track_pending() {
        let mut rig = Rig::new();
        rig.cmd(ExternalCmd::SavePlaylist { id: id(100) });
        assert!(rig.playlist(id(100)).is_none() && rig.events().is_empty(), "nessuna playlist aperta");
        rig.open(Vec::new());
        rig.cmd(ExternalCmd::SavePlaylist { id: id(100) });
        assert!(rig.playlist(id(100)).is_none() && rig.events().is_empty(), "una playlist vuota");

        rig.open(vec![item("a"), item("b")]);
        rig.cmd(ExternalCmd::SavePlaylist { id: id(100) });
        let saved = rig.playlist(id(100)).expect("la playlist c'è");
        assert_eq!((saved.name.as_str(), saved.tracks.len(), saved.pending.len()), ("Sera", 0, 2));
        assert_eq!(rig.events(), [ExternalEvent::PlaylistSaved { name: "Sera".into(), count: 2 }]);
    }

    #[test]
    fn a_full_library_saves_nothing() {
        let mut config = Config::default();
        config.core.playlists = (0..MAX_PLAYLISTS)
            .map(|n| Playlist::new(&format!("Lista {n}"), Vec::new()))
            .collect::<Vec<_>>()
            .into();
        let mut rig = Rig::with(config);
        rig.open(vec![item("a")]);
        rig.cmd(ExternalCmd::SavePlaylist { id: id(100) });
        assert!(rig.playlist(id(100)).is_none());
        assert!(rig.events().is_empty(), "niente avviso per ciò che non è successo");
    }

    // ---------- «Altro di …» ----------

    fn extras_page(name: &str, image: Option<&str>) -> Result<Box<MetaArtistPage>, AppError> {
        Ok(Box::new(MetaArtistPage {
            artist: MetaArtist { id: 9, name: name.into(), image_url: image.map(str::to_owned), fans: 12 },
            top: vec![item("t1")],
            albums: vec![evening()],
        }))
    }

    fn artist_page(n: u128) -> Arc<ArtistPage> {
        Arc::new(ArtistPage {
            id: id(n),
            name: format!("Artista {n}"),
            image_url: None,
            albums: Vec::new(),
            tracks: vec![track(n)],
        })
    }

    /// Mostra nel catalogo la pagina di un artista, come farebbe il dominio del catalogo.
    fn show(rig: &mut Rig, page: &Arc<ArtistPage>) {
        let view =
            CatalogView { artist: Some((page.id, Load::Ready(Arc::clone(page)))), ..CatalogView::default() };
        rig.state.catalog.view = Arc::new(view);
    }

    fn artist_jobs(effects: &[Effect]) -> Vec<(Uuid, String)> {
        jobs(effects)
            .into_iter()
            .filter_map(|job| match job {
                Job::Artist { id, name, .. } => Some((*id, name.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_opened_artist_page_asks_for_its_extras_once() {
        let mut rig = Rig::new();
        let page = artist_page(5);
        show(&mut rig, &page);
        let effects = rig.library(LibraryCmd::MarkNewsSeen);
        assert_eq!(artist_jobs(&effects), [(id(5), "Artista 5".to_owned())]);
        assert!(matches!(rig.view().artist(id(5)), Some(Load::Loading)));
        assert!(artist_jobs(&rig.library(LibraryCmd::MarkNewsSeen)).is_empty(), "già in arrivo");

        rig.done(Done::Artist { id: id(5), result: extras_page("Artista 5", None) });
        let extras = rig.view().artist(id(5)).and_then(|load| load.ready().cloned()).expect("pronta");
        assert_eq!((extras.top.len(), extras.albums.len()), (1, 1));
        // La stessa pagina, o un suo aggiornamento in silenzio: niente di nuovo da chiedere.
        show(&mut rig, &artist_page(5));
        assert!(artist_jobs(&rig.library(LibraryCmd::MarkNewsSeen)).is_empty());
    }

    #[test]
    fn a_failed_extras_page_is_asked_again_when_the_page_comes_back() {
        let mut rig = Rig::new();
        let page = artist_page(5);
        show(&mut rig, &page);
        rig.library(LibraryCmd::MarkNewsSeen);
        rig.done(Done::Artist { id: id(5), result: Err(AppError::Timeout) });
        assert!(matches!(rig.view().artist(id(5)), Some(Load::Failed(AppError::Timeout))));
        assert!(
            artist_jobs(&rig.library(LibraryCmd::MarkNewsSeen)).is_empty(),
            "stessa pagina, nessun ciclo"
        );

        // L'utente va altrove e torna: la pagina riappare, e si riprova.
        rig.state.catalog.view = Arc::new(CatalogView::default());
        rig.library(LibraryCmd::MarkNewsSeen);
        show(&mut rig, &page);
        assert_eq!(artist_jobs(&rig.library(LibraryCmd::MarkNewsSeen)), [(id(5), "Artista 5".to_owned())]);
    }

    #[test]
    fn the_extras_cache_forgets_the_least_used_artist() {
        let mut rig = Rig::new();
        let total = u128::try_from(ARTIST_EXTRAS).expect("small") + 6;
        for n in 1..=total {
            show(&mut rig, &artist_page(n));
            rig.library(LibraryCmd::MarkNewsSeen);
            rig.done(Done::Artist { id: id(n), result: extras_page("x", None) });
        }
        let view = rig.view();
        assert_eq!(view.artists.len(), ARTIST_EXTRAS, "una capienza sola");
        assert!(view.artist(id(1)).is_none() && view.artist(id(6)).is_none(), "escono le più vecchie");
        assert!(view.artist(id(7)).is_some() && view.artist(id(total)).is_some());
    }

    /// La scheda dell'artista dà la foto a chi lo segue e non ne ha: una volta, e solo se manca.
    #[test]
    fn the_official_photo_goes_to_a_followed_artist_without_one() {
        use crate::config::taste::FollowedArtist;
        let mut rig = Rig::new();
        let followed = |n: u128, cover: Option<&str>| FollowedArtist {
            id: id(n),
            name: format!("Artista {n}"),
            cover_url: cover.map(str::to_owned),
        };
        rig.library(LibraryCmd::ToggleFollow(followed(5, None)));
        rig.library(LibraryCmd::ToggleFollow(followed(6, Some("mia.png"))));
        let cover = |rig: &Rig, n: u128| {
            rig.state.library.view.followed.iter().find(|f| f.id == id(n)).and_then(|f| f.cover_url.clone())
        };
        show(&mut rig, &artist_page(5));
        rig.library(LibraryCmd::MarkNewsSeen);
        rig.done(Done::Artist { id: id(5), result: extras_page("Artista 5", Some("ufficiale.png")) });
        assert_eq!(cover(&rig, 5).as_deref(), Some("ufficiale.png"));
        show(&mut rig, &artist_page(6));
        rig.library(LibraryCmd::MarkNewsSeen);
        rig.done(Done::Artist { id: id(6), result: extras_page("Artista 6", Some("ufficiale.png")) });
        assert_eq!(cover(&rig, 6).as_deref(), Some("mia.png"), "una foto già scelta resta");
    }

    /// All'avvio e a ogni brano che parte: l'artista in corso e i seguiti senza foto, una volta.
    #[test]
    fn the_photos_of_the_playing_and_the_followed_artists_are_asked_at_start_and_per_track() {
        use crate::config::taste::FollowedArtist;
        let followed = |n: u128, cover: Option<&str>| FollowedArtist {
            id: id(n),
            name: format!("Artista {n}"),
            cover_url: cover.map(str::to_owned),
        };
        let mut config = Config::default();
        config.core.followed =
            vec![followed(10, None), followed(11, Some("c.png")), followed(12, None)].into();
        let mut rig = Rig::with(config);
        rig.play(vec![track(1)]);
        let effects = rig.input(Input::Start);
        let asked = artist_jobs(&effects);
        let ids: Vec<Uuid> = asked.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [track(1).artist.id, id(10), id(12)]);

        // Un altro brano parte: il suo artista è già chiesto, i seguiti non si richiedono.
        Arc::make_mut(&mut rig.state.playback.view).started += 1;
        assert!(artist_jobs(&rig.library(LibraryCmd::MarkNewsSeen)).is_empty());
        // Un artista nuovo in corso invece sì.
        rig.play(vec![track(2)]);
        Arc::make_mut(&mut rig.state.playback.view).started += 1;
        let ids: Vec<Uuid> =
            artist_jobs(&rig.library(LibraryCmd::MarkNewsSeen)).iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [track(2).artist.id]);
    }

    // ---------- Vista, eventi, lavori persi ----------

    /// Prestazioni (come in 4.2-4.6): fermo, o con ingressi che non cambiano niente, il core non
    /// pubblica; un passo di importazione è una pubblicazione sola.
    #[test]
    fn idle_publishes_nothing_and_an_import_step_publishes_once() {
        let mut rig = Rig::new();
        let start = rig.state.snapshot();
        assert!(rig.state.publish(&start).is_none(), "idle");
        for _ in 0..5 {
            rig.library(LibraryCmd::MarkNewsSeen);
            rig.cmd(ExternalCmd::RetryPlaylist);
            rig.cmd(ExternalCmd::PlayPlaylist { start: 0, video: false });
            rig.cmd(ExternalCmd::SavePlaylist { id: id(100) });
        }
        assert!(rig.state.publish(&start).is_none(), "ingressi che non cambiano niente");

        // Aprire è una pubblicazione, e la risposta un'altra.
        let evening = evening();
        rig.cmd(ExternalCmd::OpenPlaylist(evening.clone()));
        let opened = rig.state.publish(&start).expect("opened");
        assert_eq!(opened.versions.external, 1);
        rig.done(Done::Playlist {
            ticket: 1,
            source: SourceKind::Deezer,
            result: page(&evening, vec![item("a"), item("b")]),
        });
        let loaded = rig.state.publish(&opened).expect("loaded");
        assert_eq!((loaded.version, loaded.versions.external), (opened.version + 1, 2));
        assert!(rig.state.publish(&loaded).is_none());
        // Una risposta uguale (la stessa pagina, aperta di nuovo) non cambia niente.
        rig.cmd(ExternalCmd::OpenPlaylist(evening));
        assert!(rig.state.publish(&loaded).is_none());

        // Salvare: la playlist, l'avviso e la prima importazione sono un'istantanea sola.
        rig.cmd(ExternalCmd::SavePlaylist { id: id(100) });
        let saved: Snapshot = rig.state.publish(&loaded).expect("saved");
        assert_eq!(saved.version, loaded.version + 1);
        assert_eq!((saved.versions.library, saved.versions.external), (1, loaded.versions.external + 1));
        // Un brano importato: la playlist (nella libreria) e la riga «Apro…» cambiano insieme, una volta.
        let target = ImportTarget::Playlist(id(100));
        rig.imported(target, one(1));
        let step = rig.state.publish(&saved).expect("step");
        assert_eq!(step.version, saved.version + 1, "una sola pubblicazione per passo");
        assert_eq!(step.versions.library, saved.versions.library + 1);
        assert_eq!(step.versions.external, saved.versions.external + 1);
        assert_eq!(step.versions.playback, saved.versions.playback, "gli altri domini non si toccano");
        assert!(rig.state.publish(&step).is_none(), "pubblicato una volta, non si ripubblica");
    }

    #[test]
    fn the_event_list_is_bounded_and_numbers_never_repeat() {
        let mut external = External::default();
        for n in 0..EVENTS + 8 {
            external.raise(ExternalEvent::PlaylistSaved { name: "x".into(), count: n });
        }
        let view = Arc::clone(&external.view);
        assert_eq!(view.events.len(), EVENTS);
        assert_eq!(view.events.front().map(|r| r.seq), Some(9));
        assert_eq!(view.events.back().map(|r| r.seq), Some(u64::try_from(EVENTS + 8).expect("small")));
    }

    /// Un lavoro perso (panic) non lascia niente «in caricamento» e non ferma la fila.
    #[test]
    fn a_lost_job_never_stalls_the_domain() {
        let mut rig = Rig::new();
        let (playlist, effects) = rig.save(vec![item("a"), item("b")]);
        let lost_import = jobs(&effects).into_iter().map(Job::lost).next().expect("un lavoro");
        let effects = rig.done(lost_import);
        assert_eq!(
            imports(&effects),
            [(ImportTarget::Playlist(playlist), deezer("b"), false)],
            "a è «non disponibile» e la fila va avanti"
        );

        rig.cmd(ExternalCmd::OpenPlaylist(SourcePlaylist { id: "playlist/9".into(), ..evening() }));
        let ticket = rig.state.external.ticket;
        let job = Job::Playlist {
            api: Err(AppError::Network),
            ticket,
            source: SourceKind::Deezer,
            id: "playlist/9".into(),
        };
        rig.done(job.lost());
        assert!(matches!(rig.view().open.as_ref().map(|o| &o.items), Some(Load::Failed(AppError::Network))));
        let job = Job::Artist { api: Err(AppError::Network), id: id(5), name: "x".into() };
        rig.done(job.lost());
        assert!(matches!(rig.view().artist(id(5)), Some(Load::Failed(AppError::Network))));
    }

    // ---------- Avvisi (BACKLOG 8) ----------

    /// I numeri concordano: «1 brano» e «2 brani», «1 non disponibile» e «2 non disponibili», e con un
    /// brano solo la playlist non dice «i 1 brani».
    #[test]
    fn the_summaries_agree_in_number() {
        assert_eq!(ready_message("Sera", 2, 1), "«Sera» è pronta: 2 brani, 1 non disponibile.");
        assert_eq!(ready_message("Sera", 1, 0), "«Sera» è pronta: 1 brano.");
        assert_eq!(ready_message("Sera", 1, 2), "«Sera» è pronta: 1 brano, 2 non disponibili.");
        assert_eq!(ready_message("Sera", 0, 3), "«Sera» è pronta: 0 brani, 3 non disponibili.");
        assert_eq!(ready_message("Sera", 12, 0), "«Sera» è pronta: 12 brani.");
        assert_eq!(saved_message("Sera", 1), "«Sera» è nella libreria: il brano arriva a breve.");
        assert_eq!(saved_message("Sera", 3), "«Sera» è nella libreria: i 3 brani arrivano uno alla volta.");
    }

    #[test]
    fn only_errors_are_errors_and_covers_carry_no_message() {
        let imported = ExternalEvent::Imported(Arc::from(vec![track(1)]));
        assert_eq!((imported.message(), imported.is_error()), (None, false));
        let failed = ExternalEvent::PlayFailed(AppError::Network);
        assert_eq!(failed.message(), Some(AppError::Network.to_string()));
        assert!(failed.is_error() && ExternalEvent::NothingToPlay.is_error());
        assert!(!ExternalEvent::PlaylistSaved { name: "x".into(), count: 2 }.is_error());
    }
}
