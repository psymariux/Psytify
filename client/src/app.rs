//! Stato dell'applicazione e logica Elm: `Message`, `update`, `subscription`.
//!
//! Tutto lo stato vive in `App`. Le chiamate di rete e su disco partono con `Task::perform` e
//! tornano come `Message`; il thread della UI non si blocca mai.

mod artists;
mod catalog;
#[cfg(test)]
mod characterization;
mod chrome;
mod downloads;
mod external;
mod interface;
mod jam;
mod library;
mod listening;
mod menu;
mod playback;
mod settings;
mod social;
mod touch;
#[cfg(feature = "tour")]
mod tour;
mod video;

pub use artists::{ArtistsState, Discography};
pub use chrome::ResizeEdge;
pub use listening::{HERO_PERIOD, HERO_SWAP, Hero};
pub use menu::{ContextMenu, Dialog, MenuTarget, OpenDialog, TrackSource};
pub use psytify_core::catalog::{Load, clean_query};
pub use psytify_core::command::UpdateCheck;
pub use psytify_core::snapshot::OpenLibrary;
pub use psytify_core::snapshot::{ArtistExtras, ExternalView};
pub use psytify_core::snapshot::{UpdateState, UpdateStatus};
use settings::UiPrefs;
pub use social::{AuthField, AuthMode, SignInForm, SocialState};
pub use touch::{PhoneMsg, PhoneState};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::widget::text_input;
use iced::{Element, Point, Size, Subscription, Task, Theme, event, mouse, window};
use psytify_core::artwork::CoverPixels;
pub use psytify_core::command::SleepChoice;
use psytify_core::command::{CatalogCmd, Command, ExternalCmd, PlaybackCmd, Resource, SocialCmd};
use psytify_core::feed::Feed;
use psytify_core::shared::Shared;
pub use psytify_core::snapshot::{Account, SleepTimer};
use psytify_core::snapshot::{DownloadsView, JamView, PlaybackView};
use psytify_core::{AudioChoice, Core, CoreOptions, RuntimeChoice, Snapshot as CoreSnapshot, SystemChoice};
use uuid::Uuid;

use crate::Platform;
use crate::api::{
    Album, AlbumSummary, ApiClient, ArtistPage, DiscoverItem, DiscoverLibrary, DownloadQuality, MetaArtist,
    Quality, SearchResults, Sort, SourceItem, SourceKind, SourcePlaylist, Track,
};
use crate::auth::Provider;
use crate::config::{self, Config, FollowedArtist, PlayStat, Playlist, SavedAlbum};
use crate::error::AppError;
use crate::player::eq;
use crate::player::lyrics::{self as synced, Text};
use crate::player::{Queue, seek};
use crate::ui;
use crate::ui::cover::Covers;
use crate::ui::cursor;
use crate::ui::motion::{Curve, Tween};
use crate::ui::theme as tokens;
#[cfg(feature = "video")]
use video::OpenedVideo;
#[cfg(feature = "video")]
pub use video::VideoStage;

pub const SEARCH_INPUT: &str = "search";
/// Il campo del nome nelle finestre di dialogo (nuova playlist, rinomina).
pub const DIALOG_INPUT: &str = "dialog-name";
/// Quanto sposta il volume Ctrl+↑ o Ctrl+↓.
const VOLUME_STEP: f32 = 0.05;
/// Spostamento delle frecce sinistra e destra.
pub const SEEK_STEP_SECS: i64 = 5;
/// Schermate ricordate per «Indietro».
const HISTORY: usize = 50;
/// Spostamento di Maiusc+← e Maiusc+→.
const SEEK_LONG_SECS: i64 = 15;
/// Ogni quanto batte il Tick mentre suona.
const TICK: Duration = Duration::from_millis(250);
/// Quanto resta allargata la barra del volume dopo l'ultimo tocco.
const VOLUME_LINGER: Duration = Duration::from_millis(1800);
/// Quanto deve restare fermo il mouse su un brano perché si prepari la sua testata.
const HOVER_INTENT: Duration = Duration::from_millis(180);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Search,
    Library,
    Album(Uuid),
    Playlist(Uuid),
    Artist(Uuid),
    Settings,
    /// I brani scaricati sul dispositivo e i download in corso.
    Downloads,
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
    /// Il profilo aperto (`SocialView::open`, nel core).
    Profile,
    /// La playlist pubblica di un amico (`SocialView::playlist`, nel core).
    FriendPlaylist,
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

/// La Home: le tre sezioni sono la copia della vista del catalogo, il feed quella della libreria.
#[derive(Debug, Default)]
pub struct HomeState {
    pub recent: Load<Arc<[Track]>>,
    pub alphabetical: Load<Arc<[Track]>>,
    pub albums: Load<Arc<[AlbumSummary]>>,
    /// Uscite, novità, mix e artisti ricavati dai brani recenti: lo costruisce il core
    /// (`LibraryCmd::HomeTracks`, che gli dà il catalogo) e qui c'è la copia della sua vista.
    pub feed: Arc<Feed>,
    /// L'uscita in evidenza nel carosello.
    pub hero: Hero,
}

