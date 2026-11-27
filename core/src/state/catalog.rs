//! Catalogo (passo 4.4): Home, ricerca, elenco della libreria, album e artisti, librerie di Archive
//! e catalogo ufficiale, più il ritrovamento dei brani spariti e la rilettura di quelli salvati.
//!
//! Il dominio carica tutto dal backend con lavori ([`Job`]) e pubblica lo stato di ogni caricamento
//! ([`Load`]) nella vista: un frontend lo copia e lo disegna, senza aprire richieste. Le pagine di
//! album e artista si tengono in due cache a capienza fissa, con scarto del meno usato ([`Lru`]); una
//! pagina vecchia si mostra subito e si rilegge in silenzio. La ricerca aspetta 300 ms dopo l'ultimo
//! tasto con un timer del core ([`Timer::Search`]), e solo l'ultima richiesta conta: ogni tasto cambia
//! il numero della richiesta e le risposte di un numero superato si scartano.
//!
//! Il dominio parla con gli altri attraverso il passaggio che il core fa dopo ogni ingresso: ciò che
//! chiede alla libreria (i brani recenti della Home da cui nasce il feed, le copertine corrette, gli
//! artisti seguiti) e alla riproduzione (i brani ritrovati, la testata del primo brano di un album)
//! lo mette in due serbatoi ([`Catalog::take_library`], [`Catalog::take_playback`]), e ciò che gli
//! serve da loro (il brano mancante, il brano appena partito, il feed) glielo danno le sue funzioni
//! `heal`, `track_started` e `after_library`. Cambiare vista solo se qualcosa cambia davvero: un
//! comando uguale al precedente, o una risposta superata, non pubblica niente.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::api::{
    self, Album, AlbumSummary, ApiClient, ArtistPage, DiscoverItem, DiscoverLibrary, DiscoverPage,
    LibraryPage, MetaArtist, MetaSearch, Page, SearchResults, Sort, SourceItem, SourcePlaylist, Track,
    TrackQuery,
};
use crate::catalog::{Load, clean_query, portrait_url};
use crate::config;
use crate::effect::{self, Effect, TimerKey};
use crate::error::AppError;
use crate::shared::Shared;
use crate::state::library::{LibraryCmd, LibraryView};
use crate::state::playback::PlaybackCmd;

/// Attesa dopo l'ultimo tasto prima di lanciare la ricerca.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);
/// Brani per scaffale della Home (alfabetico).
const HOME_SECTION_SIZE: u32 = 12;
/// Brani recenti da cui si ricavano uscite, novità e mix (il massimo che il server dà).
const HOME_RELEASES: u32 = 100;
/// Album recenti mostrati nella Home.
const HOME_ALBUMS: u32 = 24;
/// Brani per pagina dell'elenco della libreria.
const LIBRARY_PAGE: u32 = 50;
/// Album mostrati nella libreria (una sola pagina, dalla A alla Z).
const LIBRARY_ALBUMS: u32 = 100;
/// Pagine di artista tenute in memoria: oltre, esce la meno usata.
pub const ARTIST_PAGES: usize = 40;
/// Pagine di album tenute in memoria: oltre, esce la meno usata.
pub const ALBUM_PAGES: usize = 24;
/// Dopo quanto una pagina in cache si riscarica (in silenzio) aprendola.
pub const STALE_AFTER: Duration = Duration::from_secs(90);
/// Pagine di artista caricate in anticipo per volta: l'abbondanza costerebbe al server.
const PREFETCH_BATCH: usize = 8;
/// Brani salvati senza copertina riletti dal server all'avvio, al massimo.
const STALE_REFRESH: usize = 24;
/// Brani riletti ricordati per la sessione, perché non si rileggano due volte.
const REFRESHED: usize = 256;
/// Eventi tenuti nella vista: un frontend che legge a ogni pubblicazione non ne perde.
const EVENTS: usize = 32;

/// Che cosa si carica o si ricarica.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    /// I tre scaffali della Home (recenti, alfabetico, album).
    Home,
    /// I brani e gli album dell'elenco della libreria.
    Library,
    Album(Uuid),
    Artist(Uuid),
}

/// I comandi del dominio.
#[derive(Debug, Clone)]
pub enum CatalogCmd {
    /// Mostra una risorsa: dalla cache, se c'è, altrimenti dalla rete. Se è già pronta o in arrivo
    /// non fa niente; sulla Home già carica fa rifare il feed se gli ascolti sono cambiati.
    Ensure(Resource),
    /// «Riprova»: ricarica dalla rete, anche se c'è in cache.
    Retry(Resource),
    /// La pagina successiva dei brani della libreria.
    LibraryMore,
    /// Cambia l'ordine della libreria e la ricarica.
    LibrarySort(Sort),
    /// Il testo della ricerca è cambiato: parte dopo [`SEARCH_DEBOUNCE`] dall'ultimo tasto.
    SearchInput(String),
    /// Cerca subito questo testo (all'avvio, o «Riprova»).
    Search(String),
    ArchiveLibraryOpen(DiscoverLibrary),
    ArchiveLibraryMore,
    ArchiveLibraryClose,
    /// Porta nel server l'artista del catalogo ufficiale con questo identificatore.
    OpenMetaArtist(u64),
    /// Il catalogo è cambiato (un'importazione): Home e libreria si ricaricano alla prossima visita.
    Changed,
}

/// Le tre sezioni della Home.
#[derive(Debug, Clone, Default)]
pub struct HomeView {
    pub recent: Load<Arc<[Track]>>,
    pub alphabetical: Load<Arc<[Track]>>,
    pub albums: Load<Arc<[AlbumSummary]>>,
}

/// L'elenco della libreria: brani a pagine e album in una pagina sola.
#[derive(Debug, Clone, Default)]
pub struct LibraryListing {
    pub tracks: Load<Shared<Track>>,
    pub next_offset: Option<i64>,
    pub loading_more: bool,
    /// L'errore di una pagina successiva: i brani già caricati restano.
    pub more_error: Option<AppError>,
    /// Ordine di album e brani: i più recenti o dalla A alla Z.
    pub sort: Sort,
    pub albums: Load<Arc<[AlbumSummary]>>,
    /// Il catalogo ha più album di quelli caricati.
    pub albums_truncated: bool,
}

/// Una libreria di Archive aperta: i suoi album, una pagina dopo l'altra.
#[derive(Debug, Clone)]
pub struct OpenLibrary {
    pub library: DiscoverLibrary,
    /// Prima pagina in caricamento o fallita; poi tutti gli album caricati finora.
    pub items: Load<Arc<[DiscoverItem]>>,
    pub next_page: Option<u32>,
    pub loading_more: bool,
}

impl OpenLibrary {
    /// Riceve una pagina di album: la prima sostituisce il caricamento (o diventa l'errore
    /// mostrato al posto degli album), le altre si aggiungono in fondo. Restituisce l'errore di una
    /// pagina successiva, da mostrare come avviso: gli album già caricati restano e «Altri album» si
    /// può ripremere.
    pub fn receive(&mut self, page: u32, result: Result<LibraryPage, AppError>) -> Result<(), AppError> {
        self.loading_more = false;
        let loaded = match result {
            Ok(loaded) => loaded,
            Err(err) if page == 1 => {
                self.items = Load::Failed(err);
                return Ok(());
            }
            Err(err) => return Err(err),
        };
        self.next_page = loaded.next_page;
        let mut items: Vec<DiscoverItem> = match &self.items {
            Load::Ready(earlier) if page > 1 => earlier.to_vec(),
            _ => Vec::new(),
        };
        items.extend(loaded.items);
        self.items = Load::Ready(Arc::from(items));
        Ok(())
    }
}

/// I risultati della ricerca: il catalogo, Internet Archive e il catalogo ufficiale.
#[derive(Debug, Clone, Default)]
pub struct SearchView {
    /// Il testo cercato per ultimo, già ripulito; vuoto se non si cerca niente.
    pub submitted: String,
    pub results: Load<Arc<SearchResults>>,
    /// Risultati da Internet Archive, per ciò che non è ancora nel catalogo.
    pub discover: Load<Arc<[DiscoverItem]>>,
    /// Librerie di Archive (netlabel) in cui stanno gli album trovati.
    pub libraries: Arc<[DiscoverLibrary]>,
    /// Libreria aperta dalla ricerca: al posto dei risultati di Archive si vedono i suoi album.
    pub open_library: Option<OpenLibrary>,
    /// Brani del catalogo ufficiale (Deezer).
    pub sources: Load<Arc<[SourceItem]>>,
    /// Album e playlist del catalogo ufficiale.
    pub playlists: Load<Arc<[SourcePlaylist]>>,
    /// Artisti del catalogo ufficiale.
    pub artists: Arc<[MetaArtist]>,
}

/// Qualcosa che il frontend deve far sapere o fare.
#[derive(Debug, Clone, PartialEq)]
pub enum CatalogEvent {
    /// L'artista del catalogo ufficiale ora ha una pagina: il frontend la apre.
    MetaArtistOpened(Uuid),
    MetaArtistFailed(AppError),
    /// Una pagina di album di una libreria di Archive non è arrivata: avviso, gli album restano.
    LibraryPageFailed(AppError),
    /// Un brano ritrovato o riletto dal server: la sua copertina può essere nuova.
    Replaced(Arc<Track>),
    /// Una pagina di artista è arrivata in anticipo (o aggiornata): le sue prime copertine si
    /// possono scaricare subito, così aprirla è immediato.
    Prefetched(Arc<ArtistPage>),
}

/// Un [`CatalogEvent`] con il suo numero d'ordine, che cresce a ogni evento.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogRaised {
    pub seq: u64,
    pub event: CatalogEvent,
}

/// Ciò che i frontend vedono del dominio.
#[derive(Debug, Clone, Default)]
pub struct CatalogView {
    pub home: HomeView,
    pub library: LibraryListing,
    pub search: SearchView,
    /// L'album aperto e il suo caricamento.
    pub album: Option<(Uuid, Load<Arc<Album>>)>,
    /// L'artista aperto e il suo caricamento.
    pub artist: Option<(Uuid, Load<Arc<ArtistPage>>)>,
    /// Gli ultimi eventi, dal più vecchio: un frontend mostra quelli con `seq` nuovo, in ordine. Se
    /// ne scorda al massimo [`EVENTS`] fra una lettura e l'altra, cioè mai.
    pub events: Arc<VecDeque<CatalogRaised>>,
}

