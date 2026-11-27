//! Riproduzione (passo 4.3): coda, brani preparati in anticipo, orologio, timer di spegnimento,
//! riproduzione automatica, ascolti contati e il battito del core.
//!
//! Il dominio possiede la coda, il volume, la qualità, l'equalizzatore e l'autoplay di `config.json`
//! (li consegna alle impostazioni, come la libreria) e l'uscita audio: è l'host a tenerla, sul
//! thread del core, e a eseguire le [`AudioOp`] che il riduttore chiede; ciò che l'audio dice
//! (partito o no, la posizione, il brano finito) torna come [`AudioFact`] nello stesso giro.
//!
//! **La posizione non si pubblica.** La vista porta un [`Clock`], ancorato a ogni transizione e
//! quando il battito sente uno scarto oltre `clock::DRIFT` (250 ms): ogni frontend calcola `clock.at(now)`. Il
//! battito (`Timer::Tick`, 250 ms) è armato solo mentre suona; in pausa, fermo o in caricamento il
//! core non si sveglia. Un battito che non cambia niente non pubblica.
//!
//! Con il passo 4.3b il dominio possiede anche il testo del brano (caricato solo se un frontend lo
//! mostra, [`PlaybackCmd::Lyrics`]), la radio, gli album e i mix da riprodurre, e ciò che il sistema
//! e Discord mostrano del brano (`system.rs`): i comandi dei tasti multimediali entrano come
//! `Input::Media` e diventano comandi del dominio.
//!
//! Gli esiti che il frontend deve mostrare (il timer che ferma la musica, l'autoplay che aggiunge
//! brani, un seek fallito, un brano che il server non ha più) non hanno un canale: stanno nella
//! vista come [`Raised`], l'ultimo con il suo numero d'ordine, e il frontend reagisce al numero nuovo.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::api::{self, Album, ApiClient, ArtistPage, Quality, Track};
use crate::config::{self, PlaybackFields, taste};
use crate::effect::{self, Effect, TimerKey};
use crate::error::AppError;
use crate::feed::Feed;
use crate::media;
use crate::offline::{self, LocalFile, LostReason};
use crate::player::lyrics::Text;
use crate::player::stream::StreamBuffer;
use crate::player::{Advance, AudioOutput, Clock, PlaybackState, Queue, WarmCache, eq, seek};
use crate::state::library::LibraryCmd;
use crate::state::system::{Artwork, System};

/// Ogni quanto batte il core mentre suona un brano.
pub const TICK: Duration = Duration::from_millis(250);
/// Sotto questa posizione «precedente» cambia brano, sopra riparte da capo.
const RESTART_THRESHOLD: Duration = Duration::from_secs(3);
/// Quanto prima della fine si comincia a scaricare tutto il brano successivo (la testata è già
/// calda da quando parte quello corrente): il passaggio è immediato.
const PREFETCH_BEFORE_END: Duration = Duration::from_secs(25);
/// Brani della coda preparati in anticipo mentre ne suona uno.
const WARM_AHEAD: usize = 2;
/// Brani aggiunti dalla riproduzione automatica.
const AUTOPLAY_COUNT: usize = 10;
/// Brani di una radio.
const RADIO_SIZE: usize = 25;
/// Dopo quanto un brano si considera ascoltato.
const HEARD_AFTER: Duration = Duration::from_secs(30);
/// Fra due battiti la posizione avanza di più di così solo per un salto (seek): non è ascolto.
const LISTENED_STEP: Duration = Duration::from_secs(1);
/// Il tempo ascoltato arriva alla libreria a gruppi di almeno tanti millisecondi, e a ogni
/// pausa, fine o cambio di brano: una pubblicazione ogni qualche secondo sveglierebbe i frontend
/// per niente.
const LISTENED_FLUSH_MS: u64 = 15_000;

/// Dopo quanto il timer ferma la musica.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepChoice {
    Minutes(u32),
    /// Alla fine del brano che suona.
    EndOfTrack,
}

impl SleepChoice {
    /// Le scelte del menu, in ordine.
    pub const ALL: [Self; 6] = [
        Self::Minutes(5),
        Self::Minutes(15),
        Self::Minutes(30),
        Self::Minutes(45),
        Self::Minutes(60),
        Self::EndOfTrack,
    ];

    pub fn label(self) -> String {
        match self {
            Self::Minutes(60) => "1 ora".into(),
            Self::Minutes(n) => format!("{n} minuti"),
            Self::EndOfTrack => "Alla fine del brano".into(),
        }
    }
}

/// Il timer attivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SleepTimer {
    pub choice: SleepChoice,
    pub until: Option<Instant>,
}

impl SleepTimer {
    pub fn new(choice: SleepChoice, now: Instant) -> Self {
        let until = match choice {
            SleepChoice::Minutes(n) => Some(now + Duration::from_secs(u64::from(n) * 60)),
            SleepChoice::EndOfTrack => None,
        };
        Self { choice, until }
    }

    /// Il tempo che manca, in testo breve («12 min», «fine brano»).
    pub fn remaining_label(&self, now: Instant) -> String {
        match self.until {
            Some(until) => {
                let secs = until.saturating_duration_since(now).as_secs();
                if secs >= 60 { format!("{} min", secs.div_ceil(60)) } else { format!("{secs} s") }
            }
            None => "fine brano".into(),
        }
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.until.is_some_and(|until| until <= now)
    }
}

/// Dopo quanto un brano si considera ascoltato: 30 secondi, o metà se dura meno di un minuto.
pub fn heard_enough(position: Duration, length: Duration) -> bool {
    position >= HEARD_AFTER.min(length / 2)
}

/// Il gruppo da seguire (jam): la coda, il brano, la posizione condivisa e se il gruppo suona.
#[derive(Debug, Clone)]
pub struct Align {
    /// I brani nell'ordine del gruppo, se la coda locale va rifatta (lo shuffle si spegne: l'ordine
    /// lo decide chi suona per primo). `None` se è già quella.
    pub tracks: Option<Vec<Track>>,
    /// La posizione del brano corrente nell'ordine di ascolto.
    pub index: usize,
    pub position: Duration,
    pub playing: bool,
}

/// I comandi del dominio.
#[derive(Debug, Clone)]
pub enum PlaybackCmd {
    /// Sostituisce la coda con `tracks` e fa partire quello in `start`.
    Play {
        tracks: Arc<[Track]>,
        start: usize,
    },
    /// Sostituisce la coda, accende lo shuffle e parte da un brano a caso.
    ShufflePlay(Arc<[Track]>),
    /// Salta alla voce della coda in quella posizione dell'ordine di ascolto e la fa partire.
    PlayAt(usize),
    /// Play e pausa; da fermo o dopo un errore, riparte dal brano corrente.
    Toggle,
    Next,
    Previous,
    SeekTo(Duration),
    /// Avanti (positivo) o indietro, in secondi.
    SeekBy(i64),
    /// Una frazione della durata del brano (0 – 1).
    SeekFraction(f32),
    /// Il volume (0 – 1). Non salva da solo: si salva con [`PlaybackCmd::Save`] (al rilascio dello
    /// slider) o con il prossimo cambio che si salva.
    Volume(f32),
    ToggleMute,
    Quality(Quality),
    EqualizerEnabled(bool),
    EqualizerBand {
        band: usize,
        gain: f32,
    },
    EqualizerGains([f32; eq::BANDS]),
    Sleep(Option<SleepChoice>),
    Autoplay(bool),
    /// Prepara la testata di un brano che forse si ascolterà (il mouse si è fermato su di lui).
    Warm(Uuid),
    Enqueue(Box<Track>),
    EnqueueNext(Box<Track>),
    /// Toglie la voce in quella posizione dell'ordine di ascolto.
    Remove(usize),
    Clear,
    /// Riordino a trascinamento: da una posizione dell'ordine di ascolto a un'altra.
    Move {
        from: usize,
        to: usize,
    },
    ToggleShuffle,
    CycleRepeat,
    /// `new` al posto del brano `old` in coda (ritrovato nel catalogo, o riletto dal server). Se
    /// `restart` e il brano corrente è cambiato, riparte.
    ReplaceTrack {
        old: Uuid,
        new: Box<Track>,
        restart: bool,
    },
    /// Il brano `old` non si è ritrovato nel catalogo: se è il corrente, resta in errore.
    HealFailed(Uuid),
    /// Le copertine che il catalogo ha appena rimandato, per brano.
    PatchCovers(Arc<HashMap<Uuid, Option<String>>>),
    /// Si è dentro una jam: la coda è del gruppo, l'autoplay non aggiunge nulla da solo.
    Group(bool),
    /// Allinea la riproduzione al gruppo.
    Align(Box<Align>),
    /// Consegna alle impostazioni il volume e l'equalizzatore come stanno (rilascio dello slider).
    Save,
    /// Un frontend mostra (o smette di mostrare) il testo del brano: il core lo carica solo finché
    /// serve. Mandarlo di nuovo con `true` riprova un testo che non è arrivato.
    Lyrics(bool),
    /// Riproduce un album dall'inizio: i suoi brani non sono nel feed per intero, il core li chiede
    /// al server.
    PlayAlbum(Uuid),
    /// Avvia una radio da un brano: lui, poi altri dello stesso artista e qualche brano da scoprire.
    StartRadio(Box<Track>),
    /// Riproduce il mix in quella posizione del feed della Home.
    PlayMix(usize),
    /// La copertina del brano in corso già decodificata, per la sessione multimediale di Android.
    Artwork(Box<Artwork>),
}

/// Qualcosa che il frontend deve far sapere all'ascoltatore.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackEvent {
    /// Il timer di spegnimento ha fermato la musica.
    SleepStopped,
    /// La riproduzione automatica ha aggiunto brani alla coda.
    AutoplayAdded,
    /// Lo spostamento nel brano non è riuscito.
    SeekFailed,
    /// Una radio è partita da questo brano.
    RadioStarted(Arc<Track>),
    /// Un album è partito: le sue copertine si possono preparare.
    AlbumStarted,
    /// L'album chiesto non è arrivato.
    PlayFailed(AppError),
    /// Il sistema ha chiesto di portare la finestra in primo piano (il widget del desktop).
    Raise,
}

/// Il testo del brano che un frontend mostra.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum LyricsState {
    #[default]
    Idle,
    Loading,
    Ready(Arc<Text>),
    Failed(AppError),
}

/// Il testo di `track` e a che punto è: il frontend lo mostra se `track` è il brano in corso.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LyricsView {
    pub track: Option<Uuid>,
    pub state: LyricsState,
}

/// L'ultimo [`PlaybackEvent`], con un numero d'ordine che cresce a ogni evento: il frontend lo
/// mostra quando il numero è nuovo.
#[derive(Debug, Clone, PartialEq)]
pub struct Raised {
    pub seq: u64,
    pub event: PlaybackEvent,
}

/// Ciò che i frontend vedono del dominio.
#[derive(Debug, Clone, Default)]
pub struct PlaybackView {
    pub state: PlaybackState,
    /// Dove si trova il brano e se scorre. La posizione di adesso è `clock.at(Instant::now())`.
    pub clock: Clock,
    /// Cresce a ogni caricamento: ogni brano che parte ha il suo numero.
    pub generation: u64,
    /// Il numero del caricamento che è partito per ultimo (0 se nessuno): cambia quando un brano
    /// comincia a suonare davvero.
    pub started: u64,
    /// Cresce a ogni spostamento nel brano (seek, riparti da capo, allineamento al gruppo): chi
    /// tiene un secondo orologio (il videoclip, i controlli del sistema) si riallinea quando cambia.
    pub seeks: u64,
    /// Il volume (0 – 1); 0 mentre è muto.
    pub volume: f32,
    pub muted: bool,
    pub quality: Quality,
    pub equalizer: eq::Settings,
    pub autoplay: bool,
    pub sleep: Option<SleepTimer>,
    pub queue: Arc<Queue>,
    /// Il brano in corso è già stato contato negli ascolti.
    pub play_counted: bool,
    pub event: Option<Raised>,
    /// Il testo del brano, caricato solo mentre un frontend lo chiede ([`PlaybackCmd::Lyrics`]).
    pub lyrics: LyricsView,
    /// Perché l'uscita audio non si è aperta all'avvio.
    pub device_error: Option<AppError>,
}

impl PlaybackView {
    pub fn is_playing(&self) -> bool {
        self.state.is_playing()
    }

    /// C'è un brano che suona o è in pausa.
    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    /// La posizione del brano a `now`.
    pub fn position(&self, now: Instant) -> Duration {
        self.clock.at(now)
    }
}

/// I timer del dominio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {
    /// Il battito: legge la posizione e lo stato dell'audio. Armato solo mentre suona.
    Tick,
}

/// I lavori asincroni del dominio (rete), che l'host esegue sul runtime.
#[derive(Debug)]
pub(crate) enum Job {
    /// Apre il brano per l'ascolto progressivo (con il rinnovo dell'URL scaduto, `api::with_fresh_url`).
    /// Con `local`, il file scaricato sul dispositivo ha la precedenza: se non si apre (manca, pesa
    /// diverso) si ripiega sulla rete e l'esito lo dice in `lost`.
    Load {
        api: Result<ApiClient, AppError>,
        track_id: Uuid,
        quality: Quality,
        generation: u64,
        local: Option<LocalFile>,
    },
    /// Prepara un brano: solo la testata, o per intero (il prossimo della coda, a fine brano).
    Warm { api: Result<ApiClient, AppError>, track_id: Uuid, quality: Quality, whole: bool },
    /// Altri brani dell'artista, a fine coda.
    Autoplay { api: Result<ApiClient, AppError>, artist: Uuid },
    /// Il testo di un brano.
    Lyrics { api: Result<ApiClient, AppError>, track_id: Uuid },
    /// I brani di un album da riprodurre. `ticket` dice di quale richiesta è la risposta.
    Album { api: Result<ApiClient, AppError>, id: Uuid, ticket: u64 },
    /// L'artista di una radio: i suoi brani sono la parte «stessa voce» della coda.
    Radio { api: Result<ApiClient, AppError>, start: Box<Track>, ticket: u64 },
}

/// L'esito di un [`Job`], che torna al dominio.
#[derive(Debug)]
pub(crate) enum Done {
    /// `lost`: il file scaricato che si doveva suonare non era più quello dell'indice.
    Loaded {
        generation: u64,
        result: Result<StreamBuffer, AppError>,
        lost: Option<LostReason>,
    },
    Warmed {
        track_id: Uuid,
        quality: Quality,
        result: Result<StreamBuffer, AppError>,
    },
    Autoplay(Result<Box<ArtistPage>, AppError>),
    Lyrics {
        track_id: Uuid,
        result: Result<Arc<Text>, AppError>,
    },
    Album {
        ticket: u64,
        result: Result<Box<Album>, AppError>,
    },
    Radio {
        ticket: u64,
        start: Box<Track>,
        result: Result<Box<ArtistPage>, AppError>,
    },
}

/// Un'operazione sull'uscita audio, che l'host esegue sul thread del core.
#[derive(Debug)]
pub(crate) enum AudioOp {
    Play {
        stream: StreamBuffer,
        volume: f32,
    },
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    Volume(f32),
    Equalizer(eq::Settings),
    /// Legge la posizione, se il brano è finito e se un seek è fallito.
    Poll,
}

/// Ciò che l'uscita audio risponde.
#[derive(Debug)]
pub(crate) enum AudioFact {
    /// Il brano è partito, o l'uscita non l'ha aperto.
    Started(Result<(), AppError>),
    /// La posizione vera dopo una pausa.
    Position(Duration),
    Polled {
        position: Duration,
        finished: bool,
        seek_failed: bool,
    },
}