/// La ricerca. Il testo che si scrive è del campo (`query`); i risultati sono la copia della vista
/// del catalogo, che aspetta 300 ms dopo l'ultimo tasto e tiene solo l'ultima richiesta.
#[derive(Debug, Default)]
pub struct SearchState {
    pub query: String,
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

/// L'elenco della libreria: la copia della vista del catalogo, più il filtro, che è della vista.
#[derive(Debug, Default)]
pub struct LibraryState {
    pub tracks: Load<Shared<Track>>,
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
    /// Un posto dell'interfaccia mostra il videoclip: il core lo chiede al server solo allora.
    show_video: bool,
    menu: bool,
    dialog: bool,
    /// Il brano in corso e se suona: quando cambia, la copertina grande si muove.
    media: Option<(Uuid, bool)>,
    side_tab: SideTab,
    /// Un frontend mostra il testo: il core lo carica solo allora.
    lyrics: bool,
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
    SearchInput(String),
    /// Ctrl, Alt e Maiusc premuti: servono a non scrivere nella ricerca i tasti delle scorciatoie.
    ModifiersChanged(Modifiers),
    ArchiveLibraryOpen(DiscoverLibrary),
    ArchiveLibraryMore,
    ArchiveLibraryClose,
    /// Apre la pagina di un artista del catalogo (il server gli dà un identificatore).
    OpenMetaArtist(u64),
    /// Apre una playlist esterna e ne carica i brani.
    OpenSourcePlaylist(SourcePlaylist),
    /// Salva la playlist esterna aperta nella libreria.
    SaveSourcePlaylist,
    /// Ascolta la playlist esterna aperta da questo brano in poi.
    PlaySourcePlaylist(usize),
    /// Ascolta i brani popolari di un artista dal catalogo, da questo in poi.
    PlayArtistTop(Uuid, usize),
    PlayDiscovered(String),
    PlaySource(SourceKind, String),
    /// Copertina scaricata e decodificata; `None` se non si può mostrare.
    CoverLoaded(String, Option<CoverPixels>),
    /// Chiede l'ultima release: all'avvio, dal timer o con «Controlla ora».
    UpdateCheck(UpdateCheck),
    /// Chiude il banner dell'aggiornamento (torna con la prossima release).
    UpdateDismiss,
    UpdateRequested,
    /// Apre la cartella del pacchetto scaricato, per installarlo a mano (non su Android).
    UpdateReveal,
    /// Filtra la discografia dell'artista aperto.
    ArtistFilter(Discography),
    /// «Popolari» mostra più brani, o torna a cinque.
    ArtistMorePopular,
    LibraryMore,
    LibraryFilter(LibraryFilter),
    /// Cambia l'ordine del Catalogo e lo ricarica.
    LibrarySort(Sort),
    /// Apre la libreria su un filtro (per esempio "Vedi tutti gli album" dalla Home).
    OpenLibrary(LibraryFilter),
    PlayTracks(Arc<[Track]>, usize),
    /// Riproduce i brani in ordine casuale, da un brano a caso.
    ShufflePlay(Arc<[Track]>),
    PlayQueueAt(usize),
    /// Il mouse è entrato o uscito da un elemento; se è un brano e si ferma, se ne prepara la testata.
    Hover(Option<Hovered>),
    /// Il mouse è uscito dall'elemento: conta solo se è ancora quello sotto il mouse. Due righe
    /// vicine possono mandare ingresso e uscita in ordine inverso; senza l'identità, l'uscita della
    /// riga di prima spegne l'hover della riga nuova.
    HoverEnd(Uuid),
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
    /// La barra del volume dei player a schermo intero: si allarga mentre la si usa.
    VolumeSide(f32),
    VolumeSideReleased,
    /// La barra torna a filo, se nessuno l'ha toccata di nuovo (generazione del tocco).
    VolumeCollapse(u64),
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
    /// Avvia una radio da un brano (voce del menu del brano).
    StartRadio(Box<Track>),
    /// Riproduce il mix `n` della Home.
    PlayMix(usize),
    /// Porta il carosello della Home a un'uscita, o alla successiva.
    HeroGo(usize),
    HeroNext,
    /// Gesti del telefono: foglio del player, azioni a scorrimento, riordino della coda.
    Phone(PhoneMsg),
    /// Imposta o toglie il timer di spegnimento.
    SetSleep(Option<SleepChoice>),
    /// Riproduzione automatica a fine coda.
    SetAutoplay(bool),
    /// I videoclip sulla scheda video dedicata (`true`) o sulla GPU integrata.
    SetDedicatedVideo(bool),
    /// La scheda video dedicata trovata all'avvio, col suo nome.
    GpuDetected(Option<String>),
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
    /// Il fattore di scala della finestra: serve a ingrandire gli sprite di un fattore intero.
    ScaleFactor(f32),
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
    SetDownloadQuality(DownloadQuality),
    /// Ferma un download in corso o toglie dall'elenco uno fallito o in attesa.
    DownloadCancel(Uuid),
    DownloadRetry(Uuid),
    /// Toglie un brano dal dispositivo (la voce subito, il file se è ancora quello scaricato).
    DownloadRemove(Uuid),
    /// Controlla a fondo i file scaricati.
    DownloadsVerify,
    DownloadsOpenFolder,
    /// Mostra altre righe dell'elenco dei brani sul dispositivo.
    DownloadsShowMore,
    DismissNotice,
    FocusSearch,
    CloseRequested(window::Id),
    Exit(window::Id),
    /// Il core ha pubblicato un'istantanea nuova.
    Core(Arc<CoreSnapshot>),
    SetQuality(Quality),
    /// Ricerca YouTube: tenere remix ed edit (`true`) o preferire l'originale.
    /// Apre «Accedi a Psytify».
    SignIn,
    AuthMode(AuthMode),
    AuthInput(AuthField, String),
    AuthSubmit,
    AuthProvider(Provider),
    AuthResend,
    /// Dal codice torna ai campi.
    AuthBack,
    SignOut,
    /// Nome utente o nome visibile del proprio profilo, nelle impostazioni.
    ProfileHandle(String),
    ProfileName(String),
    /// Salva il profilo; con `Some` cambia anche la condivisione dell'attività.
    ProfileSave(Option<bool>),
    /// Schermata Jam: sceglie o toglie un amico da invitare, manda gli inviti, entra o rifiuta.
    JamInviteToggle(String),
    JamInviteSend,
    JamInviteAccept(String),
    JamInviteDismiss(String),
    UserQuery(String),
    UserSearch,
    OpenProfile(String),
    /// Chiede o accetta l'amicizia (`true`), oppure la toglie, la ritira o la rifiuta.
    FriendAction(String, bool),
    OpenFriendPlaylist(String, Uuid),
    /// Rende pubblica o privata una playlist locale.
    PlaylistPublic(Uuid, bool),
    /// Mostra su Discord cosa si ascolta.
    SetDiscord(bool),
    /// Sul telefono la finestra ha perso il fuoco: l'app sta andando in background, dove Android la
    /// può chiudere. Il core scrive subito ciò che aspettava il salvataggio (BACKLOG 12).
    Backgrounded,
    /// Nel pannello «In riproduzione» il videoclip (`true`) o la copertina (`false`).
    SetPanelVideo(bool),
    /// Riprova il videoclip che non si era aperto.
    RetryVideo,
    /// Apre o chiude il videoclip a tutto schermo, con il testo sopra.
    ToggleVideoFocus,
    /// Mostra o nasconde il testo sopra il videoclip.
    ToggleVideoLyrics,
    /// Il videoclip `seq` del brano, aperto su un altro thread, pronto da mostrare.
    #[cfg(feature = "video")]
    VideoOpened(Uuid, u64, Result<OpenedVideo, AppError>),
    /// La versione migliore `seq` del videoclip, aperta di fianco a quella che suona.
    #[cfg(feature = "video")]
    VideoUpgraded(Uuid, u64, Result<OpenedVideo, AppError>),
    /// Lo stream del videoclip è caduto: lo sa il core, che decide se riprovare.
    #[cfg(feature = "video")]
    VideoFailed,
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

pub struct App {
    /// Il core senza interfaccia (`psytify_core`): gira sul runtime di Iced e si ferma alla
    /// chiusura, prima dell'uscita. `None` nei test e se non è partito.
    core: Option<Core>,
    /// L'ultima istantanea pubblicata dal core.
    core_snapshot: Arc<CoreSnapshot>,
    pub api: Result<ApiClient, AppError>,
    /// Backend in uso (variabile d'ambiente, scelta dell'utente o predefinito).
    api_base_url: String,
    /// Backend scelto nelle impostazioni; `None` = il predefinito della build.
    pub server: Option<String>,
    /// Indirizzo che si sta scrivendo nelle impostazioni.
    pub server_draft: String,
    /// La riproduzione come la pubblica il core (`PlaybackView`): suono, coda, brani preparati,
    /// timer di spegnimento e riproduzione automatica stanno nel core, che possiede l'uscita audio.
    /// Sono gli stessi `Arc` della sua vista, e `App::playback_published` li rinnova a ogni
    /// pubblicazione. Si cambiano con `PlaybackCmd`.
    pub playback: Arc<PlaybackView>,
    pub queue: Arc<Queue>,
    /// La frazione del brano su cui si sta trascinando lo slider (la posizione vera non si tocca
    /// finché non lo si rilascia).
    pub scrubbing: Option<f32>,
    /// L'ultimo evento della riproduzione già mostrato (`Raised::seq`).
    playback_event: u64,
    /// Brano sotto il mouse e generazione del passaggio, per l'attesa prima di prepararlo.
    hover_generation: u64,
    pub screen: Screen,
    pub home: HomeState,
    pub search: SearchState,
    pub library: LibraryState,
    /// L'album e l'artista aperti, com'è nella vista del catalogo.
    pub album: Option<(Uuid, Load<Arc<Album>>)>,
    pub artist: Option<(Uuid, Load<Arc<ArtistPage>>)>,
    /// Le scelte di vista della pagina artista (le pagine in memoria stanno nel core).
    pub artists: ArtistsState,
    /// L'ultimo evento del catalogo già mostrato (`CatalogRaised::seq`).
    catalog_event: u64,
    /// Le fonti esterne come le pubblica il core (`ExternalView`): la playlist esterna aperta, ciò
    /// che gli artisti hanno nel catalogo ufficiale e le importazioni in corso. È la stessa `Arc`
    /// della sua vista, e `App::external_published` la rinnova a ogni pubblicazione. Si cambia con
    /// `ExternalCmd`.
    pub external: Arc<ExternalView>,
    /// L'ultimo evento delle fonti esterne già mostrato (`ExternalRaised::seq`).
    external_event: u64,
    /// La libreria, come la pubblica il core (`LibraryView`): sono gli stessi `Arc` della sua vista,
    /// e `App::library_published` li rinnova a ogni pubblicazione. Si cambiano con `LibraryCmd`.
    pub playlists: Shared<Playlist>,
    /// I brani a cui si è messo "mi piace", il più recente in testa.
    pub liked: Shared<Track>,
    /// Gli ultimi brani ascoltati, il più recente in testa.
    pub recent: Shared<Track>,
    /// Artisti seguiti, album salvati e ascolti: da qui nascono «Novità», «I tuoi artisti» e i mix.
    pub followed: Shared<FollowedArtist>,
    pub saved_albums: Shared<SavedAlbum>,
    pub stats: Shared<PlayStat>,
    /// Tempo ascoltato in tutto, in millisecondi, com'è nella libreria del core: lo conta la
    /// riproduzione dalla posizione vera dell'audio e lo consegna a gruppi (ogni 15 secondi, alla
    /// pausa, al cambio di brano).
    pub listened_ms: u64,
    /// Ultima uscita vista in «Novità»: ciò che è più recente è da leggere.
    pub news_seen: String,
    /// Una playlist appena creata da aprire quando il core la pubblica.
    pending_playlist: Option<Uuid>,
    /// Il core dei test: lo stesso riduttore, nel thread del test, senza core vero.
    #[cfg(test)]
    local_core: Option<psytify_core::testing::LocalCore>,
    /// A fine coda continua con brani dello stesso artista (copia della vista del core).
    pub autoplay: bool,
    /// Vedi `Config::dedicated_video`.
    pub dedicated_video: bool,
    /// La scheda video dedicata che può decodificare i videoclip: se non c'è, l'opzione non si mostra.
    pub dedicated_gpu: Option<String>,
    /// Che cosa mostra la libreria nella barra laterale (non si salva).
    pub sidebar_filter: SidebarFilter,
    /// La vista immersiva «In riproduzione» è aperta.
    pub focus: bool,
    /// Il timer di spegnimento, se è attivo (copia della vista del core).
    pub sleep: Option<SleepTimer>,
    /// Menu contestuale e finestra di dialogo aperti.
    pub menu: Option<ContextMenu>,
    pub dialog: Option<OpenDialog>,
    /// Il cuore appena premuto: l'icona fa un piccolo rimbalzo.
    pub like_pop: Option<(Uuid, Instant)>,
    pub panel_open: bool,
    /// Apertura del pannello laterale, da 0 (chiuso) a 1.
    pub panel_motion: Tween,
    /// La barra del volume dei player a schermo intero: 0 a filo, 1 allargata.
    pub volume_motion: Tween,
    volume_touch: u64,
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
    /// Ultime ricerche, la più recente in testa (si salvano).
    pub search_history: Shared<String>,
    /// Schermate visitate, per «Indietro» e «Avanti».
    back: Vec<Screen>,
    forward: Vec<Screen>,
    /// Il riquadro delle scorciatoie da tastiera, aperto da quando.
    pub shortcuts: Option<Instant>,
    /// Quanti videoclip si sono fermati, l'ultima volta che il core l'ha detto: l'avviso si mostra
    /// una volta per ciascuno.
    #[cfg(feature = "video")]
    video_failures: u64,
    /// Il pannello «In riproduzione» mostra il videoclip, se il brano ne ha uno (si salva).
    pub panel_video: bool,
    /// Brani già riletti dal server in questa sessione: le copie salvate possono essere vecchie.
    /// La posizione del testo a tutto schermo, in righe: scorre verso la riga in corso.
    pub lyrics_scroll: Tween,
    /// La vista a tutto schermo mostra il videoclip invece del testo.
    pub focus_video: bool,
    /// Testo sopra il videoclip.
    pub video_lyrics: bool,
    /// Ingresso della pagina corrente, da 0 a 1.
    pub page_motion: Tween,
    pub equalizer: eq::Settings,
    /// Scheda aperta nel pannello laterale (non si salva: all'avvio si apre la coda).
    pub side_tab: SideTab,
    /// Scheda aperta nelle impostazioni (non si salva).
    pub settings_tab: SettingsTab,
    /// Il testo del brano in corso, come lo pubblica il core (caricato solo mentre un frontend lo
    /// mostra: `lyrics_wanted`), con la riga in corso per la vista sincronizzata.
    pub lyrics: Load<Arc<Text>>,
    pub lyrics_line: Option<usize>,
    /// La jam come la pubblica il core (`JamView`): il collegamento con la stanza, i membri e
    /// l'allineamento alla coda del gruppo stanno nel core dal passo 4.8. Si cambia con `JamCmd`.
    pub jam: Arc<JamView>,
    /// Il numero dell'ultimo evento della jam già detto all'utente.
    jam_event: u64,
    /// Nome con cui si entra nella jam, e codice digitato per entrare in una esistente.
    pub jam_name: String,
    pub jam_code: String,
    /// La vista dei download del core: la coda con l'avanzamento di ogni brano, il lotto, la qualità
    /// e l'aggiornamento offerto (banner e Informazioni).
    pub downloads: Arc<DownloadsView>,
    /// Il numero dell'ultimo evento dei download già detto all'utente.
    downloads_event: u64,
    /// I brani sul dispositivo come lista di brani, nello stesso ordine dell'indice: si rifà quando
    /// l'indice cambia, non a ogni vista.
    pub downloaded: Arc<[Track]>,
    /// L'indice del core da cui è stata fatta `downloaded`.
    downloaded_from: psytify_core::shared::Shared<psytify_core::offline::SavedTrack>,
    /// Quante righe dell'elenco dei brani sul dispositivo si mostrano (la pagina cresce a blocchi).
    pub downloads_rows: usize,
    /// Il cassetto del telefono è aperto dall'istante indicato.
    pub drawer: Option<Instant>,
    /// Stato dei gesti del telefono (foglio del player).
    pub phone: PhoneState,
    /// Copertine scaricate, per URL.
    pub covers: Covers,
    pub notice: Option<Notice>,
    pub toast: Option<Toast>,
    toast_generation: u64,
    pub quality: Quality,
    pub social: SocialState,
    pub discord_presence: bool,
    discord_app_id: String,
    clerk_issuer: String,
    clerk_client_id: String,
    oauth_port: u16,
    /// Il player del videoclip aperto: il videoclip lo sceglie il core, il player è di GStreamer.
    #[cfg(feature = "video")]
    pub video: Option<VideoStage>,
    #[cfg(feature = "tour")]
    tour: Option<tour::Tour>,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        // Il core parte per primo: legge lui `config.json`, e al ritorno la sua prima istantanea
        // c'è già. Senza core l'app non salva niente: mostra almeno ciò che c'è sul disco.
        let core = start_core(config::dir(), Some(config::downloads_dir()));
        let config = match &core {
            Some(core) => {
                let snapshot = core.snapshot();
                Config { core: (*snapshot.settings.config).clone(), ui: (*snapshot.settings.ui).clone() }
            }
            None => config::load(config::path().as_deref()),
        };
        let api_base_url = config::api_url(config.core.server.as_deref());
        let api = ApiClient::new(&api_base_url);
        let last_screen = UiPrefs::from_map(&config.ui).last_screen;
        let mut app = Self::from_config(config, api_base_url, api);
        if let Some(core) = core {
            app.core_snapshot = core.snapshot();
            app.core = Some(core);
            app.copy_library();
            app.copy_catalog();
            app.copy_external();
            app.copy_playback();
            app.copy_downloads();
            app.copy_social();
            app.copy_jam();
        }
        // L'uscita audio la apre il core, sul suo thread: se non si è aperta, la vista lo dice.
        let audio_error = app.playback.device_error.clone();
        let mut screen = Screen::from(last_screen);
        if let Screen::Playlist(id) = screen
            && !app.playlists.iter().any(|p| p.id == id)
        {
            screen = Screen::Home;
        }
        // La Home si carica sempre (è la prima pagina e il feed dipende da lei); il core serve il resto.
        let home = app.send_catalog(CatalogCmd::Ensure(Resource::Home));
        let restore = app.open(screen);
        let search = if screen == Screen::Search {
            app.send_catalog(CatalogCmd::Search(app.search.query.clone()))
        } else {
            Task::none()
        };
        // Le playlist salvate con brani ancora da importare riprendono da sole: ci pensa il core.
        // La coda ripresa dall'ultima sessione: le sue copertine servono subito.
        let urls: Vec<String> = app.queue.tracks_in_order().into_iter().filter_map(|t| t.cover_url).collect();
        let covers = app.fetch_covers(urls);
        // "Continua ad ascoltare" è a un clic: il brano ripreso ha già la testata pronta.
        let resume = match app.queue.current().map(|t| t.id) {
            Some(id) => app.send_core(Command::Playback(PlaybackCmd::Warm(id))),
            None => Task::none(),
        };
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
        // Il pannello riaperto sul videoclip del brano ripreso: il core lo chiede subito, altrimenti
        // resta su «Preparo il videoclip…» finché qualcosa non cambia.
        let photos = app.video_wants();
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
        (
            app,
            Task::batch([
                tour,
                photos,
                gpu,
                home,
                restore,
                search,
                covers,
                resume,
                notice,
                window::get_oldest().map(Message::WindowReady),
                window::get_oldest().and_then(window::get_scale_factor).map(Message::ScaleFactor),
            ]),
        )
    }