/// I timer del dominio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {
    /// Scade [`SEARCH_DEBOUNCE`] dopo l'ultimo tasto: parte la ricerca.
    Search,
}

/// Quale scaffale della Home.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Recent,
    Alphabetical,
}

/// Come si carica una pagina di artista.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fetch {
    /// Aperta da chi guarda: se è quella aperta, la sostituisce e se fallisce lo mostra.
    Open,
    /// In anticipo o per aggiornare in silenzio: va in cache, e un errore non si mostra.
    Background,
}

/// I lavori asincroni del dominio (rete), che l'host esegue sul runtime. Quelli che riempiono una
/// parte della vista portano il suo numero di epoca o di richiesta, per scartare le risposte
/// superate.
#[derive(Debug)]
pub(crate) enum Job {
    Home {
        api: Result<ApiClient, AppError>,
        epoch: u64,
        section: Section,
    },
    HomeAlbums {
        api: Result<ApiClient, AppError>,
        epoch: u64,
    },
    Search {
        api: Result<ApiClient, AppError>,
        ticket: u64,
        query: String,
    },
    Discover {
        api: Result<ApiClient, AppError>,
        ticket: u64,
        query: String,
    },
    Meta {
        api: Result<ApiClient, AppError>,
        ticket: u64,
        query: String,
    },
    LibraryTracks {
        api: Result<ApiClient, AppError>,
        epoch: u64,
        sort: Sort,
        offset: u32,
        append: bool,
    },
    LibraryAlbums {
        api: Result<ApiClient, AppError>,
        epoch: u64,
        sort: Sort,
    },
    Album {
        api: Result<ApiClient, AppError>,
        id: Uuid,
        silent: bool,
    },
    Artist {
        api: Result<ApiClient, AppError>,
        id: Uuid,
        fetch: Fetch,
    },
    ArchiveLibrary {
        api: Result<ApiClient, AppError>,
        identifier: String,
        page: u32,
    },
    MetaArtist {
        api: Result<ApiClient, AppError>,
        id: u64,
    },
    /// Il brano non esiste più sul server: lo cerca nel catalogo ufficiale e lo importa di nuovo.
    Heal {
        api: Result<ApiClient, AppError>,
        track: Arc<Track>,
    },
    /// Rilegge dal server un brano salvato.
    Refresh {
        api: Result<ApiClient, AppError>,
        id: Uuid,
    },
}

/// L'esito di un [`Job`], che torna al dominio.
#[derive(Debug)]
pub(crate) enum Done {
    Home { epoch: u64, section: Section, result: Result<Page<Track>, AppError> },
    HomeAlbums { epoch: u64, result: Result<Page<AlbumSummary>, AppError> },
    Search { ticket: u64, result: Result<Box<SearchResults>, AppError> },
    Discover { ticket: u64, result: Result<Box<DiscoverPage>, AppError> },
    Meta { ticket: u64, result: Result<Box<MetaSearch>, AppError> },
    LibraryTracks { epoch: u64, append: bool, result: Result<Page<Track>, AppError> },
    LibraryAlbums { epoch: u64, result: Result<Page<AlbumSummary>, AppError> },
    Album { id: Uuid, silent: bool, result: Result<Box<Album>, AppError> },
    Artist { id: Uuid, fetch: Fetch, result: Result<Box<ArtistPage>, AppError> },
    ArchiveLibrary { identifier: String, page: u32, result: Result<LibraryPage, AppError> },
    MetaArtist(Result<Uuid, AppError>),
    Healed { old: Uuid, result: Result<Box<Track>, AppError> },
    Refreshed(Result<Box<Track>, AppError>),
}

/// L'esito di un lavoro andato perso.
fn lost<T>() -> Result<T, AppError> {
    Err(AppError::Network)
}

fn is_idle<T>(load: &Load<T>) -> bool {
    matches!(load, Load::Idle)
}

/// Una cache a capienza fissa con scarto del meno usato: chi legge o scrive una pagina la porta in
/// fondo, e a capienza piena esce quella in testa. Ricorda quando ogni pagina è arrivata, per dire
/// se è vecchia.
#[derive(Debug)]
pub(crate) struct Lru<T> {
    capacity: usize,
    /// Dal meno al più usato.
    entries: VecDeque<(Uuid, Instant, Arc<T>)>,
}

impl<T> Lru<T> {
    pub(crate) fn new(capacity: usize) -> Self {
        Self { capacity, entries: VecDeque::new() }
    }

