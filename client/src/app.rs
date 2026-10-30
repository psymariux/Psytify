//! Stato dell'applicazione e logica Elm: `Message`, `update`, `subscription`.
//!
//! Tutto lo stato vive in `App`. Le chiamate di rete e su disco partono con `Task::perform` e
//! tornano come `Message`; il thread della UI non si blocca mai.

mod artists;
mod chrome;
mod external;
mod library;
mod listening;
mod menu;
mod social;
#[cfg(feature = "tour")]
mod tour;
mod video;

pub use artists::{ArtistsState, Discography};
pub use chrome::ResizeEdge;
pub use external::{ArtistExtras, ExternalState, ImportTarget};
pub use listening::{Feed, HERO_PERIOD, HERO_SWAP, Hero, SleepChoice, SleepTimer};
pub use menu::{ContextMenu, Dialog, MenuTarget, OpenDialog, TrackSource};
pub use social::{AuthField, AuthMode, SignInForm, SocialState};

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::{StreamExt, TryStreamExt};
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::widget::text_input;
use iced::{Element, Point, Size, Subscription, Task, Theme, event, mouse, window};
use uuid::Uuid;

use crate::Platform;
use crate::api::social::{Friends, MyProfile, Profile, PublicPlaylist, Relation, UserPage};
use crate::api::{
    self, Album, AlbumSummary, ApiClient, ArtistPage, DiscoverImport, DiscoverItem, DiscoverLibrary,
    DiscoverPage, DownloadQuality, LibraryPage, Lyrics, Me, MetaArtist, MetaArtistPage, MetaSearch, Page,
    Quality, SearchResults, Sort, SourceItem, SourceKind, SourcePlaylist, SourcePlaylistPage, Track,
    TrackQuery,
};
use crate::auth::{ClerkApp, ClerkUser, CodeStep, Provider, Session, Step};
use crate::config::{self, Config, FollowedArtist, PlayStat, Playlist, SavedAlbum, SavedScreen};
use crate::error::AppError;
use crate::jam::{self, Event, Link, Member, Remote, Sent, Snapshot, Target};
use crate::player::eq::{self, PRESETS};
use crate::player::lyrics::{self as synced, Text};
use crate::player::stream::StreamBuffer;
use crate::player::{Advance, AudioOutput, Playback, PlaybackState, Queue, WarmCache, seek};
use crate::system_media;
use crate::ui;
use crate::ui::cover::{CoverPixels, Covers, decode_square};
use crate::ui::cursor;
use crate::ui::motion::{Curve, Tween};
use crate::ui::pages::search::source_key;
use crate::ui::theme as tokens;
#[cfg(feature = "video")]
use video::OpenedVideo;
#[cfg(feature = "video")]
pub use video::VideoView;

pub const SEARCH_INPUT: &str = "search";
/// Il campo del nome nelle finestre di dialogo (nuova playlist, rinomina).
pub const DIALOG_INPUT: &str = "dialog-name";
/// Attesa dopo l'ultimo tasto prima di lanciare la ricerca.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);
/// Quanto sposta il volume Ctrl+↑ o Ctrl+↓.
const VOLUME_STEP: f32 = 0.05;
/// Spostamento delle frecce sinistra e destra.
pub const SEEK_STEP_SECS: i64 = 5;
/// Schermate ricordate per «Indietro».
const HISTORY: usize = 50;
/// Spostamento di Maiusc+← e Maiusc+→.
const SEEK_LONG_SECS: i64 = 15;
/// Brani salvati senza copertina riletti dal server all'avvio, al massimo.
const STALE_REFRESH: usize = 24;
/// Ogni quanto batte il Tick mentre suona.
const TICK: Duration = Duration::from_millis(250);
/// Ogni quanto, con l'accesso fatto, si chiedono al server richieste d'amicizia e inviti alle jam.
const SOCIAL_POLL: Duration = Duration::from_secs(15);
/// Sotto questa posizione "precedente" cambia brano, sopra riparte da capo.
const RESTART_THRESHOLD: Duration = Duration::from_secs(3);
/// Quanto prima della fine si comincia a scaricare tutto il brano successivo (la testata è già
/// calda da quando parte quello corrente): il passaggio è immediato.
const PREFETCH_BEFORE_END: Duration = Duration::from_secs(25);
/// Quanto deve restare fermo il mouse su un brano perché si prepari la sua testata.
const HOVER_INTENT: Duration = Duration::from_millis(180);
/// Brani della coda preparati in anticipo mentre ne suona uno.
const WARM_AHEAD: usize = 2;
const HOME_SECTION_SIZE: u32 = 12;
/// Brani recenti da cui si ricavano uscite, novità e mix (il massimo che il server dà).
const HOME_RELEASES: u32 = 100;
/// Album recenti mostrati nella Home.
const HOME_ALBUMS: u32 = 24;
const LIBRARY_PAGE: u32 = 50;
/// Album mostrati nella libreria (una sola pagina, dalla A alla Z).
const LIBRARY_ALBUMS: u32 = 100;
const MAX_QUERY_CHARS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Search,
    Library,
    Album(Uuid),
    Playlist(Uuid),
    Artist(Uuid),
    Settings,
    /// Ascolto condiviso con altre persone.
    Jam,
    /// I brani a cui si è messo "mi piace".
    Liked,
    /// Le uscite degli artisti seguiti.
    News,
    /// Una playlist (o un album) di YouTube o Bandcamp, aperta da una ricerca o da un artista.
    SourcePlaylist,
    /// La libreria personale come schermata (telefono): sul desktop sta nella barra laterale.
    Collection,
    /// «Accedi a Psytify»: password, codice via email, GitHub, Google e Discord.
    SignIn,
    /// Gli amici, le richieste e la ricerca di persone.
    Friends,
    /// Il profilo aperto (`SocialState::open`).
    Profile,
    /// La playlist pubblica di un amico (`SocialState::playlist`).
    FriendPlaylist,
}

impl From<SavedScreen> for Screen {
    fn from(saved: SavedScreen) -> Self {
        match saved {
            SavedScreen::Home => Self::Home,
            SavedScreen::Search => Self::Search,
            SavedScreen::Library => Self::Library,
            SavedScreen::Album(id) => Self::Album(id),
            SavedScreen::Playlist(id) => Self::Playlist(id),
            SavedScreen::Artist(id) => Self::Artist(id),
            SavedScreen::Settings => Self::Settings,
            SavedScreen::Jam => Self::Jam,
            SavedScreen::Liked => Self::Liked,
            SavedScreen::News => Self::News,
            SavedScreen::Collection => Self::Collection,
        }
    }
}

impl From<Screen> for SavedScreen {
    fn from(screen: Screen) -> Self {
        match screen {
            Screen::Home => Self::Home,
            // La playlist esterna non si salva: alla riapertura si torna alla ricerca.
            Screen::Search | Screen::SourcePlaylist => Self::Search,
            Screen::Library => Self::Library,
            Screen::Album(id) => Self::Album(id),
            Screen::Playlist(id) => Self::Playlist(id),
            Screen::Artist(id) => Self::Artist(id),
            Screen::Settings => Self::Settings,
            Screen::Jam => Self::Jam,
            Screen::Liked => Self::Liked,
            Screen::News => Self::News,
            Screen::Collection => Self::Collection,
            // Profili e accesso dipendono dalla rete: alla riapertura si parte dalla Home.
            Screen::SignIn | Screen::Friends | Screen::Profile | Screen::FriendPlaylist => Self::Home,
        }
    }
}

/// Stato del login con Clerk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Account {
    /// Clerk non configurato: niente pulsante di accesso.
    Unavailable,
    #[default]
    SignedOut,
    /// Login nel browser o ripristino della sessione in corso.
    Working,
    SignedIn(Me),
}

/// Stato di un dato caricato dalla rete.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Load<T> {
    #[default]
    Idle,
    Loading,
    Ready(T),
    Failed(AppError),
}

impl<T> Load<T> {
    pub fn needs_load(&self) -> bool {
        matches!(self, Self::Idle | Self::Failed(_))
    }

    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeSection {
    Recent,
    Alphabetical,
}

/// Che cosa mostra la libreria nella barra laterale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarFilter {
    #[default]
    All,
    Playlists,
    Artists,
    Albums,
}

impl SidebarFilter {
    /// I chip: «Tutto» è quando nessuno è acceso.
    pub const CHIPS: [Self; 3] = [Self::Playlists, Self::Artists, Self::Albums];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "Tutto",
            Self::Playlists => "Playlist",
            Self::Artists => "Artisti",
            Self::Albums => "Album",
        }
    }
}

/// Le schede del Catalogo (album, artisti, brani) e i filtri dei brani.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryFilter {
    /// Gli album, in griglia con la copertina grande.
    #[default]
    Albums,
    /// Gli artisti, in cerchio.
    Artists,
    /// Tutti i brani.
    All,
    Downloadable,
    Singles,
}

impl LibraryFilter {
    /// La scheda mostra brani (e non album o artisti).
    pub fn is_tracks(self) -> bool {
        !matches!(self, Self::Albums | Self::Artists)
    }

    pub fn matches(self, track: &Track) -> bool {
        match self {
            Self::All | Self::Albums | Self::Artists => true,
            Self::Downloadable => track.downloadable,
            Self::Singles => track.album.is_none(),
        }
    }
}

#[derive(Debug, Default)]
pub struct HomeState {
    pub recent: Load<Arc<[Track]>>,
    pub alphabetical: Load<Arc<[Track]>>,
    pub albums: Load<Arc<[AlbumSummary]>>,
    /// Uscite, novità, mix e artisti ricavati dai brani recenti.
    pub feed: Feed,
    /// L'uscita in evidenza nel carosello.
    pub hero: Hero,
}

/// Debounce della ricerca: ogni tasto incrementa la generazione; solo l'ultima parte davvero.
#[derive(Debug, Default)]
pub struct SearchState {
    pub query: String,
    pub generation: u64,
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
    /// Item di Archive (o delle fonti esterne, con `source_key`) in importazione dopo un play.
    pub importing: Option<String>,
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
    /// mostrato al posto degli album), le altre si aggiungono in fondo. Restituisce le copertine da
    /// scaricare, oppure l'errore di una pagina successiva, da mostrare come avviso: gli album già
    /// caricati restano e "Altri album" si può ripremere.
    pub fn receive(
        &mut self,
        page: u32,
        result: Result<LibraryPage, AppError>,
    ) -> Result<Vec<String>, AppError> {
        self.loading_more = false;
        let loaded = match result {
            Ok(loaded) => loaded,
            Err(err) if page == 1 => {
                self.items = Load::Failed(err);
                return Ok(Vec::new());
            }
            Err(err) => return Err(err),
        };
        self.next_page = loaded.next_page;
        let covers = loaded.items.iter().filter_map(|i| i.cover_url.clone()).collect();
        let mut items: Vec<DiscoverItem> = match &self.items {
            Load::Ready(earlier) if page > 1 => earlier.to_vec(),
            _ => Vec::new(),
        };
        items.extend(loaded.items);
        self.items = Load::Ready(Arc::from(items));
        Ok(covers)
    }
}

impl SearchState {
    /// Registra un nuovo testo e restituisce la generazione da attendere.
    pub fn input(&mut self, query: String) -> u64 {
        self.query = query;
        self.generation += 1;
        self.generation
    }

    pub fn is_current(&self, generation: u64) -> bool {
        generation == self.generation
    }
}

#[derive(Debug, Default)]
pub struct LibraryState {
    pub tracks: Load<Vec<Track>>,
    pub next_offset: Option<i64>,
    pub loading_more: bool,
    pub more_error: Option<AppError>,
    pub filter: LibraryFilter,
    /// Ordine di album e brani: i più recenti o dalla A alla Z.
    pub sort: Sort,
    pub albums: Load<Arc<[AlbumSummary]>>,
    /// Il catalogo ha più album di quelli caricati.
    pub albums_truncated: bool,
}

/// Scheda del pannello laterale: il brano in riproduzione, la coda o il testo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SideTab {
    #[default]
    NowPlaying,
    Queue,
    Lyrics,
}

impl SideTab {
    pub const ALL: [SideTab; 3] = [SideTab::NowPlaying, SideTab::Queue, SideTab::Lyrics];

    /// La posizione della scheda nell'intestazione del pannello: la usa il cursore che scivola.
    pub fn index(self) -> f32 {
        match self {
            SideTab::NowPlaying => 0.0,
            SideTab::Queue => 1.0,
            SideTab::Lyrics => 2.0,
        }
    }
}

/// La scheda aperta nelle impostazioni.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Playback,
    Sound,
    Account,
    About,
}

/// L'elemento sotto il mouse: serve a mostrare i suoi controlli e a preparare il suo audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hovered {
    pub id: Uuid,
    /// È un brano: fermarsi sopra ne prepara la testata.
    pub track: bool,
}