    /// Lo stato iniziale dell'app da una configurazione già letta, senza effetti: nessuna
    /// richiesta, nessun file, nessun thread. L'avvio vero e proprio (caricamenti, sessione,
    /// Discord, controllo degli aggiornamenti) lo aggiunge `new`.
    fn from_config(config: Config, api_base_url: String, api: Result<ApiClient, AppError>) -> Self {
        let prefs = UiPrefs::from_map(&config.ui);
        let config = config.core;
        let mut app = Self {
            core: None,
            core_snapshot: Arc::default(),
            notice: None,
            toast: None,
            toast_generation: 0,
            api,
            server_draft: config.server.clone().unwrap_or_default(),
            server: config.server,
            api_base_url,
            queue: Arc::clone(&config.queue),
            playback: Arc::default(),
            scrubbing: None,
            playback_event: 0,
            hovered: None,
            hover_generation: 0,
            window: None,
            window_size: Size::new(1280.0, 800.0),
            insets: None,
            fullscreen: false,
            resizing: None,
            last_title_press: None,
            modifiers: Modifiers::default(),
            search_history: config.search_history,
            back: Vec::new(),
            forward: Vec::new(),
            shortcuts: None,
            #[cfg(feature = "video")]
            video_failures: 0,
            panel_video: prefs.panel_video,
            lyrics_scroll: Tween::rest(-1.0),
            focus_video: false,
            video_lyrics: true,
            liked: config.liked,
            recent: config.recent,
            followed: config.followed,
            saved_albums: config.saved_albums,
            stats: config.stats,
            listened_ms: config.listened_ms,
            news_seen: config.news_seen,
            pending_playlist: None,
            #[cfg(test)]
            local_core: None,
            autoplay: config.autoplay,
            dedicated_video: prefs.dedicated_video,
            dedicated_gpu: None,
            focus: false,
            sidebar_filter: SidebarFilter::default(),
            sleep: None,
            menu: None,
            dialog: None,
            like_pop: None,
            screen: Screen::Home,
            home: HomeState::default(),
            search: SearchState::default(),
            library: LibraryState::default(),
            album: None,
            artist: None,
            artists: ArtistsState::default(),
            catalog_event: 0,
            external: Arc::default(),
            external_event: 0,
            playlists: config.playlists,
            panel_open: prefs.panel_open,
            panel_motion: Tween::rest(if prefs.panel_open { 1.0 } else { 0.0 }),
            volume_motion: Tween::rest(0.0),
            volume_touch: 0,
            page_motion: Tween::rest(1.0),
            cover_motion: Tween::rest(tokens::COVER_REST).with_curve(Curve::Spring),
            side_tab_motion: Tween::rest(SideTab::default().index()).with_curve(Curve::Spring),
            splash: Some(Splash { since: Instant::now(), revealed: false }),
            play_pop: None,
            equalizer: config.equalizer,
            side_tab: SideTab::default(),
            settings_tab: SettingsTab::default(),
            lyrics: Load::Idle,
            lyrics_line: None,
            jam: Arc::default(),
            jam_event: 0,
            jam_name: String::new(),
            jam_code: String::new(),
            downloads: Arc::default(),
            downloads_event: 0,
            downloaded: Arc::from([]),
            downloaded_from: psytify_core::shared::Shared::default(),
            downloads_rows: crate::ui::pages::downloads::PAGE_ROWS,
            drawer: None,
            phone: PhoneState::default(),
            covers: Covers::default(),
            quality: config.quality,
            social: SocialState::default(),
            discord_presence: config.discord_presence,
            discord_app_id: config.discord_app_id,
            clerk_issuer: config.clerk_issuer,
            clerk_client_id: config.clerk_client_id,
            oauth_port: config.oauth_port,
            #[cfg(feature = "video")]
            video: None,
            #[cfg(feature = "tour")]
            tour: None,
        };
        app.search.query = config.last_query;
        app
    }