    pub(crate) fn contains(&self, id: Uuid) -> bool {
        self.entries.iter().any(|(known, ..)| *known == id)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// La pagina, e se è vecchia (arrivata più di [`STALE_AFTER`] fa). Averla letta la rende la più
    /// recente.
    pub(crate) fn get(&mut self, id: Uuid, now: Instant) -> Option<(Arc<T>, bool)> {
        let at = self.entries.iter().position(|(known, ..)| *known == id)?;
        let entry = self.entries.remove(at)?;
        let found = (Arc::clone(&entry.2), now.saturating_duration_since(entry.1) > STALE_AFTER);
        self.entries.push_back(entry);
        Some(found)
    }

    /// Mette la pagina in fondo (sostituendo quella con lo stesso id) e scarta le più vecchie oltre
    /// la capienza.
    pub(crate) fn put(&mut self, id: Uuid, value: T, now: Instant) -> Arc<T> {
        let value = Arc::new(value);
        self.entries.retain(|(known, ..)| *known != id);
        self.entries.push_back((id, now, Arc::clone(&value)));
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
        value
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Lo stato del dominio: la vista pubblicata e ciò che resta dentro il core.
#[derive(Debug)]
pub(crate) struct Catalog {
    pub(crate) view: Arc<CatalogView>,
    api: Result<ApiClient, AppError>,
    server: Option<String>,
    /// Cambia quando il catalogo cambia o il backend è un altro: le risposte di prima si scartano.
    home_epoch: u64,
    library_epoch: u64,
    /// Il numero della ricerca: cresce a ogni tasto, e solo le risposte dell'ultimo contano.
    ticket: u64,
    /// L'ultimo testo scritto, com'è (il frontend lo ha già nel suo campo).
    draft: String,
    artists: Lru<ArtistPage>,
    albums: Lru<Album>,
    /// Pagine di artista in arrivo in sottofondo.
    loading: HashSet<Uuid>,
    /// Il numero dell'ultimo evento.
    events: u64,
    /// Brani già riletti in questa sessione, i più recenti in fondo.
    refreshed: VecDeque<Uuid>,
    /// L'ultimo caricamento di brano visto partire.
    last_started: u64,
    /// I brani salvati senza copertina vanno riletti: lo si fa una volta, all'avvio.
    startup: bool,
    /// Gli artisti della Home vanno caricati in anticipo, appena la libreria ne ha il feed.
    prefetch_home: bool,
    /// Il testo cercato è cambiato e le impostazioni non l'hanno ancora (`take_query`).
    query: Option<String>,
    /// Ciò che il dominio chiede alla libreria e alla riproduzione, in attesa del passaggio.
    library: Vec<LibraryCmd>,
    playback: Vec<PlaybackCmd>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Catalog {
    /// Il dominio con il backend scelto nelle impostazioni.
    pub(crate) fn new(server: Option<String>) -> Self {
        Self {
            view: Arc::default(),
            api: ApiClient::new(&config::api_url(server.as_deref())),
            server,
            home_epoch: 0,
            library_epoch: 0,
            ticket: 0,
            draft: String::new(),
            artists: Lru::new(ARTIST_PAGES),
            albums: Lru::new(ALBUM_PAGES),
            loading: HashSet::new(),
            events: 0,
            refreshed: VecDeque::new(),
            last_started: 0,
            startup: false,
            prefetch_home: false,
            query: None,
            library: Vec::new(),
            playback: Vec::new(),
        }
    }

    fn view_mut(&mut self) -> &mut CatalogView {
        Arc::make_mut(&mut self.view)
    }

    /// L'epoca delle risposte della Home in arrivo (per i test).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn home_epoch(&self) -> u64 {
        self.home_epoch
    }

    /// L'epoca delle risposte dell'elenco della libreria in arrivo (per i test).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn library_epoch(&self) -> u64 {
        self.library_epoch
    }

    fn api(&self) -> Result<ApiClient, AppError> {
        self.api.clone()
    }

    fn run(effects: &mut Vec<Effect>, job: Job) {
        effects.push(Effect::Run(effect::Job::Catalog(job)));
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

    /// Il testo cercato per ultimo, se è cambiato dopo l'ultima consegna alle impostazioni.
    pub(crate) fn take_query(&mut self) -> Option<String> {
        self.query.take()
    }

    /// Il core è appena partito: i brani salvati senza copertina si rileggono, alla prima occasione.
    pub(crate) fn start(&mut self) {
        self.startup = true;
    }

    /// Il backend scelto nelle impostazioni è cambiato: il client HTTP si rifà, le pagine in memoria
    /// sono di un altro server e si buttano, l'elenco della libreria e le pagine aperte si
    /// ricaricano alla prossima visita, e la Home si ricarica subito.
    pub(crate) fn set_server(&mut self, server: &Option<String>, effects: &mut Vec<Effect>) {
        if self.server == *server {
            return;
        }
        self.server.clone_from(server);
        self.api = ApiClient::new(&config::api_url(server.as_deref()));
        self.artists.clear();
        self.albums.clear();
        self.loading.clear();
        self.library_epoch += 1;
        let view = self.view_mut();
        view.library = LibraryListing { sort: view.library.sort, ..LibraryListing::default() };
        view.album = None;
        view.artist = None;
        self.load_home(effects);
    }

    /// Un brano non esiste più sul server: lo si cerca nel catalogo ufficiale.
    pub(crate) fn heal(&mut self, track: Arc<Track>, effects: &mut Vec<Effect>) {
        Self::run(effects, Job::Heal { api: self.api(), track });
    }

    /// Un altro brano ha cominciato a suonare: lo si rilegge dal server, una volta per sessione (le
    /// copie salvate possono essere di prima che il server avesse il videoclip o la copertina
    /// ufficiale).
    pub(crate) fn track_started(&mut self, started: u64, current: Option<Uuid>, effects: &mut Vec<Effect>) {
        if started == self.last_started {
            return;
        }
        self.last_started = started;
        if let Some(id) = current {
            self.refresh(id, effects);
        }
    }

    /// La libreria ha pubblicato ciò che il dominio le aveva chiesto: ora si possono fare le cose che
    /// dipendono dal suo stato (gli artisti della Home, i brani senza copertina).
    pub(crate) fn after_library(&mut self, library: &LibraryView, effects: &mut Vec<Effect>) {
        if std::mem::take(&mut self.startup) {
            let saved = library
                .liked
                .iter()
                .chain(library.recent.iter())
                .chain(library.playlists.iter().flat_map(|p| p.tracks.iter()));
            let mut ids: Vec<Uuid> = Vec::new();
            for track in saved {
                if track.cover_url.is_none() && !ids.contains(&track.id) {
                    ids.push(track.id);
                }
            }
            ids.truncate(STALE_REFRESH);
            for id in ids {
                self.refresh(id, effects);
            }
        }
        if std::mem::take(&mut self.prefetch_home) {
            // Gli artisti che la Home mette in vista (seguiti e primi del catalogo): si caricano
            // subito, così aprirne uno è immediato.
            let followed = library.followed.iter().take(4).map(|a| a.id);
            let shown = library.feed.artists.iter().take(6).map(|card| card.artist.id);
            let ids: Vec<Uuid> = followed.chain(shown).collect();
            self.prefetch(ids, effects);
        }
    }

    // ---------- Comandi ----------

    pub(crate) fn apply(&mut self, command: CatalogCmd, now: Instant, effects: &mut Vec<Effect>) {
        match command {
            CatalogCmd::Ensure(Resource::Home) => {
                let home = &self.view.home;
                if home.recent.needs_load() || home.alphabetical.needs_load() || home.albums.needs_load() {
                    self.load_home(effects);
                } else {
                    // Gli ascolti sono cambiati dall'ultima volta: i mix e gli artisti si rifanno.
                    self.library.push(LibraryCmd::RefreshFeed);
                }
            }
            CatalogCmd::Retry(Resource::Home) => self.load_home(effects),
            CatalogCmd::Ensure(Resource::Library) => {
                let listing = &self.view.library;
                let (tracks, albums) = (listing.tracks.needs_load(), listing.albums.needs_load());
                if tracks {
                    self.load_library_tracks(0, false, effects);
                }
                if albums {
                    self.load_library_albums(effects);
                }
            }
            CatalogCmd::Retry(Resource::Library) => {
                self.load_library_tracks(0, false, effects);
                self.load_library_albums(effects);
            }
            CatalogCmd::Ensure(Resource::Album(id)) => {
                if !matches!(&self.view.album, Some((open, load)) if *open == id && !load.needs_load()) {
                    self.open_album(id, false, now, effects);
                }
            }
            CatalogCmd::Retry(Resource::Album(id)) => self.open_album(id, true, now, effects),
            CatalogCmd::Ensure(Resource::Artist(id)) => {
                if !matches!(&self.view.artist, Some((open, load)) if *open == id && !load.needs_load()) {
                    self.open_artist(id, false, now, effects);
                }
            }
            CatalogCmd::Retry(Resource::Artist(id)) => self.open_artist(id, true, now, effects),
            CatalogCmd::LibraryMore => {
                let listing = &self.view.library;
                if let (Some(offset), false) = (listing.next_offset, listing.loading_more) {
                    self.view_mut().library.loading_more = true;
                    self.load_library_tracks(u32::try_from(offset).unwrap_or(0), true, effects);
                }
            }
            CatalogCmd::LibrarySort(sort) => {
                if self.view.library.sort != sort {
                    // Le pagine in arrivo con l'ordine di prima non vanno più bene.
                    self.library_epoch += 1;
                    self.view_mut().library.sort = sort;
                    self.load_library_tracks(0, false, effects);
                    self.load_library_albums(effects);
                }
            }
            CatalogCmd::SearchInput(text) => self.search_input(text, effects),
            CatalogCmd::Search(text) => {
                self.ticket += 1;
                self.draft = text;
                effects.push(Effect::CancelTimer(TimerKey::Catalog(Timer::Search)));
                self.run_search(effects);
            }
            CatalogCmd::ArchiveLibraryOpen(library) => {
                let identifier = library.identifier.clone();
                self.view_mut().search.open_library =
                    Some(OpenLibrary { library, items: Load::Loading, next_page: None, loading_more: false });
                Self::run(effects, Job::ArchiveLibrary { api: self.api(), identifier, page: 1 });
            }
            CatalogCmd::ArchiveLibraryMore => {
                let Some(open) = &self.view.search.open_library else { return };
                let Some(page) = open.next_page.filter(|_| !open.loading_more) else { return };
                let identifier = open.library.identifier.clone();
                if let Some(open) = &mut self.view_mut().search.open_library {
                    open.loading_more = true;
                }
                Self::run(effects, Job::ArchiveLibrary { api: self.api(), identifier, page });
            }
            CatalogCmd::ArchiveLibraryClose => {
                if self.view.search.open_library.is_some() {
                    self.view_mut().search.open_library = None;
                }
            }
            CatalogCmd::OpenMetaArtist(id) => Self::run(effects, Job::MetaArtist { api: self.api(), id }),
            CatalogCmd::Changed => self.changed(),
        }
    }

    pub(crate) fn fire(&mut self, timer: Timer, _now: Instant, effects: &mut Vec<Effect>) {
        match timer {
            Timer::Search => self.run_search(effects),
        }
    }

    // ---------- Home e libreria ----------

    fn load_home(&mut self, effects: &mut Vec<Effect>) {
        let home = &self.view.home;
        if !(home.recent.is_loading() && home.alphabetical.is_loading() && home.albums.is_loading()) {
            self.view_mut().home =
                HomeView { recent: Load::Loading, alphabetical: Load::Loading, albums: Load::Loading };
        }
        let epoch = self.home_epoch;
        for section in [Section::Recent, Section::Alphabetical] {
            Self::run(effects, Job::Home { api: self.api(), epoch, section });
        }
        Self::run(effects, Job::HomeAlbums { api: self.api(), epoch });
    }

    fn load_library_tracks(&mut self, offset: u32, append: bool, effects: &mut Vec<Effect>) {
        if !append {
            let listing = &mut self.view_mut().library;
            listing.tracks = Load::Loading;
            listing.loading_more = false;
        }
        let (sort, epoch) = (self.view.library.sort, self.library_epoch);
        Self::run(effects, Job::LibraryTracks { api: self.api(), epoch, sort, offset, append });
    }

    fn load_library_albums(&mut self, effects: &mut Vec<Effect>) {
        self.view_mut().library.albums = Load::Loading;
        let (sort, epoch) = (self.view.library.sort, self.library_epoch);
        Self::run(effects, Job::LibraryAlbums { api: self.api(), epoch, sort });
    }

    /// Il catalogo è cambiato: Home e libreria si ricaricano alla prossima visita, le risposte in
    /// arrivo si scartano e la libreria butta il feed che aveva costruito.
    fn changed(&mut self) {
        self.home_epoch += 1;
        self.library_epoch += 1;
        let view = &self.view;
        let untouched = is_idle(&view.home.recent)
            && is_idle(&view.home.alphabetical)
            && is_idle(&view.home.albums)
            && is_idle(&view.library.tracks)
            && is_idle(&view.library.albums)
            && view.library.next_offset.is_none()
            && !view.library.loading_more
            && view.library.more_error.is_none();
        if !untouched {
            let view = self.view_mut();
            view.home = HomeView::default();
            view.library = LibraryListing { sort: view.library.sort, ..LibraryListing::default() };
        }
        self.prefetch_home = false;
        self.library.push(LibraryCmd::HomeTracks(None));
    }

    // ---------- Album e artisti ----------

    fn open_album(&mut self, id: Uuid, force: bool, now: Instant, effects: &mut Vec<Effect>) {
        let cached = if force { None } else { self.albums.get(id, now) };
        let Some((album, stale)) = cached else {
            self.view_mut().album = Some((id, Load::Loading));
            Self::run(effects, Job::Album { api: self.api(), id, silent: false });
            return;
        };
        self.after_album(&album);
        self.view_mut().album = Some((id, Load::Ready(album)));
        if stale {
            Self::run(effects, Job::Album { api: self.api(), id, silent: true });
        }
    }

    /// Ciò che segue a un album mostrato: «Riproduci» parte quasi sempre dal primo brano, quindi la
    /// sua testata si prepara; l'album salvato prende la copertina di adesso, e le copie salvate dei
    /// suoi brani anche.
    fn after_album(&mut self, album: &Album) {
        if let Some(first) = album.tracks.first() {
            self.playback.push(PlaybackCmd::Warm(first.id));
        }
        self.library
            .push(LibraryCmd::SetSavedAlbumCover { id: album.id, cover_url: album.cover_url.clone() });
        self.patch_covers(&album.tracks);
    }

    fn open_artist(&mut self, id: Uuid, force: bool, now: Instant, effects: &mut Vec<Effect>) {
        let cached = if force { None } else { self.artists.get(id, now) };
        let Some((page, stale)) = cached else {
            self.view_mut().artist = Some((id, Load::Loading));
            Self::run(effects, Job::Artist { api: self.api(), id, fetch: Fetch::Open });
            return;
        };
        self.view_mut().artist = Some((id, Load::Ready(page)));
        if stale {
            self.refresh_artist(id, effects);
        }
    }

    /// Riscarica una pagina già in cache senza mostrare un caricamento.
    fn refresh_artist(&mut self, id: Uuid, effects: &mut Vec<Effect>) {
        if self.loading.insert(id) {
            Self::run(effects, Job::Artist { api: self.api(), id, fetch: Fetch::Background });
        }
    }

    /// Carica in anticipo le pagine di questi artisti, se non le ha già: non in cache e non già in
    /// arrivo, al massimo [`PREFETCH_BATCH`], senza ripetizioni.
    fn prefetch(&mut self, ids: impl IntoIterator<Item = Uuid>, effects: &mut Vec<Effect>) {
        let mut wanted = Vec::new();
        for id in ids {
            if wanted.len() == PREFETCH_BATCH {
                break;
            }
            if !self.artists.contains(id) && !wanted.contains(&id) && self.loading.insert(id) {
                wanted.push(id);
            }
        }
        for id in wanted {
            Self::run(effects, Job::Artist { api: self.api(), id, fetch: Fetch::Background });
        }
    }

    /// Le copertine che il catalogo ha appena rimandato correggono quelle salvate (coda, recenti,
    /// mi piace, playlist, ascolti): un brano salvato prima che il server smettesse di dare le onde
    /// di Archive come copertina ne conserverebbe l'indirizzo.
    fn patch_covers(&mut self, fresh: &[Track]) {
        if fresh.is_empty() {
            return;
        }
        let fresh: HashMap<Uuid, Option<String>> =
            fresh.iter().map(|t| (t.id, t.cover_url.clone())).collect();
        let fresh = Arc::new(fresh);
        self.library.push(LibraryCmd::PatchCovers(Arc::clone(&fresh)));
        self.playback.push(PlaybackCmd::PatchCovers(fresh));
    }

    // ---------- Ricerca ----------

    fn search_input(&mut self, text: String, effects: &mut Vec<Effect>) {
        if text == self.draft {
            return;
        }
        self.draft = text;
        self.ticket += 1;
        if clean_query(&self.draft).is_some() {
            effects.push(Effect::Timer { key: TimerKey::Catalog(Timer::Search), after: SEARCH_DEBOUNCE });
            return;
        }
        effects.push(Effect::CancelTimer(TimerKey::Catalog(Timer::Search)));
        let search = &self.view.search;
        if !matches!(search.results, Load::Idle) || !search.submitted.is_empty() {
            let search = &mut self.view_mut().search;
            search.results = Load::Idle;
            search.submitted.clear();
            self.query = Some(String::new());
        }
    }

    /// Lancia la ricerca del testo scritto, in parallelo nel catalogo, su Internet Archive e nel
    /// catalogo ufficiale (brani, artisti, album e playlist con copertine e foto).
    fn run_search(&mut self, effects: &mut Vec<Effect>) {
        let Some(query) = clean_query(&self.draft) else { return };
        let ticket = self.ticket;
        let search = &mut self.view_mut().search;
        search.submitted.clone_from(&query);
        search.results = Load::Loading;
        search.discover = Load::Loading;
        search.libraries = Arc::default();
        search.open_library = None;
        search.sources = Load::Loading;
        search.playlists = Load::Loading;
        search.artists = Arc::default();
        self.library.push(LibraryCmd::RememberSearch(query.clone()));
        self.query = Some(query.clone());
        let (a, b, c) = (self.api(), self.api(), self.api());
        Self::run(effects, Job::Search { api: a, ticket, query: query.clone() });
        Self::run(effects, Job::Discover { api: b, ticket, query: query.clone() });
        Self::run(effects, Job::Meta { api: c, ticket, query });
    }

    // ---------- Rilettura dei brani ----------

    /// Rilegge dal server un brano, se non l'ha già fatto in questa sessione.
    fn refresh(&mut self, id: Uuid, effects: &mut Vec<Effect>) {
        if self.refreshed.contains(&id) {
            return;
        }
        self.refreshed.push_back(id);
        if self.refreshed.len() > REFRESHED {
            self.refreshed.pop_front();
        }
        Self::run(effects, Job::Refresh { api: self.api(), id });
    }

    fn raise(&mut self, event: CatalogEvent) {
        self.events += 1;
        let raised = CatalogRaised { seq: self.events, event };
        let events = Arc::make_mut(&mut self.view_mut().events);
        events.push_back(raised);
        while events.len() > EVENTS {
            events.pop_front();
        }
    }

    // ---------- Risposte ----------

    pub(crate) fn done(&mut self, done: Done, now: Instant, effects: &mut Vec<Effect>) {
        match done {
            Done::Home { epoch, section, result } => {
                if epoch != self.home_epoch {
                    return;
                }
                let value = Load::of(result.map(|page| Arc::<[Track]>::from(page.items)));
                match section {
                    Section::Recent => {
                        // Con dei brani, il feed lo costruisce la libreria e la Home li mostra
                        // insieme a lui, nella stessa istantanea.
                        if let Load::Ready(tracks) = &value {
                            self.library.push(LibraryCmd::HomeTracks(Some(Arc::clone(tracks))));
                            self.patch_covers(tracks);
                            self.prefetch_home = true;
                        }
                        self.view_mut().home.recent = value;
                    }
                    Section::Alphabetical => self.view_mut().home.alphabetical = value,
                }
            }
            Done::HomeAlbums { epoch, result } => {
                if epoch == self.home_epoch {
                    self.view_mut().home.albums = Load::of(result.map(|page| Arc::from(page.items)));
                }
            }
            Done::Search { ticket, result } => {
                if ticket != self.ticket {
                    return;
                }
                if let Ok(results) = &result {
                    let ids: Vec<Uuid> = results.artists.iter().take(4).map(|a| a.id).collect();
                    self.patch_covers(&results.tracks);
                    self.prefetch(ids, effects);
                }
                self.view_mut().search.results = Load::of(result.map(|results| Arc::new(*results)));
            }
            Done::Discover { ticket, result } => {
                if ticket != self.ticket {
                    return;
                }
                let search = &mut self.view_mut().search;
                match result {
                    Ok(page) => {
                        search.discover = Load::Ready(Arc::from(page.items));
                        search.libraries = Arc::from(page.libraries);
                    }
                    Err(err) => {
                        search.discover = Load::Failed(err);
                        search.libraries = Arc::default();
                    }
                }
            }
            Done::Meta { ticket, result } => {
                if ticket != self.ticket {
                    return;
                }
                let search = &mut self.view_mut().search;
                match result {
                    Ok(found) => {
                        search.artists = Arc::from(found.artists);
                        search.sources = Load::Ready(Arc::from(found.tracks));
                        search.playlists = Load::Ready(Arc::from(found.lists));
                    }
                    Err(err) => {
                        search.artists = Arc::default();
                        search.sources = Load::Failed(err);
                        search.playlists = Load::Idle;
                    }
                }
            }
            Done::LibraryTracks { epoch, append, result } => {
                if epoch != self.library_epoch {
                    return;
                }
                let listing = &mut self.view_mut().library;
                listing.loading_more = false;
                match result {
                    Ok(page) => {
                        listing.next_offset = page.next_offset;
                        listing.more_error = None;
                        match &mut listing.tracks {
                            Load::Ready(tracks) if append => tracks.make_mut().extend(page.items),
                            other => *other = Load::Ready(page.items.into()),
                        }
                    }
                    Err(err) if append => listing.more_error = Some(err),
                    Err(err) => listing.tracks = Load::Failed(err),
                }
            }
            Done::LibraryAlbums { epoch, result } => {
                if epoch != self.library_epoch {
                    return;
                }
                let listing = &mut self.view_mut().library;
                listing.albums_truncated = result.as_ref().is_ok_and(|page| page.next_offset.is_some());
                listing.albums = Load::of(result.map(|page| Arc::from(page.items)));
            }
            Done::Album { id, silent, result } => self.album_arrived(id, silent, result, now),
            Done::Artist { id, fetch, result } => self.artist_arrived(id, fetch, result, now),
            Done::ArchiveLibrary { identifier, page, result } => {
                let Some(open) =
                    self.view.search.open_library.as_ref().filter(|o| o.library.identifier == identifier)
                else {
                    // Una risposta per una libreria già chiusa si ignora.
                    return;
                };
                let mut open = open.clone();
                let failed = open.receive(page, result).err();
                self.view_mut().search.open_library = Some(open);
                if let Some(err) = failed {
                    self.raise(CatalogEvent::LibraryPageFailed(err));
                }
            }
            Done::MetaArtist(result) => self.raise(match result {
                Ok(id) => CatalogEvent::MetaArtistOpened(id),
                Err(err) => CatalogEvent::MetaArtistFailed(err),
            }),
            Done::Healed { old, result } => match result {
                Ok(new) => {
                    // Prende il posto del vecchio in coda e fra i salvati, e il brano riparte.
                    self.playback.push(PlaybackCmd::ReplaceTrack { old, new: new.clone(), restart: true });
                    self.library.push(LibraryCmd::ReplaceTrack { old, new: (*new).clone() });
                    self.raise(CatalogEvent::Replaced(Arc::new(*new)));
                }
                // Il brano non si è ritrovato: se è quello che si voleva sentire, resta in errore.
                Err(_) => self.playback.push(PlaybackCmd::HealFailed(old)),
            },
            Done::Refreshed(result) => {
                if let Ok(fresh) = result {
                    let old = fresh.id;
                    self.playback.push(PlaybackCmd::ReplaceTrack { old, new: fresh.clone(), restart: false });
                    self.library.push(LibraryCmd::ReplaceTrack { old, new: (*fresh).clone() });
                    if fresh.cover_url.is_some() {
                        self.raise(CatalogEvent::Replaced(Arc::new(*fresh)));
                    }
                }
            }
        }
    }

    fn album_arrived(&mut self, id: Uuid, silent: bool, result: Result<Box<Album>, AppError>, now: Instant) {
        let showing = matches!(&self.view.album, Some((open, _)) if *open == id);
        match result {
            Ok(album) => {
                let album = self.albums.put(id, *album, now);
                if showing {
                    self.after_album(&album);
                    self.view_mut().album = Some((id, Load::Ready(album)));
                }
            }
            // Un aggiornamento in silenzio che fallisce lascia la pagina com'è.
            Err(_) if silent => {}
            Err(err) if showing => self.view_mut().album = Some((id, Load::Failed(err))),
            Err(_) => {}
        }
    }

    fn artist_arrived(
        &mut self,
        id: Uuid,
        fetch: Fetch,
        result: Result<Box<ArtistPage>, AppError>,
        now: Instant,
    ) {
        self.loading.remove(&id);
        let open = matches!(&self.view.artist, Some((open, _)) if *open == id);
        match (fetch, result) {
            (Fetch::Open, Ok(page)) => {
                let page = self.artists.put(id, *page, now);
                if open {
                    self.view_mut().artist = Some((id, Load::Ready(Arc::clone(&page))));
                    // L'immagine di un artista seguito segue quella che il catalogo ha adesso.
                    self.library.push(LibraryCmd::SetFollowedCover {
                        id,
                        cover_url: portrait_url(&page).map(str::to_owned),
                        only_if_missing: false,
                    });
                    self.patch_covers(&page.tracks);
                }
            }
            (Fetch::Open, Err(err)) => {
                if open {
                    self.view_mut().artist = Some((id, Load::Failed(err)));
                }
            }
            (Fetch::Background, Ok(page)) => {
                // Va in cache, e se è quella aperta la sostituisce senza che la vista salti.
                let page = self.artists.put(id, *page, now);
                let replace =
                    matches!(&self.view.artist, Some((open, Load::Ready(_) | Load::Idle)) if *open == id);
                if replace {
                    self.view_mut().artist = Some((id, Load::Ready(Arc::clone(&page))));
                }
                self.patch_covers(&page.tracks);
                self.raise(CatalogEvent::Prefetched(page));
            }
            (Fetch::Background, Err(_)) => {}
        }
    }
}

impl Job {
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Home { api, epoch, section } => {
                let (sort, limit) = match section {
                    Section::Recent => (Sort::Recent, HOME_RELEASES),
                    Section::Alphabetical => (Sort::Title, HOME_SECTION_SIZE),
                };
                let result = match api {
                    Ok(api) => api.tracks(TrackQuery { q: None, sort, limit, offset: 0 }).await,
                    Err(err) => Err(err),
                };
                Done::Home { epoch, section, result }
            }
            Self::HomeAlbums { api, epoch } => {
                let result = match api {
                    Ok(api) => api.albums(Sort::Recent, HOME_ALBUMS, 0).await,
                    Err(err) => Err(err),
                };
                Done::HomeAlbums { epoch, result }
            }
            Self::Search { api, ticket, query } => {
                let result = match api {
                    Ok(api) => api.search(&query).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Search { ticket, result }
            }
            Self::Discover { api, ticket, query } => {
                let result = match api {
                    Ok(api) => api.discover(&query).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Discover { ticket, result }
            }
            Self::Meta { api, ticket, query } => {
                let result = match api {
                    Ok(api) => api.meta_search(&query).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Meta { ticket, result }
            }
            Self::LibraryTracks { api, epoch, sort, offset, append } => {
                let result = match api {
                    Ok(api) => api.tracks(TrackQuery { q: None, sort, limit: LIBRARY_PAGE, offset }).await,
                    Err(err) => Err(err),
                };
                Done::LibraryTracks { epoch, append, result }
            }
            Self::LibraryAlbums { api, epoch, sort } => {
                let result = match api {
                    Ok(api) => api.albums(sort, LIBRARY_ALBUMS, 0).await,
                    Err(err) => Err(err),
                };
                Done::LibraryAlbums { epoch, result }
            }
            Self::Album { api, id, silent } => {
                let result = match api {
                    Ok(api) => api.album(id).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Album { id, silent, result }
            }
            Self::Artist { api, id, fetch } => {
                let result = match api {
                    Ok(api) => api.artist(id).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Artist { id, fetch, result }
            }
            Self::ArchiveLibrary { api, identifier, page } => {
                let result = match api {
                    Ok(api) => api.library(&identifier, page).await,
                    Err(err) => Err(err),
                };
                Done::ArchiveLibrary { identifier, page, result }
            }
            Self::MetaArtist { api, id } => {
                let result = match api {
                    Ok(api) => api.open_meta_artist(id).await,
                    Err(err) => Err(err),
                };
                Done::MetaArtist(result)
            }
            Self::Heal { api, track } => {
                let old = track.id;
                let result = match api {
                    Ok(api) => api::heal_track(api, (*track).clone()).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Healed { old, result }
            }
            Self::Refresh { api, id } => {
                let result = match api {
                    Ok(api) => api.track(id).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Refreshed(result)
            }
        }
    }

    /// Se il lavoro va perso, l'esito è un errore di rete: il dominio non resta ad aspettare (e una
    /// ricerca o un elenco non restano «in caricamento» per sempre).
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Home { epoch, section, .. } => {
                Done::Home { epoch: *epoch, section: *section, result: lost() }
            }
            Self::HomeAlbums { epoch, .. } => Done::HomeAlbums { epoch: *epoch, result: lost() },
            Self::Search { ticket, .. } => Done::Search { ticket: *ticket, result: lost() },
            Self::Discover { ticket, .. } => Done::Discover { ticket: *ticket, result: lost() },
            Self::Meta { ticket, .. } => Done::Meta { ticket: *ticket, result: lost() },
            Self::LibraryTracks { epoch, append, .. } => {
                Done::LibraryTracks { epoch: *epoch, append: *append, result: lost() }
            }
            Self::LibraryAlbums { epoch, .. } => Done::LibraryAlbums { epoch: *epoch, result: lost() },
            Self::Album { id, silent, .. } => Done::Album { id: *id, silent: *silent, result: lost() },
            Self::Artist { id, fetch, .. } => Done::Artist { id: *id, fetch: *fetch, result: lost() },
            Self::ArchiveLibrary { identifier, page, .. } => {
                Done::ArchiveLibrary { identifier: identifier.clone(), page: *page, result: lost() }
            }
            Self::MetaArtist { .. } => Done::MetaArtist(lost()),
            Self::Heal { track, .. } => Done::Healed { old: track.id, result: lost() },
            Self::Refresh { .. } => Done::Refreshed(lost()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures::track;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn page_of<T>(items: Vec<T>, next_offset: Option<i64>) -> Page<T> {
        Page { items, limit: 50, offset: 0, next_offset }
    }

    fn artist(n: u128) -> ArtistPage {
        ArtistPage {
            id: id(n),
            name: format!("Artista {n}"),
            image_url: None,
            albums: Vec::new(),
            tracks: vec![track(n)],
        }
    }

    fn album(n: u128) -> Album {
        Album {
            id: id(n),
            title: format!("Album {n}"),
            released_on: None,
            artist: crate::api::models::ArtistRef { id: id(1000), name: "Artista".into() },
            cover_url: Some(format!("cover-{n}")),
            tracks: vec![track(n * 10), track(n * 10 + 1)],
        }
    }

    fn network() -> AppError {
        AppError::Network
    }

    /// Il dominio da solo, con un orologio finto che si sposta a mano.
    struct Rig {
        catalog: Catalog,
        now: Instant,
    }

    impl Rig {
        fn new() -> Self {
            Self { catalog: Catalog::default(), now: Instant::now() }
        }

        fn cmd(&mut self, command: CatalogCmd) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.catalog.apply(command, self.now, &mut effects);
            effects
        }

        fn done(&mut self, done: Done) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.catalog.done(done, self.now, &mut effects);
            effects
        }

        fn fire(&mut self) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.catalog.fire(Timer::Search, self.now, &mut effects);
            effects
        }

        fn view(&self) -> Arc<CatalogView> {
            Arc::clone(&self.catalog.view)
        }

        fn wait(&mut self, by: Duration) {
            self.now += by;
        }
    }

    /// I lavori che gli effetti chiedono.
    fn jobs(effects: Vec<Effect>) -> Vec<Job> {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Run(effect::Job::Catalog(job)) => Some(job),
                _ => None,
            })
            .collect()
    }

    fn timers(effects: &[Effect]) -> Vec<Duration> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Timer { key: TimerKey::Catalog(Timer::Search), after } => Some(*after),
                _ => None,
            })
            .collect()
    }

    fn ticket_of(job: &Job) -> u64 {
        match job {
            Job::Search { ticket, .. } | Job::Discover { ticket, .. } | Job::Meta { ticket, .. } => *ticket,
            other => panic!("not a search job: {other:?}"),
        }
    }

    // ---------- Ricerca ----------

    /// `typing_debounces_and_late_results_are_dropped`: ogni tasto riarma l'attesa di 300 ms, parte
    /// solo l'ultima ricerca, e le risposte di una richiesta superata si scartano.
    #[test]
    fn typing_debounces_and_late_results_are_dropped() {
        let mut rig = Rig::new();
        let first = rig.cmd(CatalogCmd::SearchInput("lo".into()));
        assert_eq!(timers(&first), [SEARCH_DEBOUNCE], "il primo tasto arma l'attesa");
        assert!(jobs(first).is_empty(), "mentre si scrive non parte nessuna ricerca");
        let second = rig.cmd(CatalogCmd::SearchInput("low".into()));
        assert_eq!(timers(&second), [SEARCH_DEBOUNCE], "il secondo la riarma: è un debounce");
        assert!(matches!(rig.view().search.results, Load::Idle));

        // L'attesa scade: tre ricerche in parallelo, tutte della stessa richiesta.
        let started = jobs(rig.fire());
        assert_eq!(started.len(), 3);
        let ticket = ticket_of(&started[0]);
        assert!(started.iter().all(|job| ticket_of(job) == ticket));
        assert!(matches!(&started[0], Job::Search { query, .. } if query == "low"));
        let view = rig.view();
        assert_eq!(view.search.submitted, "low");
        assert!(view.search.results.is_loading() && view.search.discover.is_loading());
        assert!(view.search.sources.is_loading() && view.search.playlists.is_loading());
        let library = rig.catalog.take_library();
        assert!(matches!(library.as_slice(), [LibraryCmd::RememberSearch(q)] if q == "low"), "{library:?}");
        assert_eq!(rig.catalog.take_query(), Some("low".into()));

        // La risposta di una richiesta superata si scarta; quella giusta si mostra.
        let found = |n| SearchResults { tracks: vec![track(n)], ..SearchResults::default() };
        let before = rig.view();
        rig.done(Done::Search { ticket: ticket - 1, result: Ok(Box::new(found(1))) });
        assert!(Arc::ptr_eq(&before, &rig.view()), "una risposta superata non cambia niente");
        rig.done(Done::Search { ticket, result: Ok(Box::new(found(2))) });
        assert_eq!(rig.view().search.results, Load::Ready(Arc::new(found(2))));

        // Si scrive ancora: una risposta in ritardo non sovrascrive i risultati.
        rig.cmd(CatalogCmd::SearchInput("lowe".into()));
        let before = rig.view();
        rig.done(Done::Search { ticket, result: Ok(Box::new(found(3))) });
        assert!(Arc::ptr_eq(&before, &rig.view()));
        assert_eq!(rig.view().search.results, Load::Ready(Arc::new(found(2))));
    }

    #[test]
    fn the_three_answers_fill_their_own_parts_and_a_failure_does_not_spill() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Search("rock".into()));
        let ticket = rig.catalog.ticket;
        rig.done(Done::Discover { ticket, result: Err(network()) });
        rig.done(Done::Meta {
            ticket,
            result: Ok(Box::new(MetaSearch { tracks: Vec::new(), artists: Vec::new(), lists: Vec::new() })),
        });
        let view = rig.view();
        assert_eq!(view.search.discover, Load::Failed(network()));
        assert!(view.search.libraries.is_empty());
        assert!(
            matches!(view.search.sources, Load::Ready(_)) && matches!(view.search.playlists, Load::Ready(_))
        );
        assert!(view.search.results.is_loading(), "the first search still waits for its own answer");
        rig.done(Done::Meta { ticket, result: Err(network()) });
        assert_eq!(rig.view().search.sources, Load::Failed(network()));
        assert_eq!(rig.view().search.playlists, Load::Idle);
    }

    #[test]
    fn clearing_the_text_clears_the_results_and_cancels_the_wait() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Search("low".into()));
        let ticket = rig.catalog.ticket;
        rig.done(Done::Search { ticket, result: Ok(Box::default()) });
        assert!(matches!(rig.view().search.results, Load::Ready(_)));
        rig.catalog.take_query();

        let effects = rig.cmd(CatalogCmd::SearchInput("  \u{7} ".into()));
        assert!(
            effects.iter().any(|e| matches!(e, Effect::CancelTimer(TimerKey::Catalog(_)))),
            "{effects:?}"
        );
        assert!(timers(&effects).is_empty());
        let view = rig.view();
        assert_eq!((&view.search.results, view.search.submitted.as_str()), (&Load::Idle, ""));
        assert_eq!(rig.catalog.take_query(), Some(String::new()), "the saved query follows");
        // E se non c'era niente da cancellare, niente cambia.
        let before = rig.view();
        rig.cmd(CatalogCmd::SearchInput(String::new()));
        assert!(Arc::ptr_eq(&before, &rig.view()));
    }

    /// Prestazioni: lo stesso testo due volte non arma niente e non cambia la vista, e l'attesa
    /// scaduta senza un testo valido non fa partire niente.
    #[test]
    fn an_identical_input_changes_nothing_and_an_empty_wait_starts_nothing() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::SearchInput("low".into()));
        let before = rig.view();
        let ticket = rig.catalog.ticket;
        let again = rig.cmd(CatalogCmd::SearchInput("low".into()));
        assert!(again.is_empty(), "{again:?}");
        assert_eq!(rig.catalog.ticket, ticket, "the request number does not move");
        assert!(Arc::ptr_eq(&before, &rig.view()));

        let mut rig = Rig::new();
        assert!(rig.fire().is_empty(), "nothing typed: nothing to search");
    }

    #[test]
    fn a_search_answer_prefetches_the_first_artists_and_corrects_the_saved_covers() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Search("rock".into()));
        let ticket = rig.catalog.ticket;
        let artists: Vec<_> = (1..=6)
            .map(|n| crate::api::ArtistSummary {
                id: id(n),
                name: format!("A{n}"),
                track_count: 1,
                album_count: 1,
                cover_url: None,
            })
            .collect();
        let results = SearchResults { tracks: vec![track(1)], artists, ..SearchResults::default() };
        let effects = rig.done(Done::Search { ticket, result: Ok(Box::new(results)) });
        let prefetched: Vec<Uuid> = jobs(effects)
            .into_iter()
            .filter_map(|job| match job {
                Job::Artist { id, fetch: Fetch::Background, .. } => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(prefetched, [id(1), id(2), id(3), id(4)], "the first four");
        assert!(rig.catalog.take_library().iter().any(|c| matches!(c, LibraryCmd::PatchCovers(_))));
        assert!(rig.catalog.take_playback().iter().any(|c| matches!(c, PlaybackCmd::PatchCovers(_))));
    }

    // ---------- Artisti e album ----------

    #[test]
    fn the_artist_cache_forgets_the_least_used_page() {
        let mut cache: Lru<ArtistPage> = Lru::new(ARTIST_PAGES);
        let now = Instant::now();
        for n in 0..ARTIST_PAGES as u128 {
            cache.put(id(n), artist(n), now);
        }
        // Rileggere la più vecchia la salva: esce la successiva.
        assert!(cache.get(id(0), now).is_some());
        cache.put(id(1000), artist(1000), now);
        assert_eq!(cache.len(), ARTIST_PAGES, "the capacity is fixed");
        assert!(cache.contains(id(0)) && !cache.contains(id(1)));
        // Rimetterne una già presente la sostituisce senza far crescere la cache.
        cache.put(id(0), artist(0), now);
        assert_eq!(cache.len(), ARTIST_PAGES);
    }

    #[test]
    fn a_cached_page_is_fresh_then_stale() {
        let mut cache: Lru<ArtistPage> = Lru::new(4);
        let now = Instant::now();
        cache.put(id(1), artist(1), now);
        assert_eq!(cache.get(id(1), now).map(|(_, stale)| stale), Some(false));
        let later = now + STALE_AFTER + Duration::from_secs(1);
        assert_eq!(cache.get(id(1), later).map(|(_, stale)| stale), Some(true));
        assert!(cache.get(id(2), now).is_none());
    }

    #[test]
    fn an_opened_artist_loads_then_opens_at_once_from_the_cache_and_refreshes_when_stale() {
        let mut rig = Rig::new();
        let started = jobs(rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(7)))));
        assert!(matches!(started.as_slice(), [Job::Artist { fetch: Fetch::Open, .. }]));
        assert!(matches!(rig.view().artist, Some((open, Load::Loading)) if open == id(7)));
        // Già in arrivo: aprirla di nuovo non chiede niente.
        assert!(jobs(rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(7))))).is_empty());

        rig.done(Done::Artist { id: id(7), fetch: Fetch::Open, result: Ok(Box::new(artist(7))) });
        let view = rig.view();
        let Some((_, Load::Ready(page))) = &view.artist else { panic!("{:?}", view.artist) };
        assert_eq!(page.id, id(7));
        // Un artista seguito prende l'immagine del catalogo e le copertine salvate si correggono.
        let library = rig.catalog.take_library();
        assert!(library.iter().any(
            |c| matches!(c, LibraryCmd::SetFollowedCover { id: i, only_if_missing: false, .. } if *i == id(7))
        ));
        assert!(library.iter().any(|c| matches!(c, LibraryCmd::PatchCovers(_))));

        // Altrove, poi di nuovo qui: dalla cache, senza «in caricamento» e senza rete (è fresca).
        rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(8))));
        let effects = rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(7))));
        assert!(jobs(effects).is_empty(), "fresh: no request");
        assert!(matches!(&rig.view().artist, Some((open, Load::Ready(_))) if *open == id(7)));

        // Dopo 90 secondi si mostra subito e si rilegge in silenzio.
        rig.wait(STALE_AFTER + Duration::from_secs(1));
        rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(8))));
        let refreshed = jobs(rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(7)))));
        assert!(matches!(refreshed.as_slice(), [Job::Artist { fetch: Fetch::Background, .. }]));
        assert!(
            matches!(&rig.view().artist, Some((open, Load::Ready(_))) if *open == id(7)),
            "shown at once"
        );
        // La risposta in silenzio sostituisce la pagina aperta senza che la vista salti.
        let mut fresher = artist(7);
        fresher.name = "Nuovo nome".into();
        rig.done(Done::Artist { id: id(7), fetch: Fetch::Background, result: Ok(Box::new(fresher)) });
        let view = rig.view();
        assert!(matches!(&view.artist, Some((_, Load::Ready(page))) if page.name == "Nuovo nome"));
        assert!(
            matches!(view.events.back().map(|e| &e.event), Some(CatalogEvent::Prefetched(p)) if p.id == id(7))
        );
    }

    #[test]
    fn retrying_an_artist_ignores_the_cache_and_a_failure_is_shown() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(7))));
        rig.done(Done::Artist { id: id(7), fetch: Fetch::Open, result: Ok(Box::new(artist(7))) });
        let started = jobs(rig.cmd(CatalogCmd::Retry(Resource::Artist(id(7)))));
        assert!(matches!(started.as_slice(), [Job::Artist { fetch: Fetch::Open, .. }]));
        assert!(matches!(rig.view().artist, Some((_, Load::Loading))), "retry shows the loading state");
        rig.done(Done::Artist { id: id(7), fetch: Fetch::Open, result: Err(network()) });
        assert!(matches!(&rig.view().artist, Some((_, Load::Failed(e))) if *e == network()));
        // Una pagina che arriva per un artista che non è più aperto non tocca la vista.
        rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(9))));
        let before = rig.view();
        rig.done(Done::Artist { id: id(7), fetch: Fetch::Open, result: Ok(Box::new(artist(7))) });
        assert!(Arc::ptr_eq(&before, &rig.view()));
    }

    #[test]
    fn prefetching_skips_known_and_pending_artists_and_is_capped_per_batch() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(1))));
        rig.done(Done::Artist { id: id(1), fetch: Fetch::Open, result: Ok(Box::new(artist(1))) });
        let mut effects = Vec::new();
        rig.catalog.prefetch([1, 2, 2, 3].map(id), &mut effects);
        let asked: Vec<Uuid> = jobs(effects)
            .into_iter()
            .filter_map(|job| match job {
                Job::Artist { id, .. } => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(asked, [id(2), id(3)]);
        // Già in arrivo: nessuna seconda richiesta; arrivata, in cache: nemmeno.
        let mut effects = Vec::new();
        rig.catalog.prefetch([id(2)], &mut effects);
        assert!(effects.is_empty());
        rig.done(Done::Artist { id: id(2), fetch: Fetch::Background, result: Ok(Box::new(artist(2))) });
        rig.catalog.prefetch([id(2)], &mut effects);
        assert!(effects.is_empty(), "arrived: it is in the cache");

        let mut effects = Vec::new();
        rig.catalog.prefetch((100..150).map(id), &mut effects);
        assert_eq!(jobs(effects).len(), PREFETCH_BATCH);
    }

    #[test]
    fn an_album_opens_from_the_cache_and_warms_its_first_track() {
        let mut rig = Rig::new();
        let started = jobs(rig.cmd(CatalogCmd::Ensure(Resource::Album(id(5)))));
        assert!(matches!(started.as_slice(), [Job::Album { silent: false, .. }]));
        rig.done(Done::Album { id: id(5), silent: false, result: Ok(Box::new(album(5))) });
        assert!(
            matches!(&rig.view().album, Some((open, Load::Ready(a))) if *open == id(5) && a.tracks.len() == 2)
        );
        // La testata del primo brano, la copertina dell'album salvato e quelle dei brani.
        let playback = rig.catalog.take_playback();
        assert!(
            matches!(playback.as_slice(), [PlaybackCmd::Warm(first), PlaybackCmd::PatchCovers(_)] if *first == id(50)),
            "{playback:?}"
        );
        let library = rig.catalog.take_library();
        assert!(matches!(library.first(), Some(LibraryCmd::SetSavedAlbumCover { id: a, .. }) if *a == id(5)));

        // Rientrando dopo un altro album, dalla cache: nessuna richiesta.
        rig.cmd(CatalogCmd::Ensure(Resource::Album(id(6))));
        assert!(jobs(rig.cmd(CatalogCmd::Ensure(Resource::Album(id(5))))).is_empty());
        assert!(matches!(&rig.view().album, Some((open, Load::Ready(_))) if *open == id(5)));
        // Vecchia: si mostra subito e si rilegge in silenzio, e se l'aggiornamento fallisce resta.
        rig.wait(STALE_AFTER + Duration::from_secs(1));
        rig.cmd(CatalogCmd::Ensure(Resource::Album(id(6))));
        let silent = jobs(rig.cmd(CatalogCmd::Ensure(Resource::Album(id(5)))));
        assert!(matches!(silent.as_slice(), [Job::Album { silent: true, .. }]));
        let before = rig.view();
        rig.done(Done::Album { id: id(5), silent: true, result: Err(network()) });
        assert!(Arc::ptr_eq(&before, &rig.view()), "a failed silent refresh leaves the page as it is");
        // «Riprova» butta la cache e mostra il caricamento.
        assert!(matches!(
            jobs(rig.cmd(CatalogCmd::Retry(Resource::Album(id(5))))).as_slice(),
            [Job::Album { silent: false, .. }]
        ));
        assert!(matches!(rig.view().album, Some((_, Load::Loading))));
        rig.done(Done::Album { id: id(5), silent: false, result: Err(network()) });
        assert!(matches!(&rig.view().album, Some((_, Load::Failed(_)))));
    }

    #[test]
    fn the_album_cache_is_bounded() {
        let mut cache: Lru<Album> = Lru::new(ALBUM_PAGES);
        let now = Instant::now();
        for n in 0..(ALBUM_PAGES as u128 * 3) {
            cache.put(id(n), album(n), now);
        }
        assert_eq!(cache.len(), ALBUM_PAGES);
        assert!(!cache.contains(id(0)) && cache.contains(id(ALBUM_PAGES as u128 * 3 - 1)));
    }

    // ---------- Home ----------

    fn home_answers(rig: &mut Rig, epoch: u64) {
        rig.done(Done::Home {
            epoch,
            section: Section::Recent,
            result: Ok(page_of((1..=4).map(track).collect(), None)),
        });
        rig.done(Done::Home {
            epoch,
            section: Section::Alphabetical,
            result: Ok(page_of(vec![track(9)], None)),
        });
        rig.done(Done::HomeAlbums { epoch, result: Ok(page_of(Vec::new(), None)) });
    }

    #[test]
    fn the_home_loads_three_shelves_and_hands_the_recent_tracks_to_the_library() {
        let mut rig = Rig::new();
        let started = jobs(rig.cmd(CatalogCmd::Ensure(Resource::Home)));
        assert_eq!(started.len(), 3);
        let view = rig.view();
        assert!(
            view.home.recent.is_loading()
                && view.home.alphabetical.is_loading()
                && view.home.albums.is_loading()
        );
        // Mentre carica non si chiede di nuovo.
        assert!(rig.cmd(CatalogCmd::Ensure(Resource::Home)).iter().all(|e| !matches!(e, Effect::Run(_))));
        rig.catalog.take_library();

        let epoch = rig.catalog.home_epoch();
        home_answers(&mut rig, epoch);
        let view = rig.view();
        assert!(matches!(&view.home.recent, Load::Ready(t) if t.len() == 4));
        let library = rig.catalog.take_library();
        let [LibraryCmd::HomeTracks(Some(tracks)), LibraryCmd::PatchCovers(_)] = library.as_slice() else {
            panic!("{library:?}");
        };
        let Load::Ready(shown) = &view.home.recent else { panic!() };
        assert!(Arc::ptr_eq(tracks, shown), "la libreria costruisce il feed dagli stessi brani");

        // Pronta: rientrando si chiede solo di rifare il feed, se serve.
        let effects = rig.cmd(CatalogCmd::Ensure(Resource::Home));
        assert!(effects.is_empty());
        assert!(matches!(rig.catalog.take_library().as_slice(), [LibraryCmd::RefreshFeed]));
        // Un errore si riprova.
        rig.done(Done::HomeAlbums { epoch, result: Err(network()) });
        assert!(jobs(rig.cmd(CatalogCmd::Ensure(Resource::Home))).len() == 3);
    }

    #[test]
    fn a_changed_catalogue_resets_the_home_and_the_listing_and_drops_late_answers() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Ensure(Resource::Home));
        rig.cmd(CatalogCmd::Ensure(Resource::Library));
        let old = rig.catalog.home_epoch();
        home_answers(&mut rig, old);
        rig.catalog.take_library();

        rig.cmd(CatalogCmd::Changed);
        let view = rig.view();
        assert!(view.home.recent.needs_load() && matches!(view.home.recent, Load::Idle));
        assert!(matches!(view.library.tracks, Load::Idle) && matches!(view.library.albums, Load::Idle));
        let library = rig.catalog.take_library();
        assert!(matches!(library.as_slice(), [LibraryCmd::HomeTracks(None)]), "{library:?}");

        // Una risposta di prima del cambio non risorge.
        let before = rig.view();
        home_answers(&mut rig, old);
        rig.done(Done::LibraryTracks { epoch: 0, append: false, result: Ok(page_of(vec![track(1)], None)) });
        assert!(Arc::ptr_eq(&before, &rig.view()));
        assert!(rig.catalog.take_library().is_empty());
        // Cambiare un catalogo già vuoto non pubblica niente.
        rig.cmd(CatalogCmd::Changed);
        assert!(Arc::ptr_eq(&before, &rig.view()));
    }

    #[test]
    fn a_new_server_drops_the_caches_and_reloads_the_home() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(1))));
        rig.done(Done::Artist { id: id(1), fetch: Fetch::Open, result: Ok(Box::new(artist(1))) });
        let mut effects = Vec::new();
        rig.catalog.set_server(&None, &mut effects);
        assert!(effects.is_empty(), "the same server: nothing");
        rig.catalog.set_server(&Some("http://127.0.0.1:9".into()), &mut effects);
        assert_eq!(jobs(effects).len(), 3, "the home loads again");
        assert!(!rig.catalog.artists.contains(id(1)));
        assert!(rig.view().artist.is_none());
    }

    // ---------- Libreria ----------

    #[test]
    fn the_listing_appends_pages_and_a_new_order_drops_the_old_pages() {
        let mut rig = Rig::new();
        let started = jobs(rig.cmd(CatalogCmd::Ensure(Resource::Library)));
        assert!(matches!(
            started.as_slice(),
            [Job::LibraryTracks { offset: 0, append: false, .. }, Job::LibraryAlbums { .. }]
        ));
        let epoch = rig.catalog.library_epoch();
        rig.done(Done::LibraryTracks {
            epoch,
            append: false,
            result: Ok(page_of(vec![track(1), track(2)], Some(2))),
        });
        let view = rig.view();
        assert_eq!(view.library.next_offset, Some(2));
        assert!(matches!(&view.library.tracks, Load::Ready(t) if t.len() == 2));

        // «Altri»: una pagina alla volta, e non due richieste insieme.
        let more = jobs(rig.cmd(CatalogCmd::LibraryMore));
        assert!(matches!(more.as_slice(), [Job::LibraryTracks { offset: 2, append: true, .. }]));
        assert!(rig.view().library.loading_more);
        assert!(jobs(rig.cmd(CatalogCmd::LibraryMore)).is_empty());
        rig.done(Done::LibraryTracks { epoch, append: true, result: Ok(page_of(vec![track(3)], None)) });
        let view = rig.view();
        assert!(matches!(&view.library.tracks, Load::Ready(t) if t.len() == 3));
        assert!(!view.library.loading_more && view.library.next_offset.is_none());

        // Una pagina successiva che fallisce lascia i brani e dice l'errore.
        rig.done(Done::LibraryTracks { epoch, append: true, result: Err(network()) });
        assert_eq!(rig.view().library.more_error, Some(network()));
        assert!(matches!(&rig.view().library.tracks, Load::Ready(t) if t.len() == 3));

        // Un altro ordine: ricarica tutto e le pagine in arrivo con l'ordine di prima si scartano.
        let same = rig.cmd(CatalogCmd::LibrarySort(Sort::Recent));
        assert!(same.is_empty(), "the same order changes nothing");
        let sorted = jobs(rig.cmd(CatalogCmd::LibrarySort(Sort::Title)));
        assert!(matches!(
            sorted.as_slice(),
            [
                Job::LibraryTracks { sort: Sort::Title, offset: 0, .. },
                Job::LibraryAlbums { sort: Sort::Title, .. }
            ]
        ));
        assert!(rig.view().library.tracks.is_loading() && rig.view().library.albums.is_loading());
        let before = rig.view();
        rig.done(Done::LibraryTracks { epoch, append: true, result: Ok(page_of(vec![track(4)], None)) });
        assert!(Arc::ptr_eq(&before, &rig.view()), "an old page does not land in the new order");
        let epoch = rig.catalog.library_epoch();
        rig.done(Done::LibraryAlbums { epoch, result: Ok(page_of(Vec::new(), Some(100))) });
        assert!(rig.view().library.albums_truncated);
    }

    // ---------- Librerie di Archive e catalogo ufficiale ----------

    fn library(identifier: &str) -> DiscoverLibrary {
        DiscoverLibrary {
            identifier: identifier.into(),
            title: "Netlabel".into(),
            creator: None,
            found: 2,
            cover_url: None,
        }
    }

    fn item(identifier: &str) -> DiscoverItem {
        DiscoverItem {
            identifier: identifier.into(),
            title: identifier.into(),
            creator: None,
            licence: "cc-by".into(),
            year: None,
            cover_url: None,
            lossless: true,
        }
    }

    fn items(page: u32, next_page: Option<u32>, names: &[&str]) -> Result<LibraryPage, AppError> {
        let _ = page;
        Ok(LibraryPage { items: names.iter().map(|n| item(n)).collect(), next_page })
    }

    #[test]
    fn library_pages_are_appended_in_order_and_a_failure_is_an_event() {
        let mut rig = Rig::new();
        let first = jobs(rig.cmd(CatalogCmd::ArchiveLibraryOpen(library("net"))));
        assert!(matches!(first.as_slice(), [Job::ArchiveLibrary { page: 1, .. }]));
        assert!(matches!(&rig.view().search.open_library, Some(o) if o.items.is_loading()));
        rig.done(Done::ArchiveLibrary {
            identifier: "net".into(),
            page: 1,
            result: items(1, Some(2), &["a", "b"]),
        });
        let more = jobs(rig.cmd(CatalogCmd::ArchiveLibraryMore));
        assert!(matches!(more.as_slice(), [Job::ArchiveLibrary { page: 2, .. }]));
        assert!(jobs(rig.cmd(CatalogCmd::ArchiveLibraryMore)).is_empty(), "una pagina alla volta");
        rig.done(Done::ArchiveLibrary { identifier: "net".into(), page: 2, result: items(2, None, &["c"]) });
        let open = rig.view().search.open_library.clone().expect("open");
        let Load::Ready(all) = &open.items else { panic!() };
        assert_eq!(all.iter().map(|i| i.identifier.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!((open.next_page, open.loading_more), (None, false));

        // Una pagina successiva che fallisce: gli album restano e si può riprovare.
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::ArchiveLibraryOpen(library("net")));
        rig.done(Done::ArchiveLibrary {
            identifier: "net".into(),
            page: 1,
            result: items(1, Some(2), &["a"]),
        });
        rig.cmd(CatalogCmd::ArchiveLibraryMore);
        rig.done(Done::ArchiveLibrary { identifier: "net".into(), page: 2, result: Err(network()) });
        let open = rig.view().search.open_library.clone().expect("open");
        assert!(matches!(&open.items, Load::Ready(i) if i.len() == 1));
        assert_eq!((open.next_page, open.loading_more), (Some(2), false));
        assert!(matches!(
            rig.view().events.back().map(|e| &e.event),
            Some(CatalogEvent::LibraryPageFailed(_))
        ));

        // Una prima pagina fallita prende il posto degli album.
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::ArchiveLibraryOpen(library("net")));
        rig.done(Done::ArchiveLibrary { identifier: "net".into(), page: 1, result: Err(network()) });
        assert!(matches!(&rig.view().search.open_library, Some(o) if matches!(o.items, Load::Failed(_))));

        // Chiusa, una risposta in ritardo si ignora.
        rig.cmd(CatalogCmd::ArchiveLibraryClose);
        let before = rig.view();
        rig.done(Done::ArchiveLibrary { identifier: "net".into(), page: 1, result: items(1, None, &["x"]) });
        assert!(Arc::ptr_eq(&before, &rig.view()));
    }

    #[test]
    fn a_new_search_closes_the_open_library() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::ArchiveLibraryOpen(library("net")));
        rig.cmd(CatalogCmd::Search("altro".into()));
        assert!(rig.view().search.open_library.is_none());
    }

    #[test]
    fn opening_a_catalogue_artist_raises_an_event_with_the_new_id_or_the_error() {
        let mut rig = Rig::new();
        assert!(matches!(
            jobs(rig.cmd(CatalogCmd::OpenMetaArtist(77))).as_slice(),
            [Job::MetaArtist { id: 77, .. }]
        ));
        rig.done(Done::MetaArtist(Ok(id(5))));
        rig.done(Done::MetaArtist(Err(network())));
        let events: Vec<_> = rig.view().events.iter().map(|e| (e.seq, e.event.clone())).collect();
        assert_eq!(
            events,
            [(1, CatalogEvent::MetaArtistOpened(id(5))), (2, CatalogEvent::MetaArtistFailed(network()))]
        );
    }

    #[test]
    fn the_event_list_is_bounded_and_numbers_never_repeat() {
        let mut rig = Rig::new();
        for n in 0..(EVENTS as u128 + 10) {
            rig.done(Done::MetaArtist(Ok(id(n))));
        }
        let view = rig.view();
        assert_eq!(view.events.len(), EVENTS);
        assert_eq!(view.events.back().map(|e| e.seq), Some(EVENTS as u64 + 10));
        assert_eq!(view.events.front().map(|e| e.seq), Some(11));
    }

    // ---------- Brani spariti e riletti ----------

    #[test]
    fn a_missing_track_is_healed_and_replaces_the_saved_copies() {
        let mut rig = Rig::new();
        let missing = track(1);
        let mut effects = Vec::new();
        rig.catalog.heal(Arc::new(missing.clone()), &mut effects);
        assert!(matches!(jobs(effects).as_slice(), [Job::Heal { track, .. }] if track.id == missing.id));

        let found = track(9);
        rig.done(Done::Healed { old: missing.id, result: Ok(Box::new(found.clone())) });
        let playback = rig.catalog.take_playback();
        assert!(
            matches!(playback.as_slice(), [PlaybackCmd::ReplaceTrack { old, new, restart: true }] if *old == missing.id && new.id == found.id),
            "{playback:?}"
        );
        let library = rig.catalog.take_library();
        assert!(
            matches!(library.as_slice(), [LibraryCmd::ReplaceTrack { old, new }] if *old == missing.id && new.id == found.id)
        );
        assert!(
            matches!(rig.view().events.back().map(|e| &e.event), Some(CatalogEvent::Replaced(t)) if t.id == found.id)
        );

        // Un brano che non si ritrova: la riproduzione resta in errore.
        rig.done(Done::Healed { old: missing.id, result: Err(AppError::InvalidResponse) });
        assert!(
            matches!(rig.catalog.take_playback().as_slice(), [PlaybackCmd::HealFailed(old)] if *old == missing.id)
        );
        assert!(rig.catalog.take_library().is_empty());
    }

    #[test]
    fn a_track_is_reread_once_per_session_and_its_cover_is_announced() {
        let mut rig = Rig::new();
        let mut effects = Vec::new();
        rig.catalog.track_started(1, Some(id(4)), &mut effects);
        rig.catalog.track_started(1, Some(id(4)), &mut effects);
        assert_eq!(jobs(effects).len(), 1, "the same start twice");
        let mut effects = Vec::new();
        rig.catalog.track_started(2, Some(id(4)), &mut effects);
        assert!(effects.is_empty(), "the same track, played again: not reread");
        rig.catalog.track_started(3, Some(id(5)), &mut effects);
        assert_eq!(jobs(effects).len(), 1);

        let mut fresh = track(4);
        fresh.cover_url = Some("nuova".into());
        rig.done(Done::Refreshed(Ok(Box::new(fresh.clone()))));
        let playback = rig.catalog.take_playback();
        assert!(matches!(playback.as_slice(), [PlaybackCmd::ReplaceTrack { restart: false, .. }]));
        assert!(matches!(rig.catalog.take_library().as_slice(), [LibraryCmd::ReplaceTrack { .. }]));
        assert!(
            matches!(rig.view().events.back().map(|e| &e.event), Some(CatalogEvent::Replaced(t)) if t.cover_url.as_deref() == Some("nuova"))
        );
        // Un errore non fa niente.
        let before = rig.view();
        rig.done(Done::Refreshed(Err(network())));
        assert!(Arc::ptr_eq(&before, &rig.view()));
    }

    #[test]
    fn at_start_the_saved_tracks_without_a_cover_are_reread_up_to_the_limit() {
        use crate::config::CoreConfig;
        let mut rig = Rig::new();
        let mut config = CoreConfig::default();
        let mut with_cover = track(500);
        with_cover.cover_url = Some("c".into());
        config.liked = std::iter::once(with_cover).chain((1..=40).map(track)).collect::<Vec<_>>().into();
        let library = crate::state::library::Library::new(config.library(), 1);
        let mut effects = Vec::new();
        rig.catalog.after_library(&library.view, &mut effects);
        assert!(effects.is_empty(), "not before the start");
        rig.catalog.start();
        rig.catalog.after_library(&library.view, &mut effects);
        let ids: Vec<Uuid> = jobs(effects)
            .into_iter()
            .filter_map(|job| match job {
                Job::Refresh { id, .. } => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), STALE_REFRESH);
        assert!(!ids.contains(&id(500)), "the one with a cover is fine");
        let mut again = Vec::new();
        rig.catalog.after_library(&library.view, &mut again);
        assert!(again.is_empty(), "once");
    }

    #[test]
    fn a_lost_job_never_leaves_a_search_loading() {
        let mut rig = Rig::new();
        let started = jobs(rig.cmd(CatalogCmd::Search("rock".into())));
        for job in &started {
            let (Done::Search { .. } | Done::Discover { .. } | Done::Meta { .. }) = job.lost() else {
                panic!("{job:?}");
            };
        }
        for job in started {
            rig.done(job.lost());
        }
        let view = rig.view();
        assert!(
            matches!(view.search.results, Load::Failed(_)) && matches!(view.search.discover, Load::Failed(_))
        );
        assert!(matches!(view.search.sources, Load::Failed(_)));
    }
}