/// L'aggiornamento proposto in Informazioni, e a che punto è.
#[derive(Debug, Clone)]
pub enum UpdateState {
    Offered(api::update::Offer),
    Downloading(String),
    Downloaded(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Info(String),
    Error(String),
}

/// L'avviso mostrato in basso, con il suo movimento: resta finché l'uscita non è finita, anche
/// dopo che `App::notice` è tornato vuoto.
#[derive(Debug, Clone)]
pub struct Toast {
    pub notice: Notice,
    pub motion: Tween,
    generation: u64,
}

/// Ciò che cambia l'aspetto dell'app con un movimento: si confronta prima e dopo ogni messaggio.
#[derive(PartialEq)]
struct Observed {
    screen: Screen,
    panel_open: bool,
    notice: Option<Notice>,
    /// Il brano di cui il pannello o la pagina vogliono il videoclip.
    video: Option<Uuid>,
    menu: bool,
    dialog: bool,
    /// Il brano mostrato al sistema e se suona: quando cambia, il widget del desktop si aggiorna.
    media: Option<(Uuid, bool)>,
    side_tab: SideTab,
}

/// Lo splash d'ingresso: il marchio che salta su, il nome che compare, poi il velo che sfuma.
#[derive(Debug, Clone, Copy)]
pub struct Splash {
    pub since: Instant,
    /// La pagina sotto ha già cominciato a entrare (quando il velo inizia a sfumare).
    revealed: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Navigate(Screen),
    Retry,
    HomeLoaded(HomeSection, Result<Page<Track>, AppError>),
    HomeAlbumsLoaded(Result<Page<AlbumSummary>, AppError>),
    SearchInput(String),
    /// Ctrl, Alt e Maiusc premuti: servono a non scrivere nella ricerca i tasti delle scorciatoie.
    ModifiersChanged(Modifiers),
    SearchDebounced(u64),
    SearchLoaded(u64, Result<SearchResults, AppError>),
    DiscoverLoaded(u64, Result<DiscoverPage, AppError>),
    ArchiveLibraryOpen(DiscoverLibrary),
    ArchiveLibraryMore,
    ArchiveLibraryClose,
    ArchiveLibraryLoaded(String, u32, Result<LibraryPage, AppError>),
    /// Brani, artisti, album e playlist del catalogo ufficiale per la ricerca di questa generazione.
    MetaSearchLoaded(u64, Result<MetaSearch, AppError>),
    /// Apre la pagina di un artista del catalogo (il server gli dà un identificatore).
    OpenMetaArtist(u64),
    MetaArtistOpened(Result<Uuid, AppError>),
    /// Apre una playlist esterna e ne carica i brani.
    OpenSourcePlaylist(SourcePlaylist),
    SourcePlaylistLoaded(SourceKind, Result<SourcePlaylistPage, AppError>),
    /// Salva la playlist esterna aperta nella libreria.
    SaveSourcePlaylist,
    /// Ascolta la playlist esterna aperta da questo brano in poi.
    PlaySourcePlaylist(usize),
    /// Ascolta i brani popolari di un artista dal catalogo, da questo in poi.
    PlayArtistTop(Uuid, usize),
    /// Un brano di una playlist esterna importato (o no).
    SourceImported(ImportTarget, Result<DiscoverImport, AppError>),
    /// Fine della pausa dopo un «troppe richieste».
    ImportResume,
    /// Brani popolari e discografia completa di un artista dal catalogo ufficiale.
    ArtistExtrasLoaded(Uuid, Result<MetaArtistPage, AppError>),
    PlayDiscovered(String),
    PlaySource(SourceKind, String),
    /// Copertina scaricata e decodificata; `None` se non si può mostrare.
    CoverLoaded(String, Option<CoverPixels>),
    /// L'ultima release letta dal backend, se è da installare.
    UpdateChecked(Option<api::update::Offer>),
    UpdateRequested,
    UpdateDownloaded(Result<PathBuf, AppError>),
    Imported(String, Result<DiscoverImport, AppError>),
    /// Un brano salvato che il server non aveva più, ritrovato e importato di nuovo.
    TrackHealed(Uuid, Result<Track, AppError>),
    ArtistLoaded(Uuid, Result<ArtistPage, AppError>),
    /// Una pagina artista caricata in anticipo, o aggiornata in silenzio.
    ArtistPrefetched(Uuid, Result<ArtistPage, AppError>),
    /// Filtra la discografia dell'artista aperto.
    ArtistFilter(Discography),
    /// «Popolari» mostra più brani, o torna a cinque.
    ArtistMorePopular,
    LibraryLoaded {
        append: bool,
        result: Result<Page<Track>, AppError>,
    },
    LibraryMore,
    LibraryFilter(LibraryFilter),
    /// Cambia l'ordine del Catalogo e lo ricarica.
    LibrarySort(Sort),
    LibraryAlbumsLoaded(Result<Page<AlbumSummary>, AppError>),
    /// Apre la libreria su un filtro (per esempio "Vedi tutti gli album" dalla Home).
    OpenLibrary(LibraryFilter),
    AlbumLoaded(Uuid, Result<Album, AppError>),
    PlayTracks(Arc<[Track]>, usize),
    /// Riproduce i brani in ordine casuale, da un brano a caso.
    ShufflePlay(Arc<[Track]>),
    PlayQueueAt(usize),
    AudioLoaded(u64, Result<StreamBuffer, AppError>),
    /// Un brano preparato in anticipo (la testata, o tutto se è il prossimo della coda).
    Warmed(Uuid, Quality, Result<StreamBuffer, AppError>),
    /// Il mouse è entrato o uscito da un elemento; se è un brano e si ferma, se ne prepara la testata.
    Hover(Option<Hovered>),
    /// Il mouse è rimasto sul brano abbastanza a lungo (generazione del passaggio).
    HoverSettled(u64, Uuid),
    TogglePlay,
    Next,
    Previous,
    SeekBy(i64),
    Scrub(f32),
    ScrubReleased,
    Volume(f32),
    VolumeReleased,
    ToggleMute,
    ToggleShuffle,
    CycleRepeat,
    Tick,
    /// Un frame della finestra, solo mentre qualcosa si muove.
    Frame,
    /// Un clic sullo splash d'ingresso: lo chiude subito.
    SkipSplash,
    /// Il tempo di un avviso informativo è scaduto (generazione dell'avviso).
    NoticeExpired(u64),
    /// Apre il pannello sulla scheda, o lo chiude se è già su quella.
    OpenPanel(SideTab),
    ClosePanel,
    EqualizerToggle(bool),
    /// Guadagno di una banda mentre lo slider si muove: si salva al rilascio.
    EqualizerBand(usize, f32),
    EqualizerPreset(usize),
    EqualizerReleased,
    SideTab(SideTab),
    SettingsTab(SettingsTab),
    /// Testo di un brano, con l'identificatore del brano per scartare le risposte in ritardo.
    LyricsLoaded(Uuid, Result<Lyrics, AppError>),
    /// Clic su una riga del testo: la riproduzione va a quel punto.
    LyricSeek(Duration),
    /// Impostazioni: indirizzo del backend digitato, applicato o riportato al predefinito.
    ServerDraft(String),
    ServerApply,
    ServerReset,
    /// Dal player: chiude il testo a tutto schermo e apre la Jam.
    OpenJam,
    /// Schermata Jam: nome in stanza, codice da inserire, creazione, ingresso e uscita.
    JamName(String),
    JamCode(String),
    JamCreate,
    JamJoin,
    JamLeave,
    /// Brani della coda condivisa arrivati dal catalogo, con lo stato del gruppo da allineare.
    JamTracksLoaded(Remote, Result<Vec<Track>, AppError>),
    Enqueue(Track),
    RemoveFromQueue(usize),
    ClearQueue,
    /// Mette o toglie "mi piace".
    ToggleLike(Track),
    /// Cambia il filtro della libreria nella barra laterale.
    SidebarFilter(SidebarFilter),
    /// Segue o smette di seguire un artista.
    ToggleFollow(FollowedArtist),
    /// Salva un album nella libreria o lo toglie.
    ToggleSaveAlbum(SavedAlbum),
    /// Riproduce un album dall'inizio (si chiedono i brani al server).
    PlayAlbum(Uuid),
    AlbumPlayLoaded(Result<Album, AppError>),
    /// Avvia una radio da un brano (voce del menu del brano).
    StartRadio(Box<Track>),
    RadioLoaded(Box<Track>, Result<ArtistPage, AppError>),
    /// Riproduce il mix `n` della Home.
    PlayMix(usize),
    /// Porta il carosello della Home a un'uscita, o alla successiva.
    HeroGo(usize),
    HeroNext,
    /// Imposta o toglie il timer di spegnimento.
    SetSleep(Option<SleepChoice>),
    /// Riproduzione automatica a fine coda.
    SetAutoplay(bool),
    /// I videoclip sulla scheda video dedicata (`true`) o sulla GPU integrata.
    SetDedicatedVideo(bool),
    /// La scheda video dedicata trovata all'avvio, col suo nome.
    GpuDetected(Option<String>),
    AutoplayLoaded(Result<ArtistPage, AppError>),
    /// Copia un testo negli appunti.
    CopyText(String),
    /// Apre o chiude la vista immersiva «In riproduzione».
    ToggleFocus,
    /// Apre le impostazioni su una scheda.
    OpenSettingsTab(SettingsTab),
    /// «Mi piace» al brano che suona (scorciatoia L).
    LikeCurrent,
    /// Sposta il volume di una quantità (scorciatoie Ctrl+↑ e Ctrl+↓).
    VolumeStep(f32),
    /// Il brano suonerà subito dopo quello corrente.
    EnqueueNext(Track),
    /// Apre il menu contestuale dove si trova il mouse.
    OpenMenu(MenuTarget),
    CloseMenu,
    /// L'elenco delle playlist accanto al menu si apre o si chiude.
    MenuPlaylists(bool),
    /// Apre o chiude l'elenco delle qualità di download accanto al menu.
    MenuDownloads(bool),
    /// Apre o chiude l'elenco dei timer di spegnimento nel menu del player (telefono).
    MenuTimers(bool),
    /// Apre il cassetto dell'avatar (telefono).
    OpenDrawer,
    CloseDrawer,
    /// Chiude il cassetto ed esegue il messaggio: la voce scelta.
    DrawerGo(Box<Message>),
    OpenDialog(Dialog),
    DialogInput(String),
    DialogSubmit,
    DialogCancel,
    AddToPlaylist(Uuid, Track),
    RemoveFromPlaylist(Uuid, usize),
    /// Una voce del menu contestuale: lo chiude ed esegue il messaggio.
    Chosen(Box<Message>),
    /// Esc: chiude ciò che sta sopra, dal più in alto.
    Escape,
    /// «Indietro» del sistema (il tasto di Android): come Esc, e se non c'è niente sopra torna
    /// alla pagina precedente.
    SystemBack,
    // Finestra senza cornice di sistema.
    WindowReady(Option<window::Id>),
    WindowResized(Size),
    TitleBarPressed,
    Minimize,
    ToggleFullscreen,
    Quit,
    ResizeStart(ResizeEdge),
    ResizeMove(Point),
    ResizeEnd,
    Download(Track),
    /// Scarica in una qualità scelta al momento (dal menu di un brano).
    DownloadAs(Track, DownloadQuality),
    DownloadAll(Arc<[Track]>),
    DownloadFinished(Uuid, Result<api::Downloaded, AppError>),
    SetDownloadQuality(DownloadQuality),
    DismissNotice,
    FocusSearch,
    Saved(Result<(), AppError>),
    CloseRequested(window::Id),
    Exit(window::Id),
    SetQuality(Quality),
    /// Ricerca YouTube: tenere remix ed edit (`true`) o preferire l'originale.
    /// Apre «Accedi a Psytify».
    SignIn,
    AuthMode(AuthMode),
    AuthInput(AuthField, String),
    AuthSubmit,
    AuthProvider(Provider),
    AuthStepped(Result<Step, AppError>),
    AuthResend,
    AuthResent(Result<CodeStep, AppError>),
    /// Dal codice torna ai campi.
    AuthBack,
    /// `restored`: ripristino silenzioso all'avvio, senza avvisi se la sessione è scaduta.
    SignedIn {
        restored: bool,
        result: Result<(Session, ClerkUser, Me), AppError>,
    },
    SignOut,
    SessionStored(Result<(), AppError>),
    /// Nome utente o nome visibile del proprio profilo, nelle impostazioni.
    ProfileHandle(String),
    ProfileName(String),
    /// Salva il profilo; con `Some` cambia anche la condivisione dell'attività.
    ProfileSave(Option<bool>),
    ProfileSaved(Result<MyProfile, AppError>),
    FriendsLoaded(Result<Friends, AppError>),
    /// Ogni tanto, con l'accesso fatto: amici, richieste e inviti alle jam si aggiornano da soli.
    SocialPoll,
    JamInvitesLoaded(Result<Vec<crate::api::social::JamInvite>, AppError>),
    /// Schermata Jam: sceglie o toglie un amico da invitare, manda gli inviti, entra o rifiuta.
    JamInviteToggle(String),
    JamInviteSend,
    JamInviteSent(Result<usize, AppError>),
    JamInviteAccept(String),
    JamInviteDismiss(String),
    JamInviteDismissed(Result<(), AppError>),
    UserQuery(String),
    UserSearch,
    UsersFound(Result<Vec<Profile>, AppError>),
    OpenProfile(String),
    ProfileLoaded(String, Result<UserPage, AppError>),
    /// Chiede o accetta l'amicizia (`true`), oppure la toglie, la ritira o la rifiuta.
    FriendAction(String, bool),
    FriendChanged(String, Result<Relation, AppError>),
    OpenFriendPlaylist(String, Uuid),
    FriendPlaylistLoaded(Result<Arc<PublicPlaylist>, AppError>),
    /// Rende pubblica o privata una playlist locale.
    PlaylistPublic(Uuid, bool),
    /// Esito di un invio al server che non mostra nulla (brano in ascolto, ascolti, playlist).
    SocialSynced(Result<(), AppError>),
    /// Mostra su Discord cosa si ascolta.
    SetDiscord(bool),
    /// Nel pannello «In riproduzione» il videoclip (`true`) o la copertina (`false`).
    SetPanelVideo(bool),
    /// Riprova il videoclip che non si era aperto.
    RetryVideo,
    /// I dati aggiornati del brano appena partito (videoclip, copertina) letti dal server.
    TrackRefreshed(Result<Track, AppError>),
    /// Apre o chiude il videoclip a tutto schermo, con il testo sopra.
    ToggleVideoFocus,
    /// Mostra o nasconde il testo sopra il videoclip.
    ToggleVideoLyrics,
    /// Il videoclip aperto su un altro thread, pronto da mostrare.
    #[cfg(feature = "video")]
    VideoOpened(Uuid, Result<Option<OpenedVideo>, AppError>),
    /// È ora di chiedere se la versione completa del videoclip è pronta.
    #[cfg(feature = "video")]
    VideoUpgrade(Uuid),
    /// La versione completa del videoclip (`None`: non ancora pronta).
    #[cfg(feature = "video")]
    VideoUpgraded(Uuid, Result<Option<OpenedVideo>, AppError>),
    #[cfg(feature = "video")]
    VideoFailed,
    /// Un comando dai controlli multimediali del sistema (tasti multimediali, widget del desktop).
    SystemMedia(system_media::Event),
    /// Indietro e avanti fra le schermate visitate (Alt+←/→, tasti laterali del mouse).
    GoBack,
    GoForward,
    /// Salta a una frazione del brano (i tasti da 0 a 9).
    SeekFraction(f32),
    /// Apre o chiude il riquadro delle scorciatoie da tastiera.
    ToggleShortcuts,
    /// Toglie una ricerca recente (`None`: tutte).
    ForgetSearch(Option<String>),
    /// Un passo del giro automatico delle viste (solo con la feature `tour`).
    #[cfg(feature = "tour")]
    Tour(tour::TourEvent),
}

/// Una jam in corso, vista da questo client. Il collegamento vive quanto questo valore.
pub struct JamSession {
    link: Link,
    pub code: Option<String>,
    pub you: Option<Uuid>,
    pub members: Vec<Member>,
    /// Ultimo stato scambiato con la stanza: il riferimento per decidere se rimandare.
    last_sent: Option<Sent>,
    /// La stanza ha detto il suo stato, o non ne ha: solo allora si pubblica il proprio.
    synced: bool,
    /// Brani della coda condivisa in arrivo: nel frattempo non si pubblica nulla.
    fetching: bool,
}

/// Il brano del gruppo e la posizione da cui partire, quando il suo audio è pronto.
#[derive(Debug, Clone, Copy)]
struct JamTarget {
    track: Uuid,
    position: Duration,
    playing: bool,
}

pub struct App {
    pub api: Result<ApiClient, AppError>,
    /// Backend in uso (variabile d'ambiente, scelta dell'utente o predefinito).
    api_base_url: String,
    /// Backend scelto nelle impostazioni; `None` = il predefinito della build.
    pub server: Option<String>,
    /// Indirizzo che si sta scrivendo nelle impostazioni.
    pub server_draft: String,
    pub audio: Option<AudioOutput>,
    pub queue: Queue,
    pub playback: Playback,
    /// Buffer dell'ultimo brano (con la sua qualità), per ripeterlo senza riscaricarlo.
    cached_audio: Option<(Uuid, Quality, StreamBuffer)>,
    /// Brani preparati in anticipo: il primo che si ascolterà parte senza aspettare la rete.
    warm: WarmCache,
    /// Brano sotto il mouse e generazione del passaggio, per l'attesa prima di prepararlo.
    hover_generation: u64,
    volume_before_mute: Option<f32>,
    pub screen: Screen,
    pub home: HomeState,
    pub search: SearchState,
    pub library: LibraryState,
    pub album: Option<(Uuid, Load<Album>)>,
    pub artist: Option<(Uuid, Load<ArtistPage>)>,
    /// Pagine artista in memoria, caricamenti in anticipo e vista della discografia.
    pub artists: ArtistsState,
    /// Playlist esterna aperta, ciò che gli artisti hanno fuori dal catalogo, importazioni.
    pub external: ExternalState,
    pub playlists: Vec<Playlist>,
    /// I brani a cui si è messo "mi piace", il più recente in testa.
    pub liked: Vec<Track>,
    /// Gli ultimi brani ascoltati, il più recente in testa.
    pub recent: Vec<Track>,
    /// Artisti seguiti, album salvati e ascolti: da qui nascono «Novità», «I tuoi artisti» e i mix.
    pub followed: Vec<FollowedArtist>,
    pub saved_albums: Vec<SavedAlbum>,
    pub stats: Vec<PlayStat>,
    /// Tempo ascoltato in tutto, in millisecondi.
    pub listened_ms: u64,
    /// Ultima uscita vista in «Novità»: ciò che è più recente è da leggere.
    pub news_seen: String,
    /// A fine coda continua con brani dello stesso artista.
    pub autoplay: bool,
    /// Vedi `Config::dedicated_video`.
    pub dedicated_video: bool,
    /// La scheda video dedicata che può decodificare i videoclip: se non c'è, l'opzione non si mostra.
    pub dedicated_gpu: Option<String>,
    /// Che cosa mostra la libreria nella barra laterale (non si salva).
    pub sidebar_filter: SidebarFilter,
    /// La vista immersiva «In riproduzione» è aperta.
    pub focus: bool,
    /// Il seme dei mix: fisso per la sessione, così non cambiano a ogni disegno.
    taste_seed: u64,
    /// Il timer di spegnimento, se è attivo.
    pub sleep: Option<SleepTimer>,
    /// Il brano in corso è già stato contato negli ascolti.
    play_counted: bool,
    /// Si stanno chiedendo altri brani per la riproduzione automatica.
    autoplay_busy: bool,
    /// Menu contestuale e finestra di dialogo aperti.
    pub menu: Option<ContextMenu>,
    pub dialog: Option<OpenDialog>,
    /// Il cuore appena premuto: l'icona fa un piccolo rimbalzo.
    pub like_pop: Option<(Uuid, Instant)>,
    pub panel_open: bool,
    /// Apertura del pannello laterale, da 0 (chiuso) a 1.
    pub panel_motion: Tween,
    /// Scala della copertina grande: 1 mentre suona, `COVER_REST` in pausa (con la molla).
    pub cover_motion: Tween,
    /// Il cursore delle schede del pannello, misurato in schede (0, 1, 2), con la molla.
    pub side_tab_motion: Tween,
    /// Lo splash d'ingresso, finché è sullo schermo.
    pub splash: Option<Splash>,
    /// Quando il tasto Play ha cambiato stato: l'icona fa un piccolo rimbalzo.
    pub play_pop: Option<Instant>,
    /// Elemento sotto il mouse e istante in cui ci è entrato.
    pub hovered: Option<(Hovered, Instant)>,
    // Finestra
    window: Option<window::Id>,
    pub window_size: Size,
    /// Lo spazio coperto dalle barre del sistema (Android); `None` finché non si conosce.
    pub insets: Option<crate::Insets>,
    pub fullscreen: bool,
    resizing: Option<chrome::ResizeDrag>,
    last_title_press: Option<Instant>,
    /// I modificatori premuti adesso (vedi `Message::ModifiersChanged`).
    modifiers: Modifiers,
    /// Il collegamento con i controlli multimediali del sistema, quando è pronto.
    system_media: Option<system_media::Handle>,
    /// Ultime ricerche, la più recente in testa (si salvano).
    pub search_history: Vec<String>,
    /// Schermate visitate, per «Indietro» e «Avanti».
    back: Vec<Screen>,
    forward: Vec<Screen>,
    /// Il riquadro delle scorciatoie da tastiera, aperto da quando.
    pub shortcuts: Option<Instant>,
    /// Brano il cui videoclip ha fallito: non si riprova finché non cambia.
    video_blocked: Option<Uuid>,
    /// Brani di cui si è già chiesto al server di preparare il videoclip.
    videos_prepared: HashSet<Uuid>,
    /// Il pannello «In riproduzione» mostra il videoclip, se il brano ne ha uno (si salva).
    pub panel_video: bool,
    /// Brani già riletti dal server in questa sessione: le copie salvate possono essere vecchie.
    refreshed: HashSet<Uuid>,
    /// La posizione del testo a tutto schermo, in righe: scorre verso la riga in corso.
    pub lyrics_scroll: Tween,
    /// La vista a tutto schermo mostra il videoclip invece del testo.
    pub focus_video: bool,
    /// Testo sopra il videoclip.
    pub video_lyrics: bool,
    /// Brani salvati che il server non conosce più, in corso di ricerca nel catalogo.
    healing: HashSet<Uuid>,
    /// Ingresso della pagina corrente, da 0 a 1.
    pub page_motion: Tween,
    pub equalizer: eq::Settings,
    /// Scheda aperta nel pannello laterale (non si salva: all'avvio si apre la coda).
    pub side_tab: SideTab,
    /// Scheda aperta nelle impostazioni (non si salva).
    pub settings_tab: SettingsTab,
    /// Testo del brano `lyrics_track`, con la riga in corso per la vista sincronizzata.
    pub lyrics: Load<Arc<Text>>,
    pub lyrics_track: Option<Uuid>,
    pub lyrics_line: Option<usize>,
    /// Jam in corso, se c'è.
    pub jam: Option<JamSession>,
    /// Nome con cui si entra nella jam, e codice digitato per entrare in una esistente.
    pub jam_name: String,
    pub jam_code: String,
    /// Brano del gruppo da raggiungere: quando il suo audio parte, si salta alla posizione condivisa.
    jam_target: Option<JamTarget>,
    pub downloading: HashSet<Uuid>,
    /// «Scarica tutto» in corso: i brani vanno giù due alla volta e arriva un solo avviso alla fine.
    pub download_batch: Option<DownloadBatch>,
    /// Il cassetto del telefono è aperto dall'istante indicato.
    pub drawer: Option<Instant>,
    /// Copertine scaricate, per URL.
    pub covers: Covers,
    pub notice: Option<Notice>,
    pub toast: Option<Toast>,
    /// L'aggiornamento offerto in Informazioni, e a che punto è il download.
    pub update: Option<UpdateState>,
    toast_generation: u64,
    saving: bool,
    save_pending: bool,
    pub quality: Quality,
    /// Vedi `Config::download_quality`.
    pub download_quality: DownloadQuality,
    pub account: Account,
    pub social: SocialState,
    /// Il collegamento a Discord, se acceso e con un Application ID.
    #[cfg(not(target_os = "android"))]
    discord: Option<crate::discord::Presence>,
    pub discord_presence: bool,
    discord_app_id: String,
    clerk_app: Option<ClerkApp>,
    clerk_issuer: String,
    clerk_client_id: String,
    oauth_port: u16,
    #[cfg(feature = "video")]
    pub video: Option<VideoView>,
    #[cfg(feature = "tour")]
    tour: Option<tour::Tour>,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let config = config::load();
        let api_base_url = config::api_url(config.server.as_deref());
        let api = ApiClient::new(&api_base_url);
        let clerk_app = config.clerk_app();
        let (audio, audio_error) = match AudioOutput::open() {
            Ok(output) => {
                output.set_equalizer(config.equalizer);
                (Some(output), None)
            }
            Err(err) => (None, Some(err)),
        };
        let mut app = Self {
            notice: None,
            toast: None,
            update: None,
            toast_generation: 0,
            api,
            server_draft: config.server.clone().unwrap_or_default(),
            server: config.server,
            api_base_url,
            audio,
            queue: config.queue,
            playback: Playback::new(config.volume),
            cached_audio: None,
            warm: WarmCache::default(),
            hovered: None,
            hover_generation: 0,
            window: None,
            window_size: Size::new(1280.0, 800.0),
            insets: None,
            fullscreen: false,
            resizing: None,
            last_title_press: None,
            modifiers: Modifiers::default(),
            system_media: None,
            // Le ricerche senza lettere né cifre (un «,» scappato con Ctrl+,) non tornano.
            search_history: config
                .search_history
                .iter()
                .filter(|q| config::worth_remembering(q))
                .cloned()
                .collect(),
            back: Vec::new(),
            forward: Vec::new(),
            shortcuts: None,
            video_blocked: None,
            videos_prepared: HashSet::new(),
            panel_video: config.panel_video,
            refreshed: HashSet::new(),
            lyrics_scroll: Tween::rest(-1.0),
            focus_video: false,
            video_lyrics: true,
            healing: HashSet::new(),
            liked: config.liked,
            recent: config.recent,
            followed: config.followed,
            saved_albums: config.saved_albums,
            stats: config.stats,
            listened_ms: config.listened_ms,
            news_seen: config.news_seen,
            autoplay: config.autoplay,
            dedicated_video: config.dedicated_video,
            dedicated_gpu: None,
            focus: false,
            sidebar_filter: SidebarFilter::default(),
            taste_seed: seed(),
            sleep: None,
            play_counted: false,
            autoplay_busy: false,
            menu: None,
            dialog: None,
            like_pop: None,
            volume_before_mute: None,
            screen: Screen::Home,
            home: HomeState::default(),
            search: SearchState::default(),
            library: LibraryState::default(),
            album: None,
            artist: None,
            artists: ArtistsState::default(),
            external: ExternalState::default(),
            playlists: config.playlists,
            panel_open: config.panel_open,
            panel_motion: Tween::rest(if config.panel_open { 1.0 } else { 0.0 }),
            page_motion: Tween::rest(1.0),
            cover_motion: Tween::rest(tokens::COVER_REST).with_curve(Curve::Spring),
            side_tab_motion: Tween::rest(SideTab::default().index()).with_curve(Curve::Spring),
            splash: Some(Splash { since: Instant::now(), revealed: false }),
            play_pop: None,
            equalizer: config.equalizer,
            side_tab: SideTab::default(),
            settings_tab: SettingsTab::default(),
            lyrics: Load::Idle,
            lyrics_track: None,
            lyrics_line: None,
            jam: None,
            jam_name: String::new(),
            jam_code: String::new(),
            jam_target: None,
            downloading: HashSet::new(),
            download_batch: None,
            drawer: None,
            covers: Covers::default(),
            saving: false,
            save_pending: false,
            quality: config.quality,
            download_quality: config.download_quality,
            account: if clerk_app.is_some() { Account::SignedOut } else { Account::Unavailable },
            social: SocialState::default(),
            #[cfg(not(target_os = "android"))]
            discord: None,
            discord_presence: config.discord_presence,
            discord_app_id: config.discord_app_id,
            clerk_app,
            clerk_issuer: config.clerk_issuer,
            clerk_client_id: config.clerk_client_id,
            oauth_port: config.oauth_port,
            #[cfg(feature = "video")]
            video: None,
            #[cfg(feature = "tour")]
            tour: None,
        };
        let mut screen = Screen::from(config.last_screen);
        if let Screen::Playlist(id) = screen
            && !app.playlists.iter().any(|p| p.id == id)
        {
            screen = Screen::Home;
        }
        app.search.query = config.last_query;
        let home = app.load_home();
        let restore = app.open(screen);
        let search = if screen == Screen::Search { app.run_search() } else { Task::none() };
        let session = app.restore_session();
        app.start_discord();
        // Le playlist salvate con brani ancora da importare riprendono da dove erano.
        let imports = app.next_import();
        // La coda ripresa dall'ultima sessione: le sue copertine servono subito.
        let urls: Vec<String> = app.queue.tracks_in_order().into_iter().filter_map(|t| t.cover_url).collect();
        let covers = app.fetch_covers(urls);
        // "Continua ad ascoltare" è a un clic: il brano ripreso ha già la testata pronta.
        let resume = app.queue.current().map(|t| t.id).map_or_else(Task::none, |id| app.warm_track(id));
        // Un errore all'avvio (backend non valido, nessuna uscita audio) si mostra come avviso.
        let startup = app.api.as_ref().err().or(audio_error.as_ref()).map(|e| Notice::Error(e.to_string()));
        let notice = app.show_notice(startup);
        #[cfg(feature = "tour")]
        let tour = match tour::Tour::from_env() {
            Some((tour, start)) => {
                app.tour = Some(tour);
                start
            }
            None => Task::none(),
        };
        #[cfg(not(feature = "tour"))]
        let tour = Task::none();
        // Il pannello riaperto sul videoclip del brano ripreso: lo si apre subito, altrimenti resta
        // su «Preparo il videoclip…» finché qualcosa non cambia.
        let photos = Task::batch([app.load_artist_photos(), app.refresh_stale_saved(), app.refresh_video()]);
        // La scheda video dedicata si cerca in sottofondo: la prima lettura del registro di
        // GStreamer prende qualche decimo di secondo.
        let dedicated = app.dedicated_video;
        let gpu = Task::perform(
            async move {
                tokio::task::spawn_blocking(move || video::detect_dedicated_gpu(dedicated))
                    .await
                    .ok()
                    .flatten()
            },
            Message::GpuDetected,
        );
        let update = app.check_update();
        (
            app,
            Task::batch([
                tour,
                photos,
                gpu,
                home,
                restore,
                search,
                session,
                imports,
                covers,
                resume,
                notice,
                update,
                window::get_oldest().map(Message::WindowReady),
            ]),
        )
    }