/// Esegue `op` sull'uscita `audio` (assente se non c'è la scheda audio). Sta qui perché lo usano
/// l'host e il core sincrono dei test.
pub(crate) fn run_audio(audio: &mut Option<AudioOutput>, op: AudioOp) -> Option<AudioFact> {
    match (op, audio.as_mut()) {
        (AudioOp::Play { stream, volume }, Some(audio)) => {
            Some(AudioFact::Started(audio.play(&stream, volume)))
        }
        (AudioOp::Play { .. }, None) => Some(AudioFact::Started(Err(AppError::AudioDevice))),
        (AudioOp::Pause, Some(audio)) => {
            audio.pause();
            Some(AudioFact::Position(audio.position()))
        }
        (AudioOp::Resume, Some(audio)) => {
            audio.resume();
            None
        }
        (AudioOp::Stop, Some(audio)) => {
            audio.stop();
            None
        }
        (AudioOp::Seek(to), Some(audio)) => {
            audio.seek(to);
            None
        }
        (AudioOp::Volume(volume), Some(audio)) => {
            audio.set_volume(volume);
            None
        }
        (AudioOp::Equalizer(settings), Some(audio)) => {
            audio.set_equalizer(settings);
            None
        }
        (AudioOp::Poll, Some(audio)) => Some(AudioFact::Polled {
            position: audio.position(),
            finished: audio.finished(),
            seek_failed: audio.take_seek_failure(),
        }),
        (_, None) => None,
    }
}

/// Il brano del gruppo da raggiungere quando il suo audio parte.
#[derive(Debug, Clone, Copy)]
struct Target {
    track: Uuid,
    position: Duration,
    playing: bool,
}

/// Lo stato del dominio: la vista pubblicata e ciò che resta dentro il core.
#[derive(Debug)]
pub(crate) struct Playback {
    pub(crate) view: Arc<PlaybackView>,
    api: Result<ApiClient, AppError>,
    /// Il backend scelto che ha dato `api`: se cambia nelle impostazioni, `api` si rifà.
    server: Option<String>,
    /// I brani recenti del catalogo (da cui nasce il feed della Home): il serbatoio dei brani di
    /// riserva per la riproduzione automatica.
    pool: Option<Arc<[Track]>>,
    warm: WarmCache,
    /// L'ultimo brano suonato, con la sua qualità: si riascolta dal buffer, senza riscaricarlo.
    cached: Option<(Uuid, Quality, StreamBuffer)>,
    /// Il brano che si sta avviando sull'uscita audio.
    starting: Option<(Uuid, Quality, StreamBuffer)>,
    /// I file scaricati da cui suonare, dal dominio dei download: un brano che ha il suo file si
    /// suona da lì, senza rete.
    local: Arc<HashMap<Uuid, LocalFile>>,
    /// I file scaricati che non si sono potuti suonare, da dire ai download (`take_lost_files`).
    lost_files: Vec<(Uuid, LostReason)>,
    /// Il volume di prima del muto.
    before_mute: Option<f32>,
    autoplay_busy: Option<u64>,
    group: bool,
    healing: HashSet<Uuid>,
    /// I brani che il server non ha più, da far ritrovare al catalogo (`take_heal`): lui risponde
    /// con `ReplaceTrack` o `HealFailed`.
    heal: Vec<Arc<Track>>,
    /// I brani appena contati come ascoltati, da dire al dominio sociale (`take_listens`): il suo
    /// profilo li manda al server, se c'è un accesso in quel momento.
    listens: Vec<Uuid>,
    target: Option<Target>,
    /// Where to resume the current entry after a restart (read from `config.json`, or kept at
    /// shutdown). Only «play» on the stopped entry uses it; starting anything else forgets it.
    resume: Option<(Uuid, Duration)>,
    /// La posizione dell'audio al battito precedente: il tempo ascoltato è la differenza.
    last_polled: Duration,
    /// Millisecondi ascoltati che la libreria non ha ancora.
    listened_ms: u64,
    ticking: bool,
    /// Un campo di `config.json` è cambiato e le impostazioni non l'hanno ancora (`take_persist`).
    persist: bool,
    seed: u64,
    events: u64,
    /// L'ora del sistema all'avvio, in secondi dal 1970, e il primo istante visto: l'ora di un
    /// ascolto contato si ricava da qui, perché il riduttore non legge l'orologio.
    unix: u64,
    origin: Option<Instant>,
    /// Ciò che il dominio chiede alla libreria (ascolti, brani sostituiti): lo smista `CoreState`.
    library: Vec<LibraryCmd>,
    /// Un frontend mostra il testo.
    lyrics_wanted: bool,
    /// Il numero dell'ultima richiesta di un album o di una radio: la risposta di una richiesta
    /// superata (un'altra scelta nel frattempo) si ignora.
    ticket: u64,
    /// Il feed della Home, da cui vengono i mix.
    feed: Option<Arc<Feed>>,
    /// Ciò che il sistema e Discord mostrano.
    system: System,
}

impl Default for Playback {
    fn default() -> Self {
        let fields = config::CoreConfig::default().playback();
        Self::new(fields, None, 0, 0)
    }
}