    /// Un'app per i test, chiusa al mondo esterno: configurazione di default (il `config.json`
    /// dell'utente non si legge), cartella dei dati temporanea, backend e login Clerk su una porta
    /// chiusa di 127.0.0.1, nessuna uscita audio, nessun Discord e nessuna richiesta all'avvio.
    /// Nessun core vero, quindi niente si salva: la riproduzione è quella del core sincrono dei test
    /// (`LocalCore`), con un'uscita audio staccata se serve il suono ([`App::for_tests_with`]); chi
    /// vuole il core vero, [`App::for_tests_with_core`].
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self::for_tests_with(None, |_| ())
    }

    /// Come [`App::for_tests`], con un'uscita audio staccata nel core dei test (`audio`) e la
    /// configurazione ritoccata da `tweak`, per esempio con una coda ripresa dall'ultima sessione.
    #[cfg(test)]
    pub(crate) fn for_tests_with(
        audio: Option<crate::player::AudioOutput>,
        tweak: impl FnOnce(&mut config::CoreConfig),
    ) -> Self {
        let data = test_data_dir();
        assert!(
            config::dir().is_some_and(|dir| dir == data),
            "i test scrivono solo nella loro cartella temporanea, mai nei dati dell'utente"
        );
        let mut core = config::CoreConfig { clerk_issuer: TEST_CLERK_ISSUER.into(), ..Default::default() };
        tweak(&mut core);
        let config = Config { core, ..Config::default() };
        let api = ApiClient::new(TEST_API_URL);
        // Senza release: un build di distribuzione (`PSYTIFY_BUILD_TAG`) non deve far partire controlli.
        let mut local = psytify_core::testing::LocalCore::new(config.clone(), 1).with_build(None);
        if let Some(audio) = audio {
            local = local.with_audio(audio);
        }
        let mut app = Self::from_config(config, TEST_API_URL.into(), api);
        app.core_snapshot = local.snapshot();
        app.local_core = Some(local);
        app.copy_playback();
        app.copy_downloads();
        app.copy_social();
        app.copy_jam();
        app
    }