    /// Chiede una volta, all'avvio, se c'è una release più recente di questa copia. Senza rete, o
    /// con una compilazione locale, non dice nulla.
    fn check_update(&self) -> Task<Message> {
        if api::update::BUILD_TAG.is_none() {
            return Task::none();
        }
        let Ok(api) = self.api.clone() else { return Task::none() };
        Task::perform(
            async move {
                api.latest_release()
                    .await
                    .ok()
                    .and_then(|release| api::update::offer(release, api::update::BUILD_TAG))
            },
            Message::UpdateChecked,
        )
    }

    pub fn title(&self) -> String {
        match self.queue.current() {
            Some(track) if self.playback.is_active() => {
                format!("{} · {} — Psytify", track.title, track.artist.name)
            }
            _ => "Psytify".into(),
        }
    }

    pub fn theme(&self) -> Theme {
        ui::theme::theme()
    }

    pub fn view(&self) -> Element<'_, Message> {
        ui::view(self)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            keyboard::on_key_press(shortcut),
            event::listen_with(global_event),
            system_media::subscription().map(Message::SystemMedia),
            window::close_requests().map(Message::CloseRequested),
            window::resize_events().map(|(_, size)| Message::WindowResized(size)),
            // Ricorda dov'è il mouse (per il menu contestuale) senza produrre messaggi.
            event::listen_with(cursor::track),
        ];
        if self.resizing.is_some() {
            subscriptions.push(event::listen_with(chrome::resize_events));
        }
        // Ogni messaggio ridisegna la finestra (~1,4 ms): i frame pieni, uno per vsync, servono solo
        // alle animazioni. La barra di avanzamento si aggiorna col Tick (4 al secondo, sotto il pixel).
        if self.is_animating() {
            subscriptions.push(window::frames().map(|_| Message::Frame));
        } else if self.is_scrolling_lyrics() {
            // Il testo che scorre è il movimento più frequente (a ogni verso): basta la metà dei frame.
            subscriptions.push(iced::time::every(tokens::AMBIENT_FRAME).map(|_| Message::Frame));
        }
        if self.needs_tick() {
            subscriptions.push(iced::time::every(TICK).map(|_| Message::Tick));
        }
        if self.polls_social() {
            subscriptions.push(iced::time::every(SOCIAL_POLL).map(|_| Message::SocialPoll));
        }
        if self.hero_rotates() {
            subscriptions.push(iced::time::every(HERO_PERIOD).map(|_| Message::HeroNext));
        }
        Subscription::batch(subscriptions)
    }

    /// Il Tick serve mentre suona un brano e in una jam, anche a player fermo: è lui che legge gli
    /// eventi della stanza (ingresso, membri, stato degli altri) e manda il proprio stato.
    fn needs_tick(&self) -> bool {
        self.playback.is_playing() || self.jam.is_some()
    }

    pub fn api_host(&self) -> String {
        self.api.as_ref().map(ApiClient::host).unwrap_or_default()
    }

    pub fn playlist(&self, id: Uuid) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == id)
    }

    pub fn is_current(&self, track_id: Uuid) -> bool {
        self.queue.current().is_some_and(|t| t.id == track_id)
    }

    /// Posizione mostrata: durante il trascinamento segue lo slider.
    pub fn shown_position(&self) -> Duration {
        match (self.playback.scrubbing, self.queue.current()) {
            (Some(fraction), Some(track)) => seek::seek_fraction(fraction, track.duration()),
            _ => self.playback.position,
        }
    }

    /// Gestisce il messaggio, poi avvia i movimenti di ciò che è cambiato: pagina, pannello,
    /// avviso. Così nessun punto che cambia schermata o avviso deve ricordarsi dell'animazione.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let before = self.observed();
        let mut task = self.handle(message);
        if let Some(sync) = self.social_sync() {
            task = Task::batch([task, sync]);
        }
        let after = self.observed();
        if before == after {
            return task;
        }
        let keyboard = Task::none();
        if before.screen != after.screen {
            self.page_motion = Tween::new(0.0, 1.0, tokens::MOTION_BASE);
            // Sul telefono la tastiera non si apre da sola entrando in Cerca: coprirebbe la pagina
            // prima che si veda cosa c'è. Si apre toccando il campo; uscendo da Cerca se ne va.
            if is_phone() && before.screen == Screen::Search && after.screen != Screen::Search {
                crate::soft_keyboard(false);
            }
        }
        if before.panel_open != after.panel_open {
            let (to, duration) =
                if after.panel_open { (1.0, tokens::MOTION_PANEL) } else { (0.0, tokens::MOTION_FAST) };
            self.panel_motion.retarget(to, duration);
        }
        if before.media != after.media {
            self.publish_media();
            // La copertina grande respira con il Play: scende in pausa, torna con la molla.
            let playing = after.media.is_some_and(|(_, playing)| playing);
            let scale = if playing { 1.0 } else { tokens::COVER_REST };
            self.cover_motion.retarget(scale, tokens::MOTION_SPRING);
            let was_playing = before.media.is_some_and(|(_, playing)| playing);
            if was_playing != playing {
                self.play_pop = Some(Instant::now());
            }
        }
        if before.side_tab != after.side_tab {
            self.side_tab_motion.retarget(after.side_tab.index(), tokens::MOTION_SPRING);
        }
        let video = if before.video == after.video { Task::none() } else { self.refresh_video() };
        let notice =
            if before.notice == after.notice { Task::none() } else { self.show_notice(after.notice) };
        Task::batch([task, notice, video, keyboard])
    }

    fn observed(&self) -> Observed {
        Observed {
            screen: self.screen,
            panel_open: self.panel_open,
            notice: self.notice.clone(),
            video: self.wanted_video(),
            menu: self.menu.is_some(),
            dialog: self.dialog.is_some(),
            media: self
                .queue
                .current()
                .filter(|_| self.playback.is_active())
                .map(|track| (track.id, self.playback.is_playing())),
            side_tab: self.side_tab,
        }
    }

    /// Fa entrare il nuovo avviso, o uscire quello mostrato se non ce n'è più. Un avviso
    /// informativo ha un tempo: allo scadere arriva `NoticeExpired`.
    fn show_notice(&mut self, notice: Option<Notice>) -> Task<Message> {
        self.notice = notice.clone();
        let Some(notice) = notice else {
            if let Some(toast) = &mut self.toast {
                toast.motion.retarget(0.0, tokens::MOTION_FAST);
            }
            return Task::none();
        };
        self.toast_generation += 1;
        let generation = self.toast_generation;
        // Un avviso che ne sostituisce un altro ancora visibile non rientra da zero.
        let from = self.toast.as_ref().map_or(0.0, |toast| toast.motion.value());
        let info = matches!(notice, Notice::Info(_));
        self.toast = Some(Toast { notice, motion: Tween::new(from, 1.0, tokens::MOTION_BASE), generation });
        if !info {
            return Task::none();
        }
        Task::perform(tokio::time::sleep(tokens::TOAST_INFO_TIME), move |()| {
            Message::NoticeExpired(generation)
        })
    }

    /// Qualcosa si sta muovendo e servono frame: un passaggio, un avviso che esce, una copertina
    /// che compare, uno scheletro che pulsa.
    fn is_animating(&self) -> bool {
        self.page_motion.is_running()
            // Le righe di una lista entrano una dopo l'altra: l'ultima finisce dopo la pagina.
            || self.page_motion.elapsed() < tokens::PAGE_ENTRY
            || self.panel_motion.is_running()
            || self.toast.as_ref().is_some_and(|toast| toast.motion.is_running() || self.notice.is_none())
            || self.covers.is_fading()
            || self.cover_motion.is_running()
            || (self.panel_open && self.side_tab_motion.elapsed() < tokens::MOTION_SPRING)
            || self.splash.is_some()
            || self.hero_swapping()
            || self.shows_skeleton()
            || self.hovered.is_some_and(|(_, since)| since.elapsed() < tokens::MOTION_BASE)
            || self.like_pop.is_some_and(|(_, since)| since.elapsed() < tokens::MOTION_PANEL)
            || self.play_pop.is_some_and(|since| since.elapsed() < tokens::MOTION_PANEL)
            || self.menu.as_ref().is_some_and(|menu| menu.since.elapsed() < tokens::MOTION_BASE)
            || self.drawer.is_some_and(|since| since.elapsed() < tokens::MOTION_PANEL)
            || self.dialog.as_ref().is_some_and(|open| open.since.elapsed() < tokens::MOTION_PANEL)
            || self.shortcuts.is_some_and(|since| since.elapsed() < tokens::MOTION_PANEL)
    }

    /// Il testo visibile (nel pannello o a tutto schermo) sta scorrendo verso il verso nuovo.
    fn is_scrolling_lyrics(&self) -> bool {
        (self.focus || self.panel_lyrics_shown()) && self.lyrics_scroll.is_running()
    }

    /// Il testo sincronizzato è visibile nel pannello laterale.
    fn panel_lyrics_shown(&self) -> bool {
        self.panel_open && self.side_tab == SideTab::Lyrics
    }

    /// La pagina corrente sta aspettando dati e mostra gli scheletri.
    fn shows_skeleton(&self) -> bool {
        match self.screen {
            Screen::Home => {
                matches!(self.home.recent, Load::Loading)
                    || matches!(self.home.alphabetical, Load::Loading)
                    || matches!(self.home.albums, Load::Loading)
            }
            Screen::Library => {
                matches!(self.library.tracks, Load::Loading) || matches!(self.library.albums, Load::Loading)
            }
            Screen::Album(id) => matches!(&self.album, Some((open, Load::Loading)) if *open == id),
            Screen::Artist(id) => matches!(&self.artist, Some((open, Load::Loading)) if *open == id),
            _ => false,
        }
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(screen) => {
                // Sul telefono, Cerca toccata di nuovo riapre la tastiera chiusa con Indietro.
                if is_phone() && screen == Screen::Search && self.screen == Screen::Search {
                    crate::soft_keyboard(true);
                }
                let task = self.open(screen);
                Task::batch([task, self.persist()])
            }
            Message::ForgetSearch(query) => {
                match query {
                    Some(query) => self.search_history.retain(|old| *old != query),
                    None => self.search_history.clear(),
                }
                self.persist()
            }
            Message::SystemMedia(event) => self.system_media_event(event),
            Message::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
                Task::none()
            }
            Message::GoBack => self.go_back(),
            Message::GoForward => self.go_forward(),
            Message::SeekFraction(fraction) => {
                if self.playback.is_active()
                    && let Some(duration) = self.queue.current().map(Track::duration)
                {
                    self.seek_to(duration.mul_f32(fraction.clamp(0.0, 1.0)));
                }
                Task::none()
            }
            Message::ToggleShortcuts => {
                self.shortcuts = match self.shortcuts {
                    Some(_) => None,
                    None => Some(Instant::now()),
                };
                Task::none()
            }
            Message::Retry => self.reload(),
            Message::HomeLoaded(section, result) => {
                let value = into_load(result.map(|page| Arc::from(page.items)));
                let covers = self.fetch_covers(track_covers(value.ready().map(|t: &Arc<[Track]>| &t[..])));
                match section {
                    HomeSection::Recent => {
                        self.home.recent = value;
                        self.rebuild_feed();
                        self.hero_go(0);
                        let fresh: Vec<Track> =
                            self.home.recent.ready().map(|t| t.to_vec()).unwrap_or_default();
                        let fixed = self.refresh_saved_covers(&fresh);
                        return Task::batch([covers, fixed, self.prefetch_for_home()]);
                    }
                    HomeSection::Alphabetical => self.home.alphabetical = value,
                }
                covers
            }
            Message::HomeAlbumsLoaded(result) => {
                self.home.albums = into_load(result.map(|page| Arc::from(page.items)));
                let urls = album_covers(self.home.albums.ready().map(|a| &a[..]));
                self.fetch_covers(urls)
            }
            Message::SearchInput(query) => {
                // Iced scrive il carattere anche con Ctrl premuto: Ctrl+, o Ctrl+1 nella ricerca
                // aggiungerebbero «,» o «1» invece di aprire la schermata.
                if self.modifiers.command()
                    && inserted_char(&self.search.query, &query)
                        .is_some_and(|c| c == ',' || c.is_ascii_digit())
                {
                    return Task::none();
                }
                let generation = self.search.input(query);
                if clean_query(&self.search.query).is_none() {
                    self.search.results = Load::Idle;
                    self.search.submitted.clear();
                    return Task::none();
                }
                let navigate =
                    if self.screen == Screen::Search { Task::none() } else { self.open(Screen::Search) };
                let wait = Task::perform(tokio::time::sleep(SEARCH_DEBOUNCE), move |()| {
                    Message::SearchDebounced(generation)
                });
                Task::batch([navigate, wait])
            }
            Message::SearchDebounced(generation) => {
                if self.search.is_current(generation) {
                    self.run_search()
                } else {
                    Task::none()
                }
            }
            Message::SearchLoaded(generation, result) => {
                if self.search.is_current(generation) {
                    self.search.results = into_load(result.map(Arc::new));
                    if let Some(results) = self.search.results.ready() {
                        let mut urls = track_covers(Some(&results.tracks));
                        urls.extend(album_covers(Some(&results.albums)));
                        urls.extend(results.artists.iter().filter_map(|a| a.cover_url.clone()));
                        let ids: Vec<Uuid> = results.artists.iter().take(4).map(|a| a.id).collect();
                        let fresh = results.tracks.clone();
                        return Task::batch([
                            self.fetch_covers(urls),
                            self.prefetch_artists(ids),
                            self.refresh_saved_covers(&fresh),
                        ]);
                    }
                }
                Task::none()
            }
            Message::DiscoverLoaded(generation, result) => {
                if !self.search.is_current(generation) {
                    return Task::none();
                }
                let (items, libraries) = match result {
                    Ok(page) => (Ok(page.items), page.libraries),
                    Err(err) => (Err(err), Vec::new()),
                };
                self.search.discover = into_load(items.map(Arc::from));
                let mut urls: Vec<String> = libraries.iter().filter_map(|l| l.cover_url.clone()).collect();
                if let Some(items) = self.search.discover.ready() {
                    urls.extend(items.iter().filter_map(|i| i.cover_url.clone()));
                }
                self.search.libraries = Arc::from(libraries);
                self.fetch_covers(urls)
            }
            Message::ArchiveLibraryOpen(library) => self.open_archive_library(library),
            Message::ArchiveLibraryMore => self.more_archive_library(),
            Message::ArchiveLibraryClose => {
                self.search.open_library = None;
                Task::none()
            }
            Message::ArchiveLibraryLoaded(identifier, page, result) => {
                self.archive_library_loaded(&identifier, page, result)
            }
            Message::MetaSearchLoaded(generation, result) => {
                if self.search.is_current(generation) {
                    match result {
                        Ok(found) => {
                            self.search.artists = Arc::from(found.artists);
                            self.search.sources = Load::Ready(Arc::from(found.tracks));
                            self.search.playlists = Load::Ready(Arc::from(found.lists));
                            let covers = self.search_covers();
                            return self.fetch_covers(covers);
                        }
                        Err(err) => {
                            self.search.artists = Arc::from([]);
                            self.search.sources = Load::Failed(err.clone());
                            self.search.playlists = Load::Idle;
                        }
                    }
                }
                Task::none()
            }
            Message::OpenMetaArtist(id) => self.open_meta_artist(id),
            Message::MetaArtistOpened(result) => match result {
                Ok(id) => self.open(Screen::Artist(id)),
                Err(err) => {
                    self.notice = Some(Notice::Error(err.to_string()));
                    Task::none()
                }
            },
            Message::OpenSourcePlaylist(playlist) => self.open_source_playlist(playlist),
            Message::SourcePlaylistLoaded(source, result) => self.source_playlist_loaded(source, result),
            Message::SaveSourcePlaylist => self.save_source_playlist(),
            Message::PlaySourcePlaylist(start) => self.play_source_playlist(start),
            Message::PlayArtistTop(artist, start) => self.play_artist_top(artist, start),
            Message::SourceImported(target, result) => self.source_imported(target, result),
            Message::ImportResume => self.import_resume(),
            Message::ArtistExtrasLoaded(id, result) => self.artist_extras_loaded(id, result),
            Message::PlayDiscovered(identifier) => self.play_discovered(identifier),
            Message::PlaySource(source, id) => self.play_source(source, id),
            Message::CoverLoaded(url, pixels) => {
                // La copertina del brano in corso arriva dopo il brano: il sistema la riceve ora.
                let current =
                    self.queue.current().is_some_and(|t| t.cover_url.as_deref() == Some(url.as_str()));
                self.covers.loaded(url, pixels);
                if current {
                    self.publish_media();
                }
                Task::none()
            }
            Message::Imported(identifier, result) => {
                if self.search.importing.as_deref() == Some(identifier.as_str()) {
                    self.search.importing = None;
                }
                match result {
                    Ok(imported) if !imported.tracks.is_empty() => {
                        let covers = self.fetch_covers(track_covers(Some(&imported.tracks)));
                        self.queue.replace(imported.tracks, 0, seed());
                        // Il catalogo è cambiato: Home e Libreria si ricaricano alla prossima visita.
                        self.home = HomeState::default();
                        self.library.tracks = Load::Idle;
                        self.library.albums = Load::Idle;
                        Task::batch([self.start_current(), self.persist(), covers])
                    }
                    Ok(_) => {
                        self.notice =
                            Some(Notice::Error("Questo contenuto non ha brani da ascoltare.".into()));
                        Task::none()
                    }
                    Err(err) => {
                        self.notice = Some(Notice::Error(err.to_string()));
                        Task::none()
                    }
                }
            }
            Message::ArtistLoaded(id, result) => self.artist_loaded(id, result),
            Message::ArtistPrefetched(id, result) => self.artist_prefetched(id, result),
            Message::ArtistFilter(filter) => {
                self.artists.filter = filter;
                Task::none()
            }
            Message::ArtistMorePopular => {
                self.artists.more_popular = !self.artists.more_popular;
                Task::none()
            }
            Message::LibraryLoaded { append, result } => {
                self.library.loading_more = false;
                match result {
                    Ok(page) => {
                        self.library.next_offset = page.next_offset;
                        self.library.more_error = None;
                        let covers = self.fetch_covers(track_covers(Some(&page.items)));
                        match &mut self.library.tracks {
                            Load::Ready(tracks) if append => tracks.extend(page.items),
                            other => *other = Load::Ready(page.items),
                        }
                        return covers;
                    }
                    Err(err) if append => self.library.more_error = Some(err),
                    Err(err) => self.library.tracks = Load::Failed(err),
                }
                Task::none()
            }
            Message::LibraryMore => match (self.library.next_offset, self.library.loading_more) {
                (Some(offset), false) => {
                    self.library.loading_more = true;
                    self.load_library(u32::try_from(offset).unwrap_or(0), true)
                }
                _ => Task::none(),
            },
            Message::LibraryFilter(filter) => {
                self.library.filter = filter;
                Task::none()
            }
            Message::LibrarySort(sort) => {
                if self.library.sort == sort {
                    return Task::none();
                }
                self.library.sort = sort;
                Task::batch([self.load_library(0, false), self.load_library_albums()])
            }
            Message::OpenLibrary(filter) => {
                self.library.filter = filter;
                let open = self.open(Screen::Library);
                Task::batch([open, self.persist()])
            }
            Message::LibraryAlbumsLoaded(result) => {
                self.library.albums_truncated = result.as_ref().is_ok_and(|page| page.next_offset.is_some());
                self.library.albums = into_load(result.map(|page| Arc::from(page.items)));
                let urls = album_covers(self.library.albums.ready().map(|a| &a[..]));
                self.fetch_covers(urls)
            }
            Message::AlbumLoaded(id, result) => {
                let mut urls = Vec::new();
                let mut first = None;
                let mut fresh = None;
                if let Some((current, load)) = &mut self.album
                    && *current == id
                {
                    *load = into_load(result);
                    if let Some(album) = load.ready() {
                        urls = track_covers(Some(&album.tracks));
                        urls.extend(album.cover_url.clone());
                        first = album.tracks.first().map(|t| t.id);
                        fresh = Some((album.id, album.cover_url.clone(), album.tracks.clone()));
                    }
                }
                // "Riproduci" su un album parte quasi sempre dal primo brano: la sua testata è pronta.
                let warm = first.map_or_else(Task::none, |id| self.warm_track(id));
                let fixed = match fresh {
                    Some((album_id, cover, tracks)) => {
                        if let Some(saved) = self.saved_albums.iter_mut().find(|a| a.id == album_id) {
                            saved.cover_url = cover;
                        }
                        self.refresh_saved_covers(&tracks)
                    }
                    None => Task::none(),
                };
                Task::batch([self.fetch_covers(urls), warm, fixed])
            }
            Message::PlayTracks(tracks, index) => {
                self.queue.replace(tracks.to_vec(), index, seed());
                Task::batch([self.start_current(), self.persist()])
            }
            Message::ShufflePlay(tracks) => {
                if tracks.is_empty() {
                    return Task::none();
                }
                let seed = seed();
                let start = usize::try_from(seed % tracks.len() as u64).unwrap_or(0);
                self.queue.set_shuffle(true, seed);
                self.queue.replace(tracks.to_vec(), start, seed);
                Task::batch([self.start_current(), self.persist()])
            }
            Message::PlayQueueAt(position) => {
                self.queue.jump(position);
                Task::batch([self.start_current(), self.persist()])
            }
            Message::Warmed(track_id, quality, result) => {
                // Se non riesce, il brano si apre normalmente quando parte.
                self.warm.finish(track_id, quality, result.ok());
                Task::none()
            }
            Message::Hover(hovered) => {
                self.hovered = hovered.map(|hovered| (hovered, Instant::now()));
                self.hover_generation += 1;
                let (generation, Some(Hovered { id, track: true })) = (self.hover_generation, hovered) else {
                    return Task::none();
                };
                Task::perform(tokio::time::sleep(HOVER_INTENT), move |()| {
                    Message::HoverSettled(generation, id)
                })
            }
            Message::HoverSettled(generation, id) => {
                let still_there = self.hovered.is_some_and(|(hovered, _)| hovered.id == id);
                if generation == self.hover_generation && still_there {
                    self.warm_track(id)
                } else {
                    Task::none()
                }
            }
            Message::AudioLoaded(generation, result) => {
                if !self.playback.is_current(generation) {
                    return Task::none();
                }
                match result.and_then(|bytes| self.start_audio(bytes)) {
                    Ok(()) => {
                        self.playback.state = PlaybackState::Playing;
                        self.playback.position = Duration::ZERO;
                        self.play_counted = false;
                        self.sync_video(Some(Duration::ZERO));
                        self.apply_jam_target();
                        if let Some(track) = self.queue.current() {
                            config::remember_listen(&mut self.recent, track);
                        }
                        return Task::batch([
                            self.warm_ahead(),
                            self.prepare_videos(),
                            self.load_artist_photos(),
                            self.persist(),
                            self.refresh_current(),
                        ]);
                    }
                    Err(err) if err.is_not_found() => {
                        // Il server non ha più questo brano: lo si ritrova nel catalogo ufficiale.
                        if let Some(healing) = self.heal_current() {
                            return healing;
                        }
                        self.playback.state = PlaybackState::Error(err.to_string());
                    }
                    Err(err) => self.playback.state = PlaybackState::Error(err.to_string()),
                }
                Task::none()
            }
            Message::TrackHealed(old, result) => self.track_healed(old, result),
            Message::TogglePlay => self.toggle_play(),
            Message::Next => self.skip(),
            Message::Previous => {
                if self.playback.is_active() && self.audio_position() > RESTART_THRESHOLD {
                    self.seek_to(Duration::ZERO);
                    return Task::none();
                }
                if self.queue.previous().is_some() { self.start_current() } else { Task::none() }
            }
            Message::SeekBy(delta) => {
                if self.playback.is_active() {
                    let duration = self.queue.current().map(Track::duration);
                    let target = seek::seek_by(self.audio_position(), delta, duration);
                    self.seek_to(target);
                }
                Task::none()
            }
            Message::Scrub(fraction) => {
                if self.playback.is_active() {
                    self.playback.scrubbing = Some(fraction);
                }
                Task::none()
            }
            Message::ScrubReleased => {
                if let (Some(fraction), Some(track)) = (self.playback.scrubbing.take(), self.queue.current())
                {
                    let target = seek::seek_fraction(fraction, track.duration());
                    self.seek_to(target);
                }
                Task::none()
            }
            Message::Volume(volume) => {
                self.set_volume(volume);
                self.volume_before_mute = None;
                Task::none()
            }
            Message::VolumeReleased => self.persist(),
            Message::ToggleMute => {
                match self.volume_before_mute.take() {
                    Some(previous) => self.set_volume(previous),
                    None => {
                        self.volume_before_mute = Some(self.playback.volume);
                        self.set_volume(0.0);
                    }
                }
                self.persist()
            }
            Message::ToggleShuffle => {
                self.queue.set_shuffle(!self.queue.shuffle(), seed());
                self.persist()
            }
            Message::CycleRepeat => {
                self.queue.cycle_repeat();
                self.persist()
            }
            Message::Tick => {
                self.resync_video();
                Task::batch([self.tick(), self.jam_tick()])
            }
            Message::Frame => {
                if self.playback.is_playing()
                    && let Some(audio) = &self.audio
                {
                    self.playback.position = audio.position();
                }
                self.advance_splash();
                // L'avviso è uscito del tutto: si toglie.
                if self.notice.is_none()
                    && self.toast.as_ref().is_some_and(|toast| !toast.motion.is_running())
                {
                    self.toast = None;
                }
                Task::none()
            }
            Message::SkipSplash => {
                self.splash = None;
                self.page_motion = Tween::new(0.0, 1.0, tokens::MOTION_BASE);
                Task::none()
            }
            Message::NoticeExpired(generation) => {
                if self.toast.as_ref().is_some_and(|toast| toast.generation == generation) {
                    self.notice = None;
                }
                Task::none()
            }
            Message::OpenPanel(tab) => {
                let on_it = self.panel_open && self.side_tab == tab;
                self.panel_open = !on_it;
                self.side_tab = tab;
                Task::batch([self.ensure_lyrics(), self.persist()])
            }
            Message::ClosePanel => {
                self.panel_open = false;
                self.persist()
            }
            Message::SettingsTab(tab) => {
                self.settings_tab = tab;
                Task::none()
            }
            Message::SideTab(tab) => {
                self.side_tab = tab;
                self.ensure_lyrics()
            }
            Message::LyricsLoaded(track_id, result) => {
                // Una risposta di un brano già passato non si mostra.
                if self.lyrics_track != Some(track_id) {
                    return Task::none();
                }
                self.lyrics = into_load(result.map(|lyrics| Arc::new(Text::of(&lyrics))));
                self.lyrics_line = None;
                self.lyrics_scroll = Tween::rest(-1.0);
                self.follow_lyrics()
            }
            Message::LyricSeek(at) => {
                if self.playback.is_active() {
                    self.seek_to(at);
                }
                Task::none()
            }
            Message::ServerDraft(url) => {
                self.server_draft = url;
                Task::none()
            }
            Message::ServerApply => {
                let server = config::normalize_server(&self.server_draft);
                self.set_server(server)
            }
            Message::ServerReset => self.set_server(None),
            Message::JamName(name) => {
                self.jam_name = name.chars().take(jam::MAX_NAME_CHARS).collect();
                Task::none()
            }
            Message::JamCode(code) => {
                self.jam_code = code
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .take(jam::CODE_LEN)
                    .map(|c| c.to_ascii_uppercase())
                    .collect();
                Task::none()
            }
            Message::JamCreate => self.start_jam(Target::Create),
            Message::JamJoin => {
                if self.jam_code.len() == jam::CODE_LEN {
                    let code = self.jam_code.clone();
                    self.start_jam(Target::Join(code))
                } else {
                    self.notice = Some(Notice::Error("Il codice ha 6 lettere o cifre".into()));
                    Task::none()
                }
            }
            Message::JamLeave => {
                // Chiudere il collegamento basta: la stanza libera il posto e avvisa gli altri.
                self.jam = None;
                self.jam_target = None;
                self.social.invitees.clear();
                self.notice = Some(Notice::Info("Sei uscito dalla jam".into()));
                Task::none()
            }
            Message::JamTracksLoaded(remote, result) => self.jam_tracks_loaded(remote, result),
            Message::EqualizerToggle(enabled) => {
                self.equalizer.enabled = enabled;
                self.apply_equalizer();
                self.persist()
            }
            Message::EqualizerBand(band, gain) => {
                if let Some(slot) = self.equalizer.gains.get_mut(band) {
                    *slot = gain;
                }
                self.apply_equalizer();
                Task::none()
            }
            Message::EqualizerPreset(index) => match PRESETS.get(index) {
                Some((_, gains)) => {
                    self.equalizer.gains = *gains;
                    self.apply_equalizer();
                    self.persist()
                }
                None => Task::none(),
            },
            Message::EqualizerReleased => self.persist(),
            Message::Enqueue(track) => {
                let title = track.title.clone();
                self.queue.enqueue(track);
                self.notice = Some(Notice::Info(format!("«{title}» aggiunto alla coda")));
                self.persist()
            }
            Message::RemoveFromQueue(position) => {
                let was_current = self.queue.remove(position);
                let restart = if was_current && self.playback.is_active() {
                    self.start_current()
                } else if self.queue.is_empty() {
                    self.stop();
                    Task::none()
                } else {
                    Task::none()
                };
                Task::batch([restart, self.persist()])
            }
            Message::ClearQueue => {
                self.queue.clear();
                self.stop();
                self.persist()
            }
            Message::ToggleLike(track) => self.toggle_like(track),
            Message::SidebarFilter(filter) => {
                // Un secondo clic sul chip acceso lo spegne: si torna a «Tutto».
                self.sidebar_filter = if self.sidebar_filter == filter { SidebarFilter::All } else { filter };
                Task::none()
            }
            Message::ToggleFollow(artist) => self.toggle_follow(artist),
            Message::ToggleSaveAlbum(album) => self.toggle_save_album(album),
            Message::PlayAlbum(id) => self.play_album(id),
            Message::AlbumPlayLoaded(result) => self.album_play_loaded(result),
            Message::StartRadio(track) => self.start_radio(*track),
            Message::RadioLoaded(track, result) => self.radio_loaded(*track, result),
            Message::PlayMix(index) => self.play_mix(index),
            Message::HeroGo(index) => {
                self.hero_go(index);
                Task::none()
            }
            Message::HeroNext => {
                self.hero_next();
                Task::none()
            }
            Message::SetSleep(choice) => {
                self.set_sleep(choice);
                Task::none()
            }
            Message::SetAutoplay(on) => {
                self.autoplay = on;
                self.persist()
            }
            Message::AutoplayLoaded(result) => self.autoplay_loaded(result),
            Message::SetDedicatedVideo(on) => {
                self.dedicated_video = on;
                // Il videoclip aperto riparte con il decoder scelto; il prossimo lo usa da sé.
                Task::batch([self.persist(), self.reopen_video()])
            }
            Message::UpdateChecked(offer) => {
                self.update = offer.map(UpdateState::Offered);
                Task::none()
            }
            Message::UpdateRequested => {
                let (Some(UpdateState::Offered(offer)), Ok(api)) = (&self.update, self.api.clone()) else {
                    return Task::none();
                };
                let offer = offer.clone();
                self.update = Some(UpdateState::Downloading(offer.tag.clone()));
                let dir = api::update::updates_dir();
                Task::perform(
                    async move {
                        tokio::fs::create_dir_all(&dir)
                            .await
                            .map_err(|_| AppError::Io(dir.display().to_string()))?;
                        let dest = dir.join(&offer.asset.name);
                        api.download_release(&offer.asset, &dest).await?;
                        Ok::<PathBuf, AppError>(dest)
                    },
                    Message::UpdateDownloaded,
                )
            }
            Message::UpdateDownloaded(Ok(path)) => {
                let notice = if cfg!(target_os = "android") {
                    "Scaricato. Android ti chiede di confermare l'installazione.".to_owned()
                } else {
                    format!("Scaricato in {}. Installalo a mano.", path.display())
                };
                #[cfg(target_os = "android")]
                crate::phone_install(&path);
                self.update = Some(UpdateState::Downloaded(path));
                self.show_notice(Some(Notice::Info(notice)))
            }
            Message::UpdateDownloaded(Err(err)) => {
                self.update = None;
                self.show_notice(Some(Notice::Error(format!("Download non riuscito: {err}"))))
            }
            Message::GpuDetected(gpu) => {
                self.dedicated_gpu = gpu;
                Task::none()
            }
            Message::ToggleFocus => {
                // Il testo a tutto schermo e il videoclip a tutto schermo sono due viste: il tasto
                // del testo da una vista aperta la chiude, da quella del video passa al testo.
                let open = !self.focus && self.queue.current().is_some();
                self.focus_video = false;
                self.focus = open;
                self.ensure_lyrics()
            }
            Message::OpenSettingsTab(tab) => {
                self.settings_tab = tab;
                let open = self.open(Screen::Settings);
                Task::batch([open, self.persist()])
            }
            Message::LikeCurrent => match self.queue.current().cloned() {
                Some(track) => self.toggle_like(track),
                None => Task::none(),
            },
            Message::VolumeStep(delta) => {
                let volume = (self.playback.volume + delta).clamp(0.0, 1.0);
                self.handle(Message::Volume(volume))
            }
            Message::CopyText(text) => {
                self.notice = Some(Notice::Info("Copiato negli appunti".into()));
                iced::clipboard::write(text)
            }
            Message::EnqueueNext(track) => self.enqueue_next(track),
            Message::OpenMenu(target) => self.open_menu(target),
            Message::CloseMenu => {
                self.menu = None;
                Task::none()
            }
            Message::MenuPlaylists(open) => {
                if let Some(menu) = &mut self.menu {
                    menu.playlists = open;
                    menu.downloads &= !open;
                    menu.timers &= !open;
                }
                Task::none()
            }
            Message::MenuDownloads(open) => {
                if let Some(menu) = &mut self.menu {
                    menu.downloads = open;
                    menu.playlists &= !open;
                    menu.timers &= !open;
                }
                Task::none()
            }
            Message::OpenDrawer => {
                self.drawer = Some(Instant::now());
                Task::none()
            }
            Message::CloseDrawer => {
                self.drawer = None;
                Task::none()
            }
            Message::DrawerGo(message) => {
                self.drawer = None;
                self.handle(*message)
            }
            Message::MenuTimers(open) => {
                if let Some(menu) = &mut self.menu {
                    menu.timers = open;
                    menu.playlists &= !open;
                    menu.downloads &= !open;
                }
                Task::none()
            }
            Message::OpenDialog(dialog) => {
                self.menu = None;
                let named = matches!(dialog, Dialog::NewPlaylist { .. } | Dialog::Rename { .. });
                self.dialog = Some(OpenDialog { dialog, since: Instant::now() });
                if named { text_input::focus(text_input::Id::new(DIALOG_INPUT)) } else { Task::none() }
            }
            Message::DialogInput(text) => {
                self.dialog_input(text);
                Task::none()
            }
            Message::DialogSubmit => self.dialog_submit(),
            Message::DialogCancel => {
                self.dialog = None;
                Task::none()
            }
            Message::AddToPlaylist(id, track) => self.add_to_playlist(id, track),
            Message::RemoveFromPlaylist(id, index) => self.remove_from_playlist(id, index),
            Message::Chosen(message) => {
                self.menu = None;
                self.handle(*message)
            }
            Message::Escape => self.escape(),
            Message::SystemBack => {
                if self.menu.is_some()
                    || self.drawer.is_some()
                    || self.dialog.is_some()
                    || self.shortcuts.is_some()
                    || self.focus
                {
                    self.escape()
                } else {
                    self.go_back()
                }
            }
            Message::WindowReady(id) => {
                self.window = id;
                match id {
                    Some(id) => window::get_size(id).map(Message::WindowResized),
                    None => Task::none(),
                }
            }
            Message::WindowResized(size) => {
                self.window_size = size;
                self.insets = crate::system_insets().or(self.insets);
                Task::none()
            }
            Message::TitleBarPressed => self.title_bar_pressed(),
            Message::Minimize => self.minimize(),
            Message::ToggleFullscreen => self.toggle_fullscreen(),
            Message::Quit => self.quit(),
            Message::ResizeStart(edge) => {
                self.resize_started(edge);
                Task::none()
            }
            Message::ResizeMove(at) => self.resize_moved(at),
            Message::ResizeEnd => {
                self.resizing = None;
                Task::none()
            }
            Message::Download(track) => {
                let quality = self.download_quality;
                self.download(track, quality)
            }
            Message::DownloadAs(track, quality) => self.download(track, quality),
            Message::DownloadAll(tracks) => {
                if !tracks.iter().any(|t| t.downloadable) {
                    self.notice = Some(Notice::Info(
                        "Nessuno di questi brani si può scaricare: la licenza non lo permette".into(),
                    ));
                    return Task::none();
                }
                let batch = self.download_batch.get_or_insert_with(DownloadBatch::default);
                for track in tracks.iter().filter(|t| t.downloadable) {
                    let known = batch.pending.iter().any(|queued| queued.id == track.id)
                        || batch.active.contains(&track.id)
                        || self.downloading.contains(&track.id);
                    if !known {
                        batch.pending.push_back(track.clone());
                        batch.total += 1;
                    }
                }
                if batch.total == 0 {
                    self.download_batch = None;
                    self.notice = Some(Notice::Info("Niente da scaricare: i brani sono già in corso".into()));
                    return Task::none();
                }
                self.pump_downloads()
            }
            Message::DownloadFinished(id, result) => {
                self.downloading.remove(&id);
                if let Some(batch) = self.download_batch.as_mut().filter(|b| b.active.contains(&id)) {
                    batch.active.remove(&id);
                    match &result {
                        Ok(_) => batch.done += 1,
                        Err(err) => {
                            batch.failed += 1;
                            batch.last_error = Some(err.to_string());
                        }
                    }
                    return self.pump_downloads();
                }
                self.notice = Some(match result {
                    Ok(done) => Notice::Info(saved_message(&done)),
                    Err(err) => Notice::Error(format!("Download non riuscito. {err}")),
                });
                Task::none()
            }
            Message::SetDownloadQuality(quality) => {
                self.download_quality = quality;
                self.persist()
            }
            Message::DismissNotice => {
                self.notice = None;
                Task::none()
            }
            Message::FocusSearch => text_input::focus(text_input::Id::new(SEARCH_INPUT)),
            Message::Saved(result) => {
                self.saving = false;
                if let Err(err) = result {
                    self.notice = Some(Notice::Error(format!("Impostazioni non salvate. {err}")));
                }
                if std::mem::take(&mut self.save_pending) { self.persist() } else { Task::none() }
            }
            Message::CloseRequested(id) => {
                if let Some(audio) = &mut self.audio {
                    audio.stop();
                }
                self.warm.clear();
                Task::perform(config::save(self.snapshot()), move |_| Message::Exit(id))
            }
            Message::Exit(id) => window::close(id),
            Message::SetQuality(quality) => {
                if quality != self.quality {
                    self.quality = quality;
                    self.notice =
                        Some(Notice::Info(format!("Qualità {}: vale dal prossimo brano", quality.label())));
                }
                self.persist()
            }
            Message::SignIn => self.open_sign_in(),
            Message::AuthMode(mode) => self.auth_mode(mode),
            Message::AuthInput(field, value) => self.auth_input(field, value),
            Message::AuthSubmit => self.auth_submit(),
            Message::AuthProvider(provider) => self.auth_provider(provider),
            Message::AuthStepped(result) => self.auth_stepped(result),
            Message::AuthResend => self.auth_resend(),
            Message::AuthResent(result) => self.auth_resent(result),
            Message::AuthBack => self.auth_back(),
            Message::SignedIn { restored, result } => self.signed_in(restored, result),
            Message::SignOut => self.sign_out(),
            Message::ProfileHandle(handle) => self.profile_input(Some(handle), None),
            Message::ProfileName(name) => self.profile_input(None, Some(name)),
            Message::ProfileSave(share) => self.save_profile(share),
            Message::ProfileSaved(result) => self.profile_saved(result),
            Message::FriendsLoaded(result) => self.friends_loaded(result),
            Message::SocialPoll => self.poll_social(),
            Message::JamInvitesLoaded(result) => self.jam_invites_loaded(result),
            Message::JamInviteToggle(handle) => {
                self.toggle_invitee(handle);
                Task::none()
            }
            Message::JamInviteSend => self.send_jam_invites(),
            Message::JamInviteSent(result) => self.jam_invite_sent(result),
            Message::JamInviteAccept(code) => self.accept_jam_invite(code),
            Message::JamInviteDismiss(code) => self.dismiss_jam_invite(code),
            Message::JamInviteDismissed(result) => self.social_synced(result),
            Message::UserQuery(query) => self.user_query(query),
            Message::UserSearch => self.search_users(),
            Message::UsersFound(result) => self.users_found(result),
            Message::OpenJam => {
                self.focus = false;
                self.focus_video = false;
                self.open(Screen::Jam)
            }
            Message::OpenProfile(handle) => self.open_profile(handle),
            Message::ProfileLoaded(handle, result) => self.profile_loaded(handle, result),
            Message::FriendAction(handle, add) => self.friend_action(handle, add),
            Message::FriendChanged(handle, result) => self.friend_changed(handle, result),
            Message::OpenFriendPlaylist(handle, id) => self.open_friend_playlist(handle, id),
            Message::FriendPlaylistLoaded(result) => self.friend_playlist_loaded(result),
            Message::PlaylistPublic(id, public) => self.set_playlist_public(id, public),
            Message::SocialSynced(result) => self.social_synced(result),
            Message::SetDiscord(on) => {
                self.discord_presence = on;
                self.start_discord();
                self.publish_media();
                self.persist()
            }
            Message::SessionStored(result) => {
                if let Err(err) = result {
                    self.notice = Some(Notice::Error(err.to_string()));
                }
                Task::none()
            }
            Message::SetPanelVideo(on) => {
                self.panel_video = on;
                self.video_blocked = None;
                Task::batch([self.ensure_lyrics(), self.prepare_videos(), self.persist()])
            }
            Message::RetryVideo => {
                self.video_blocked = None;
                self.ensure_lyrics()
            }
            Message::TrackRefreshed(result) => match result {
                Ok(fresh) => {
                    let covers = self.fetch_covers(fresh.cover_url.clone().into_iter().collect());
                    let changed = self.replace_saved(fresh.id, &fresh);
                    Task::batch([covers, if changed { self.persist() } else { Task::none() }])
                }
                Err(_) => Task::none(),
            },
            Message::ToggleVideoFocus => {
                if self.focus && self.focus_video {
                    self.focus = false;
                    self.focus_video = false;
                    return Task::none();
                }
                let Some(id) = self.queue.current().filter(|t| t.has_video).map(|t| t.id) else {
                    return Task::none();
                };
                self.focus = true;
                self.focus_video = true;
                if self.video_blocked == Some(id) {
                    self.video_blocked = None;
                }
                self.ensure_lyrics()
            }
            Message::ToggleVideoLyrics => {
                self.video_lyrics = !self.video_lyrics;
                Task::none()
            }
            #[cfg(feature = "video")]
            Message::VideoOpened(track_id, result) => self.video_opened(track_id, result),
            #[cfg(feature = "video")]
            Message::VideoUpgrade(track_id) => self.video_upgrade(track_id),
            #[cfg(feature = "video")]
            Message::VideoUpgraded(track_id, result) => self.video_upgraded(track_id, result),
            #[cfg(feature = "video")]
            Message::VideoFailed => self.video_failed(),
            #[cfg(feature = "tour")]
            Message::Tour(event) => self.tour_event(event),
        }
    }

    // ---------- Account ----------

    // ---------- Navigazione e caricamenti ----------

    /// Va a una schermata e la ricorda per «Indietro».
    fn open(&mut self, screen: Screen) -> Task<Message> {
        if screen != self.screen {
            self.back.push(self.screen);
            if self.back.len() > HISTORY {
                self.back.remove(0);
            }
            self.forward.clear();
        }
        self.show(screen)
    }

    /// Torna alla schermata precedente; quelle che non esistono più (una playlist eliminata) si
    /// saltano.
    fn go_back(&mut self) -> Task<Message> {
        while let Some(screen) = self.back.pop() {
            if self.still_there(screen) {
                self.forward.push(self.screen);
                return Task::batch([self.show(screen), self.persist()]);
            }
        }
        Task::none()
    }

    fn go_forward(&mut self) -> Task<Message> {
        while let Some(screen) = self.forward.pop() {
            if self.still_there(screen) {
                self.back.push(self.screen);
                return Task::batch([self.show(screen), self.persist()]);
            }
        }
        Task::none()
    }

    pub fn can_go_back(&self) -> bool {
        self.back.iter().any(|&screen| self.still_there(screen))
    }

    /// La scheda del telefono accesa: quella della schermata o, per una pagina aperta da un'altra
    /// (un album, un artista), quella da cui ci si è arrivati.
    pub fn phone_tab(&self) -> Screen {
        std::iter::once(&self.screen)
            .chain(self.back.iter().rev())
            .find_map(|&screen| phone_tab_of(screen))
            .unwrap_or(Screen::Home)
    }

    pub fn can_go_forward(&self) -> bool {
        self.forward.iter().any(|&screen| self.still_there(screen))
    }

    fn still_there(&self, screen: Screen) -> bool {
        match screen {
            Screen::Playlist(id) => self.playlist(id).is_some(),
            Screen::SourcePlaylist => self.external.open.is_some(),
            Screen::Profile => self.social.open.is_some(),
            Screen::FriendPlaylist => self.social.playlist.is_some(),
            // Si torna al modulo solo se non si è già entrati.
            Screen::SignIn => !matches!(self.account, Account::SignedIn(_)),
            _ => true,
        }
    }

    /// Mostra una schermata, senza toccare la cronologia.
    fn show(&mut self, screen: Screen) -> Task<Message> {
        // Uscendo da «Novità» ciò che c'era si considera visto.
        if self.screen == Screen::News && screen != Screen::News {
            self.mark_news_seen();
        }
        self.screen = screen;
        match screen {
            Screen::Home
                if self.home.recent.needs_load()
                    || self.home.alphabetical.needs_load()
                    || self.home.albums.needs_load() =>
            {
                self.load_home()
            }
            // Gli ascolti sono cambiati dall'ultima volta: i mix e gli artisti si rifanno.
            Screen::Home => {
                self.rebuild_feed();
                Task::none()
            }
            Screen::Library => {
                let tracks =
                    if self.library.tracks.needs_load() { self.load_library(0, false) } else { Task::none() };
                let albums =
                    if self.library.albums.needs_load() { self.load_library_albums() } else { Task::none() };
                Task::batch([tracks, albums])
            }
            Screen::Album(id) => match &self.album {
                Some((current, load)) if *current == id && !load.needs_load() => Task::none(),
                _ => self.load_album(id),
            },
            Screen::Artist(id) => match &self.artist {
                Some((current, load)) if *current == id && !load.needs_load() => Task::none(),
                _ => self.open_artist(id),
            },
            // Sul telefono il campo prende il fuoco solo se lo si tocca.
            Screen::Search if is_phone() => Task::none(),
            Screen::Search => text_input::focus(text_input::Id::new(SEARCH_INPUT)),
            Screen::Friends => self.load_friends(),
            _ => Task::none(),
        }
    }

    /// "Riprova": ricarica i dati della schermata corrente.
    fn reload(&mut self) -> Task<Message> {
        match self.screen {
            Screen::Home => self.load_home(),
            Screen::Library => Task::batch([self.load_library(0, false), self.load_library_albums()]),
            Screen::Album(id) => self.load_album(id),
            Screen::Artist(id) => self.load_artist(id),
            Screen::Search => self.run_search(),
            Screen::SourcePlaylist => match self.external.open.as_ref() {
                Some(open) => {
                    let playlist = open.playlist.clone();
                    self.external.open = None;
                    self.open_source_playlist(playlist)
                }
                None => Task::none(),
            },
            Screen::Friends => self.load_friends(),
            Screen::Profile => match self.social.open.take() {
                Some((handle, _)) => self.open_profile(handle),
                None => Task::none(),
            },
            Screen::Playlist(_)
            | Screen::Settings
            | Screen::Jam
            | Screen::Liked
            | Screen::News
            | Screen::Collection
            | Screen::SignIn
            | Screen::FriendPlaylist => Task::none(),
        }
    }

    /// Cambia il backend: un indirizzo non valido resta nel campo con l'errore, senza toccare quello
    /// in uso. Il catalogo si ricarica dal nuovo server.
    fn set_server(&mut self, server: Option<String>) -> Task<Message> {
        let url = config::api_url(server.as_deref());
        let api = match ApiClient::new(&url) {
            Ok(api) => api,
            Err(err) => {
                self.notice = Some(Notice::Error(err.to_string()));
                return Task::none();
            }
        };
        self.server_draft = server.clone().unwrap_or_default();
        if server == self.server && url == self.api_base_url {
            return Task::none();
        }
        self.notice = Some(Notice::Info(format!("Server: {}", api.host())));
        self.api = Ok(api);
        self.api_base_url = url;
        self.server = server;
        self.artists = ArtistsState::default();
        Task::batch([self.persist(), self.load_home()])
    }

    fn api(&self) -> Result<ApiClient, AppError> {
        self.api.clone()
    }

    fn load_home(&mut self) -> Task<Message> {
        self.home.recent = Load::Loading;
        self.home.alphabetical = Load::Loading;
        self.home.albums = Load::Loading;
        let sections = [
            (HomeSection::Recent, Sort::Recent, HOME_RELEASES),
            (HomeSection::Alphabetical, Sort::Title, HOME_SECTION_SIZE),
        ];
        let tracks = Task::batch(sections.map(|(section, sort, limit)| {
            let api = self.api();
            let query = TrackQuery { q: None, sort, limit, offset: 0 };
            Task::perform(async move { api?.tracks(query).await }, move |result| {
                Message::HomeLoaded(section, result)
            })
        }));
        let api = self.api();
        let albums = Task::perform(
            async move { api?.albums(Sort::Recent, HOME_ALBUMS, 0).await },
            Message::HomeAlbumsLoaded,
        );
        Task::batch([tracks, albums])
    }

    fn load_library_albums(&mut self) -> Task<Message> {
        self.library.albums = Load::Loading;
        let (api, sort) = (self.api(), self.library.sort);
        Task::perform(async move { api?.albums(sort, LIBRARY_ALBUMS, 0).await }, Message::LibraryAlbumsLoaded)
    }

    /// Scarica le copertine mai richieste. Un errore lascia la copertina disegnata.
    fn fetch_covers(&mut self, urls: Vec<String>) -> Task<Message> {
        let wanted = self.covers.request(urls.iter().map(String::as_str));
        Task::batch(wanted.into_iter().map(|url| {
            let api = self.api();
            let request = url.clone();
            // La decodifica avviene nel task, non nel thread della UI.
            Task::perform(
                async move {
                    let bytes = api.ok()?.cover(&request).await.ok()?;
                    decode_square(&bytes)
                },
                move |pixels| Message::CoverLoaded(url.clone(), pixels),
            )
        }))
    }

    fn load_library(&mut self, offset: u32, append: bool) -> Task<Message> {
        if !append {
            self.library.tracks = Load::Loading;
        }
        let api = self.api();
        let query = TrackQuery { q: None, sort: self.library.sort, limit: LIBRARY_PAGE, offset };
        Task::perform(async move { api?.tracks(query).await }, move |result| Message::LibraryLoaded {
            append,
            result,
        })
    }

    fn load_album(&mut self, id: Uuid) -> Task<Message> {
        self.album = Some((id, Load::Loading));
        let api = self.api();
        Task::perform(async move { api?.album(id).await }, move |result| Message::AlbumLoaded(id, result))
    }

    fn load_artist(&mut self, id: Uuid) -> Task<Message> {
        self.artist = Some((id, Load::Loading));
        let api = self.api();
        Task::perform(async move { api?.artist(id).await }, move |result| Message::ArtistLoaded(id, result))
    }

    /// Play su un risultato di Archive: il server porta l'item nel catalogo, poi si suona.
    fn play_discovered(&mut self, identifier: String) -> Task<Message> {
        if self.search.importing.is_some() {
            return Task::none();
        }
        self.search.importing = Some(identifier.clone());
        let api = self.api();
        let request = identifier.clone();
        Task::perform(async move { api?.import_item(&request).await }, move |result| {
            Message::Imported(identifier.clone(), result)
        })
    }

    /// Apre una libreria di Archive e ne carica la prima pagina di album.
    fn open_archive_library(&mut self, library: DiscoverLibrary) -> Task<Message> {
        let identifier = library.identifier.clone();
        self.search.open_library =
            Some(OpenLibrary { library, items: Load::Loading, next_page: None, loading_more: false });
        self.load_archive_library(identifier, 1)
    }

    /// Carica la pagina successiva della libreria aperta, se c'è e non è già in arrivo.
    fn more_archive_library(&mut self) -> Task<Message> {
        let Some(open) = &mut self.search.open_library else { return Task::none() };
        let Some(page) = open.next_page.filter(|_| !open.loading_more) else { return Task::none() };
        open.loading_more = true;
        let identifier = open.library.identifier.clone();
        self.load_archive_library(identifier, page)
    }

    fn load_archive_library(&self, identifier: String, page: u32) -> Task<Message> {
        let api = self.api();
        let request = identifier.clone();
        Task::perform(async move { api?.library(&request, page).await }, move |result| {
            Message::ArchiveLibraryLoaded(identifier.clone(), page, result)
        })
    }

    /// Una pagina di album è arrivata: la prima sostituisce il caricamento, le altre si aggiungono.
    /// Una risposta per una libreria già chiusa si ignora.
    fn archive_library_loaded(
        &mut self,
        identifier: &str,
        page: u32,
        result: Result<LibraryPage, AppError>,
    ) -> Task<Message> {
        let Some(open) = self.search.open_library.as_mut().filter(|o| o.library.identifier == identifier)
        else {
            return Task::none();
        };
        match open.receive(page, result) {
            Ok(covers) => self.fetch_covers(covers),
            Err(err) => {
                self.notice = Some(Notice::Error(err.to_string()));
                Task::none()
            }
        }
    }

    /// Play su un risultato della ricerca esterna: il server verifica licenza e durata, scarica
    /// l'audio e lo porta nel catalogo, poi si suona.
    fn play_source(&mut self, source: SourceKind, id: String) -> Task<Message> {
        if self.search.importing.is_some() {
            return Task::none();
        }
        let key = source_key(source, &id);
        self.search.importing = Some(key.clone());
        let api = self.api();
        // Il brano sta per suonare: se il pannello mostra i videoclip, il server scarica anche quello.
        let video = cfg!(feature = "video") && self.panel_video;
        Task::perform(async move { api?.import_source(source, &id, video).await }, move |result| {
            Message::Imported(key.clone(), result)
        })
    }

    fn run_search(&mut self) -> Task<Message> {
        let Some(query) = clean_query(&self.search.query) else {
            return Task::none();
        };
        let generation = self.search.generation;
        self.search.submitted = query.clone();
        config::remember_search(&mut self.search_history, &query);
        self.search.results = Load::Loading;
        let save = self.persist();
        let api = self.api();
        let request = query.clone();
        let search = Task::perform(async move { api?.search(&request).await }, move |result| {
            Message::SearchLoaded(generation, result)
        });
        // In parallelo, la stessa ricerca su Internet Archive e nel catalogo ufficiale.
        self.search.discover = Load::Loading;
        self.search.libraries = Arc::from([]);
        self.search.open_library = None;
        let api = self.api();
        let request = query.clone();
        let discover = Task::perform(async move { api?.discover(&request).await }, move |result| {
            Message::DiscoverLoaded(generation, result)
        });
        // Il catalogo ufficiale: brani, artisti, album e playlist con copertine e foto.
        self.search.sources = Load::Loading;
        self.search.playlists = Load::Loading;
        self.search.artists = Arc::from([]);
        let api = self.api();
        let meta = Task::perform(async move { api?.meta_search(&query).await }, move |result| {
            Message::MetaSearchLoaded(generation, result)
        });
        Task::batch([search, discover, meta, save])
    }

    // ---------- Riproduzione ----------

    /// Il brano corrente non esiste più sul server (database ricreato): lo cerca nel catalogo
    /// ufficiale e lo importa di nuovo. Una volta sola per brano.
    fn heal_current(&mut self) -> Option<Task<Message>> {
        let track = self.queue.current()?.clone();
        if !self.healing.insert(track.id) {
            return None;
        }
        let old = track.id;
        let api = self.api();
        Some(Task::perform(async move { api::heal_track(api?, track).await }, move |result| {
            Message::TrackHealed(old, result)
        }))
    }

    /// Rilegge dal server il brano appena partito, una volta per sessione: le copie salvate in coda,
    /// nelle playlist e fra i «mi piace» possono essere di prima che il server avesse il videoclip o
    /// la copertina ufficiale.
    fn refresh_current(&mut self) -> Task<Message> {
        let Some(id) = self.queue.current().map(|t| t.id) else { return Task::none() };
        if !self.refreshed.insert(id) {
            return Task::none();
        }
        let api = self.api();
        Task::perform(async move { api?.track(id).await }, Message::TrackRefreshed)
    }

    /// All'avvio, i brani salvati senza copertina (salvati prima che il server ne avesse una) si
    /// rileggono dal server, al massimo `STALE_REFRESH` per sessione.
    fn refresh_stale_saved(&mut self) -> Task<Message> {
        let mut ids: Vec<Uuid> = Vec::new();
        let playlists = self.playlists.iter().flat_map(|p| p.tracks.iter());
        for track in self.liked.iter().chain(self.recent.iter()).chain(playlists) {
            if track.cover_url.is_none() && !ids.contains(&track.id) {
                ids.push(track.id);
            }
        }
        ids.truncate(STALE_REFRESH);
        ids.retain(|id| self.refreshed.insert(*id));
        let tasks: Vec<Task<Message>> = ids
            .into_iter()
            .map(|id| {
                let api = self.api();
                Task::perform(async move { api?.track(id).await }, Message::TrackRefreshed)
            })
            .collect();
        Task::batch(tasks)
    }

    /// Mette `new` al posto di ogni copia salvata del brano `old` (coda, mi piace, recenti,
    /// playlist, ascolti). `true` se qualcosa è cambiato.
    fn replace_saved(&mut self, old: Uuid, new: &Track) -> bool {
        let swap = |t: &mut Track| {
            let hit = t.id == old && t != new;
            if hit {
                *t = new.clone();
            }
            hit
        };
        let mut changed = self.queue.update_tracks(swap);
        for track in self.liked.iter_mut().chain(self.recent.iter_mut()) {
            changed |= swap(track);
        }
        for playlist in &mut self.playlists {
            for track in &mut playlist.tracks {
                changed |= swap(track);
            }
        }
        for stat in &mut self.stats {
            changed |= swap(&mut stat.track);
        }
        changed
    }

    fn track_healed(&mut self, old: Uuid, result: Result<Track, AppError>) -> Task<Message> {
        let was_current = self.queue.current().is_some_and(|t| t.id == old);
        let new = match result {
            Ok(track) => track,
            Err(_) => {
                if was_current {
                    self.playback.state = PlaybackState::Error(
                        "Questo brano non è più disponibile sul server e non l'ho ritrovato nel catalogo."
                            .into(),
                    );
                }
                return Task::none();
            }
        };
        let changed = self.replace_saved(old, &new);
        let covers = self.fetch_covers(new.cover_url.clone().into_iter().collect());
        let start = if was_current && changed { self.start_current() } else { Task::none() };
        Task::batch([start, self.persist(), covers])
    }

    /// Carica e avvia il brano corrente della coda; se il pannello mostra il testo, lo carica anche.
    fn start_current(&mut self) -> Task<Message> {
        let playback = self.start_playback();
        Task::batch([playback, self.ensure_lyrics()])
    }

    /// Carica e avvia il brano corrente della coda.
    fn start_playback(&mut self) -> Task<Message> {
        let Some(track_id) = self.queue.current().map(|t| t.id) else {
            self.stop();
            return Task::none();
        };
        if let Some(audio) = &mut self.audio {
            audio.stop();
        }
        let generation = self.playback.begin_loading(track_id);
        let quality = self.quality;
        if let Some((cached_id, cached_quality, stream)) = &self.cached_audio
            && *cached_id == track_id
            && *cached_quality == quality
            && !stream.is_cancelled()
        {
            return Task::done(Message::AudioLoaded(generation, Ok(stream.clone())));
        }
        if let Some(stream) = self.warm.take(track_id, quality) {
            return Task::done(Message::AudioLoaded(generation, Ok(stream)));
        }
        let api = self.api();
        Task::perform(async move { api::stream_audio(api?, track_id, quality).await }, move |result| {
            Message::AudioLoaded(generation, result)
        })
    }

    fn start_audio(&mut self, stream: StreamBuffer) -> Result<(), AppError> {
        let track_id = self.queue.current().map(|t| t.id).ok_or(AppError::AudioDecode)?;
        let audio = self.audio.as_mut().ok_or(AppError::AudioDevice)?;
        audio.play(&stream, self.playback.volume)?;
        // Il download del brano precedente, se non è finito, non serve più.
        if let Some((previous_id, _, previous)) = self.cached_audio.replace((track_id, self.quality, stream))
            && previous_id != track_id
            && !previous.is_complete()
        {
            previous.cancel();
        }
        Ok(())
    }

    fn toggle_play(&mut self) -> Task<Message> {
        match &self.playback.state {
            PlaybackState::Playing => {
                if let Some(audio) = &self.audio {
                    audio.pause();
                }
                self.playback.state = PlaybackState::Paused;
                self.sync_video(None);
                Task::none()
            }
            PlaybackState::Paused => {
                self.drop_expired_sleep();
                if let Some(audio) = &self.audio {
                    audio.resume();
                }
                self.playback.state = PlaybackState::Playing;
                self.sync_video(Some(self.audio_position()));
                Task::none()
            }
            PlaybackState::Stopped | PlaybackState::Error(_) => self.start_current(),
            PlaybackState::Loading { .. } => Task::none(),
        }
    }

    fn skip(&mut self) -> Task<Message> {
        if self.queue.advance(Advance::Skip).is_some() {
            self.start_current()
        } else {
            self.stop();
            Task::none()
        }
    }

    fn tick(&mut self) -> Task<Message> {
        let Some(audio) = &self.audio else {
            return Task::none();
        };
        let seek_failed = audio.take_seek_failure();
        let finished = audio.finished();
        let position = audio.position();
        if seek_failed {
            self.notice = Some(Notice::Error(AppError::Seek.to_string()));
        }
        if !self.playback.is_playing() {
            return Task::none();
        }
        if finished {
            if self.autoplay_busy {
                return Task::none();
            }
            if self.sleep_at_end_of_track() {
                // Il prossimo brano è pronto in coda, ma non parte: si riprende con Play.
                let _ = self.queue.advance(Advance::Finished);
                return Task::none();
            }
            return if self.queue.advance(Advance::Finished).is_some() {
                self.start_current()
            } else {
                self.queue_ended()
            };
        }
        self.listened_ms = self.listened_ms.saturating_add(TICK.as_millis() as u64);
        self.count_play_when_heard(position);
        if self.sleep_due() {
            return self.persist();
        }
        self.playback.position = position;
        Task::batch([self.follow_lyrics(), self.prefetch_next()])
    }

    /// Negli ultimi secondi di un brano fa scaricare per intero il successivo della coda (la sua
    /// testata è già calda), così parte senza attesa.
    fn prefetch_next(&mut self) -> Task<Message> {
        let (Some(current), Some(next)) = (self.queue.current(), self.queue.peek_next()) else {
            return Task::none();
        };
        let remaining = current.duration().saturating_sub(self.playback.position);
        if current.duration().is_zero() || remaining > PREFETCH_BEFORE_END || next.id == current.id {
            return Task::none();
        }
        let (track_id, quality) = (next.id, self.quality);
        if self.warm.release(track_id, quality) || !self.warm.begin(track_id, quality) {
            return Task::none();
        }
        let api = self.api();
        Task::perform(async move { api::stream_audio(api?, track_id, quality).await }, move |result| {
            Message::Warmed(track_id, quality, result)
        })
    }

    /// Prepara la testata di un brano che forse si ascolterà. Non fa nulla se è già caldo o in apertura.
    fn warm_track(&mut self, track_id: Uuid) -> Task<Message> {
        let quality = self.quality;
        let already_playing =
            self.cached_audio.as_ref().is_some_and(|(id, q, _)| *id == track_id && *q == quality);
        if already_playing || !self.warm.begin(track_id, quality) {
            return Task::none();
        }
        let api = self.api();
        Task::perform(async move { api::warm_audio(api?, track_id, quality).await }, move |result| {
            Message::Warmed(track_id, quality, result)
        })
    }

    /// Mentre un brano suona, prepara i prossimi della coda.
    fn warm_ahead(&mut self) -> Task<Message> {
        let ahead: Vec<Uuid> = self.queue.upcoming().take(WARM_AHEAD).map(|(_, track)| track.id).collect();
        Task::batch(ahead.into_iter().map(|id| self.warm_track(id)))
    }

    /// Carica il testo del brano in riproduzione, ma solo se il pannello lo mostra. Un brano già
    /// caricato, o in caricamento, non si richiede di nuovo.
    fn ensure_lyrics(&mut self) -> Task<Message> {
        // Il testo serve anche sopra il videoclip del pannello.
        let panel_lyrics = self.panel_open
            && (self.side_tab == SideTab::Lyrics
                || (self.side_tab == SideTab::NowPlaying && self.wanted_video().is_some()));
        if !(self.focus || panel_lyrics) {
            return Task::none();
        }
        let Some(track_id) = self.queue.current().map(|t| t.id) else {
            return Task::none();
        };
        if self.lyrics_track == Some(track_id) && !self.lyrics.needs_load() {
            return Task::none();
        }
        self.lyrics_track = Some(track_id);
        self.lyrics = Load::Loading;
        self.lyrics_line = None;
        self.lyrics_scroll = Tween::rest(-1.0);
        let api = self.api();
        Task::perform(async move { api?.lyrics(track_id).await }, move |result| {
            Message::LyricsLoaded(track_id, result)
        })
    }

    /// Con il testo sincronizzato la riga in corso segue la riproduzione: la lista scorre fino a lei.
    fn follow_lyrics(&mut self) -> Task<Message> {
        let Some(text) = self.lyrics.ready() else {
            return Task::none();
        };
        if !text.is_synced() {
            return Task::none();
        }
        let line = synced::current(&text.lines, self.playback.position);
        if line == self.lyrics_line {
            return Task::none();
        }
        self.lyrics_line = line;
        // Il testo (nel pannello e a tutto schermo) scorre verso la nuova riga invece di saltarci.
        self.lyrics_scroll.retarget(line.map_or(-1.0, |i| i as f32), tokens::MOTION_LYRICS);
        Task::none()
    }

    /// Lo splash fa il suo corso: quando il velo comincia a sfumare la pagina sotto entra, e alla fine
    /// lo splash si toglie.
    fn advance_splash(&mut self) {
        let Some(splash) = &mut self.splash else {
            return;
        };
        let elapsed = splash.since.elapsed();
        if elapsed >= tokens::SPLASH_TOTAL {
            self.splash = None;
        } else if elapsed >= tokens::SPLASH_EXIT && !splash.revealed {
            splash.revealed = true;
            self.page_motion = Tween::new(0.0, 1.0, tokens::MOTION_PANEL);
            // A questo punto la finestra è attaccata: il sistema sa dire dove stanno le sue barre.
            self.insets = crate::system_insets().or(self.insets);
        }
    }

    fn stop(&mut self) {
        if let Some(audio) = &mut self.audio {
            audio.stop();
        }
        self.playback.state = PlaybackState::Stopped;
        self.playback.position = Duration::ZERO;
        self.playback.scrubbing = None;
        self.sync_video(None);
    }

    fn audio_position(&self) -> Duration {
        self.audio.as_ref().map_or(Duration::ZERO, AudioOutput::position)
    }

    fn seek_to(&mut self, target: Duration) {
        let Some(audio) = &self.audio else { return };
        // Il seek finisce sul thread audio: la posizione si aggiorna subito, un errore arriva al Tick.
        audio.seek(target);
        self.playback.position = target;
        self.sync_video(Some(target));
        self.publish_media();
    }

    /// Dice al sistema cosa suona, a che punto e se è in pausa.
    fn publish_media(&self) {
        self.publish_discord();
        let Some(handle) = &self.system_media else { return };
        let now = self.queue.current().filter(|_| self.playback.is_active()).map(|track| {
            system_media::NowPlaying {
                title: &track.title,
                artist: &track.artist.name,
                album: track.album.as_ref().map(|album| album.title.as_str()),
                cover_url: track.cover_url.as_deref(),
                duration: track.duration(),
                playing: self.playback.is_playing(),
                position: self.audio_position(),
                artwork: self.covers.pixels(track.cover_url.as_deref()),
            }
        });
        handle.show(now.as_ref());
    }

    fn system_media_event(&mut self, event: system_media::Event) -> Task<Message> {
        use system_media::Event;
        match event {
            Event::Ready(handle) => {
                handle.set_volume(self.playback.volume);
                self.system_media = Some(handle);
                self.publish_media();
                Task::none()
            }
            Event::Play if !self.playback.is_playing() => self.toggle_play(),
            Event::Pause if self.playback.is_playing() => self.toggle_play(),
            Event::Play | Event::Pause => Task::none(),
            Event::Toggle => self.toggle_play(),
            Event::Next => self.handle(Message::Next),
            Event::Previous => self.handle(Message::Previous),
            Event::SeekBy(secs) => self.handle(Message::SeekBy(secs)),
            Event::SetPosition(at) => {
                if self.playback.is_active() {
                    self.seek_to(at);
                }
                Task::none()
            }
            Event::SetVolume(volume) => {
                let _ = self.handle(Message::Volume(volume));
                self.persist()
            }
            Event::Raise => self.window.map_or_else(Task::none, window::gain_focus),
        }
    }

    /// Manda le regolazioni dell'equalizzatore all'uscita audio: valgono dal prossimo campione.
    fn apply_equalizer(&mut self) {
        self.equalizer = self.equalizer.clamped();
        if let Some(audio) = &self.audio {
            audio.set_equalizer(self.equalizer);
        }
    }

    fn set_volume(&mut self, volume: f32) {
        self.playback.volume = volume.clamp(0.0, 1.0);
        if let Some(audio) = &self.audio {
            audio.set_volume(self.playback.volume);
        }
        if let Some(handle) = &self.system_media {
            handle.set_volume(self.playback.volume);
        }
    }

    // ---------- Download e persistenza ----------

    /// Fa partire i prossimi brani di «Scarica tutto» fino a `BATCH_PARALLEL` insieme; finiti tutti,
    /// un solo avviso riassume com'è andata.
    fn pump_downloads(&mut self) -> Task<Message> {
        let quality = self.download_quality;
        let mut starts = Vec::new();
        while let Some(batch) = self.download_batch.as_mut().filter(|b| b.active.len() < BATCH_PARALLEL) {
            let Some(track) = batch.pending.pop_front() else { break };
            batch.active.insert(track.id);
            starts.push(track);
        }
        let tasks: Vec<_> = starts.into_iter().map(|track| self.download(track, quality)).collect();
        if let Some(batch) =
            self.download_batch.as_ref().filter(|b| b.pending.is_empty() && b.active.is_empty())
        {
            self.notice = Some(batch.summary());
            self.download_batch = None;
        }
        Task::batch(tasks)
    }

    /// Quanti brani di «Scarica tutto» sono finiti e quanti erano in tutto, se c'è un lotto in corso.
    pub fn download_progress(&self) -> Option<(usize, usize)> {
        self.download_batch.as_ref().map(|b| (b.done + b.failed, b.total))
    }

    fn download(&mut self, track: Track, quality: DownloadQuality) -> Task<Message> {
        // Il controllo sta qui: il player del telefono mostra il tasto anche per i brani che non si
        // possono scaricare, e dice perché.
        if !track.downloadable {
            self.notice =
                Some(Notice::Info("Questo brano non si può scaricare: la licenza non lo permette".into()));
            return Task::none();
        }
        if !self.downloading.insert(track.id) {
            return Task::none();
        }
        let id = track.id;
        let api = self.api();
        let dir = download_dir();
        Task::perform(async move { api::download_track(api?, track, dir, quality).await }, move |result| {
            Message::DownloadFinished(id, result)
        })
    }

    /// Avvia una jam: crea una stanza o entra in una esistente. Il collegamento gira su un thread
    /// proprio, quindi la richiesta torna subito.
    fn start_jam(&mut self, target: Target) -> Task<Message> {
        if self.jam.is_some() {
            self.notice = Some(Notice::Info("Sei già in una jam".into()));
            return Task::none();
        }
        // Senza un nome scelto per la stanza, con un profilo ci si presenta con il proprio.
        let typed = self.jam_name.trim();
        let name = match self.signed_in_profile() {
            Some(profile) if typed.is_empty() => profile.display_name.clone(),
            _ => typed.to_owned(),
        };
        let link = Link::start(&self.api_base_url, target, name);
        self.jam = Some(JamSession {
            link,
            code: None,
            you: None,
            members: Vec::new(),
            last_sent: None,
            synced: false,
            fetching: false,
        });
        Task::none()
    }

    /// Ogni tick: si leggono gli eventi della stanza e poi, se serve, si manda lo stato locale.
    fn jam_tick(&mut self) -> Task<Message> {
        let Some(session) = self.jam.as_mut() else { return Task::none() };
        let events: Vec<Event> = std::iter::from_fn(|| session.link.events.try_recv().ok()).collect();
        let tasks: Vec<Task<Message>> = events.into_iter().map(|event| self.jam_event(event)).collect();
        self.jam_publish();
        Task::batch(tasks)
    }

    fn jam_event(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Joined { code, you, members, state } => {
                self.notice = Some(Notice::Info(format!("Sei nella jam {code}")));
                if let Some(session) = self.jam.as_mut() {
                    session.code = Some(code);
                    session.you = Some(you);
                    session.members = members;
                }
                match state {
                    Some(remote) => self.apply_jam_state(remote),
                    // Stanza nuova: il primo stato è il proprio.
                    None => {
                        if let Some(session) = self.jam.as_mut() {
                            session.synced = true;
                        }
                        Task::none()
                    }
                }
            }
            Event::Members(members) => {
                if let Some(session) = self.jam.as_mut() {
                    session.members = members;
                }
                Task::none()
            }
            Event::State(remote) => self.apply_jam_state(remote),
            Event::Rejected(reason) => {
                self.notice = Some(Notice::Error(reason));
                Task::none()
            }
            Event::Closed(reason) => {
                self.jam = None;
                self.jam_target = None;
                self.social.invitees.clear();
                self.notice = Some(Notice::Error(reason));
                Task::none()
            }
        }
    }

    /// Lo stato del gruppo diventa quello locale: si caricano i brani che mancano, poi si allinea.
    fn apply_jam_state(&mut self, remote: Remote) -> Task<Message> {
        let Some(session) = self.jam.as_mut() else { return Task::none() };
        session.synced = true;
        // Lo stato ricevuto è il riferimento: non va rispedito come se fosse locale.
        session.last_sent = Some(Sent { snapshot: remote.snapshot.clone(), at: Instant::now() });
        if remote.snapshot.queue.is_empty() {
            self.queue.clear();
            self.stop();
            return Task::none();
        }
        if self.queue.current().map(|t| t.id) != remote.snapshot.queue.get(remote.snapshot.index).copied() {
            self.notice = Some(Notice::Info(format!("{} ha cambiato brano", remote.from)));
        }
        let known: Vec<Uuid> = self.queue.entries().map(|(_, t)| t.id).collect();
        let missing: Vec<Uuid> =
            remote.snapshot.queue.iter().copied().filter(|id| !known.contains(id)).collect();
        if missing.is_empty() {
            return self.adopt_jam_queue(remote, Vec::new());
        }
        if let Some(session) = self.jam.as_mut() {
            session.fetching = true;
        }
        let api = self.api();
        Task::perform(async move { fetch_tracks(api?, missing).await }, move |result| {
            Message::JamTracksLoaded(remote.clone(), result)
        })
    }

    fn jam_tracks_loaded(&mut self, remote: Remote, result: Result<Vec<Track>, AppError>) -> Task<Message> {
        if let Some(session) = self.jam.as_mut() {
            session.fetching = false;
        } else {
            return Task::none();
        }
        match result {
            Ok(fetched) => self.adopt_jam_queue(remote, fetched),
            Err(err) => {
                self.notice = Some(Notice::Error(format!("Brani della jam non caricati: {err}")));
                Task::none()
            }
        }
    }

    /// Mette la coda locale nell'ordine del gruppo, con i brani già in coda e quelli arrivati dal
    /// catalogo. Lo shuffle si spegne: l'ordine lo decide chi suona per primo.
    fn adopt_jam_queue(&mut self, remote: Remote, fetched: Vec<Track>) -> Task<Message> {
        let snapshot = &remote.snapshot;
        let local: Vec<Uuid> = self.queue.entries().map(|(_, t)| t.id).collect();
        if local != snapshot.queue {
            let mut known: Vec<Track> = self.queue.entries().map(|(_, t)| t.clone()).collect();
            known.extend(fetched);
            let tracks: Option<Vec<Track>> =
                snapshot.queue.iter().map(|id| known.iter().find(|t| t.id == *id).cloned()).collect();
            let Some(tracks) = tracks else {
                self.notice = Some(Notice::Error("Alcuni brani della jam non sono disponibili".into()));
                return Task::none();
            };
            self.queue.set_shuffle(false, seed());
            self.queue.replace(tracks, snapshot.index, seed());
        }
        self.align_jam(snapshot)
    }

    /// Allinea la riproduzione al gruppo: stesso brano e stessa posizione, oppure si cambia brano.
    fn align_jam(&mut self, remote: &Snapshot) -> Task<Message> {
        let Some(track) = remote.queue.get(remote.index).copied() else { return Task::none() };
        let target =
            JamTarget { track, position: Duration::from_millis(remote.position_ms), playing: remote.playing };
        // Il brano è già in caricamento: basta ricordare la posizione, senza ricominciare.
        if let PlaybackState::Loading { track_id } = self.playback.state
            && track_id == track
        {
            self.jam_target = Some(target);
            return Task::none();
        }
        let same_track =
            self.queue.current().map(|t| t.id) == Some(track) && self.queue.position() == Some(remote.index);
        if !same_track || !self.playback.is_active() {
            self.queue.jump(remote.index);
            self.jam_target = Some(target);
            return self.start_playback();
        }
        if jam::drifted(jam::millis(self.audio_position()), remote.position_ms) {
            self.seek_to(target.position);
        }
        if remote.playing != self.playback.is_playing() { self.toggle_play() } else { Task::none() }
    }

    /// Dopo il caricamento di un brano del gruppo: si salta alla posizione condivisa e, se il gruppo
    /// è in pausa, ci si ferma lì. Un brano diverso da quello atteso fa scartare il riferimento.
    fn apply_jam_target(&mut self) {
        let Some(target) = self.jam_target.take() else { return };
        if self.queue.current().map(|t| t.id) != Some(target.track) {
            return;
        }
        self.seek_to(target.position);
        if !target.playing {
            if let Some(audio) = &self.audio {
                audio.pause();
            }
            self.playback.state = PlaybackState::Paused;
        }
    }

    /// Stato locale nella forma condivisa: la coda in ordine di ascolto, il brano e la posizione.
    /// `None` se non c'è nulla da condividere o la coda è troppo lunga per il server.
    fn jam_snapshot(&self) -> Option<Snapshot> {
        let index = self.queue.position()?;
        let queue: Vec<Uuid> = self.queue.entries().map(|(_, t)| t.id).take(jam::MAX_QUEUE).collect();
        if queue.len() != self.queue.len() {
            return None;
        }
        Some(Snapshot {
            queue,
            index,
            position_ms: jam::millis(self.audio_position()),
            playing: self.playback.is_playing(),
        })
    }

    /// Manda lo stato locale alla stanza, se è cambiato o se è tempo di rimandare la posizione.
    /// Non si manda nulla mentre il gruppo non ha ancora detto il suo stato, né mentre un brano
    /// arrivato dal gruppo è in caricamento: altrimenti si sovrascriverebbe la stanza con dati vecchi.
    fn jam_publish(&mut self) {
        let busy = self.jam_target.is_some() || matches!(self.playback.state, PlaybackState::Loading { .. });
        let Some(snapshot) = self.jam_snapshot() else { return };
        let Some(session) = self.jam.as_mut() else { return };
        if !session.synced || session.fetching || busy {
            return;
        }
        let now = Instant::now();
        if jam::should_send(session.last_sent.as_ref(), &snapshot, now) {
            session.link.send(snapshot.clone());
            session.last_sent = Some(Sent { snapshot, at: now });
        }
    }

    fn snapshot(&self) -> Config {
        Config {
            server: self.server.clone(),
            legacy_api: None,
            volume: self.volume_before_mute.unwrap_or(self.playback.volume),
            last_screen: self.screen.into(),
            queue: self.queue.clone(),
            playlists: self.playlists.clone(),
            panel_open: self.panel_open,
            liked: self.liked.clone(),
            recent: self.recent.clone(),
            followed: self.followed.clone(),
            saved_albums: self.saved_albums.clone(),
            stats: self.stats.clone(),
            listened_ms: self.listened_ms,
            news_seen: self.news_seen.clone(),
            autoplay: self.autoplay,
            dedicated_video: self.dedicated_video,
            panel_video: self.panel_video,
            search_history: self.search_history.clone(),
            equalizer: self.equalizer,
            last_query: self.search.submitted.clone(),
            quality: self.quality,
            download_quality: self.download_quality,
            clerk_issuer: self.clerk_issuer.clone(),
            clerk_client_id: self.clerk_client_id.clone(),
            discord_presence: self.discord_presence,
            discord_app_id: self.discord_app_id.clone(),
            oauth_port: self.oauth_port,
        }
    }

    /// Salva la configurazione. I salvataggi non si sovrappongono: se uno è in corso, il
    /// successivo parte alla sua conclusione con lo stato più recente.
    fn persist(&mut self) -> Task<Message> {
        if self.saving {
            self.save_pending = true;
            return Task::none();
        }
        self.saving = true;
        Task::perform(config::save(self.snapshot()), Message::Saved)
    }
}