/// Il passo successivo di un generatore SplitMix64: da un seme ne esce un altro, senza orologio.
fn next_seed(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Playback {
    /// Il dominio con i campi letti all'avvio. `seed` e `unix` (secondi dal 1970) li sceglie l'host.
    pub(crate) fn new(fields: PlaybackFields, server: Option<String>, seed: u64, unix: u64) -> Self {
        let resume = fields
            .queue
            .current()
            .filter(|_| fields.position_ms > 0)
            .map(|track| (track.id, Duration::from_millis(fields.position_ms)));
        let view = PlaybackView {
            volume: fields.volume.clamp(0.0, 1.0),
            quality: fields.quality,
            equalizer: fields.equalizer,
            autoplay: fields.autoplay,
            queue: fields.queue,
            // The stopped entry shows where it will resume.
            clock: Clock { position: resume.map_or(Duration::ZERO, |(_, at)| at), ..Clock::default() },
            ..PlaybackView::default()
        };
        Self {
            view: Arc::new(view),
            api: ApiClient::new(&config::api_url(server.as_deref())),
            server,
            pool: None,
            warm: WarmCache::default(),
            cached: None,
            starting: None,
            local: Arc::default(),
            lost_files: Vec::new(),
            before_mute: None,
            autoplay_busy: None,
            group: false,
            healing: HashSet::new(),
            heal: Vec::new(),
            listens: Vec::new(),
            target: None,
            resume,
            last_polled: Duration::ZERO,
            listened_ms: 0,
            ticking: false,
            persist: false,
            seed,
            events: 0,
            unix,
            origin: None,
            library: Vec::new(),
            lyrics_wanted: false,
            ticket: 0,
            feed: None,
            system: System::default(),
        }
    }

    /// L'uscita audio non si è aperta: la vista lo dice, e il primo brano riceve questo errore.
    pub(crate) fn set_device_error(&mut self, error: Option<AppError>) {
        if self.view.device_error != error {
            Arc::make_mut(&mut self.view).device_error = error;
        }
    }

    /// Il backend scelto nelle impostazioni è cambiato: il client HTTP si rifà.
    pub(crate) fn set_server(&mut self, server: &Option<String>) {
        if self.server != *server {
            self.server.clone_from(server);
            self.api = ApiClient::new(&config::api_url(server.as_deref()));
        }
    }

    pub(crate) fn set_pool(&mut self, pool: Option<&Arc<[Track]>>) {
        let same = match (&self.pool, pool) {
            (Some(held), Some(new)) => Arc::ptr_eq(held, new),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.pool = pool.cloned();
        }
    }

    /// I campi di `config.json` di questo dominio come stanno adesso: il volume è quello di prima
    /// del muto.
    pub(crate) fn fields(&self, now: Instant) -> PlaybackFields {
        PlaybackFields {
            volume: self.before_mute.unwrap_or(self.view.volume),
            queue: Arc::clone(&self.view.queue),
            position_ms: u64::try_from(self.resume_point(now).as_millis()).unwrap_or(u64::MAX),
            quality: self.view.quality,
            equalizer: self.view.equalizer,
            autoplay: self.view.autoplay,
        }
    }

    /// Where the current entry would resume: the clock while it plays or is paused, the target
    /// while it loads to resume, the saved point while it is stopped, otherwise its start.
    fn resume_point(&self, now: Instant) -> Duration {
        let current = self.view.queue.current().map(|track| track.id);
        let at = |(track, position): (Uuid, Duration)| (Some(track) == current).then_some(position);
        match self.view.state {
            PlaybackState::Playing | PlaybackState::Paused => self.view.clock.at(now),
            PlaybackState::Loading { .. } => {
                self.target.as_ref().and_then(|t| at((t.track, t.position))).unwrap_or_default()
            }
            PlaybackState::Stopped | PlaybackState::Error(_) => self.resume.and_then(at).unwrap_or_default(),
        }
    }

    /// La riproduzione è in un gruppo (una jam): per i test della jam.
    #[cfg(test)]
    pub(crate) fn grouped(&self) -> bool {
        self.group
    }

    /// Dice se c'è qualcosa da consegnare alle impostazioni, e lo azzera.
    pub(crate) fn take_persist(&mut self) -> bool {
        std::mem::take(&mut self.persist)
    }

    /// Ciò che il dominio chiede alla libreria, e lo svuota.
    pub(crate) fn take_library(&mut self) -> Vec<LibraryCmd> {
        std::mem::take(&mut self.library)
    }

    /// I file scaricati da cui suonare, dal dominio dei download.
    pub(crate) fn set_local(&mut self, local: &Arc<HashMap<Uuid, LocalFile>>) {
        if !Arc::ptr_eq(&self.local, local) {
            self.local = Arc::clone(local);
        }
    }

    /// I file scaricati che non si sono potuti suonare, e li svuota: i download li segnano persi.
    pub(crate) fn take_lost_files(&mut self) -> Vec<(Uuid, LostReason)> {
        std::mem::take(&mut self.lost_files)
    }

    /// I brani appena contati come ascoltati, e li svuota: il profilo li manda al server.
    pub(crate) fn take_listens(&mut self) -> Vec<Uuid> {
        std::mem::take(&mut self.listens)
    }

    /// I brani che il server non ha più, e li svuota: il catalogo li ritrova.
    pub(crate) fn take_heal(&mut self) -> Vec<Arc<Track>> {
        std::mem::take(&mut self.heal)
    }

    /// Il feed della Home è cambiato: da lì vengono i mix.
    pub(crate) fn set_feed(&mut self, feed: &Arc<Feed>) {
        if !self.feed.as_ref().is_some_and(|held| Arc::ptr_eq(held, feed)) {
            self.feed = Some(Arc::clone(feed));
        }
    }

    /// L'impostazione di Discord (interruttore e app) letta dalle impostazioni: se cambia accende o
    /// spegne il collegamento.
    pub(crate) fn set_discord(
        &mut self,
        presence: bool,
        app_id: &str,
        now: Instant,
        effects: &mut Vec<Effect>,
    ) {
        if self.system.set_discord(presence, app_id, effects) {
            self.system.sync(&self.view, now, effects);
        }
    }

    /// Il collegamento multimediale del sistema è pronto: gli si dice ciò che suona.
    pub(crate) fn media_ready(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        self.system.ready();
        self.sync(now, effects);
    }

    /// Un comando arrivato dal sistema (tasti multimediali, widget del desktop, cuffie): diventa il
    /// comando che l'ascoltatore avrebbe dato dall'interfaccia.
    pub(crate) fn media(&mut self, event: media::Event, now: Instant, effects: &mut Vec<Effect>) {
        use media::Event;
        let playing = self.view.is_playing();
        let command = match event {
            Event::Ready(_) => return self.media_ready(now, effects),
            Event::Play if !playing => PlaybackCmd::Toggle,
            Event::Pause if playing => PlaybackCmd::Toggle,
            Event::Play | Event::Pause => return,
            Event::Toggle => PlaybackCmd::Toggle,
            Event::Next => PlaybackCmd::Next,
            Event::Previous => PlaybackCmd::Previous,
            Event::SeekBy(secs) => PlaybackCmd::SeekBy(secs),
            Event::SetPosition(at) => PlaybackCmd::SeekTo(at),
            Event::SetVolume(volume) => {
                self.apply(PlaybackCmd::Volume(volume), now, effects);
                PlaybackCmd::Save
            }
            Event::Raise => {
                self.raise(PlaybackEvent::Raise);
                return self.sync(now, effects);
            }
        };
        self.apply(command, now, effects);
    }

    pub(crate) fn apply(&mut self, command: PlaybackCmd, now: Instant, effects: &mut Vec<Effect>) {
        self.origin.get_or_insert(now);
        match command {
            PlaybackCmd::Play { tracks, start } => {
                self.next_ticket();
                let seed = self.next_seed();
                self.queue_mut().replace(tracks.to_vec(), start, seed);
                self.start_current(now, effects);
            }
            PlaybackCmd::ShufflePlay(tracks) => {
                if tracks.is_empty() {
                    return;
                }
                self.next_ticket();
                let seed = self.next_seed();
                let start = usize::try_from(seed % tracks.len() as u64).unwrap_or(0);
                let queue = self.queue_mut();
                queue.set_shuffle(true, seed);
                queue.replace(tracks.to_vec(), start, seed);
                self.start_current(now, effects);
            }
            PlaybackCmd::PlayAt(position) => {
                self.next_ticket();
                self.queue_mut().jump(position);
                self.start_current(now, effects);
            }
            PlaybackCmd::Toggle => self.toggle(now, effects),
            PlaybackCmd::Next => self.skip(now, effects),
            PlaybackCmd::Previous => {
                if self.view.is_active() && self.view.clock.at(now) > RESTART_THRESHOLD {
                    self.seek_to(Duration::ZERO, now, effects);
                } else if self.queue_mut().previous().is_some() {
                    self.start_current(now, effects);
                }
            }
            PlaybackCmd::SeekTo(at) => {
                if self.view.is_active() {
                    self.seek_to(at, now, effects);
                }
            }
            PlaybackCmd::SeekBy(delta) => {
                if self.view.is_active() {
                    let duration = self.view.queue.current().map(Track::duration);
                    let target = seek::seek_by(self.view.clock.at(now), delta, duration);
                    self.seek_to(target, now, effects);
                }
            }
            PlaybackCmd::SeekFraction(fraction) => {
                if self.view.is_active()
                    && let Some(duration) = self.view.queue.current().map(Track::duration)
                {
                    self.seek_to(seek::seek_fraction(fraction, duration), now, effects);
                }
            }
            PlaybackCmd::Volume(level) => {
                self.before_mute = None;
                self.set_volume(level, effects);
            }
            PlaybackCmd::ToggleMute => {
                match self.before_mute.take() {
                    Some(previous) => self.set_volume(previous, effects),
                    None => {
                        self.before_mute = Some(self.view.volume);
                        self.set_volume(0.0, effects);
                    }
                }
                self.persist = true;
            }
            PlaybackCmd::Quality(quality) => {
                if quality != self.view.quality {
                    self.view_mut().quality = quality;
                    self.persist = true;
                }
            }
            PlaybackCmd::EqualizerEnabled(enabled) => {
                let mut settings = self.view.equalizer;
                settings.enabled = enabled;
                self.set_equalizer(settings, effects);
                self.persist = true;
            }
            PlaybackCmd::EqualizerBand { band, gain } => {
                let mut settings = self.view.equalizer;
                if let Some(slot) = settings.gains.get_mut(band) {
                    *slot = gain;
                    self.set_equalizer(settings, effects);
                }
            }
            PlaybackCmd::EqualizerGains(gains) => {
                let mut settings = self.view.equalizer;
                settings.gains = gains;
                self.set_equalizer(settings, effects);
                self.persist = true;
            }
            PlaybackCmd::Sleep(choice) => {
                let timer = choice.map(|choice| SleepTimer::new(choice, now));
                if self.view.sleep != timer {
                    self.view_mut().sleep = timer;
                }
            }
            PlaybackCmd::Autoplay(on) => {
                if self.view.autoplay != on {
                    self.view_mut().autoplay = on;
                    self.persist = true;
                }
            }
            PlaybackCmd::Warm(id) => self.warm_track(id, effects),
            PlaybackCmd::Enqueue(track) => self.queue_mut().enqueue(*track),
            PlaybackCmd::EnqueueNext(track) => self.queue_mut().enqueue_next(*track),
            PlaybackCmd::Remove(position) => {
                if position >= self.view.queue.len() {
                    return;
                }
                let was_current = self.queue_mut().remove(position);
                if was_current && self.view.is_active() {
                    self.start_current(now, effects);
                } else if self.view.queue.is_empty() {
                    self.stop(now, effects);
                }
            }
            PlaybackCmd::Clear => {
                if !self.view.queue.is_empty() {
                    self.queue_mut().clear();
                }
                self.stop(now, effects);
            }
            PlaybackCmd::Move { from, to } => {
                let len = self.view.queue.len();
                if from < len && to < len && from != to {
                    self.queue_mut().move_entry(from, to);
                }
            }
            PlaybackCmd::ToggleShuffle => {
                let seed = self.next_seed();
                let on = !self.view.queue.shuffle();
                self.queue_mut().set_shuffle(on, seed);
            }
            PlaybackCmd::CycleRepeat => self.queue_mut().cycle_repeat(),
            PlaybackCmd::ReplaceTrack { old, new, restart } => {
                let was_current = self.view.queue.current().is_some_and(|t| t.id == old);
                let stale = |t: &Track| t.id == old && *t != *new;
                if self.view.queue.entries().any(|(_, t)| stale(t)) {
                    self.queue_mut().update_tracks(|t| {
                        let hit = stale(t);
                        if hit {
                            *t = (*new).clone();
                        }
                        hit
                    });
                    if restart && was_current {
                        self.start_current(now, effects);
                    }
                }
            }
            PlaybackCmd::HealFailed(old) => {
                if self.view.queue.current().is_some_and(|t| t.id == old) {
                    self.set_state(PlaybackState::Error(
                        "Questo brano non è più disponibile sul server e non l'ho ritrovato nel catalogo."
                            .into(),
                    ));
                }
            }
            PlaybackCmd::PatchCovers(fresh) => {
                let stale = |t: &Track| fresh.get(&t.id).is_some_and(|cover| t.cover_url != *cover);
                if self.view.queue.entries().any(|(_, t)| stale(t)) {
                    self.queue_mut().update_tracks(|t| taste::patch_covers(std::slice::from_mut(t), &fresh));
                }
            }
            PlaybackCmd::Group(on) => self.group = on,
            PlaybackCmd::Align(align) => self.align(*align, now, effects),
            PlaybackCmd::Save => self.persist = true,
            PlaybackCmd::Lyrics(on) => {
                self.lyrics_wanted = on;
                self.sync_lyrics(true, effects);
            }
            PlaybackCmd::PlayAlbum(id) => {
                let ticket = self.next_ticket();
                effects.push(Effect::Run(effect::Job::Playback(Job::Album {
                    api: self.api.clone(),
                    id,
                    ticket,
                })));
            }
            PlaybackCmd::StartRadio(start) => {
                let ticket = self.next_ticket();
                effects.push(Effect::Run(effect::Job::Playback(Job::Radio {
                    api: self.api.clone(),
                    start,
                    ticket,
                })));
            }
            PlaybackCmd::PlayMix(index) => {
                let tracks =
                    self.feed.as_deref().and_then(|feed| feed.mixes.get(index)).map(|m| m.tracks.clone());
                if let Some(tracks) = tracks.filter(|tracks| !tracks.is_empty()) {
                    self.next_ticket();
                    self.play_all(tracks, now, effects);
                }
            }
            PlaybackCmd::Artwork(art) => self.system.set_art(*art),
        }
        self.sync(now, effects);
    }

    pub(crate) fn fire(&mut self, timer: Timer, now: Instant, effects: &mut Vec<Effect>) {
        match timer {
            Timer::Tick => {
                self.ticking = false;
                if self.view.is_playing() {
                    effects.push(Effect::Audio(AudioOp::Poll));
                }
            }
        }
        self.sync(now, effects);
    }

    pub(crate) fn done(&mut self, done: Done, now: Instant, effects: &mut Vec<Effect>) {
        match done {
            Done::Loaded { generation, result, lost } => {
                if let Some(reason) = lost
                    && generation == self.view.generation
                    && let Some(track) = self.view.queue.current()
                {
                    self.lost_files.push((track.id, reason));
                }
                self.loaded(generation, result, effects);
            }
            Done::Warmed { track_id, quality, result } => {
                // Se non riesce, il brano si apre normalmente quando parte.
                self.warm.finish(track_id, quality, result.ok());
            }
            Done::Autoplay(result) => self.autoplay_loaded(result.map(|page| *page), now, effects),
            Done::Lyrics { track_id, result } => {
                // Il testo di un brano già passato non si mostra.
                if self.view.lyrics.track == Some(track_id) {
                    let state = match result {
                        Ok(text) => LyricsState::Ready(text),
                        Err(err) => LyricsState::Failed(err),
                    };
                    self.view_mut().lyrics.state = state;
                }
            }
            Done::Album { ticket, result } => {
                if ticket != self.ticket {
                    return self.sync(now, effects);
                }
                match result {
                    Ok(album) if !album.tracks.is_empty() => {
                        self.raise(PlaybackEvent::AlbumStarted);
                        self.play_all(album.tracks, now, effects);
                    }
                    Ok(_) => {}
                    Err(err) => self.raise(PlaybackEvent::PlayFailed(err)),
                }
            }
            Done::Radio { ticket, start, result } => {
                if ticket != self.ticket {
                    return self.sync(now, effects);
                }
                // Se l'artista non risponde la radio parte lo stesso, dal catalogo recente.
                let own = result.map(|page| page.tracks).unwrap_or_default();
                let seed = self.next_seed();
                let queue = taste::radio(&start, &own, self.pool.as_deref().unwrap_or(&[]), RADIO_SIZE, seed);
                self.raise(PlaybackEvent::RadioStarted(Arc::new(*start)));
                self.play_all(queue, now, effects);
            }
        }
        self.sync(now, effects);
    }

    pub(crate) fn audio(&mut self, fact: AudioFact, now: Instant, effects: &mut Vec<Effect>) {
        match fact {
            AudioFact::Started(result) => self.started(result, now, effects),
            AudioFact::Position(position) => {
                if self.view.state == PlaybackState::Paused && self.view.clock.position != position {
                    self.set_clock(Clock::paused(position, now));
                    self.persist = true;
                }
            }
            AudioFact::Polled { position, finished, seek_failed } => {
                self.polled(position, finished, seek_failed, now, effects);
            }
        }
        self.sync(now, effects);
    }

    /// Il core si ferma: l'audio tace, le connessioni dei brani caldi si chiudono e il tempo
    /// ascoltato arriva alla libreria prima che le impostazioni scrivano l'ultima volta.
    pub(crate) fn close(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        // The last save keeps where the entry was, so the next start resumes it there.
        if self.view.is_active()
            && let Some(track) = self.view.queue.current()
        {
            self.resume = Some((track.id, self.view.clock.at(now)));
            self.persist = true;
        }
        self.stop(now, effects);
        self.warm.clear();
        self.flush_listened();
        self.sync(now, effects);
    }

    // ---------- Vista ----------

    /// La vista da cambiare: copiata se un frontend tiene ancora la precedente. Si chiama solo
    /// quando qualcosa cambia davvero.
    fn view_mut(&mut self) -> &mut PlaybackView {
        Arc::make_mut(&mut self.view)
    }

    /// La coda da cambiare. Una coda che cambia si salva.
    fn queue_mut(&mut self) -> &mut Queue {
        self.persist = true;
        Arc::make_mut(&mut self.view_mut().queue)
    }

    fn set_state(&mut self, state: PlaybackState) {
        if self.view.state != state {
            self.view_mut().state = state;
        }
    }

    fn set_clock(&mut self, clock: Clock) {
        if self.view.clock != clock {
            self.view_mut().clock = clock;
        }
    }

    fn raise(&mut self, event: PlaybackEvent) {
        self.events += 1;
        let seq = self.events;
        self.view_mut().event = Some(Raised { seq, event });
    }

    fn next_seed(&mut self) -> u64 {
        self.seed = next_seed(self.seed);
        self.seed
    }

    /// Dopo ogni ingresso: il battito, il testo del brano in corso e ciò che il sistema mostra.
    fn sync(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        self.sync_tick(effects);
        self.sync_lyrics(false, effects);
        self.system.sync(&self.view, now, effects);
    }

    /// Carica il testo del brano in corso se un frontend lo mostra e non è già quello (o, con
    /// `retry`, se non è arrivato). Chiamata a ogni ingresso: senza novità confronta e basta.
    fn sync_lyrics(&mut self, retry: bool, effects: &mut Vec<Effect>) {
        if !self.lyrics_wanted {
            return;
        }
        let Some(track_id) = self.view.queue.current().map(|t| t.id) else { return };
        let held = self.view.lyrics.track == Some(track_id);
        let failed = matches!(self.view.lyrics.state, LyricsState::Failed(_));
        if held && !(retry && failed) {
            return;
        }
        self.view_mut().lyrics = LyricsView { track: Some(track_id), state: LyricsState::Loading };
        effects.push(Effect::Run(effect::Job::Playback(Job::Lyrics { api: self.api.clone(), track_id })));
    }

    /// Una richiesta nuova di album o di radio: le risposte delle precedenti non valgono più.
    fn next_ticket(&mut self) -> u64 {
        self.ticket += 1;
        self.ticket
    }

    /// Sostituisce la coda con `tracks` e fa partire il primo.
    fn play_all(&mut self, tracks: Vec<Track>, now: Instant, effects: &mut Vec<Effect>) {
        let seed = self.next_seed();
        self.queue_mut().replace(tracks, 0, seed);
        self.start_current(now, effects);
    }

    /// Il battito gira solo mentre suona: lo arma quando comincia, lo annulla quando finisce.
    fn sync_tick(&mut self, effects: &mut Vec<Effect>) {
        let playing = self.view.is_playing();
        let key = TimerKey::Playback(Timer::Tick);
        if playing && !self.ticking {
            self.ticking = true;
            effects.push(Effect::Timer { key, after: TICK });
        } else if !playing && self.ticking {
            self.ticking = false;
            effects.push(Effect::CancelTimer(key));
        }
    }

    /// L'ora di `now` in secondi dal 1970.
    fn wall(&self, now: Instant) -> u64 {
        let origin = self.origin.unwrap_or(now);
        self.unix.saturating_add(now.saturating_duration_since(origin).as_secs())
    }

    // ---------- Transizioni ----------

    /// Carica e avvia il brano corrente della coda.
    fn start_current(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        let Some(track_id) = self.view.queue.current().map(|t| t.id) else {
            self.stop(now, effects);
            return;
        };
        effects.push(Effect::Audio(AudioOp::Stop));
        let generation = self.begin_loading(track_id, now);
        let quality = self.view.quality;
        // Un brano scaricato sul dispositivo si suona dal suo file: ha la precedenza sul buffer di
        // un ascolto precedente e sulla testata calda (che per lui non si prepara nemmeno).
        let local = self.local.get(&track_id).cloned();
        if local.is_none() {
            if let Some((cached_id, cached_quality, stream)) = &self.cached
                && *cached_id == track_id
                && *cached_quality == quality
                && !stream.is_cancelled()
            {
                let stream = stream.clone();
                self.loaded(generation, Ok(stream), effects);
                return;
            }
            if let Some(stream) = self.warm.take(track_id, quality) {
                self.loaded(generation, Ok(stream), effects);
                return;
            }
        }
        let api = self.api.clone();
        effects.push(Effect::Run(effect::Job::Playback(Job::Load {
            api,
            track_id,
            quality,
            generation,
            local,
        })));
    }

    /// Comincia un caricamento: restituisce il suo numero. Le risposte di caricamenti superati si
    /// ignorano.
    fn begin_loading(&mut self, track_id: Uuid, now: Instant) -> u64 {
        self.flush_listened();
        self.starting = None;
        self.resume = None;
        self.last_polled = Duration::ZERO;
        let view = self.view_mut();
        view.generation += 1;
        view.state = PlaybackState::Loading { track_id };
        view.clock = Clock::paused(Duration::ZERO, now);
        view.play_counted = false;
        view.generation
    }

    /// Il brano aperto del caricamento `generation` è arrivato, o non si è aperto.
    fn loaded(&mut self, generation: u64, result: Result<StreamBuffer, AppError>, effects: &mut Vec<Effect>) {
        if generation != self.view.generation {
            return;
        }
        match result {
            Ok(stream) => {
                let Some(track_id) = self.view.queue.current().map(|t| t.id) else {
                    self.set_state(PlaybackState::Error(AppError::AudioDecode.to_string()));
                    return;
                };
                self.starting = Some((track_id, self.view.quality, stream.clone()));
                effects.push(Effect::Audio(AudioOp::Play { stream, volume: self.view.volume }));
            }
            Err(err) if err.is_not_found() => {
                // Il server non ha più questo brano (database ricreato): il frontend lo ritrova nel
                // catalogo ufficiale, una volta sola per brano.
                match self.view.queue.current().cloned().filter(|track| self.healing.insert(track.id)) {
                    Some(track) => self.heal.push(Arc::new(track)),
                    None => self.set_state(PlaybackState::Error(err.to_string())),
                }
            }
            Err(err) => self.set_state(PlaybackState::Error(err.to_string())),
        }
    }

    /// L'uscita audio ha risposto alla richiesta di avviare il brano.
    fn started(&mut self, result: Result<(), AppError>, now: Instant, effects: &mut Vec<Effect>) {
        let Some((track_id, quality, stream)) = self.starting.take() else { return };
        if let Err(err) = result {
            if stream.is_file() {
                // Un file scaricato che non si decodifica (cambiato senza cambiare peso): lo si segna
                // perso e il brano parte dalla rete, una volta sola (questo caricamento non ha `local`).
                self.lost_files.push((track_id, LostReason::Changed));
                let api = self.api.clone();
                let generation = self.view.generation;
                effects.push(Effect::Run(effect::Job::Playback(Job::Load {
                    api,
                    track_id,
                    quality,
                    generation,
                    local: None,
                })));
                return;
            }
            self.set_state(PlaybackState::Error(err.to_string()));
            return;
        }
        // Il download del brano precedente, se non è finito, non serve più.
        if let Some((previous_id, _, previous)) = self.cached.replace((track_id, quality, stream))
            && previous_id != track_id
            && !previous.is_complete()
        {
            previous.cancel();
        }
        let view = self.view_mut();
        view.state = PlaybackState::Playing;
        view.clock = Clock::running(Duration::ZERO, now);
        view.started = view.generation;
        self.persist = true;
        if let Some(track) = self.view.queue.current().cloned() {
            self.library.push(LibraryCmd::RememberListen(track));
        }
        self.warm_ahead(effects);
        // Un brano del gruppo: ci si sposta sulla posizione condivisa e, se il gruppo è in pausa, ci
        // si ferma lì. Un brano diverso da quello atteso fa scartare il riferimento.
        if let Some(target) = self.target.take()
            && self.view.queue.current().is_some_and(|t| t.id == target.track)
        {
            self.seek_to(target.position, now, effects);
            if !target.playing {
                self.pause(now, effects);
            }
        }
    }

    fn toggle(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        match self.view.state {
            PlaybackState::Playing => self.pause(now, effects),
            PlaybackState::Paused => {
                // Un timer a tempo scaduto mentre la musica era ferma non deve fermarla alla ripresa.
                if self.view.sleep.is_some_and(|timer| timer.expired(now)) {
                    self.view_mut().sleep = None;
                }
                effects.push(Effect::Audio(AudioOp::Resume));
                let position = self.view.clock.position;
                self.set_state(PlaybackState::Playing);
                self.set_clock(Clock::running(position, now));
            }
            PlaybackState::Stopped | PlaybackState::Error(_) => {
                // After a restart the entry resumes where the last save left it.
                let resume = self.resume.take();
                self.start_current(now, effects);
                if let Some((track, position)) = resume
                    && self.view.queue.current().is_some_and(|current| current.id == track)
                {
                    self.target = Some(Target { track, position, playing: true });
                }
            }
            PlaybackState::Loading { .. } => {}
        }
    }

    fn pause(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        effects.push(Effect::Audio(AudioOp::Pause));
        let position = self.view.clock.at(now);
        self.set_state(PlaybackState::Paused);
        self.set_clock(Clock::paused(position, now));
        // A pause is a good moment to remember where the entry is.
        self.persist = true;
        self.flush_listened();
    }

    fn stop(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        effects.push(Effect::Audio(AudioOp::Stop));
        self.flush_listened();
        self.starting = None;
        // Una richiesta di brani per la riproduzione automatica ancora in volo non vale più: la
        // generazione 0 non è mai quella di un brano.
        if let Some(pending) = &mut self.autoplay_busy {
            *pending = 0;
        }
        self.set_state(PlaybackState::Stopped);
        // Già fermo e a zero: l'orologio non si riancora, altrimenti un comando senza effetto
        // pubblicherebbe una vista nuova per niente.
        if self.view.clock.playing || !self.view.clock.position.is_zero() {
            self.set_clock(Clock::paused(Duration::ZERO, now));
        }
    }

    fn skip(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        if self.queue_mut().advance(Advance::Skip).is_some() {
            self.start_current(now, effects);
        } else {
            self.stop(now, effects);
        }
    }

    fn seek_to(&mut self, target: Duration, now: Instant, effects: &mut Vec<Effect>) {
        effects.push(Effect::Audio(AudioOp::Seek(target)));
        self.last_polled = target;
        let clock =
            if self.view.is_playing() { Clock::running(target, now) } else { Clock::paused(target, now) };
        let view = self.view_mut();
        view.clock = clock;
        view.seeks += 1;
    }

    fn set_volume(&mut self, level: f32, effects: &mut Vec<Effect>) {
        let volume = if level.is_finite() { level.clamp(0.0, 1.0) } else { self.view.volume };
        let muted = self.before_mute.is_some();
        if self.view.volume != volume || self.view.muted != muted {
            let view = self.view_mut();
            view.volume = volume;
            view.muted = muted;
        }
        effects.push(Effect::Audio(AudioOp::Volume(volume)));
    }

    fn set_equalizer(&mut self, settings: eq::Settings, effects: &mut Vec<Effect>) {
        let settings = settings.clamped();
        if self.view.equalizer != settings {
            self.view_mut().equalizer = settings;
            effects.push(Effect::Audio(AudioOp::Equalizer(settings)));
        }
    }

    // ---------- Gruppo (jam) ----------

    /// Allinea la riproduzione al gruppo: stesso brano e stessa posizione, oppure si cambia brano.
    fn align(&mut self, align: Align, now: Instant, effects: &mut Vec<Effect>) {
        let Align { tracks, index, position, playing } = align;
        if let Some(tracks) = tracks {
            let seed = self.next_seed();
            let queue = self.queue_mut();
            queue.set_shuffle(false, seed);
            queue.replace(tracks, index, seed);
        }
        let Some(track) = self.view.queue.entries().nth(index).map(|(_, t)| t.id) else { return };
        let target = Target { track, position, playing };
        // Il brano è già in caricamento: basta ricordare la posizione, senza ricominciare.
        if let PlaybackState::Loading { track_id } = self.view.state
            && track_id == track
        {
            self.target = Some(target);
            return;
        }
        let same_track = self.view.queue.current().map(|t| t.id) == Some(track)
            && self.view.queue.position() == Some(index);
        if !same_track || !self.view.is_active() {
            self.queue_mut().jump(index);
            self.target = Some(target);
            self.start_current(now, effects);
            return;
        }
        let local_ms = crate::jam::millis(self.view.clock.at(now));
        if crate::jam::drifted(local_ms, crate::jam::millis(position)) {
            self.seek_to(position, now, effects);
        }
        if playing != self.view.is_playing() {
            self.toggle(now, effects);
        }
    }

    // ---------- Battito ----------

    fn polled(
        &mut self,
        position: Duration,
        finished: bool,
        seek_failed: bool,
        now: Instant,
        effects: &mut Vec<Effect>,
    ) {
        if seek_failed {
            self.raise(PlaybackEvent::SeekFailed);
        }
        if !self.view.is_playing() {
            return;
        }
        if finished {
            self.track_ended(now, effects);
            return;
        }
        // Il tempo ascoltato segue la posizione vera dell'audio: un seek è un salto, non ascolto, e
        // un'uscita che non scorre (nessun campione letto) non conta niente.
        let heard = position.checked_sub(self.last_polled).filter(|step| *step <= LISTENED_STEP);
        self.last_polled = position;
        self.listened_ms = self.listened_ms.saturating_add(heard.map_or(0, |step| step.as_millis() as u64));
        self.count_play(position, now);
        if self.sleep_due(now) {
            self.pause(now, effects);
            self.raise(PlaybackEvent::SleepStopped);
            return;
        }
        if self.view.clock.drifted(position, now) {
            self.set_clock(Clock::running(position, now));
        }
        self.prefetch_next(position, effects);
        if self.listened_ms >= LISTENED_FLUSH_MS {
            self.flush_listened();
        }
    }

    /// Un brano conta come ascoltato dopo 30 secondi (o metà, se è più corto): saltarlo non lo conta.
    fn count_play(&mut self, position: Duration, now: Instant) {
        if self.view.play_counted {
            return;
        }
        let Some(track) = self.view.queue.current() else { return };
        if !heard_enough(position, track.duration()) {
            return;
        }
        let track = track.clone();
        self.view_mut().play_counted = true;
        let at = self.wall(now);
        self.listens.push(track.id);
        self.library.push(LibraryCmd::CountPlay { track, at });
    }

    /// Il timer a tempo è scaduto: `true` se va fermata la musica.
    fn sleep_due(&mut self, now: Instant) -> bool {
        if !self.view.sleep.is_some_and(|timer| timer.expired(now)) {
            return false;
        }
        self.view_mut().sleep = None;
        true
    }

    /// Il brano è finito: passa al successivo, o alla riproduzione automatica a fine coda.
    fn track_ended(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        if self.autoplay_busy.is_some() {
            return;
        }
        // Il timer era «alla fine del brano»: si ferma qui, e Play riparte dal brano dopo.
        if self.view.sleep.is_some_and(|timer| timer.choice == SleepChoice::EndOfTrack) {
            self.view_mut().sleep = None;
            self.stop(now, effects);
            let _ = self.queue_mut().advance(Advance::Finished);
            self.raise(PlaybackEvent::SleepStopped);
            return;
        }
        if self.queue_mut().advance(Advance::Finished).is_some() {
            self.start_current(now, effects);
        } else {
            self.queue_ended(now, effects);
        }
    }

    /// La coda è finita: se la riproduzione automatica è attiva, chiede altri brani dell'artista.
    /// Intanto l'audio è finito, e lo stato lo dice: il brano non suona più.
    fn queue_ended(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        let Some(track) = self.view.queue.current() else {
            self.stop(now, effects);
            return;
        };
        let (artist, track_id) = (track.artist.id, track.id);
        // In una jam la coda è del gruppo: non si aggiunge nulla da soli.
        if !self.view.autoplay || self.group {
            self.stop(now, effects);
            return;
        }
        self.autoplay_busy = Some(self.view.generation);
        effects.push(Effect::Audio(AudioOp::Stop));
        self.flush_listened();
        self.set_state(PlaybackState::Loading { track_id });
        self.set_clock(Clock::paused(Duration::ZERO, now));
        effects.push(Effect::Run(effect::Job::Playback(Job::Autoplay { api: self.api.clone(), artist })));
    }

    fn autoplay_loaded(
        &mut self,
        result: Result<ArtistPage, AppError>,
        now: Instant,
        effects: &mut Vec<Effect>,
    ) {
        let Some(asked) = self.autoplay_busy.take() else { return };
        // Nel frattempo si è scelto altro: i brani arrivati non servono più.
        if asked != self.view.generation {
            return;
        }
        let queued: Vec<Uuid> = self.view.queue.entries().map(|(_, t)| t.id).collect();
        let pool: Vec<Track> = result.map(|page| page.tracks).unwrap_or_default();
        let mut fresh: Vec<Track> = pool.into_iter().filter(|t| !queued.contains(&t.id)).collect();
        if fresh.is_empty() {
            // L'artista non ha altro: si passa alle uscite recenti del catalogo.
            let current = self.view.queue.current().map(|t| t.artist.id);
            fresh = self
                .pool
                .as_deref()
                .map(|tracks| {
                    tracks
                        .iter()
                        .filter(|t| !queued.contains(&t.id) && Some(t.artist.id) != current)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
        }
        if fresh.is_empty() {
            self.stop(now, effects);
            return;
        }
        let seed = self.next_seed();
        taste::shuffle(&mut fresh, seed);
        fresh.truncate(AUTOPLAY_COUNT);
        let queue = self.queue_mut();
        for track in fresh {
            queue.enqueue(track);
        }
        self.raise(PlaybackEvent::AutoplayAdded);
        if self.queue_mut().advance(Advance::Finished).is_some() {
            self.start_current(now, effects);
        } else {
            self.stop(now, effects);
        }
    }

    // ---------- Brani caldi ----------

    /// Negli ultimi secondi di un brano fa scaricare per intero il successivo della coda (la sua
    /// testata è già calda), così parte senza attesa.
    fn prefetch_next(&mut self, position: Duration, effects: &mut Vec<Effect>) {
        let (Some(current), Some(next)) = (self.view.queue.current(), self.view.queue.peek_next()) else {
            return;
        };
        let remaining = current.duration().saturating_sub(position);
        if current.duration().is_zero() || remaining > PREFETCH_BEFORE_END || next.id == current.id {
            return;
        }
        let (track_id, quality) = (next.id, self.view.quality);
        if self.local.contains_key(&track_id) {
            return;
        }
        if self.warm.release(track_id, quality) || !self.warm.begin(track_id, quality) {
            return;
        }
        let api = self.api.clone();
        effects.push(Effect::Run(effect::Job::Playback(Job::Warm { api, track_id, quality, whole: true })));
    }

    /// Prepara la testata di un brano che forse si ascolterà. Non fa nulla se è già caldo o in apertura.
    fn warm_track(&mut self, track_id: Uuid, effects: &mut Vec<Effect>) {
        let quality = self.view.quality;
        let already_playing =
            self.cached.as_ref().is_some_and(|(id, cached, _)| *id == track_id && *cached == quality);
        // Un brano sul dispositivo non ha bisogno di una testata calda dalla rete.
        if already_playing || self.local.contains_key(&track_id) || !self.warm.begin(track_id, quality) {
            return;
        }
        let api = self.api.clone();
        effects.push(Effect::Run(effect::Job::Playback(Job::Warm { api, track_id, quality, whole: false })));
    }

    /// Mentre un brano suona, prepara i prossimi della coda.
    fn warm_ahead(&mut self, effects: &mut Vec<Effect>) {
        let ahead: Vec<Uuid> =
            self.view.queue.upcoming().take(WARM_AHEAD).map(|(_, track)| track.id).collect();
        for id in ahead {
            self.warm_track(id, effects);
        }
    }

    // ---------- Ascolti ----------

    /// Il tempo ascoltato che la libreria non ha ancora passa a lei.
    fn flush_listened(&mut self) {
        if self.listened_ms > 0 {
            self.library.push(LibraryCmd::Listen(std::mem::take(&mut self.listened_ms)));
        }
    }
}

impl Job {
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Load { api, track_id, quality, generation, local } => {
                let mut lost = None;
                if let Some(file) = local {
                    // Il file si apre fuori dal runtime (legge il disco).
                    let opened = tokio::task::spawn_blocking(move || offline::open_local(&file))
                        .await
                        .unwrap_or(Err(None));
                    match opened {
                        Ok(stream) => return Done::Loaded { generation, result: Ok(stream), lost: None },
                        Err(reason) => lost = reason,
                    }
                }
                let result = match api {
                    Ok(api) => api::stream_audio(api, track_id, quality).await,
                    Err(err) => Err(err),
                };
                Done::Loaded { generation, result, lost }
            }
            Self::Warm { api, track_id, quality, whole } => {
                let result = match api {
                    Ok(api) if whole => api::stream_audio(api, track_id, quality).await,
                    Ok(api) => api::warm_audio(api, track_id, quality).await,
                    Err(err) => Err(err),
                };
                Done::Warmed { track_id, quality, result }
            }
            Self::Autoplay { api, artist } => {
                let result = match api {
                    Ok(api) => api.artist(artist).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Autoplay(result)
            }
            Self::Lyrics { api, track_id } => {
                let result = match api {
                    // Il testo si interpreta qui, sul runtime, non nella vista.
                    Ok(api) => api.lyrics(track_id).await.map(|lyrics| Arc::new(Text::of(&lyrics))),
                    Err(err) => Err(err),
                };
                Done::Lyrics { track_id, result }
            }
            Self::Album { api, id, ticket } => {
                let result = match api {
                    Ok(api) => api.album(id).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Album { ticket, result }
            }
            Self::Radio { api, start, ticket } => {
                let result = match api {
                    Ok(api) => api.artist(start.artist.id).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Radio { ticket, start, result }
            }
        }
    }

    /// Se il lavoro va perso, l'esito è un errore di rete: il dominio non resta ad aspettare.
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Load { generation, .. } => {
                Done::Loaded { generation: *generation, result: Err(AppError::Network), lost: None }
            }
            Self::Warm { track_id, quality, .. } => {
                Done::Warmed { track_id: *track_id, quality: *quality, result: Err(AppError::Network) }
            }
            Self::Autoplay { .. } => Done::Autoplay(Err(AppError::Network)),
            Self::Lyrics { track_id, .. } => {
                Done::Lyrics { track_id: *track_id, result: Err(AppError::Network) }
            }
            Self::Album { ticket, .. } => Done::Album { ticket: *ticket, result: Err(AppError::Network) },
            Self::Radio { start, ticket, .. } => {
                Done::Radio { ticket: *ticket, start: start.clone(), result: Err(AppError::Network) }
            }
        }
    }
}

/// I passi 0 di questo dominio (`characterization/playback.rs` del client), portati sul riduttore: il
/// tempo è finto (`now` lo fa avanzare il test), l'audio lo simula chi guida (le `AudioOp` si
/// raccolgono, le `AudioFact` si consegnano a mano) e la rete non c'è (i `Job` si raccolgono).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures;
    use crate::config::CoreConfig;
    use crate::player::Repeat;
    use crate::state::system::{DiscordOp, MediaOp};

    fn track(n: u128, seconds: u64) -> Track {
        let mut track = fixtures::track(n);
        track.duration_ms = seconds * 1000;
        track
    }

    fn id(n: u128) -> Uuid {
        fixtures::track(n).id
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn millis(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    /// Il dominio con il tempo e l'audio in mano al test.
    struct Rig {
        playback: Playback,
        now: Instant,
        ops: Vec<AudioOp>,
        jobs: Vec<Job>,
        /// Il battito è armato.
        armed: bool,
        /// Quante volte è stato armato.
        arms: u32,
        media: Vec<MediaOp>,
        discord: Vec<DiscordOp>,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                playback: Playback::new(CoreConfig::default().playback(), None, 7, 1_000),
                now: Instant::now(),
                ops: Vec::new(),
                jobs: Vec::new(),
                armed: false,
                arms: 0,
                media: Vec::new(),
                discord: Vec::new(),
            }
        }

        fn record(&mut self, effects: Vec<Effect>) {
            for effect in effects {
                match effect {
                    Effect::Audio(op) => self.ops.push(op),
                    Effect::Run(effect::Job::Playback(job)) => self.jobs.push(job),
                    Effect::Timer { key: TimerKey::Playback(Timer::Tick), after } => {
                        assert_eq!(after, TICK);
                        self.armed = true;
                        self.arms += 1;
                    }
                    Effect::CancelTimer(TimerKey::Playback(Timer::Tick)) => self.armed = false,
                    Effect::Media(op) => self.media.push(op),
                    Effect::Discord(op) => self.discord.push(op),
                    other => panic!("unexpected effect {other:?}"),
                }
            }
        }

        fn cmd(&mut self, command: PlaybackCmd) {
            let mut effects = Vec::new();
            self.playback.apply(command, self.now, &mut effects);
            self.record(effects);
        }

        fn done(&mut self, done: Done) {
            let mut effects = Vec::new();
            self.playback.done(done, self.now, &mut effects);
            self.record(effects);
        }

        fn audio(&mut self, fact: AudioFact) {
            let mut effects = Vec::new();
            self.playback.audio(fact, self.now, &mut effects);
            self.record(effects);
        }

        fn elapse(&mut self, duration: Duration) {
            self.now += duration;
        }

        fn view(&self) -> &PlaybackView {
            &self.playback.view
        }

        fn state(&self) -> PlaybackState {
            self.view().state.clone()
        }

        fn current(&self) -> Option<Uuid> {
            self.view().queue.current().map(|t| t.id)
        }

        /// I caricamenti chiesti dall'ultima volta, come (brano, generazione).
        fn loads(&mut self) -> Vec<(Uuid, u64)> {
            let jobs = std::mem::take(&mut self.jobs);
            jobs.iter()
                .filter_map(|job| match job {
                    Job::Load { track_id, generation, .. } => Some((*track_id, *generation)),
                    _ => None,
                })
                .collect()
        }

        fn plays_asked(&mut self) -> usize {
            let ops = std::mem::take(&mut self.ops);
            ops.iter().filter(|op| matches!(op, AudioOp::Play { .. })).count()
        }

        /// Il server manda il brano del caricamento `generation`.
        fn load_arrives(&mut self, generation: u64) {
            self.done(Done::Loaded { generation, result: Ok(StreamBuffer::new()), lost: None });
        }

        /// L'uscita audio ha aperto il brano.
        fn started(&mut self) {
            assert_eq!(self.plays_asked(), 1, "una sola richiesta di Play");
            self.audio(AudioFact::Started(Ok(())));
        }

        /// `tracks` da `start`, caricati e partiti.
        fn play(&mut self, tracks: Vec<Track>, start: usize) {
            self.cmd(PlaybackCmd::Play { tracks: tracks.into(), start });
            let (_, generation) = self.loads().pop().expect("un caricamento");
            self.load_arrives(generation);
            self.started();
            assert!(self.view().is_playing(), "{:?}", self.state());
        }

        /// Scocca il battito: il core chiede la posizione dell'audio e questa risponde.
        fn poll(&mut self, position: Duration, finished: bool) {
            let mut effects = Vec::new();
            self.playback.fire(Timer::Tick, self.now, &mut effects);
            self.record(effects);
            let polls = std::mem::take(&mut self.ops).iter().filter(|op| matches!(op, AudioOp::Poll)).count();
            assert_eq!(polls, 1, "un battito chiede una lettura");
            self.audio(AudioFact::Polled { position, finished, seek_failed: false });
        }

        /// Un battito ogni 250 ms fino a `until`, con l'audio che avanza come il tempo.
        fn listen_until(&mut self, from: Duration, until: Duration) {
            let mut at = from;
            while at < until {
                at += TICK;
                self.elapse(TICK);
                self.poll(at, false);
            }
        }

        fn library(&mut self) -> Vec<LibraryCmd> {
            self.playback.take_library()
        }
    }

    #[test]
    fn a_song_counts_after_thirty_seconds_or_half_when_short() {
        let long = secs(240);
        assert!(!heard_enough(secs(29), long));
        assert!(heard_enough(secs(30), long));
        let clip = secs(20);
        assert!(!heard_enough(secs(9), clip));
        assert!(heard_enough(secs(10), clip));
    }

    #[test]
    fn the_sleep_timers_expire_as_they_should() {
        let start = Instant::now();
        let timer = SleepTimer::new(SleepChoice::Minutes(15), start);
        assert!(!timer.expired(start + secs(14 * 60)));
        assert!(timer.expired(start + secs(15 * 60)));
        assert_eq!(timer.remaining_label(start), "15 min");
        assert_eq!(timer.remaining_label(start + secs(15 * 60 - 20)), "20 s");
        let end = SleepTimer::new(SleepChoice::EndOfTrack, start);
        assert!(!end.expired(start + secs(3600)));
        assert_eq!(end.remaining_label(start), "fine brano");
    }

    #[test]
    fn an_idle_domain_asks_for_nothing_and_arms_no_timer() {
        let mut rig = Rig::new();
        assert!(!rig.armed);
        rig.cmd(PlaybackCmd::Toggle);
        assert!(rig.loads().is_empty(), "nothing in the queue: nothing to load");
        assert_eq!(rig.state(), PlaybackState::Stopped);
        assert!(!rig.armed && rig.arms == 0);
    }

    #[test]
    fn a_stale_audio_generation_is_ignored() {
        let mut rig = Rig::new();
        rig.cmd(PlaybackCmd::Play { tracks: vec![track(1, 4), track(2, 4)].into(), start: 0 });
        let first = rig.loads()[0].1;
        rig.cmd(PlaybackCmd::Next);
        let second = rig.loads()[0].1;
        assert_eq!(second, first + 1, "ogni caricamento ha la sua generazione");
        assert_eq!(rig.state(), PlaybackState::Loading { track_id: id(2) });

        // Le risposte del caricamento superato, buone o cattive, arrivano tardi e non cambiano nulla.
        rig.load_arrives(first);
        rig.done(Done::Loaded { generation: first, result: Err(AppError::Network), lost: None });
        assert_eq!(rig.state(), PlaybackState::Loading { track_id: id(2) });
        assert_eq!(rig.plays_asked(), 0);
        assert!(rig.library().is_empty(), "nessun ascolto registrato");

        rig.load_arrives(second);
        rig.started();
        assert_eq!(rig.state(), PlaybackState::Playing);
        assert!(matches!(rig.library().as_slice(), [LibraryCmd::RememberListen(t)] if t.id == id(2)));
    }

    #[test]
    fn play_from_stopped_loads_the_current_track() {
        let mut rig = Rig::new();
        // La coda ripresa dall'ultima sessione, ferma sul secondo brano.
        let mut config = CoreConfig::default();
        Arc::make_mut(&mut config.queue).replace(vec![track(1, 4), track(2, 4)], 1, 7);
        rig.playback = Playback::new(config.playback(), None, 7, 1_000);
        assert_eq!(rig.state(), PlaybackState::Stopped);

        rig.cmd(PlaybackCmd::Toggle);
        assert_eq!(rig.state(), PlaybackState::Loading { track_id: id(2) }, "parte il brano corrente");
        assert_eq!(rig.loads(), [(id(2), 1)]);
        assert_eq!(rig.view().queue.position(), Some(1));
        assert_eq!(rig.view().clock.at(rig.now), Duration::ZERO);

        // Play durante il caricamento non fa nulla.
        rig.cmd(PlaybackCmd::Toggle);
        assert_eq!((rig.state(), rig.view().generation), (PlaybackState::Loading { track_id: id(2) }, 1));
        assert!(rig.loads().is_empty());

        // Dopo un errore, Play riprova lo stesso brano con un nuovo caricamento.
        rig.done(Done::Loaded { generation: 1, result: Err(AppError::Network), lost: None });
        assert_eq!(rig.state(), PlaybackState::Error(AppError::Network.to_string()));
        rig.cmd(PlaybackCmd::Toggle);
        assert_eq!((rig.state(), rig.view().generation), (PlaybackState::Loading { track_id: id(2) }, 2));
        assert_eq!(rig.loads(), [(id(2), 2)]);
    }

    /// The domain of a core started again from `fields`.
    fn restarted(fields: PlaybackFields) -> Rig {
        Rig { playback: Playback::new(fields, None, 7, 1_000), ..Rig::new() }
    }

    #[test]
    fn a_paused_entry_is_saved_with_its_position_and_resumes_there_after_a_restart() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 60), track(2, 60)], 0);
        rig.listen_until(Duration::ZERO, secs(20));
        assert!(rig.playback.take_persist(), "starting a track saves");
        rig.cmd(PlaybackCmd::Toggle);
        assert!(rig.playback.take_persist(), "a pause saves where it is");
        let fields = rig.playback.fields(rig.now);
        assert_eq!(fields.position_ms, 20_000);

        let mut rig = restarted(fields);
        assert_eq!((rig.state(), rig.current()), (PlaybackState::Stopped, Some(id(1))));
        assert_eq!(rig.view().clock.position, secs(20), "the stopped entry shows where it resumes");
        assert_eq!(rig.playback.fields(rig.now).position_ms, 20_000, "a save before play keeps it");
        rig.cmd(PlaybackCmd::Toggle);
        let (_, generation) = rig.loads().pop().expect("a load");
        assert_eq!(rig.playback.fields(rig.now).position_ms, 20_000, "and so does a save while it loads");
        rig.load_arrives(generation);
        rig.started();
        assert!(rig.ops.iter().any(|op| matches!(op, AudioOp::Seek(at) if *at == secs(20))), "{:?}", rig.ops);
        assert_eq!((rig.state(), rig.view().clock.at(rig.now)), (PlaybackState::Playing, secs(20)));
    }

    #[test]
    fn shutting_down_keeps_the_position_of_the_playing_entry() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 60)], 0);
        rig.listen_until(Duration::ZERO, secs(7));
        let mut effects = Vec::new();
        rig.playback.close(rig.now, &mut effects);
        assert_eq!(rig.state(), PlaybackState::Stopped);
        assert!(rig.playback.take_persist());
        assert_eq!(rig.playback.fields(rig.now).position_ms, 7_000);
    }

    #[test]
    fn starting_anything_else_forgets_the_resume_point() {
        let mut fields = CoreConfig::default().playback();
        let mut queue = Queue::default();
        queue.replace(vec![track(1, 60), track(2, 60)], 0, 3);
        fields.queue = Arc::new(queue);
        fields.position_ms = 30_000;
        let mut rig = restarted(fields);
        rig.cmd(PlaybackCmd::PlayAt(1));
        let (_, generation) = rig.loads().pop().expect("a load");
        rig.load_arrives(generation);
        rig.started();
        assert!(!rig.ops.iter().any(|op| matches!(op, AudioOp::Seek(_))), "{:?}", rig.ops);
        rig.cmd(PlaybackCmd::PlayAt(0));
        assert_eq!(rig.playback.fields(rig.now).position_ms, 0, "the first entry starts again from zero");
    }

    #[test]
    fn nothing_to_resume_without_a_current_entry() {
        let mut fields = CoreConfig::default().playback();
        fields.position_ms = 30_000;
        let rig = restarted(fields);
        assert_eq!(rig.view().clock.position, Duration::ZERO);
        assert_eq!(rig.playback.fields(rig.now).position_ms, 0);
    }

    #[test]
    fn pause_then_resume_keeps_the_position() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 4)], 0);
        let generation = rig.view().generation;
        rig.listen_until(Duration::ZERO, secs(1));
        assert_eq!(rig.view().clock.at(rig.now), secs(1));

        rig.cmd(PlaybackCmd::Toggle);
        assert_eq!(rig.state(), PlaybackState::Paused);
        assert!(!rig.armed, "in pausa il core non si sveglia");
        // L'uscita dice dov'è davvero: la pausa riancora l'orologio lì.
        rig.audio(AudioFact::Position(millis(1_040)));
        rig.elapse(secs(5));
        assert_eq!(rig.view().clock.at(rig.now), millis(1_040), "in pausa la posizione non avanza");

        rig.cmd(PlaybackCmd::Toggle);
        assert_eq!(rig.state(), PlaybackState::Playing);
        assert!(rig.armed);
        rig.elapse(secs(1));
        assert_eq!(rig.view().clock.at(rig.now), millis(2_040), "si riprende da dov'era");
        assert_eq!(rig.view().generation, generation, "pausa e ripresa non ricaricano il brano");
        assert!(rig.loads().is_empty());
    }

    #[test]
    fn no_device_reports_the_device_error() {
        // Senza scheda audio l'host risponde così a ogni Play (`run_audio` senza uscita).
        let mut none = None;
        let fact = run_audio(&mut none, AudioOp::Play { stream: StreamBuffer::new(), volume: 1.0 });
        let Some(AudioFact::Started(Err(AppError::AudioDevice))) = fact else { panic!("{fact:?}") };

        let mut rig = Rig::new();
        rig.cmd(PlaybackCmd::Play { tracks: vec![track(1, 4)].into(), start: 0 });
        let generation = rig.loads()[0].1;
        rig.load_arrives(generation);
        rig.audio(AudioFact::Started(Err(AppError::AudioDevice)));
        assert_eq!(rig.state(), PlaybackState::Error(AppError::AudioDevice.to_string()));
        assert!(rig.library().is_empty(), "un brano che non suona non è un ascolto");
        assert!(!rig.armed, "in errore il battito non gira: niente tempo ascoltato");
    }

    #[test]
    fn a_missing_track_is_healed() {
        let not_found =
            || AppError::Api { status: 404, code: "not_found".into(), message: "Brano non trovato".into() };
        let mut rig = Rig::new();
        let missing = track(1, 4);
        rig.cmd(PlaybackCmd::Play { tracks: vec![missing.clone(), track(2, 4)].into(), start: 0 });
        let generation = rig.loads()[0].1;

        // Il server non ha più il brano: lo chiede al catalogo (il serbatoio del passaggio fra i
        // domini), e intanto si resta in caricamento, senza errore.
        rig.done(Done::Loaded { generation, result: Err(not_found()), lost: None });
        assert_eq!(rig.state(), PlaybackState::Loading { track_id: id(1) });
        let wanted = rig.playback.take_heal();
        assert_eq!(wanted.iter().map(|t| t.id).collect::<Vec<_>>(), [missing.id]);
        assert!(rig.playback.take_heal().is_empty(), "il serbatoio si svuota");

        // Ritrovato con un altro identificatore: prende il suo posto in coda, e parte.
        let found = track(9, 4);
        rig.cmd(PlaybackCmd::ReplaceTrack { old: missing.id, new: Box::new(found.clone()), restart: true });
        assert_eq!(rig.current(), Some(found.id));
        assert_eq!(rig.state(), PlaybackState::Loading { track_id: found.id });
        assert_eq!(rig.view().generation, generation + 1);

        // Un brano che non si ritrova resta in errore, e lo si cerca una volta sola per sessione.
        let mut rig = Rig::new();
        rig.cmd(PlaybackCmd::Play { tracks: vec![missing.clone()].into(), start: 0 });
        let generation = rig.loads()[0].1;
        rig.done(Done::Loaded { generation, result: Err(not_found()), lost: None });
        assert_eq!(rig.playback.take_heal().len(), 1);
        rig.cmd(PlaybackCmd::HealFailed(missing.id));
        assert_eq!(
            rig.state(),
            PlaybackState::Error(
                "Questo brano non è più disponibile sul server e non l'ho ritrovato nel catalogo.".into()
            )
        );
        rig.cmd(PlaybackCmd::Toggle);
        let generation = rig.loads()[0].1;
        rig.done(Done::Loaded { generation, result: Err(not_found()), lost: None });
        assert_eq!(rig.state(), PlaybackState::Error(not_found().to_string()));
        assert!(rig.playback.take_heal().is_empty(), "la seconda volta non si cerca di nuovo");
    }

    #[test]
    fn replacing_a_track_changes_nothing_when_the_queue_does_not_hold_it() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 4)], 0);
        let before = Arc::clone(&rig.playback.view);
        rig.cmd(PlaybackCmd::ReplaceTrack { old: id(5), new: Box::new(track(6, 4)), restart: true });
        rig.cmd(PlaybackCmd::PatchCovers(Arc::new(HashMap::from([(id(5), Some("x".to_owned()))]))));
        assert!(Arc::ptr_eq(&before, &rig.playback.view));
    }

    #[test]
    fn previous_after_three_seconds_restarts() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 8), track(2, 8)], 1);
        rig.elapse(millis(3_500));
        let generation = rig.view().generation;

        // Oltre tre secondi «precedente» riparte da capo, sullo stesso brano e senza ricaricarlo.
        rig.cmd(PlaybackCmd::Previous);
        assert_eq!(rig.view().queue.position(), Some(1));
        assert_eq!(rig.view().clock.at(rig.now), Duration::ZERO);
        assert_eq!(rig.state(), PlaybackState::Playing);
        assert_eq!(rig.view().generation, generation);
        assert!(rig.ops.iter().any(|op| matches!(op, AudioOp::Seek(at) if *at == Duration::ZERO)));
        assert_eq!(rig.view().seeks, 1, "il riparti da capo è uno spostamento");

        // Entro tre secondi passa al brano prima.
        rig.elapse(secs(1));
        rig.cmd(PlaybackCmd::Previous);
        assert_eq!(rig.view().queue.position(), Some(0));
        assert_eq!(rig.state(), PlaybackState::Loading { track_id: id(1) });
        assert_eq!(rig.view().generation, generation + 1);
    }

    #[test]
    fn a_cached_or_warm_buffer_skips_the_network() {
        let mut rig = Rig::new();
        let quality = rig.view().quality;
        rig.play(vec![track(1, 4), track(2, 4)], 0);
        // Partendo, il core prepara la testata del brano dopo.
        assert!(
            rig.jobs
                .iter()
                .any(|job| matches!(job, Job::Warm { track_id, whole: false, .. } if *track_id == id(2))),
            "{:?}",
            rig.jobs
        );
        rig.jobs.clear();

        // Il brano appena suonato si riascolta dal suo buffer: nessuna richiesta, l'audio riparte.
        rig.cmd(PlaybackCmd::PlayAt(0));
        assert!(rig.loads().is_empty());
        rig.started();
        assert!(rig.view().is_playing());

        // Un brano preparato in anticipo parte dalla sua testata calda, che esce dalla cache.
        rig.done(Done::Warmed { track_id: id(2), quality, result: Ok(StreamBuffer::new()) });
        assert!(!rig.playback.warm.wants(id(2), quality), "la testata è pronta");
        rig.cmd(PlaybackCmd::Next);
        assert!(rig.loads().is_empty());
        rig.started();
        assert!(rig.playback.warm.wants(id(2), quality), "chi suona il buffer lo toglie dalla cache");
    }

    #[test]
    fn the_next_track_is_released_or_fetched_whole_in_the_last_seconds() {
        // La testata del prossimo è calda e ferma: negli ultimi secondi la si lascia scaricare.
        let mut rig = Rig::new();
        let quality = rig.view().quality;
        rig.play(vec![track(1, 60), track(2, 60)], 0);
        let head = StreamBuffer::new();
        head.hold_at(0);
        assert!(head.is_held());
        rig.done(Done::Warmed { track_id: id(2), quality, result: Ok(head.clone()) });
        rig.jobs.clear();
        rig.listen_until(Duration::ZERO, secs(34));
        assert!(head.is_held(), "troppo presto");
        rig.listen_until(secs(34), secs(36));
        assert!(!head.is_held(), "negli ultimi 25 secondi il download continua");
        assert!(!rig.jobs.iter().any(|job| matches!(job, Job::Warm { .. })), "{:?}", rig.jobs);

        // Se la testata non c'è (l'apertura è fallita), si chiede il brano per intero, una volta sola.
        let mut rig = Rig::new();
        rig.play(vec![track(1, 60), track(2, 60)], 0);
        rig.done(Done::Warmed { track_id: id(2), quality, result: Err(AppError::Network) });
        rig.jobs.clear();
        rig.listen_until(Duration::ZERO, secs(34));
        assert!(rig.jobs.is_empty(), "troppo presto");
        rig.listen_until(secs(34), secs(40));
        let whole = rig.jobs.iter().filter(|job| matches!(job, Job::Warm { whole: true, .. })).count();
        assert_eq!(whole, 1, "{:?}", rig.jobs);
    }

    #[test]
    fn a_finished_track_advances() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 1), track(2, 1)], 0);
        rig.listen_until(Duration::ZERO, millis(750));
        let generation = rig.view().generation;
        rig.library();

        rig.elapse(TICK);
        rig.poll(secs(1), true);
        assert_eq!(rig.view().queue.position(), Some(1));
        assert_eq!(rig.state(), PlaybackState::Loading { track_id: id(2) });
        assert_eq!(rig.view().generation, generation + 1);
        // Il tempo ascoltato fino al battito prima arriva alla libreria quando il brano cambia.
        assert!(matches!(rig.library().as_slice(), [LibraryCmd::Listen(750)]));
        assert!(!rig.armed, "in caricamento il core non si sveglia");
    }

    #[test]
    fn repeat_one_replays() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 1), track(2, 1)], 0);
        rig.cmd(PlaybackCmd::CycleRepeat);
        rig.cmd(PlaybackCmd::CycleRepeat);
        assert_eq!(rig.view().queue.repeat(), Repeat::One);
        let generation = rig.view().generation;

        rig.elapse(secs(1));
        rig.poll(secs(1), true);
        assert_eq!(rig.view().queue.position(), Some(0), "resta sullo stesso brano");
        assert_eq!(rig.state(), PlaybackState::Loading { track_id: id(1) }, "o meglio, riparte");
        // Lo stesso brano riparte dal buffer in memoria, senza rete: il Play è già chiesto.
        assert!(rig.loads().is_empty());
        rig.started();
        assert_eq!(rig.state(), PlaybackState::Playing);
        assert_eq!(rig.view().generation, generation + 1);
        rig.elapse(millis(500));
        assert_eq!(rig.view().clock.at(rig.now), millis(500), "ricomincia da capo");
    }

    /// BACKLOG 4: a fine coda con l'autoplay acceso, finché i brani arrivano lo stato non è più
    /// `Playing` (l'audio è finito), e il battito si ferma.
    #[test]
    fn end_of_queue_asks_autoplay_once_and_is_not_playing_meanwhile() {
        let mut rig = Rig::new();
        assert!(rig.view().autoplay, "la riproduzione automatica è accesa di default");
        rig.play(vec![track(1, 1)], 0);
        rig.jobs.clear();
        rig.elapse(secs(1));
        rig.poll(secs(1), true);

        // A fine coda si chiedono altri brani dello stesso artista.
        assert!(
            matches!(rig.jobs.as_slice(), [Job::Autoplay { artist, .. }] if *artist == track(1, 1).artist.id)
        );
        assert_eq!(rig.state(), PlaybackState::Loading { track_id: id(1) });
        assert!(!rig.armed, "il battito non gira più: nessuna seconda richiesta");

        // La richiesta fallita, senza un catalogo recente da cui pescare, ferma la musica.
        rig.done(Done::Autoplay(Err(AppError::Network)));
        assert_eq!(rig.state(), PlaybackState::Stopped);
        assert!(rig.playback.autoplay_busy.is_none());

        // Con dei brani nuovi, invece, si accodano (tolti quelli già in coda) e parte il primo.
        let mut rig = Rig::new();
        rig.play(vec![track(1, 1)], 0);
        rig.jobs.clear();
        rig.elapse(secs(1));
        rig.poll(secs(1), true);
        let page = ArtistPage {
            id: track(1, 1).artist.id,
            name: track(1, 1).artist.name,
            image_url: None,
            albums: Vec::new(),
            tracks: vec![track(1, 1), track(4, 1), track(7, 1)],
        };
        rig.done(Done::Autoplay(Ok(Box::new(page))));
        assert_eq!(rig.view().queue.len(), 3);
        assert_eq!(rig.view().queue.position(), Some(1));
        let next = rig.current();
        assert!(next == Some(id(4)) || next == Some(id(7)), "{next:?}");
        assert_eq!(rig.state(), PlaybackState::Loading { track_id: next.unwrap_or_default() });
        assert!(matches!(&rig.view().event, Some(Raised { event: PlaybackEvent::AutoplayAdded, .. })));
    }

    #[test]
    fn autoplay_falls_back_to_the_recent_catalogue_and_ignores_a_stale_answer() {
        let mut rig = Rig::new();
        let pool: Arc<[Track]> = vec![track(4, 1), track(5, 1)].into();
        rig.playback.set_pool(Some(&pool));
        rig.play(vec![track(1, 1)], 0);
        rig.elapse(secs(1));
        rig.poll(secs(1), true);
        // L'artista non ha altro: si passa alle uscite recenti di altri artisti.
        rig.done(Done::Autoplay(Ok(Box::new(ArtistPage {
            id: track(1, 1).artist.id,
            name: String::new(),
            image_url: None,
            albums: Vec::new(),
            tracks: vec![track(1, 1)],
        }))));
        assert_eq!(rig.view().queue.len(), 2, "le uscite dello stesso artista restano fuori");
        assert_eq!(rig.current(), Some(id(5)));

        // Si sceglie altro mentre i brani arrivano: la risposta tardiva non fa niente.
        let mut rig = Rig::new();
        rig.play(vec![track(1, 1)], 0);
        rig.elapse(secs(1));
        rig.poll(secs(1), true);
        rig.cmd(PlaybackCmd::Play { tracks: vec![track(8, 5)].into(), start: 0 });
        let state = rig.state();
        rig.done(Done::Autoplay(Ok(Box::new(ArtistPage {
            id: track(1, 1).artist.id,
            name: String::new(),
            image_url: None,
            albums: Vec::new(),
            tracks: vec![track(4, 1)],
        }))));
        assert_eq!((rig.state(), rig.view().queue.len()), (state, 1));

        // In una jam la coda è del gruppo: l'autoplay non chiede niente.
        let mut rig = Rig::new();
        rig.cmd(PlaybackCmd::Group(true));
        rig.play(vec![track(1, 1)], 0);
        rig.jobs.clear();
        rig.elapse(secs(1));
        rig.poll(secs(1), true);
        assert!(rig.jobs.is_empty());
        assert_eq!(rig.state(), PlaybackState::Stopped);
    }

    #[test]
    fn sleep_at_end_of_track_holds_the_next() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 1), track(2, 1)], 0);
        rig.cmd(PlaybackCmd::Sleep(Some(SleepChoice::EndOfTrack)));
        let generation = rig.view().generation;

        rig.elapse(secs(1));
        rig.poll(secs(1), true);
        assert_eq!(rig.state(), PlaybackState::Stopped);
        assert_eq!(rig.view().queue.position(), Some(1), "il prossimo brano è pronto in coda, ma non parte");
        assert_eq!(rig.view().generation, generation);
        assert!(rig.view().sleep.is_none(), "il timer si consuma");
        assert!(matches!(&rig.view().event, Some(Raised { event: PlaybackEvent::SleepStopped, .. })));

        // Play riparte dal brano dopo.
        rig.cmd(PlaybackCmd::Toggle);
        assert_eq!(rig.state(), PlaybackState::Loading { track_id: id(2) });
    }

    #[test]
    fn a_minute_timer_pauses_when_it_expires_and_not_after_a_long_pause() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 600)], 0);
        rig.cmd(PlaybackCmd::Sleep(Some(SleepChoice::Minutes(5))));
        rig.listen_until(Duration::ZERO, secs(299));
        assert!(rig.view().is_playing());
        rig.listen_until(secs(299), secs(300));
        assert_eq!(rig.state(), PlaybackState::Paused);
        assert!(rig.view().sleep.is_none());
        assert!(matches!(&rig.view().event, Some(Raised { event: PlaybackEvent::SleepStopped, .. })));

        // Un timer scaduto mentre la musica era ferma non deve fermarla alla ripresa.
        let mut rig = Rig::new();
        rig.play(vec![track(1, 600)], 0);
        rig.cmd(PlaybackCmd::Sleep(Some(SleepChoice::Minutes(5))));
        rig.cmd(PlaybackCmd::Toggle);
        rig.elapse(secs(600));
        rig.cmd(PlaybackCmd::Toggle);
        assert!(rig.view().sleep.is_none(), "il timer scaduto in pausa si butta");
        rig.elapse(TICK);
        rig.poll(millis(250), false);
        assert!(rig.view().is_playing());
    }

    #[test]
    fn a_play_counts_after_thirty_seconds_or_half() {
        // Un brano di quattro secondi conta a metà, all'ora del sistema che gli compete.
        let mut rig = Rig::new();
        let short = track(1, 4);
        rig.play(vec![short.clone()], 0);
        rig.library();
        rig.listen_until(Duration::ZERO, millis(1_500));
        assert!(!rig.view().play_counted, "un secondo e mezzo non basta");
        rig.listen_until(millis(1_500), millis(2_250));
        assert!(rig.view().play_counted);
        let counted: Vec<_> = rig
            .library()
            .into_iter()
            .filter_map(|c| match c {
                LibraryCmd::CountPlay { track, at } => Some((track.id, at)),
                _ => None,
            })
            .collect();
        assert_eq!(counted, [(short.id, 1_002)], "unix di avvio + i secondi passati dal primo comando");
        // Passo 4.7: lo stesso ascolto va una volta sola anche al dominio sociale (il profilo).
        assert_eq!(rig.playback.take_listens(), [short.id]);
        rig.listen_until(millis(2_250), millis(3_000));
        assert!(rig.playback.take_listens().is_empty(), "una volta per ascolto");
        assert!(
            !rig.library().iter().any(|c| matches!(c, LibraryCmd::CountPlay { .. })),
            "una volta per ascolto"
        );

        // Uno lungo conta dopo trenta secondi: qui ci si arriva con un salto in avanti.
        let mut rig = Rig::new();
        rig.play(vec![track(2, 180)], 0);
        rig.listen_until(Duration::ZERO, secs(1));
        assert!(!rig.view().play_counted);
        rig.cmd(PlaybackCmd::SeekBy(30));
        rig.elapse(TICK);
        rig.poll(secs(31) + TICK, false);
        assert!(rig.view().play_counted);
    }

    /// BACKLOG 6 e 15: il tempo ascoltato segue la posizione dell'audio, non i battiti.
    #[test]
    fn listened_time_follows_the_audio_position() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 600)], 0);
        rig.library();

        // Nessun campione letto: i battiti passano, la posizione no, e non si ascolta niente.
        for _ in 0..3 {
            rig.elapse(TICK);
            rig.poll(Duration::ZERO, false);
        }
        assert_eq!(rig.playback.listened_ms, 0);

        rig.listen_until(Duration::ZERO, secs(2));
        assert_eq!(rig.playback.listened_ms, 2_000);

        // Un seek è un salto, non ascolto: conta solo ciò che si sente dopo.
        rig.cmd(PlaybackCmd::SeekTo(secs(100)));
        rig.elapse(TICK);
        rig.poll(secs(100) + TICK, false);
        assert_eq!(rig.playback.listened_ms, 2_250);

        // In pausa no.
        rig.cmd(PlaybackCmd::Toggle);
        let sent: Vec<_> = rig.library().into_iter().filter(|c| matches!(c, LibraryCmd::Listen(_))).collect();
        assert!(
            matches!(sent.as_slice(), [LibraryCmd::Listen(2_250)]),
            "la pausa consegna il totale: {sent:?}"
        );
        assert_eq!(rig.playback.listened_ms, 0);
    }

    #[test]
    fn listened_time_goes_to_the_library_in_groups_not_at_every_beat() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 600)], 0);
        rig.library();
        rig.listen_until(Duration::ZERO, secs(14));
        assert!(rig.library().is_empty(), "meno di quindici secondi: ancora niente");
        rig.listen_until(secs(14), secs(16));
        let sent = rig.library();
        assert!(
            matches!(sent.as_slice(), [LibraryCmd::Listen(ms)] if (15_000..=16_000).contains(ms)),
            "{sent:?}"
        );
    }

    #[test]
    fn the_clock_is_reanchored_on_transitions_and_on_drift_only() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 600)], 0);
        let anchored = Arc::clone(&rig.playback.view);

        // Il battito che trova l'audio dove l'orologio lo calcola non cambia niente.
        rig.elapse(TICK);
        rig.poll(TICK + millis(40), false);
        assert!(Arc::ptr_eq(&anchored, &rig.playback.view) || rig.view().clock == anchored.clock);
        assert_eq!(rig.view().clock, anchored.clock, "40 ms di scarto: l'ancora resta");

        // Oltre 250 ms l'orologio si riancora sulla posizione vera.
        rig.elapse(TICK);
        rig.poll(secs(2), false);
        assert_eq!(rig.view().clock.position, secs(2));
        assert_eq!(rig.view().clock.at(rig.now), secs(2));

        // Un seek riancora subito, e da fermo la posizione resta quella.
        rig.cmd(PlaybackCmd::SeekTo(secs(100)));
        assert_eq!(rig.view().clock.at(rig.now), secs(100));
        rig.cmd(PlaybackCmd::Toggle);
        rig.elapse(secs(10));
        assert_eq!(rig.view().clock.at(rig.now), secs(100));

        // Fermo del tutto (coda svuotata): torna all'inizio.
        rig.cmd(PlaybackCmd::Clear);
        assert_eq!((rig.state(), rig.view().clock.at(rig.now)), (PlaybackState::Stopped, Duration::ZERO));
    }

    #[test]
    fn the_tick_is_armed_only_while_a_track_plays() {
        let mut rig = Rig::new();
        rig.cmd(PlaybackCmd::Play { tracks: vec![track(1, 60)].into(), start: 0 });
        assert!(!rig.armed, "in caricamento no");
        let generation = rig.loads()[0].1;
        rig.load_arrives(generation);
        rig.started();
        assert!((rig.armed, rig.arms) == (true, 1));
        rig.cmd(PlaybackCmd::Toggle);
        assert!(!rig.armed, "pausa");
        rig.cmd(PlaybackCmd::Toggle);
        assert!(rig.armed, "ripresa");
        rig.cmd(PlaybackCmd::Next);
        assert!(!rig.armed, "ultimo brano, niente dopo: fermo");
        let mut effects = Vec::new();
        rig.playback.close(rig.now, &mut effects);
        assert!(!effects.iter().any(|e| matches!(e, Effect::Timer { .. })));
    }

    #[test]
    fn volume_is_saved_on_release_and_the_mute_remembers_the_level() {
        let mut rig = Rig::new();
        assert!(!rig.playback.take_persist());
        rig.cmd(PlaybackCmd::Volume(0.4));
        rig.cmd(PlaybackCmd::Volume(0.5));
        assert_eq!(rig.view().volume, 0.5);
        assert!(!rig.playback.take_persist(), "il volume non salva mentre si trascina");
        assert!(matches!(rig.ops.last(), Some(AudioOp::Volume(v)) if *v == 0.5));
        rig.cmd(PlaybackCmd::Save);
        assert!(rig.playback.take_persist());
        assert_eq!(rig.playback.fields(rig.now).volume, 0.5);

        rig.cmd(PlaybackCmd::ToggleMute);
        assert!(rig.view().muted && rig.view().volume == 0.0);
        assert_eq!(rig.playback.fields(rig.now).volume, 0.5, "si salva il volume di prima del muto");
        assert!(rig.playback.take_persist());
        rig.cmd(PlaybackCmd::ToggleMute);
        assert!(!rig.view().muted && rig.view().volume == 0.5);
        rig.cmd(PlaybackCmd::ToggleMute);
        rig.cmd(PlaybackCmd::Volume(0.3));
        assert!(!rig.view().muted, "toccare il volume toglie il muto");
        rig.cmd(PlaybackCmd::Volume(7.0));
        assert_eq!(rig.view().volume, 1.0);
    }

    #[test]
    fn queue_changes_are_saved_and_a_no_op_publishes_nothing() {
        let mut rig = Rig::new();
        rig.cmd(PlaybackCmd::Enqueue(Box::new(track(1, 4))));
        rig.cmd(PlaybackCmd::Enqueue(Box::new(track(2, 4))));
        rig.cmd(PlaybackCmd::EnqueueNext(Box::new(track(3, 4))));
        let order: Vec<_> = rig.view().queue.entries().map(|(_, t)| t.id).collect();
        assert_eq!(order, [id(1), id(3), id(2)]);
        assert!(rig.playback.take_persist());

        let before = Arc::clone(&rig.playback.view);
        rig.cmd(PlaybackCmd::Move { from: 1, to: 1 });
        rig.cmd(PlaybackCmd::Move { from: 0, to: 9 });
        rig.cmd(PlaybackCmd::Remove(9));
        rig.cmd(PlaybackCmd::SeekTo(secs(3)));
        rig.cmd(PlaybackCmd::SeekFraction(0.5));
        rig.cmd(PlaybackCmd::Quality(rig.view().quality));
        rig.cmd(PlaybackCmd::Autoplay(true));
        rig.cmd(PlaybackCmd::Sleep(None));
        rig.cmd(PlaybackCmd::Group(false));
        assert!(Arc::ptr_eq(&before, &rig.playback.view), "nothing changed, nothing published");
        assert!(!rig.playback.take_persist());

        // Il riordino a trascinamento del telefono: il brano corrente resta lo stesso.
        rig.cmd(PlaybackCmd::Move { from: 0, to: 2 });
        let order: Vec<_> = rig.view().queue.entries().map(|(_, t)| t.id).collect();
        assert_eq!(order, [id(3), id(2), id(1)]);
        assert_eq!(rig.current(), Some(id(1)), "the current entry moves with the queue");
        assert!(rig.playback.take_persist());

        rig.cmd(PlaybackCmd::Remove(0));
        rig.cmd(PlaybackCmd::ToggleShuffle);
        assert!(rig.view().queue.shuffle());
        assert_eq!(rig.view().queue.len(), 2);
    }

    #[test]
    fn the_equalizer_goes_to_the_audio_and_is_saved_on_toggle_and_preset() {
        let mut rig = Rig::new();
        rig.cmd(PlaybackCmd::EqualizerBand { band: 2, gain: 99.0 });
        assert_eq!(rig.view().equalizer.gains[2], eq::MAX_GAIN_DB, "il guadagno si limita");
        assert!(!rig.playback.take_persist(), "una banda trascinata non salva");
        rig.cmd(PlaybackCmd::EqualizerBand { band: 99, gain: 1.0 });
        rig.cmd(PlaybackCmd::EqualizerEnabled(true));
        assert!(rig.view().equalizer.enabled && rig.playback.take_persist());
        rig.cmd(PlaybackCmd::EqualizerGains([1.0; eq::BANDS]));
        assert_eq!(rig.view().equalizer.gains, [1.0; eq::BANDS]);
        assert!(rig.ops.iter().any(|op| matches!(op, AudioOp::Equalizer(s) if s.enabled)));
    }

    #[test]
    fn a_failed_seek_is_reported_once() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 60)], 0);
        let mut effects = Vec::new();
        rig.playback.fire(Timer::Tick, rig.now, &mut effects);
        rig.record(effects);
        rig.audio(AudioFact::Polled { position: Duration::ZERO, finished: false, seek_failed: true });
        assert!(matches!(&rig.view().event, Some(Raised { seq: 1, event: PlaybackEvent::SeekFailed })));
    }

    // ---------- Passo 4.3b: testo, radio, album, mix, sistema e Discord ----------

    fn page(artist: &Track, tracks: Vec<Track>) -> ArtistPage {
        ArtistPage {
            id: artist.artist.id,
            name: artist.artist.name.clone(),
            image_url: None,
            albums: Vec::new(),
            tracks,
        }
    }

    fn lyrics_jobs(rig: &mut Rig) -> Vec<Uuid> {
        let jobs = std::mem::take(&mut rig.jobs);
        let (lyrics, other): (Vec<_>, Vec<_>) =
            jobs.into_iter().partition(|job| matches!(job, Job::Lyrics { .. }));
        rig.jobs = other;
        lyrics
            .into_iter()
            .filter_map(|job| match job {
                Job::Lyrics { track_id, .. } => Some(track_id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn lyrics_load_only_while_a_frontend_wants_them_and_follow_the_track() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 60), track(2, 60), track(3, 60)], 0);
        assert!(lyrics_jobs(&mut rig).is_empty(), "nessuno li mostra: nessuna richiesta");
        assert_eq!(rig.view().lyrics, LyricsView::default());

        // Un frontend li vuole: si chiedono quelli del brano in corso, una volta sola.
        rig.cmd(PlaybackCmd::Lyrics(true));
        assert_eq!(lyrics_jobs(&mut rig), vec![id(1)]);
        assert_eq!(rig.view().lyrics, LyricsView { track: Some(id(1)), state: LyricsState::Loading });
        rig.cmd(PlaybackCmd::Lyrics(true));
        assert!(lyrics_jobs(&mut rig).is_empty());

        let text = Arc::new(Text { lines: Vec::new(), plain: Some("ciao".into()) });
        rig.done(Done::Lyrics { track_id: id(1), result: Ok(Arc::clone(&text)) });
        assert_eq!(rig.view().lyrics.state, LyricsState::Ready(Arc::clone(&text)));

        // Un altro brano: si chiedono quelli nuovi, e la risposta di quello vecchio non conta.
        rig.cmd(PlaybackCmd::Next);
        assert_eq!(lyrics_jobs(&mut rig), vec![id(2)]);
        rig.done(Done::Lyrics { track_id: id(1), result: Ok(Arc::clone(&text)) });
        assert_eq!(rig.view().lyrics, LyricsView { track: Some(id(2)), state: LyricsState::Loading });

        // Un testo che non arriva si riprova solo se il frontend lo richiede.
        rig.done(Done::Lyrics { track_id: id(2), result: Err(AppError::Network) });
        assert_eq!(rig.view().lyrics.state, LyricsState::Failed(AppError::Network));
        rig.cmd(PlaybackCmd::Lyrics(true));
        assert_eq!(lyrics_jobs(&mut rig), vec![id(2)]);

        // Il pannello si chiude: i brani seguenti non chiedono più niente.
        rig.cmd(PlaybackCmd::Lyrics(false));
        rig.cmd(PlaybackCmd::Next);
        assert!(lyrics_jobs(&mut rig).is_empty());
        assert_eq!(rig.view().lyrics.track, Some(id(2)));
    }

    #[test]
    fn a_radio_plays_its_artist_then_discoveries_and_only_the_last_request_counts() {
        let mut rig = Rig::new();
        let start = track(1, 60);
        let pool: Arc<[Track]> = (10..30).map(|n| track(n, 60)).collect::<Vec<_>>().into();
        rig.playback.set_pool(Some(&pool));
        rig.cmd(PlaybackCmd::StartRadio(Box::new(start.clone())));
        let first = match rig.jobs.pop() {
            Some(Job::Radio { ticket, start, .. }) => (ticket, start),
            other => panic!("{other:?}"),
        };
        // Intanto si sceglie altro: la radio arriva tardi e non cambia niente.
        rig.play(vec![track(5, 60)], 0);
        rig.done(Done::Radio {
            ticket: first.0,
            start: first.1.clone(),
            result: Ok(Box::new(page(&start, vec![track(4, 60)]))),
        });
        assert_eq!(rig.current(), Some(id(5)));
        assert!(rig.view().event.is_none());

        // Questa volta è l'ultima richiesta.
        rig.cmd(PlaybackCmd::StartRadio(Box::new(start.clone())));
        let ticket = match rig.jobs.pop() {
            Some(Job::Radio { ticket, .. }) => ticket,
            other => panic!("{other:?}"),
        };
        let own: Vec<Track> = (3..40).step_by(3).map(|n| track(n, 60)).collect();
        rig.done(Done::Radio {
            ticket,
            start: Box::new(start.clone()),
            result: Ok(Box::new(page(&start, own))),
        });
        assert_eq!(rig.current(), Some(id(1)), "parte da lui");
        assert!(rig.view().queue.len() > 10 && rig.view().queue.len() <= RADIO_SIZE);
        assert!(matches!(rig.state(), PlaybackState::Loading { .. }));
        assert!(
            matches!(&rig.view().event, Some(Raised { event: PlaybackEvent::RadioStarted(t), .. }) if t.id == id(1))
        );

        // Se l'artista non risponde, la radio parte lo stesso dal catalogo recente.
        let mut rig = Rig::new();
        rig.playback.set_pool(Some(&pool));
        rig.cmd(PlaybackCmd::StartRadio(Box::new(start.clone())));
        let ticket = match rig.jobs.pop() {
            Some(Job::Radio { ticket, .. }) => ticket,
            other => panic!("{other:?}"),
        };
        rig.done(Done::Radio { ticket, start: Box::new(start), result: Err(AppError::Network) });
        assert_eq!(rig.current(), Some(id(1)));
        assert!(rig.view().queue.len() > 1);
    }

    #[test]
    fn an_album_plays_from_the_start_and_a_failure_is_reported() {
        let album = |tracks: Vec<Track>| Album {
            id: Uuid::from_u128(77),
            title: "Album".into(),
            released_on: None,
            artist: track(1, 60).artist,
            cover_url: None,
            tracks,
        };
        let ask = |rig: &mut Rig| {
            rig.cmd(PlaybackCmd::PlayAlbum(Uuid::from_u128(77)));
            match rig.jobs.pop() {
                Some(Job::Album { id, ticket, .. }) => {
                    assert_eq!(id, Uuid::from_u128(77));
                    ticket
                }
                other => panic!("{other:?}"),
            }
        };
        let mut rig = Rig::new();
        let ticket = ask(&mut rig);
        rig.done(Done::Album { ticket, result: Ok(Box::new(album(vec![track(7, 60), track(8, 60)]))) });
        assert_eq!(rig.current(), Some(id(7)));
        assert_eq!(rig.view().queue.len(), 2);
        assert!(matches!(&rig.view().event, Some(Raised { event: PlaybackEvent::AlbumStarted, .. })));
        assert_eq!(rig.loads().len(), 1, "il primo brano si carica");

        // Un album vuoto non fa niente; un errore si dice.
        let ticket = ask(&mut rig);
        rig.done(Done::Album { ticket, result: Ok(Box::new(album(Vec::new()))) });
        assert_eq!(rig.current(), Some(id(7)));
        let ticket = ask(&mut rig);
        rig.done(Done::Album { ticket, result: Err(AppError::Network) });
        assert!(matches!(
            &rig.view().event,
            Some(Raised { event: PlaybackEvent::PlayFailed(AppError::Network), .. })
        ));

        // Una risposta superata dalla scelta di un altro brano non lo scavalca.
        let ticket = ask(&mut rig);
        rig.cmd(PlaybackCmd::PlayAt(1));
        rig.done(Done::Album { ticket, result: Ok(Box::new(album(vec![track(9, 60)]))) });
        assert_eq!(rig.current(), Some(id(8)));
    }

    #[test]
    fn a_mix_plays_from_the_feed_and_an_unknown_one_does_nothing() {
        use crate::config::taste::Mix;
        let mut rig = Rig::new();
        let feed = Arc::new(Feed {
            mixes: vec![Mix {
                title: "Mix".into(),
                subtitle: String::new(),
                tracks: vec![track(3, 60), track(4, 60)],
            }],
            ..Feed::default()
        });
        rig.cmd(PlaybackCmd::PlayMix(0));
        assert!(rig.view().queue.is_empty(), "senza feed non c'è nessun mix");
        rig.playback.set_feed(&feed);
        rig.cmd(PlaybackCmd::PlayMix(5));
        assert!(rig.view().queue.is_empty());
        rig.cmd(PlaybackCmd::PlayMix(0));
        assert_eq!(rig.current(), Some(id(3)));
        assert_eq!(rig.view().queue.len(), 2);
        assert_eq!(rig.loads().len(), 1);
    }

    /// Un frontend con il collegamento multimediale pronto.
    fn rig_with_media() -> Rig {
        let mut rig = Rig::new();
        let mut effects = Vec::new();
        rig.playback.media_ready(rig.now, &mut effects);
        rig.record(effects);
        rig
    }

    fn shown(rig: &mut Rig) -> Vec<Option<(String, bool)>> {
        std::mem::take(&mut rig.media)
            .into_iter()
            .filter_map(|op| match op {
                MediaOp::Show(now) => Some(now.map(|now| (now.title, now.playing))),
                MediaOp::Volume(_) => None,
            })
            .collect()
    }

    #[test]
    fn the_system_is_told_at_a_transition_and_never_at_a_beat() {
        let mut rig = rig_with_media();
        // Appena pronto: niente suona, e il volume.
        assert_eq!(shown(&mut rig), vec![None]);
        rig.media.clear();

        rig.cmd(PlaybackCmd::Play { tracks: vec![track(1, 60)].into(), start: 0 });
        // Il brano che si carica non suona ancora: il sistema non ne sa niente.
        assert!(shown(&mut rig).is_empty());
        let (_, generation) = rig.loads().pop().expect("un caricamento");
        rig.load_arrives(generation);
        rig.started();
        assert_eq!(shown(&mut rig), vec![Some(("Brano 1".to_string(), true))]);

        // I battiti che non cambiano niente non dicono niente al sistema.
        rig.listen_until(Duration::ZERO, secs(3));
        assert!(rig.media.is_empty(), "{:?}", rig.media);

        // Un salto, la pausa e la ripresa sì, ognuno una volta; la posizione è quella dell'orologio.
        rig.cmd(PlaybackCmd::SeekTo(secs(30)));
        let positions: Vec<Duration> = std::mem::take(&mut rig.media)
            .into_iter()
            .filter_map(|op| match op {
                MediaOp::Show(Some(now)) => Some(now.position),
                _ => None,
            })
            .collect();
        assert_eq!(positions, vec![secs(30)]);
        rig.cmd(PlaybackCmd::Toggle);
        assert_eq!(shown(&mut rig), vec![Some(("Brano 1".to_string(), false))]);
        rig.cmd(PlaybackCmd::Toggle);
        assert_eq!(shown(&mut rig), vec![Some(("Brano 1".to_string(), true))]);

        // La coda svuotata: il sistema torna a «niente».
        rig.cmd(PlaybackCmd::Clear);
        assert_eq!(shown(&mut rig), vec![None]);
    }

    #[test]
    fn the_volume_reaches_the_system_once_per_change() {
        let mut rig = rig_with_media();
        rig.media.clear();
        rig.cmd(PlaybackCmd::Volume(0.4));
        rig.cmd(PlaybackCmd::Volume(0.4));
        let volumes: Vec<f32> = std::mem::take(&mut rig.media)
            .into_iter()
            .filter_map(|op| match op {
                MediaOp::Volume(volume) => Some(volume),
                MediaOp::Show(_) => None,
            })
            .collect();
        assert_eq!(volumes, vec![0.4]);
    }

    #[test]
    fn the_cover_pixels_are_shown_only_for_the_current_track() {
        let mut rig = rig_with_media();
        let mut with_cover = track(1, 60);
        with_cover.cover_url = Some("https://covers.example/1.jpg".into());
        rig.play(vec![with_cover], 0);
        rig.media.clear();
        let art = |url: &str| {
            PlaybackCmd::Artwork(Box::new(Artwork {
                url: url.into(),
                side: 2,
                rgba: bytes::Bytes::from_static(&[0; 16]),
            }))
        };
        // La copertina di un altro brano non serve.
        rig.cmd(art("https://covers.example/other.jpg"));
        assert!(rig.media.is_empty());
        // Quella del brano in corso la riceve il sistema, una volta.
        rig.cmd(art("https://covers.example/1.jpg"));
        let artwork: Vec<Option<u32>> = std::mem::take(&mut rig.media)
            .into_iter()
            .filter_map(|op| match op {
                MediaOp::Show(Some(now)) => Some(now.artwork.map(|(side, _)| side)),
                _ => None,
            })
            .collect();
        assert_eq!(artwork, vec![Some(2)]);
    }

    #[test]
    fn system_commands_become_the_listeners_commands() {
        use crate::media::Event;
        let command = |rig: &mut Rig, event: Event| {
            let mut effects = Vec::new();
            rig.playback.media(event, rig.now, &mut effects);
            rig.record(effects);
        };
        let mut rig = Rig::new();
        rig.play(vec![track(1, 120), track(2, 120)], 0);

        // «Play» mentre suona non fa niente; «Pause» mette in pausa; «Play» riprende.
        command(&mut rig, Event::Play);
        assert!(rig.view().is_playing());
        command(&mut rig, Event::Pause);
        assert_eq!(rig.state(), PlaybackState::Paused);
        command(&mut rig, Event::Pause);
        assert_eq!(rig.state(), PlaybackState::Paused);
        command(&mut rig, Event::Play);
        assert!(rig.view().is_playing());
        command(&mut rig, Event::Toggle);
        assert_eq!(rig.state(), PlaybackState::Paused);

        command(&mut rig, Event::SetPosition(secs(40)));
        assert_eq!(rig.view().clock.position, secs(40));
        command(&mut rig, Event::SeekBy(-10));
        assert_eq!(rig.view().clock.position, secs(30));
        command(&mut rig, Event::Next);
        assert_eq!(rig.current(), Some(id(2)));
        command(&mut rig, Event::Previous);
        assert_eq!(rig.current(), Some(id(1)));

        // Il volume dal sistema si salva, come lo slider rilasciato.
        command(&mut rig, Event::SetVolume(0.3));
        assert!((rig.view().volume - 0.3).abs() < f32::EPSILON);
        assert!(rig.playback.take_persist());

        // «Raise» arriva al frontend come un evento.
        command(&mut rig, Event::Raise);
        assert!(matches!(&rig.view().event, Some(Raised { event: PlaybackEvent::Raise, .. })));
    }

    #[test]
    fn discord_follows_the_setting_and_shows_only_what_plays() {
        let mut rig = Rig::new();
        let set = |rig: &mut Rig, on: bool, app: &str| {
            let mut effects = Vec::new();
            rig.playback.set_discord(on, app, rig.now, &mut effects);
            rig.record(effects);
        };
        let ops = |rig: &mut Rig| -> Vec<String> {
            std::mem::take(&mut rig.discord)
                .into_iter()
                .map(|op| match op {
                    DiscordOp::Start(id) => format!("start {id}"),
                    DiscordOp::Stop => "stop".into(),
                    DiscordOp::Set(None) => "nothing".into(),
                    DiscordOp::Set(Some(now)) => format!("{} @ {:?}", now.title, now.position),
                })
                .collect()
        };

        // Spento, o con lo stesso valore: niente.
        set(&mut rig, false, "123");
        assert!(ops(&mut rig).is_empty());
        set(&mut rig, true, " 123 ");
        assert_eq!(ops(&mut rig), vec!["start 123", "nothing"]);
        set(&mut rig, true, "123");
        assert!(ops(&mut rig).is_empty(), "nessun cambiamento");

        // Suona: Discord lo mostra; i battiti no; in pausa sparisce; un salto lo rimanda.
        rig.play(vec![track(1, 60)], 0);
        let after_play = ops(&mut rig);
        assert_eq!(after_play.last().map(String::as_str), Some("Brano 1 @ 0ns"), "{after_play:?}");
        rig.listen_until(Duration::ZERO, secs(2));
        assert!(ops(&mut rig).is_empty());
        rig.cmd(PlaybackCmd::SeekTo(secs(20)));
        assert_eq!(ops(&mut rig), vec!["Brano 1 @ 20s"]);
        rig.cmd(PlaybackCmd::Toggle);
        assert_eq!(ops(&mut rig), vec!["nothing"]);

        // L'app cambia: il thread si riavvia; l'interruttore spento lo ferma.
        set(&mut rig, true, "456");
        assert_eq!(ops(&mut rig), vec!["stop", "start 456", "nothing"]);
        set(&mut rig, false, "456");
        assert_eq!(ops(&mut rig), vec!["stop"]);
    }

    // ---------- Gruppo (jam) ----------

    fn align(tracks: Option<Vec<Track>>, index: usize, position: Duration, playing: bool) -> PlaybackCmd {
        PlaybackCmd::Align(Box::new(Align { tracks, index, position, playing }))
    }

    #[test]
    fn a_group_track_starts_at_the_shared_position_and_stays_paused_if_the_group_is() {
        let mut rig = Rig::new();
        let queue = vec![track(1, 200), track(2, 200)];
        rig.cmd(align(Some(queue.clone()), 1, secs(30), false));
        assert_eq!(rig.state(), PlaybackState::Loading { track_id: id(2) });
        let (_, generation) = rig.loads().pop().expect("a load");
        // La stessa risposta del gruppo arriva di nuovo mentre il brano si carica: si ricorda la
        // posizione, senza ricominciare.
        rig.cmd(align(None, 1, secs(31), false));
        assert!(rig.loads().is_empty());

        rig.load_arrives(generation);
        rig.started();
        assert_eq!(rig.state(), PlaybackState::Paused, "il gruppo è in pausa: ci si ferma lì");
        assert_eq!(rig.view().clock.at(rig.now), secs(31));
        assert!(rig.ops.iter().any(|op| matches!(op, AudioOp::Seek(at) if *at == secs(31))));
        assert!(!rig.armed);
    }

    #[test]
    fn aligning_to_the_group_seeks_on_drift_and_follows_its_pause() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 200), track(2, 200)], 0);
        rig.ops.clear();
        // Stesso brano, pochi decimi di scarto: niente da fare.
        rig.cmd(align(None, 0, millis(500), true));
        assert!(rig.ops.is_empty());
        // Oltre la soglia ci si sposta, e se il gruppo è in pausa ci si ferma.
        rig.cmd(align(None, 0, secs(60), false));
        assert!(rig.ops.iter().any(|op| matches!(op, AudioOp::Seek(at) if *at == secs(60))));
        assert_eq!(rig.state(), PlaybackState::Paused);
        // Un altro brano del gruppo si carica da capo.
        rig.cmd(align(None, 1, secs(5), true));
        assert_eq!(rig.state(), PlaybackState::Loading { track_id: id(2) });
    }

    #[test]
    fn closing_stops_the_audio_and_hands_the_listened_time_over() {
        let mut rig = Rig::new();
        rig.play(vec![track(1, 600)], 0);
        rig.library();
        rig.listen_until(Duration::ZERO, secs(2));
        let mut effects = Vec::new();
        rig.playback.close(rig.now, &mut effects);
        rig.record(effects);
        assert!(rig.ops.iter().any(|op| matches!(op, AudioOp::Stop)));
        assert!(matches!(rig.library().as_slice(), [LibraryCmd::Listen(2_000)]));
        assert!(!rig.armed);
    }

    // ---------- Brani sul dispositivo (passo 4.6b) ----------

    /// Un file di `size` byte in una cartella nuova, per i test che lo aprono davvero.
    fn local_file(size: usize) -> LocalFile {
        let dir = std::env::temp_dir().join(format!("psytify-playback-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("brano.flac");
        std::fs::write(&path, vec![0u8; size]).unwrap();
        LocalFile { path, size: size as u64 }
    }

    fn local_map(entries: &[(u128, &LocalFile)]) -> Arc<HashMap<Uuid, LocalFile>> {
        Arc::new(entries.iter().map(|(n, file)| (id(*n), (*file).clone())).collect())
    }

    /// I caricamenti chiesti dall'ultima volta, con il file locale di ciascuno.
    fn load_jobs(rig: &mut Rig) -> Vec<(Uuid, u64, Option<LocalFile>)> {
        let jobs = std::mem::take(&mut rig.jobs);
        jobs.into_iter()
            .filter_map(|job| match job {
                Job::Load { track_id, generation, local, .. } => Some((track_id, generation, local)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_track_saved_on_the_device_loads_from_its_file_and_gets_no_warm_head() {
        let file = local_file(10);
        let mut rig = Rig::new();
        rig.playback.set_local(&local_map(&[(1, &file), (2, &file)]));
        rig.cmd(PlaybackCmd::Play {
            tracks: vec![track(1, 200), track(2, 200), track(3, 200)].into(),
            start: 0,
        });
        let loads = load_jobs(&mut rig);
        assert_eq!(loads, [(id(1), 1, Some(file.clone()))], "il file locale ha la precedenza");

        rig.load_arrives(1);
        rig.started();
        let warmed: Vec<Uuid> = rig
            .jobs
            .iter()
            .filter_map(|job| match job {
                Job::Warm { track_id, .. } => Some(*track_id),
                _ => None,
            })
            .collect();
        assert_eq!(warmed, [id(3)], "il brano 2 è sul dispositivo: nessuna testata calda dalla rete");
    }

    #[test]
    fn the_local_file_beats_the_cached_stream_of_a_previous_listen() {
        let file = local_file(10);
        let mut rig = Rig::new();
        rig.play(vec![track(1, 200)], 0);
        rig.jobs.clear();
        // Senza file locale, riascoltarlo riusa il buffer in memoria: nessun caricamento.
        rig.cmd(PlaybackCmd::Play { tracks: vec![track(1, 200)].into(), start: 0 });
        assert!(load_jobs(&mut rig).is_empty());
        rig.started();

        // Scaricato nel frattempo: si suona dal file, non dal buffer.
        rig.playback.set_local(&local_map(&[(1, &file)]));
        rig.cmd(PlaybackCmd::Play { tracks: vec![track(1, 200)].into(), start: 0 });
        assert_eq!(load_jobs(&mut rig).len(), 1);
    }

    #[test]
    fn a_saved_file_that_could_not_open_is_reported_once_for_the_current_track_only() {
        let file = local_file(10);
        let mut rig = Rig::new();
        rig.playback.set_local(&local_map(&[(1, &file)]));
        rig.cmd(PlaybackCmd::Play { tracks: vec![track(1, 200)].into(), start: 0 });
        let (_, generation, _) = load_jobs(&mut rig).pop().expect("a load");

        // Una risposta di un caricamento superato non dice niente sul brano di adesso.
        rig.done(Done::Loaded {
            generation: generation + 7,
            result: Ok(StreamBuffer::new()),
            lost: Some(LostReason::Missing),
        });
        assert!(rig.playback.take_lost_files().is_empty());

        // Il file non c'era: il brano è arrivato dalla rete e il dominio lo dice ai download.
        rig.done(Done::Loaded {
            generation,
            result: Ok(StreamBuffer::new()),
            lost: Some(LostReason::Missing),
        });
        assert_eq!(rig.playback.take_lost_files(), [(id(1), LostReason::Missing)]);
        assert!(rig.playback.take_lost_files().is_empty(), "si svuota");
        rig.started();
        assert!(rig.view().is_playing());
    }

    #[test]
    fn a_saved_file_that_does_not_decode_falls_back_to_the_network_once() {
        let file = local_file(10);
        let mut rig = Rig::new();
        rig.playback.set_local(&local_map(&[(1, &file)]));
        rig.cmd(PlaybackCmd::Play { tracks: vec![track(1, 200)].into(), start: 0 });
        let (_, generation, _) = load_jobs(&mut rig).pop().expect("a load");
        let from_disk = StreamBuffer::open_file(&file.path, file.size).unwrap();
        rig.done(Done::Loaded { generation, result: Ok(from_disk), lost: None });
        assert_eq!(rig.plays_asked(), 1);

        // L'uscita audio non lo decodifica: niente errore, il brano riparte dalla rete, senza file locale.
        rig.audio(AudioFact::Started(Err(AppError::AudioDecode)));
        assert!(matches!(rig.state(), PlaybackState::Loading { .. }), "{:?}", rig.state());
        assert_eq!(rig.playback.take_lost_files(), [(id(1), LostReason::Changed)]);
        assert_eq!(load_jobs(&mut rig), [(id(1), generation, None)]);

        rig.load_arrives(generation);
        rig.started();
        assert!(rig.view().is_playing());
        // Un errore di una sorgente di rete resta un errore.
        rig.cmd(PlaybackCmd::Play { tracks: vec![track(2, 200)].into(), start: 0 });
        let (_, generation, _) = load_jobs(&mut rig).pop().expect("a load");
        rig.load_arrives(generation);
        rig.audio(AudioFact::Started(Err(AppError::AudioDecode)));
        assert!(matches!(rig.state(), PlaybackState::Error(_)));
    }

    #[test]
    fn the_load_job_opens_the_local_file_from_disk_and_falls_back_when_it_is_gone() {
        let file = local_file(2048);
        let load = |local: Option<LocalFile>| Job::Load {
            api: ApiClient::new("http://127.0.0.1:9"),
            track_id: id(1),
            quality: Quality::Lossless,
            generation: 4,
            local,
        };
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let Done::Loaded { generation: 4, result: Ok(stream), lost: None } =
            runtime.block_on(load(Some(file.clone())).run())
        else {
            panic!("the file opens")
        };
        assert!(stream.is_file() && stream.is_complete());

        // Il file è sparito: ripiega sulla rete (qui una porta chiusa: l'errore è quello della rete) e dice perché.
        std::fs::remove_file(&file.path).unwrap();
        let Done::Loaded { result: Err(_), lost, .. } = runtime.block_on(load(Some(file)).run()) else {
            panic!("falls back to the network and fails")
        };
        assert_eq!(lost, Some(LostReason::Missing));
    }
}