    /// Come [`App::for_tests`], con un core vero sul runtime del test: `config.json` lo legge e lo
    /// scrive lui, solo in `data_dir` (con `None` tiene tutto in memoria). Va chiamata dentro un
    /// runtime di tokio.
    #[cfg(test)]
    pub(crate) fn for_tests_with_core(data_dir: Option<PathBuf>) -> Self {
        let mut app = Self::for_tests();
        let runtime = RuntimeChoice::Borrow(tokio::runtime::Handle::current());
        // Il core vero apre un'uscita staccata (mai la scheda audio del computer che prova) e parla
        // con il backend di prova, su una porta chiusa: nessun test arriva al backend vero.
        if let Some(dir) = &data_dir {
            let _ = std::fs::create_dir_all(dir);
            let file = config::file_in(dir);
            if !file.exists() {
                let _ = std::fs::write(&file, format!(r#"{{"server":"{TEST_API_URL}"}}"#));
            }
        }
        let audio = AudioChoice::Detached;
        // I download dei test finiscono nella cartella dei dati del test, mai in quella dell'utente.
        let downloads_dir = data_dir.as_ref().map(|dir| dir.join("Download"));
        let core =
            Core::start(CoreOptions { runtime, audio, system: SystemChoice::None, data_dir, downloads_dir })
                .expect("il core dei test parte");
        app.core_snapshot = core.snapshot();
        // Il backend è quello che il core ha letto: se l'app ne mandasse un altro, il catalogo
        // lo vedrebbe come un cambio e ricaricherebbe la Home.
        app.server.clone_from(&app.core_snapshot.settings.config.server);
        app.local_core = None;
        app.core = Some(core);
        app.copy_library();
        app.copy_catalog();
        app.copy_external();
        app.copy_playback();
        app.copy_downloads();
        app.copy_social();
        app.copy_jam();
        app
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
            window::close_requests().map(Message::CloseRequested),
            window::resize_events().map(|(_, size)| Message::WindowResized(size)),
            // Ricorda dov'è il mouse (per il menu contestuale) senza produrre messaggi.
            event::listen_with(cursor::track),
        ];
        if let Some(core) = &self.core {
            // Un solo abbonamento per tutto il core: produce un messaggio solo quando pubblica.
            subscriptions.push(Subscription::run_with_id("psytify-core", core_changes(core.changes())));
        }
        if self.resizing.is_some() {
            subscriptions.push(event::listen_with(chrome::resize_events));
        }
        if is_phone() {
            subscriptions.push(event::listen_with(background_event));
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
        if self.hero_rotates() {
            subscriptions.push(iced::time::every(HERO_PERIOD).map(|_| Message::HeroNext));
        }
        Subscription::batch(subscriptions)
    }

    /// Tiene l'istantanea più nuova del core e passa a ogni dominio la sua vista di prima, se è
    /// cambiata (`Arc::ptr_eq`): solo allora il dominio reagisce. Un dominio che la UI legge vi
    /// copierà anche il suo `Arc` (`docs/transformation/04-core-architecture.md`, passo 4).
    fn core_published(&mut self, snapshot: Arc<CoreSnapshot>) -> Task<Message> {
        if snapshot.version <= self.core_snapshot.version {
            return Task::none();
        }
        let previous = std::mem::replace(&mut self.core_snapshot, snapshot);
        if !Arc::ptr_eq(&previous.settings, &self.core_snapshot.settings) {
            self.settings_published(&previous.settings);
        }
        let mut tasks = Vec::new();
        if !Arc::ptr_eq(&previous.library, &self.core_snapshot.library) {
            tasks.push(self.library_published(&previous.library));
        }
        if !Arc::ptr_eq(&previous.catalog, &self.core_snapshot.catalog) {
            // Dopo la libreria: la Home arriva con il suo feed nella stessa istantanea.
            tasks.push(self.catalog_published(&previous.catalog));
        }
        if !Arc::ptr_eq(&previous.external, &self.core_snapshot.external) {
            tasks.push(self.external_published(&previous.external));
        }
        if !Arc::ptr_eq(&previous.playback, &self.core_snapshot.playback) {
            tasks.push(self.playback_published(&previous.playback));
        }
        if !Arc::ptr_eq(&previous.downloads, &self.core_snapshot.downloads) {
            self.downloads_published();
        }
        if !Arc::ptr_eq(&previous.social, &self.core_snapshot.social) {
            tasks.push(self.social_published(&previous.social));
        }
        if !Arc::ptr_eq(&previous.jam, &self.core_snapshot.jam) {
            self.jam_published(&previous.jam);
        }
        // Dopo la riproduzione: il player si apre all'orologio che la riproduzione ha appena dato.
        #[cfg(feature = "video")]
        if !Arc::ptr_eq(&previous.video, &self.core_snapshot.video) {
            tasks.push(self.video_published());
        }
        Task::batch(tasks)
    }

    /// Il Tick serve solo mentre suona un brano: ridisegna la barra di avanzamento (la posizione la
    /// dà l'orologio del core). Una jam non lo vuole: gli eventi della stanza entrano nel core da
    /// soli e arrivano come istantanee (dal passo 4.8), anche a player fermo.
    fn needs_tick(&self) -> bool {
        self.playback.is_playing()
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

    /// La posizione del brano adesso, dall'orologio che il core pubblica (la posizione non sta
    /// nell'istantanea: avanza da sola e il core la riancora solo se serve).
    pub fn position(&self) -> Duration {
        self.playback.position(Instant::now())
    }

    /// Posizione mostrata: durante il trascinamento segue lo slider.
    pub fn shown_position(&self) -> Duration {
        match (self.scrubbing, self.queue.current()) {
            (Some(fraction), Some(track)) => seek::seek_fraction(fraction, track.duration()),
            _ => self.position(),
        }
    }

    /// Gestisce il messaggio, poi avvia i movimenti di ciò che è cambiato: pagina, pannello,
    /// avviso. Così nessun punto che cambia schermata o avviso deve ricordarsi dell'animazione.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let before = self.observed();
        let task = self.handle(message);
        let after = self.observed();
        if before == after {
            return task;
        }
        let keyboard = Task::none();
        if before.screen != after.screen {
            self.phone_page_changed(before.screen, after.screen);
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
        let video = if before.show_video == after.show_video {
            Task::none()
        } else {
            self.show_video(after.show_video)
        };
        let notice =
            if before.notice == after.notice { Task::none() } else { self.show_notice(after.notice) };
        let lyrics =
            if before.lyrics == after.lyrics { Task::none() } else { self.want_lyrics(after.lyrics) };
        Task::batch([task, notice, video, keyboard, lyrics])
    }

    fn observed(&self) -> Observed {
        Observed {
            screen: self.screen,
            panel_open: self.panel_open,
            notice: self.notice.clone(),
            show_video: self.video_surface(),
            lyrics: self.lyrics_wanted(self.wanted_video()),
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
            || self.volume_motion.is_running()
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
            || self.drawer.is_some_and(|since| since.elapsed() < tokens::MOTION_SPRING)
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

    /// Smista il messaggio al file del suo dominio (`app/<dominio>.rs`), così il lavoro su un
    /// dominio non tocca gli altri (`docs/transformation/04-core-architecture.md`, passo 4.0).
    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Core(snapshot) => self.core_published(snapshot),
            Message::ServerDraft(..)
            | Message::ServerApply
            | Message::ServerReset
            | Message::SetDiscord(..)
            | Message::Backgrounded => self.handle_settings(message),
            Message::ToggleLike(..)
            | Message::ToggleFollow(..)
            | Message::ToggleSaveAlbum(..)
            | Message::AddToPlaylist(..)
            | Message::RemoveFromPlaylist(..)
            | Message::DialogSubmit
            | Message::ForgetSearch(..) => self.handle_library(message),
            Message::TogglePlay
            | Message::Next
            | Message::Previous
            | Message::SeekBy(..)
            | Message::SeekFraction(..)
            | Message::Scrub(..)
            | Message::ScrubReleased
            | Message::LyricSeek(..)
            | Message::Volume(..)
            | Message::VolumeReleased
            | Message::VolumeStep(..)
            | Message::VolumeSide(..)
            | Message::VolumeSideReleased
            | Message::VolumeCollapse(..)
            | Message::ToggleMute
            | Message::SetQuality(..)
            | Message::EqualizerToggle(..)
            | Message::EqualizerBand(..)
            | Message::EqualizerPreset(..)
            | Message::EqualizerReleased
            | Message::SetSleep(..)
            | Message::SetAutoplay(..)
            | Message::PlayTracks(..)
            | Message::ShufflePlay(..)
            | Message::PlayQueueAt(..)
            | Message::PlayAlbum(..)
            | Message::PlayMix(..)
            | Message::StartRadio(..)
            | Message::LikeCurrent
            | Message::Hover(..)
            | Message::HoverEnd(..)
            | Message::HoverSettled(..)
            | Message::Tick
            | Message::Enqueue(..)
            | Message::EnqueueNext(..)
            | Message::RemoveFromQueue(..)
            | Message::ClearQueue
            | Message::ToggleShuffle
            | Message::CycleRepeat => self.handle_playback(message),
            Message::Retry
            | Message::SearchInput(..)
            | Message::LibraryMore
            | Message::LibraryFilter(..)
            | Message::LibrarySort(..)
            | Message::OpenLibrary(..)
            | Message::ArtistMorePopular
            | Message::ArtistFilter(..)
            | Message::ArchiveLibraryOpen(..)
            | Message::ArchiveLibraryMore
            | Message::ArchiveLibraryClose
            | Message::OpenMetaArtist(..)
            | Message::CoverLoaded(..) => self.handle_catalog(message),
            Message::OpenSourcePlaylist(..)
            | Message::SaveSourcePlaylist
            | Message::PlaySourcePlaylist(..)
            | Message::PlayArtistTop(..)
            | Message::PlayDiscovered(..)
            | Message::PlaySource(..) => self.handle_external(message),
            Message::Download(..)
            | Message::DownloadAs(..)
            | Message::DownloadAll(..)
            | Message::SetDownloadQuality(..)
            | Message::DownloadCancel(..)
            | Message::DownloadRetry(..)
            | Message::DownloadRemove(..)
            | Message::DownloadsVerify
            | Message::DownloadsOpenFolder
            | Message::DownloadsShowMore
            | Message::UpdateCheck(..)
            | Message::UpdateDismiss
            | Message::UpdateRequested
            | Message::UpdateReveal => self.handle_downloads(message),
            Message::SignIn
            | Message::AuthMode(..)
            | Message::AuthInput(..)
            | Message::AuthSubmit
            | Message::AuthProvider(..)
            | Message::AuthResend
            | Message::AuthBack
            | Message::SignOut
            | Message::ProfileHandle(..)
            | Message::ProfileName(..)
            | Message::ProfileSave(..)
            | Message::JamInviteToggle(..)
            | Message::JamInviteSend
            | Message::JamInviteAccept(..)
            | Message::JamInviteDismiss(..)
            | Message::UserQuery(..)
            | Message::UserSearch
            | Message::OpenProfile(..)
            | Message::FriendAction(..)
            | Message::OpenFriendPlaylist(..)
            | Message::PlaylistPublic(..) => self.handle_social(message),
            Message::Phone(message) => self.handle_phone(message),
            Message::OpenJam
            | Message::JamName(..)
            | Message::JamCode(..)
            | Message::JamCreate
            | Message::JamJoin
            | Message::JamLeave => self.handle_jam(message),
            Message::SetPanelVideo(..)
            | Message::SetDedicatedVideo(..)
            | Message::GpuDetected(..)
            | Message::RetryVideo
            | Message::ToggleVideoFocus
            | Message::ToggleVideoLyrics => self.handle_video(message),
            #[cfg(feature = "video")]
            Message::VideoOpened(..) | Message::VideoUpgraded(..) | Message::VideoFailed => {
                self.handle_video(message)
            }
            Message::Navigate(..)
            | Message::GoBack
            | Message::GoForward
            | Message::SystemBack
            | Message::Escape
            | Message::FocusSearch
            | Message::ModifiersChanged(..)
            | Message::ToggleShortcuts
            | Message::OpenPanel(..)
            | Message::ClosePanel
            | Message::SettingsTab(..)
            | Message::SideTab(..)
            | Message::OpenSettingsTab(..)
            | Message::ToggleFocus
            | Message::OpenMenu(..)
            | Message::CloseMenu
            | Message::MenuPlaylists(..)
            | Message::MenuDownloads(..)
            | Message::MenuTimers(..)
            | Message::OpenDrawer
            | Message::CloseDrawer
            | Message::DrawerGo(..)
            | Message::OpenDialog(..)
            | Message::DialogInput(..)
            | Message::DialogCancel
            | Message::Chosen(..)
            | Message::WindowReady(..)
            | Message::TitleBarPressed
            | Message::Minimize
            | Message::ToggleFullscreen
            | Message::Quit
            | Message::ResizeStart(..)
            | Message::ResizeMove(..)
            | Message::ResizeEnd
            | Message::CloseRequested(..)
            | Message::Exit(..)
            | Message::Frame
            | Message::SkipSplash
            | Message::NoticeExpired(..)
            | Message::DismissNotice
            | Message::CopyText(..)
            | Message::SidebarFilter(..)
            | Message::HeroGo(..)
            | Message::HeroNext => self.handle_interface(message),
            // L'altezza dei pixel d'arte dipende dal fattore di scala (`ui::icons`): lo si rilegge a
            // ogni ridimensionamento, che comprende il passaggio a un altro schermo.
            Message::WindowResized(size) => {
                let resized = self.handle_interface(Message::WindowResized(size));
                match self.window {
                    Some(id) => {
                        Task::batch([resized, window::get_scale_factor(id).map(Message::ScaleFactor)])
                    }
                    None => resized,
                }
            }
            Message::ScaleFactor(factor) => {
                ui::icons::set_scale_factor(factor);
                Task::none()
            }
            #[cfg(feature = "tour")]
            Message::Tour(..) => self.handle_interface(message),
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
            Screen::Profile => self.social.view.open.is_some(),
            Screen::FriendPlaylist => self.social.view.playlist.is_some(),
            // Si torna al modulo solo se non si è già entrati.
            Screen::SignIn => !matches!(self.social.view.account, Account::SignedIn(_)),
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
            // Il core carica se serve; sulla Home già carica rifà il feed se gli ascolti sono cambiati.
            Screen::Home => self.send_catalog(CatalogCmd::Ensure(Resource::Home)),
            Screen::Library => self.send_catalog(CatalogCmd::Ensure(Resource::Library)),
            Screen::Album(id) => self.send_catalog(CatalogCmd::Ensure(Resource::Album(id))),
            Screen::Artist(id) => self.open_artist(id),
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
            Screen::Home => self.send_catalog(CatalogCmd::Retry(Resource::Home)),
            Screen::Library => self.send_catalog(CatalogCmd::Retry(Resource::Library)),
            Screen::Album(id) => self.send_catalog(CatalogCmd::Retry(Resource::Album(id))),
            Screen::Artist(id) => self.send_catalog(CatalogCmd::Retry(Resource::Artist(id))),
            Screen::Search => self.send_catalog(CatalogCmd::Search(self.search.query.clone())),
            Screen::SourcePlaylist => self.send_external(ExternalCmd::RetryPlaylist),
            Screen::Friends => self.load_friends(),
            Screen::Profile => self.send_social(SocialCmd::ReloadProfile),
            Screen::Playlist(_)
            | Screen::Settings
            | Screen::Jam
            | Screen::Liked
            | Screen::Downloads
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
        // Il core vede il backend nuovo dalle impostazioni: butta le sue pagine e ricarica la Home.
        self.persist()
    }

    fn api(&self) -> Result<ApiClient, AppError> {
        self.api.clone()
    }

    /// Scarica le copertine mai richieste. Un errore lascia la copertina disegnata.
    fn fetch_covers(&mut self, urls: Vec<String>) -> Task<Message> {
        let wanted = self.covers.request(urls.iter().map(String::as_str));
        Task::batch(wanted.into_iter().map(|url| {
            let api = self.api();
            let request = url.clone();
            // La decodifica avviene su un thread bloccante del runtime (`artwork::decode`): né il
            // thread della UI né un worker asincrono restano fermi sull'immagine.
            Task::perform(
                async move {
                    let bytes = api.ok()?.cover(&request).await.ok()?;
                    psytify_core::artwork::decode(bytes).await
                },
                move |pixels| Message::CoverLoaded(url.clone(), pixels),
            )
        }))
    }

    // ---------- Riproduzione ----------

    /// Un frontend mostra il testo del brano in corso: quello del pannello (anche sopra il videoclip)
    /// o la vista a tutto schermo. Il core lo carica solo finché serve.
    fn lyrics_wanted(&self, video: Option<Uuid>) -> bool {
        let panel = self.panel_open
            && (self.side_tab == SideTab::Lyrics
                || (self.side_tab == SideTab::NowPlaying && video.is_some()));
        self.focus || panel
    }

    /// Dice al core se il testo serve. Se il testo che c'è è di un altro brano, intanto si mostra
    /// «in caricamento» e non quello vecchio, in attesa che il core pubblichi.
    fn want_lyrics(&mut self, wanted: bool) -> Task<Message> {
        let current = self.queue.current().map(|track| track.id);
        if wanted && current.is_some() && self.playback.lyrics.track != current {
            self.lyrics = Load::Loading;
            self.lyrics_line = None;
            self.lyrics_scroll = Tween::rest(-1.0);
        }
        self.send_playback(PlaybackCmd::Lyrics(wanted))
    }

    /// Con il testo sincronizzato la riga in corso segue la riproduzione: la lista scorre fino a lei.
    fn follow_lyrics(&mut self) -> Task<Message> {
        let Some(text) = self.lyrics.ready() else {
            return Task::none();
        };
        if !text.is_synced() {
            return Task::none();
        }
        let line = synced::current(&text.lines, self.position());
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
}

/// URL delle copertine dei brani, senza ripetizioni consecutive (i brani di un album ne hanno una).
fn track_covers(tracks: Option<&[Track]>) -> Vec<String> {
    let mut urls: Vec<String> =
        tracks.unwrap_or_default().iter().filter_map(|t| t.cover_url.clone()).collect();
    urls.dedup();
    urls
}

fn album_covers(albums: Option<&[AlbumSummary]>) -> Vec<String> {
    albums.unwrap_or_default().iter().filter_map(|a| a.cover_url.clone()).collect()
}

/// Testo di ricerca pronto per il backend: senza caratteri di controllo, da 1 a 100 caratteri.
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
        Screen::Collection | Screen::Liked | Screen::Downloads | Screen::Playlist(_) | Screen::Library => {
            Some(Screen::Collection)
        }
        Screen::News => Some(Screen::News),
        Screen::Album(_) | Screen::Artist(_) | Screen::Settings | Screen::Jam => None,
        Screen::SignIn | Screen::Friends | Screen::Profile | Screen::FriendPlaylist => Some(Screen::Profile),
    }
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

/// Sul telefono: andando in background (Home, un'altra app, lo schermo spento) winit dice che la
/// finestra perde il fuoco (`MainEvent::LostFocus`), prima che Android la distrugga. Succede anche
/// con la tendina delle notifiche aperta: un salvataggio in più, solo se c'è qualcosa da scrivere.
fn background_event(event: iced::Event, _status: event::Status, _window: window::Id) -> Option<Message> {
    matches!(event, iced::Event::Window(window::Event::Unfocused)).then_some(Message::Backgrounded)
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

/// Il backend dei test: una porta chiusa di 127.0.0.1, così una richiesta sfuggita fallisce subito
/// senza uscire dal computer.
#[cfg(test)]
pub(crate) const TEST_API_URL: &str = "http://127.0.0.1:9";
/// Il login Clerk dei test, anche lui su una porta chiusa: nessuna richiesta all'istanza vera.
#[cfg(test)]
const TEST_CLERK_ISSUER: &str = "https://127.0.0.1:9";

/// La cartella dei dati dei test, una per processo nella cartella temporanea: sessione e download
/// dei test finiscono qui, mai in quelle dell'utente. `config.json` invece lo scrive solo un core,
/// nella sua cartella (`App::for_tests_with_core`).
#[cfg(test)]
pub(crate) fn test_data_dir() -> &'static std::path::Path {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("psytify-tests-{}", Uuid::new_v4()));
        config::set_data_dir(dir.clone());
        dir
    })
}

/// Avvia il core sul runtime di Iced, che lo ha già aperto attorno a `App::new`: resta un solo
/// runtime nel processo. Il core legge e scrive `config.json` in `data_dir`. Se non parte, l'app va
/// avanti senza, ma non salva niente.
fn start_core(data_dir: Option<PathBuf>, downloads_dir: Option<PathBuf>) -> Option<Core> {
    let runtime = match tokio::runtime::Handle::try_current() {
        Ok(handle) => RuntimeChoice::Borrow(handle),
        Err(_) => RuntimeChoice::Own { workers: 2 },
    };
    match Core::start(CoreOptions {
        runtime,
        audio: AudioChoice::System,
        system: SystemChoice::Desktop,
        data_dir,
        downloads_dir,
    }) {
        Ok(core) => Some(core),
        Err(err) => {
            eprintln!("psytify: {err}");
            None
        }
    }
}

/// Le istantanee del core come messaggi: una per pubblicazione (le raffiche si fondono nell'ultima),
/// nessuna a core fermo. Il ricevitore preso dopo `start` non riporta la prima, che `App::new` ha già.
fn core_changes(
    changes: tokio::sync::watch::Receiver<Arc<CoreSnapshot>>,
) -> impl futures_util::Stream<Item = Message> {
    futures_util::stream::unfold(changes, |mut changes| async move {
        changes.changed().await.ok()?;
        let snapshot = Arc::clone(&changes.borrow_and_update());
        Some((Message::Core(snapshot), changes))
    })
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;

    #[tokio::test]
    async fn the_core_wakes_the_app_only_when_it_publishes() {
        let (publish, changes) = tokio::sync::watch::channel(Arc::new(CoreSnapshot::default()));
        let mut messages = std::pin::pin!(core_changes(changes));
        assert!(
            futures_util::FutureExt::now_or_never(messages.next()).is_none(),
            "nessun messaggio finché il core non pubblica"
        );
        publish.send_replace(Arc::new(CoreSnapshot { version: 1, ..CoreSnapshot::default() }));
        publish.send_replace(Arc::new(CoreSnapshot { version: 2, ..CoreSnapshot::default() }));
        let Some(Message::Core(snapshot)) = messages.next().await else { panic!("nessuna istantanea") };
        assert_eq!(snapshot.version, 2, "due pubblicazioni di fila arrivano come una, l'ultima");
        assert!(futures_util::FutureExt::now_or_never(messages.next()).is_none());
        drop(publish);
        assert!(messages.next().await.is_none(), "a core fermo l'abbonamento finisce");
    }

    #[test]
    fn an_older_snapshot_never_replaces_a_newer_one() {
        let mut app = App::for_tests();
        let _ = app.update(Message::Core(Arc::new(CoreSnapshot { version: 3, ..CoreSnapshot::default() })));
        let _ = app.update(Message::Core(Arc::new(CoreSnapshot { version: 2, ..CoreSnapshot::default() })));
        assert_eq!(app.core_snapshot.version, 3);
    }

    #[test]
    fn one_inserted_character_is_found_anywhere() {
        assert_eq!(inserted_char("eminem", "eminem,"), Some(','));
        assert_eq!(inserted_char("emnem", "eminem"), Some('i'));
        assert_eq!(inserted_char("", "1"), Some('1'));
        assert_eq!(inserted_char("eminem", "eminem 2"), None);
        assert_eq!(inserted_char("abc", "xbcd"), None);
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
        let mut app = App::for_tests();
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
    fn the_exit_of_a_previous_row_does_not_switch_off_the_new_hover() {
        let mut app = App::for_tests();
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let _ = app.update(Message::Hover(Some(Hovered { id: b, track: false })));
        // Salendo di riga, il mouse entra nella A e solo dopo esce dalla B.
        let _ = app.update(Message::Hover(Some(Hovered { id: a, track: false })));
        let _ = app.update(Message::HoverEnd(b));
        assert_eq!(app.hovered.map(|(hovered, _)| hovered.id), Some(a));
        let _ = app.update(Message::HoverEnd(a));
        assert!(app.hovered.is_none());
    }

    #[test]
    fn the_side_volume_widens_while_used_and_collapses_only_after_the_last_touch() {
        let mut app = App::for_tests();
        let _ = app.update(Message::VolumeSide(0.4));
        assert_eq!(app.playback.volume, 0.4);
        assert_eq!(app.volume_motion.target(), 1.0, "toccata, la barra si allarga");
        let first = app.volume_touch;

        // Un secondo tocco prima che scada il tempo: il ritorno del primo non deve chiuderla.
        let _ = app.update(Message::VolumeSide(0.5));
        let _ = app.update(Message::VolumeCollapse(first));
        assert_eq!(app.volume_motion.target(), 1.0);
        assert_eq!(app.playback.volume, 0.5);

        let _ = app.update(Message::VolumeCollapse(app.volume_touch));
        assert_eq!(app.volume_motion.target(), 0.0, "dopo l'ultimo tocco torna a filo");
    }

    /// Passo 4.8: gli eventi della stanza entrano nel core da soli, quindi una jam non vuole il
    /// Tick: a player fermo, in una jam o no, non gira niente.
    #[test]
    fn a_jam_does_not_need_the_tick() {
        let mut app = App::for_tests();
        assert!(!app.needs_tick());
        let _ = app.update(Message::JamCreate);
        assert!(app.jam.is_active(), "la jam è partita: la stanza non ha ancora risposto");
        assert!(!app.playback.is_playing());
        assert!(!app.needs_tick(), "una jam ferma non ha bisogno di svegliare l'interfaccia");
    }

    #[test]
    fn library_filter_singles_excludes_album_tracks() {
        let single = crate::api::models::fixtures::track(1);
        let album_track = crate::api::models::fixtures::track(2);
        assert!(LibraryFilter::Singles.matches(&single) && !LibraryFilter::Singles.matches(&album_track));
    }
}