/// URL delle copertine dei brani, senza ripetizioni consecutive (i brani di un album ne hanno una).
/// Brani del catalogo per identificativo, pochi alla volta. L'ordine non conta: il chiamante
/// riallinea per id.
async fn fetch_tracks(api: ApiClient, ids: Vec<Uuid>) -> Result<Vec<Track>, AppError> {
    const PARALLEL: usize = 8;
    futures_util::stream::iter(ids).map(|id| api.track(id)).buffer_unordered(PARALLEL).try_collect().await
}

fn track_covers(tracks: Option<&[Track]>) -> Vec<String> {
    let mut urls: Vec<String> =
        tracks.unwrap_or_default().iter().filter_map(|t| t.cover_url.clone()).collect();
    urls.dedup();
    urls
}

fn album_covers(albums: Option<&[AlbumSummary]>) -> Vec<String> {
    albums.unwrap_or_default().iter().filter_map(|a| a.cover_url.clone()).collect()
}

fn into_load<T>(result: Result<T, AppError>) -> Load<T> {
    match result {
        Ok(value) => Load::Ready(value),
        Err(err) => Load::Failed(err),
    }
}

/// Testo di ricerca pronto per il backend: senza caratteri di controllo, da 1 a 100 caratteri.
/// Quanti brani di «Scarica tutto» scaricano insieme: abbastanza per tenere occupata la rete, pochi
/// per non soffocare il telefono.
const BATCH_PARALLEL: usize = 2;

/// Un «Scarica tutto» in corso.
#[derive(Debug, Default)]
pub struct DownloadBatch {
    pending: VecDeque<Track>,
    active: HashSet<Uuid>,
    total: usize,
    done: usize,
    failed: usize,
    last_error: Option<String>,
}

impl DownloadBatch {
    /// L'avviso finale: quanti brani sono arrivati e, se qualcuno no, l'ultimo errore.
    fn summary(&self) -> Notice {
        match (self.failed, &self.last_error) {
            (0, _) => Notice::Info(format!("Scaricati {} brani", self.done)),
            (_, Some(err)) => {
                Notice::Error(format!("Scaricati {} brani su {}. {err}", self.done, self.done + self.failed))
            }
            _ => Notice::Error(format!("Scaricati {} brani su {}", self.done, self.done + self.failed)),
        }
    }
}

/// La piattaforma scelta all'avvio (vedi `crate::run`).
static PLATFORM: std::sync::OnceLock<Platform> = std::sync::OnceLock::new();

pub fn set_platform(platform: Platform) {
    let _ = PLATFORM.set(platform);
}

/// L'app gira su un telefono (o lo simula, `PSYTIFY_PHONE=1`): impaginazione compatta.
pub fn is_phone() -> bool {
    PLATFORM.get() == Some(&Platform::Phone)
}

/// Le quattro schede del telefono e le schermate che vi appartengono; `None` per le pagine che si
/// aprono da qualunque scheda (album, artista, impostazioni).
pub fn phone_tab_of(screen: Screen) -> Option<Screen> {
    match screen {
        Screen::Home => Some(Screen::Home),
        Screen::Search | Screen::SourcePlaylist => Some(Screen::Search),
        Screen::Collection | Screen::Liked | Screen::Playlist(_) | Screen::Library => {
            Some(Screen::Collection)
        }
        Screen::News => Some(Screen::News),
        Screen::Album(_) | Screen::Artist(_) | Screen::Settings | Screen::Jam => None,
        Screen::SignIn | Screen::Friends | Screen::Profile | Screen::FriendPlaylist => Some(Screen::Profile),
    }
}

pub fn clean_query(raw: &str) -> Option<String> {
    let cleaned: String = raw.chars().filter(|c| !c.is_control()).collect();
    let trimmed: String = cleaned.trim().chars().take(MAX_QUERY_CHARS).collect();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// L'avviso dopo un download: dove è finito e, se la qualità non è quella chiesta, quale è arrivata.
fn saved_message(done: &api::Downloaded) -> String {
    let place = done.path.display();
    match done.delivered {
        Some(got) if done.is_lower_than_asked() => {
            format!(
                "Salvato in {place} · qualità {}, la migliore che il brano ha",
                got.label().to_lowercase()
            )
        }
        Some(got) => format!("Salvato in {place} · qualità {}", got.label().to_lowercase()),
        None => format!("Salvato in {place}"),
    }
}

fn download_dir() -> PathBuf {
    config::downloads_dir()
}

fn seed() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x5eed)
}

/// Le scorciatoie da tastiera; l'elenco per chi usa l'app è `ui::shortcuts::GROUPS`.
/// I tasti premuti dentro un campo di testo non arrivano qui: il campo li cattura.
fn shortcut(key: Key, modifiers: Modifiers) -> Option<Message> {
    let plain = !modifiers.command() && !modifiers.alt();
    let ctrl = modifiers.command() && !modifiers.alt();
    match key.as_ref() {
        Key::Named(Named::Space) => Some(Message::TogglePlay),
        Key::Named(Named::ArrowLeft) if modifiers.alt() => Some(Message::GoBack),
        Key::Named(Named::ArrowRight) if modifiers.alt() => Some(Message::GoForward),
        Key::Named(Named::ArrowLeft) if modifiers.command() => Some(Message::Previous),
        Key::Named(Named::ArrowRight) if modifiers.command() => Some(Message::Next),
        Key::Named(Named::ArrowUp) if modifiers.command() => Some(Message::VolumeStep(VOLUME_STEP)),
        Key::Named(Named::ArrowDown) if modifiers.command() => Some(Message::VolumeStep(-VOLUME_STEP)),
        Key::Named(Named::ArrowLeft) if modifiers.shift() => Some(Message::SeekBy(-SEEK_LONG_SECS)),
        Key::Named(Named::ArrowRight) if modifiers.shift() => Some(Message::SeekBy(SEEK_LONG_SECS)),
        Key::Named(Named::ArrowLeft) => Some(Message::SeekBy(-SEEK_STEP_SECS)),
        Key::Named(Named::ArrowRight) => Some(Message::SeekBy(SEEK_STEP_SECS)),
        Key::Named(Named::Escape) => Some(Message::Escape),
        Key::Named(Named::BrowserBack) => Some(Message::SystemBack),
        Key::Named(Named::F11) => Some(Message::ToggleFullscreen),
        Key::Character("f") if ctrl => Some(Message::FocusSearch),
        Key::Character("n") if ctrl => {
            Some(Message::OpenDialog(Dialog::NewPlaylist { name: String::new(), seed: Vec::new() }))
        }
        Key::Character(",") if ctrl => Some(Message::Navigate(Screen::Settings)),
        Key::Character(digit) if ctrl => match digit {
            "1" => Some(Message::Navigate(Screen::Home)),
            "2" => Some(Message::Navigate(Screen::Search)),
            "3" => Some(Message::Navigate(Screen::Library)),
            "4" => Some(Message::Navigate(Screen::Liked)),
            "5" => Some(Message::Navigate(Screen::News)),
            "6" => Some(Message::Navigate(Screen::Jam)),
            _ => None,
        },
        Key::Character(typed) if plain => match typed {
            "/" => Some(Message::FocusSearch),
            "?" => Some(Message::ToggleShortcuts),
            "f" => Some(Message::ToggleFocus),
            "v" => Some(Message::ToggleVideoFocus),
            "m" => Some(Message::ToggleMute),
            "l" => Some(Message::LikeCurrent),
            "s" => Some(Message::ToggleShuffle),
            "r" => Some(Message::CycleRepeat),
            "n" => Some(Message::Next),
            "p" => Some(Message::Previous),
            "q" => Some(Message::OpenPanel(SideTab::Queue)),
            "t" => Some(Message::OpenPanel(SideTab::Lyrics)),
            "i" => Some(Message::OpenPanel(SideTab::NowPlaying)),
            "+" | "=" => Some(Message::VolumeStep(VOLUME_STEP)),
            "-" => Some(Message::VolumeStep(-VOLUME_STEP)),
            digit => digit
                .parse::<u8>()
                .ok()
                .filter(|d| *d <= 9)
                .map(|d| Message::SeekFraction(f32::from(d) / 10.0)),
        },
        _ => None,
    }
}

/// I tasti laterali del mouse (indietro e avanti) muovono la cronologia, come in un browser.
/// Eventi che contano anche quando un widget li ha già presi: i tasti laterali del mouse, i
/// modificatori e le scorciatoie con Ctrl a caratteri (Ctrl+, Ctrl+1…6) dentro la ricerca, dove la
/// casella di testo cattura il tasto e `on_key_press` non lo vedrebbe.
fn global_event(event: iced::Event, status: event::Status, _window: window::Id) -> Option<Message> {
    match event {
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Back)) => Some(Message::GoBack),
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Forward)) => Some(Message::GoForward),
        iced::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
            Some(Message::ModifiersChanged(modifiers))
        }
        iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: key @ Key::Character(_),
            modifiers,
            ..
        }) if status == event::Status::Captured && modifiers.command() && !modifiers.alt() => {
            shortcut(key, modifiers)
        }
        _ => None,
    }
}

/// Il carattere aggiunto passando da `old` a `new`, se è stato aggiunto esattamente uno.
fn inserted_char(old: &str, new: &str) -> Option<char> {
    if new.chars().count() != old.chars().count() + 1 {
        return None;
    }
    let prefix = old.chars().zip(new.chars()).take_while(|(a, b)| a == b).count();
    let added = new.chars().nth(prefix)?;
    let rest_old: String = old.chars().skip(prefix).collect();
    let rest_new: String = new.chars().skip(prefix + 1).collect();
    (rest_old == rest_new).then_some(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_batch_summary_counts_what_arrived() {
        let batch = DownloadBatch { total: 3, done: 3, ..DownloadBatch::default() };
        assert!(matches!(batch.summary(), Notice::Info(text) if text == "Scaricati 3 brani"));
        let batch = DownloadBatch {
            total: 3,
            done: 2,
            failed: 1,
            last_error: Some("rete".into()),
            ..DownloadBatch::default()
        };
        assert!(matches!(batch.summary(), Notice::Error(text) if text.starts_with("Scaricati 2 brani su 3")));
    }

    #[test]
    fn one_inserted_character_is_found_anywhere() {
        assert_eq!(inserted_char("eminem", "eminem,"), Some(','));
        assert_eq!(inserted_char("emnem", "eminem"), Some('i'));
        assert_eq!(inserted_char("", "1"), Some('1'));
        assert_eq!(inserted_char("eminem", "eminem 2"), None);
        assert_eq!(inserted_char("abc", "xbcd"), None);
    }

    fn archive_item(identifier: &str) -> DiscoverItem {
        DiscoverItem {
            identifier: identifier.into(),
            title: identifier.into(),
            creator: None,
            licence: "cc-by".into(),
            year: None,
            cover_url: Some(format!("https://archive.org/services/img/{identifier}")),
            lossless: false,
        }
    }

    fn opened() -> OpenLibrary {
        let library = DiscoverLibrary {
            identifier: "label".into(),
            title: "Label".into(),
            creator: None,
            found: 1,
            cover_url: None,
        };
        OpenLibrary { library, items: Load::Loading, next_page: None, loading_more: false }
    }

    fn identifiers(open: &OpenLibrary) -> Vec<String> {
        open.items
            .ready()
            .map(|items| items.iter().map(|i| i.identifier.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn library_pages_are_appended_in_order() {
        let mut open = opened();
        let first = LibraryPage { items: vec![archive_item("a"), archive_item("b")], next_page: Some(2) };
        assert_eq!(open.receive(1, Ok(first)).unwrap().len(), 2);
        open.loading_more = true;
        open.receive(2, Ok(LibraryPage { items: vec![archive_item("c")], next_page: None })).unwrap();
        assert_eq!(identifiers(&open), ["a", "b", "c"]);
        assert_eq!((open.next_page, open.loading_more), (None, false));
    }

    #[test]
    fn a_failed_later_page_keeps_the_albums_and_can_be_retried() {
        let mut open = opened();
        open.receive(1, Ok(LibraryPage { items: vec![archive_item("a")], next_page: Some(2) })).unwrap();
        open.loading_more = true;
        assert!(open.receive(2, Err(AppError::Network)).is_err());
        assert_eq!(identifiers(&open), ["a"]);
        assert_eq!((open.next_page, open.loading_more), (Some(2), false));
    }

    #[test]
    fn a_failed_first_page_is_shown_in_place_of_the_albums() {
        let mut open = opened();
        assert_eq!(open.receive(1, Err(AppError::Network)).unwrap(), Vec::<String>::new());
        assert!(matches!(open.items, Load::Failed(_)));
    }

    #[test]
    fn only_the_latest_search_generation_is_current() {
        let mut search = SearchState::default();
        let first = search.input("lo".into());
        let second = search.input("low".into());
        assert!(!search.is_current(first) && search.is_current(second));
    }

    #[test]
    fn query_is_trimmed_and_cleaned() {
        assert_eq!(clean_query("  low\u{7}tide  "), Some("lowtide".into()));
    }

    #[test]
    fn blank_query_is_not_sent() {
        assert_eq!(clean_query(" \t "), None);
    }

    #[test]
    fn query_is_capped_at_100_characters() {
        assert_eq!(clean_query(&"è".repeat(150)).map(|q| q.chars().count()), Some(100));
    }

    #[test]
    fn space_toggles_playback() {
        assert!(matches!(shortcut(Key::Named(Named::Space), Modifiers::empty()), Some(Message::TogglePlay)));
        assert!(matches!(
            shortcut(Key::Named(Named::BrowserBack), Modifiers::empty()),
            Some(Message::SystemBack)
        ));
    }

    #[test]
    fn arrows_seek_five_seconds() {
        assert!(matches!(
            shortcut(Key::Named(Named::ArrowLeft), Modifiers::empty()),
            Some(Message::SeekBy(-5))
        ));
    }

    #[test]
    fn ctrl_f_focuses_search() {
        assert!(matches!(shortcut(Key::Character("f".into()), Modifiers::CTRL), Some(Message::FocusSearch)));
    }

    #[test]
    fn plain_f_opens_the_immersive_view() {
        assert!(matches!(
            shortcut(Key::Character("f".into()), Modifiers::empty()),
            Some(Message::ToggleFocus)
        ));
    }

    #[test]
    fn ctrl_arrows_change_track_and_volume() {
        let ctrl = |named| shortcut(Key::Named(named), Modifiers::CTRL);
        assert!(matches!(ctrl(Named::ArrowRight), Some(Message::Next)));
        assert!(matches!(ctrl(Named::ArrowLeft), Some(Message::Previous)));
        assert!(matches!(ctrl(Named::ArrowUp), Some(Message::VolumeStep(step)) if step > 0.0));
        assert!(matches!(ctrl(Named::ArrowDown), Some(Message::VolumeStep(step)) if step < 0.0));
    }

    #[test]
    fn single_letters_drive_the_player() {
        let plain = |c: &str| shortcut(Key::Character(c.into()), Modifiers::empty());
        assert!(matches!(plain("m"), Some(Message::ToggleMute)));
        assert!(matches!(plain("l"), Some(Message::LikeCurrent)));
        assert!(matches!(plain("s"), Some(Message::ToggleShuffle)));
        assert!(matches!(plain("r"), Some(Message::CycleRepeat)));
        assert!(plain("x").is_none());
    }

    #[test]
    fn letters_with_ctrl_are_not_player_shortcuts() {
        assert!(shortcut(Key::Character("m".into()), Modifiers::CTRL).is_none());
    }

    #[test]
    fn digits_jump_inside_the_track_and_ctrl_digits_navigate() {
        let key = |c: &str, modifiers| shortcut(Key::Character(c.into()), modifiers);
        assert!(matches!(key("0", Modifiers::empty()), Some(Message::SeekFraction(f)) if f == 0.0));
        assert!(
            matches!(key("7", Modifiers::empty()), Some(Message::SeekFraction(f)) if (f - 0.7).abs() < 1e-6)
        );
        assert!(matches!(key("1", Modifiers::CTRL), Some(Message::Navigate(Screen::Home))));
        assert!(matches!(key("6", Modifiers::CTRL), Some(Message::Navigate(Screen::Jam))));
        assert!(key("9", Modifiers::CTRL).is_none());
        assert!(matches!(key(",", Modifiers::CTRL), Some(Message::Navigate(Screen::Settings))));
    }

    #[test]
    fn alt_arrows_move_in_the_history_and_shift_arrows_seek_further() {
        let arrow = |named, modifiers| shortcut(Key::Named(named), modifiers);
        assert!(matches!(arrow(Named::ArrowLeft, Modifiers::ALT), Some(Message::GoBack)));
        assert!(matches!(arrow(Named::ArrowRight, Modifiers::ALT), Some(Message::GoForward)));
        assert!(
            matches!(arrow(Named::ArrowLeft, Modifiers::SHIFT), Some(Message::SeekBy(s)) if s == -SEEK_LONG_SECS)
        );
    }

    #[test]
    fn panels_volume_and_help_have_their_keys() {
        let plain = |c: &str| shortcut(Key::Character(c.into()), Modifiers::empty());
        assert!(matches!(plain("q"), Some(Message::OpenPanel(SideTab::Queue))));
        assert!(matches!(plain("t"), Some(Message::OpenPanel(SideTab::Lyrics))));
        assert!(matches!(plain("i"), Some(Message::OpenPanel(SideTab::NowPlaying))));
        assert!(matches!(plain("+"), Some(Message::VolumeStep(step)) if step > 0.0));
        assert!(matches!(plain("-"), Some(Message::VolumeStep(step)) if step < 0.0));
        assert!(matches!(plain("?"), Some(Message::ToggleShortcuts)));
        assert!(matches!(plain("/"), Some(Message::FocusSearch)));
        assert!(matches!(plain("n"), Some(Message::Next)));
    }

    #[test]
    fn going_back_skips_deleted_playlists_and_forward_returns() {
        let mut app = App::new().0;
        let gone = Uuid::from_u128(42);
        let _ = app.open(Screen::Search);
        let _ = app.open(Screen::Playlist(gone));
        let _ = app.open(Screen::Settings);
        let _ = app.go_back();
        assert_eq!(app.screen, Screen::Search, "the deleted playlist is skipped");
        let _ = app.go_forward();
        assert_eq!(app.screen, Screen::Settings);
        assert!(!app.can_go_forward());
    }

    #[test]
    fn a_jam_needs_the_tick_even_when_nothing_plays() {
        let mut app = App::new().0;
        assert!(!app.needs_tick());
        // Porta chiusa: il collegamento fallisce subito, ma intanto la sessione esiste.
        let link = Link::start("http://127.0.0.1:9", Target::Join("ABCDEF".into()), String::new());
        app.jam = Some(JamSession {
            link,
            code: None,
            you: None,
            members: Vec::new(),
            last_sent: None,
            synced: false,
            fetching: false,
        });
        assert!(!app.playback.is_playing());
        assert!(app.needs_tick(), "senza Tick la stanza non si legge mai e la jam resta in connessione");
    }

    #[test]
    fn library_filter_singles_excludes_album_tracks() {
        let single = crate::api::models::fixtures::track(1);
        let album_track = crate::api::models::fixtures::track(2);
        assert!(LibraryFilter::Singles.matches(&single) && !LibraryFilter::Singles.matches(&album_track));
    }
}
