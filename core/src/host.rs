//! L'host del core: il thread `psytify-core` che tiene lo stato, riceve i comandi, esegue gli effetti
//! chiesti dal riduttore e pubblica le istantanee. I frontend vedono solo [`Core`]: comandi in
//! ingresso, istantanee in uscita.
//!
//! Fermo, il core non lavora: aspetta un comando, l'esito di un lavoro o il prossimo timer armato,
//! e senza timer non si sveglia. Pubblica solo quando un dominio cambia, una volta per giro: un
//! frontend fermo non riceve nulla, e una raffica di comandi arriva come una sola istantanea.

use std::collections::VecDeque;
use std::io;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::{FutureExt, Stream, StreamExt};
use tokio::runtime::{Builder, Handle};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::api::ApiClient;
use crate::artwork::{self, CoverPixels};
use crate::command::Command;
use crate::config;
use crate::effect::{Done, Effect, TimerKey};
use crate::error::AppError;
use crate::jam;
use crate::media;
use crate::player::AudioOutput;
use crate::snapshot::Snapshot;
use crate::state::jam::JamOp;
use crate::state::playback::run_audio;
use crate::state::system::{DiscordOp, MediaOp};
use crate::state::{CoreState, Input};
use crate::system_motion::SystemMotion;

/// Quanto aspetta, alla chiusura, il lavoro bloccante ancora in corso sul runtime proprio del core.
const RUNTIME_GRACE: Duration = Duration::from_secs(1);

/// Al più tanti comandi già in coda entrano nello stesso giro (e nella stessa istantanea).
const BURST: usize = 64;

/// Su quale runtime di tokio gira il lavoro asincrono del core.
#[derive(Debug, Clone)]
pub enum RuntimeChoice {
    /// Quello del frontend (Iced ne ha uno multi-thread): resta un solo runtime nel processo. Il
    /// frontend aspetta [`Core::shutdown`] prima di chiuderlo.
    Borrow(Handle),
    /// Uno del core, con `workers` thread (almeno uno), per i frontend che non usano tokio.
    Own { workers: usize },
}

/// Quale uscita audio apre il core. Il core la tiene per tutta la sua vita, sul proprio thread: la
/// scheda audio non si tocca da nessun altro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioChoice {
    /// La scheda audio predefinita del sistema. Se non si apre, la riproduzione risponde con
    /// l'errore a ogni brano e la vista lo dice (`PlaybackView::device_error`).
    #[default]
    System,
    /// Nessuna uscita (un frontend senza suono).
    None,
    /// Un'uscita senza scheda audio, da cui nessuno legge i campioni: i test che avviano un core
    /// vero senza toccare la scheda.
    #[cfg(any(test, feature = "test-support"))]
    Detached,
}

/// I collegamenti col sistema che il core tiene sul proprio thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SystemChoice {
    /// I controlli multimediali del sistema (MPRIS su Linux, la sessione multimediale su Android:
    /// i comandi entrano nel core senza passare dall'interfaccia) e, se l'utente lo vuole, Discord.
    #[default]
    Desktop,
    /// Nessun collegamento: un frontend senza widget del sistema, e i test (che non devono
    /// registrarsi sul bus della sessione).
    None,
}

/// Come si avvia il core.
#[derive(Debug, Clone)]
pub struct CoreOptions {
    pub runtime: RuntimeChoice,
    pub audio: AudioChoice,
    pub system: SystemChoice,
    /// La cartella dei dati di questo core: `config.json` si legge all'avvio e si scrive lì. Senza,
    /// il core parte dai default e non scrive niente (i test, un frontend che non salva).
    pub data_dir: Option<PathBuf>,
    /// Dove finiscono i brani scaricati. Senza, `data_dir/Download`; senza nemmeno quella, una
    /// cartella temporanea (un core di prova non scrive mai nella cartella Download di chi lo
    /// prova). I pacchetti degli aggiornamenti stanno in `data_dir/updates`.
    pub downloads_dir: Option<PathBuf>,
}

/// Il core non è partito, o non ha salvato prima di fermarsi.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// Il sistema non ha dato il thread o il runtime.
    #[error("il core non è partito: {0}")]
    Start(#[from] io::Error),
    /// L'ultima scrittura di `config.json`, alla chiusura, non è riuscita: il file resta quello di
    /// prima, intero.
    #[error("impostazioni non salvate: {0}")]
    Save(AppError),
}

/// Chi aspetta la fine del core, e com'è andato l'ultimo salvataggio.
type Stopped = oneshot::Sender<Result<(), CoreError>>;

/// Ciò che arriva al thread del core.
#[derive(Debug)]
enum Control {
    Command(Command),
    /// Fermarsi dopo aver salvato, e dirlo a chi aspetta quando è fatto.
    Shutdown(Stopped),
}

/// La prima istantanea e il runtime su cui gira il lavoro del core, che il thread del core manda a
/// `start` quando è pronto.
type Ready = io::Result<(watch::Receiver<Arc<Snapshot>>, Handle)>;

/// Il core in esecuzione, visto da un frontend. Lasciarlo cadere ferma il core anche senza
/// [`Core::shutdown`], ma senza aspettarlo e senza scrivere le modifiche ancora in attesa.
#[derive(Debug)]
pub struct Core {
    commands: mpsc::UnboundedSender<Control>,
    changes: watch::Receiver<Arc<Snapshot>>,
    /// Il runtime del core: ci girano le copertine chieste da un frontend ([`Core::artwork`]).
    runtime: Handle,
    /// Il client HTTP delle copertine, rifatto solo se il backend cambia (le connessioni si riusano).
    covers: Mutex<Option<ApiClient>>,
}

impl Core {
    /// Avvia il thread del core e aspetta che sia pronto: al ritorno c'è già la prima istantanea.
    pub fn start(options: CoreOptions) -> Result<Core, CoreError> {
        let (commands, inbox) = mpsc::unbounded_channel();
        let (ready, started) = std_mpsc::sync_channel(1);
        std::thread::Builder::new().name("psytify-core".into()).spawn(move || host(options, inbox, ready))?;
        let (changes, runtime) =
            started.recv().map_err(|_| io::Error::other("il thread del core si è fermato all'avvio"))??;
        Ok(Core { commands, changes, runtime, covers: Mutex::new(None) })
    }

    /// Manda un comando al core. Non blocca mai; a core fermo il comando si perde.
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(Control::Command(command));
    }

    /// L'ultima istantanea pubblicata.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.changes.borrow())
    }

    /// Le istantanee a venire: `changed().await` si sveglia solo quando il core ne pubblica una.
    /// Un ricevitore preso subito dopo `start` non perde nulla di ciò che segue la prima.
    pub fn changes(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.changes.clone()
    }

    /// Scarica e decodifica una copertina, fuori dal thread del frontend: l'indirizzo va al backend
    /// (`/v1/cover`, mai alla fonte) e la risposta è un quadrato RGBA già ridotto
    /// ([`artwork::CoverPixels`]). `None` se l'indirizzo non è ammesso, se il backend non risponde,
    /// se l'immagine non è valida o se il core si ferma prima: il frontend resta sulla copertina
    /// disegnata. Il frontend tiene la sua cache di texture e chiede ogni indirizzo una volta sola.
    pub fn artwork(&self, url: String) -> oneshot::Receiver<Option<CoverPixels>> {
        self.artwork_side(url, artwork::COVER_SIDE)
    }

    /// [`Core::artwork`] reduced to at most `side` pixels (at most [`artwork::COVER_SIDE_MAX`]), for a
    /// frontend that draws a cover larger than the default.
    pub fn artwork_side(&self, url: String, side: u32) -> oneshot::Receiver<Option<CoverPixels>> {
        let (done, answer) = oneshot::channel();
        let api = self.cover_client();
        self.runtime.spawn(async move {
            let pixels = match api {
                Some(api) => artwork::fetch_side(&api, &url, side).await,
                None => None,
            };
            let _ = done.send(pixels);
        });
        answer
    }

    /// Il client delle copertine per il backend scelto adesso nelle impostazioni.
    fn cover_client(&self) -> Option<ApiClient> {
        let snapshot = self.snapshot();
        let base = config::api_url(snapshot.settings.config.server.as_deref());
        let mut cached = self.covers.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(api) = cached.as_ref().filter(|api| api.base_url() == base) {
            return Some(api.clone());
        }
        let api = ApiClient::new(&base).ok()?;
        *cached = Some(api.clone());
        Some(api)
    }

    /// Ferma il core dopo aver salvato: aspetta la scrittura di `config.json` in corso e scrive
    /// subito ciò che manca. Il ricevitore si risolve quando il thread ha chiuso tutto (lo stato e
    /// il runtime proprio, se c'è): un frontend che ha prestato il suo runtime lo aspetta prima di
    /// chiuderlo. Dentro c'è [`CoreError::Save`] se l'ultima scrittura non è riuscita; un errore di
    /// ricezione vuol dire che il thread si era già fermato.
    pub fn shutdown(self) -> oneshot::Receiver<Result<(), CoreError>> {
        let (done, stopped) = oneshot::channel();
        let _ = self.commands.send(Control::Shutdown(done));
        stopped
    }
}

/// Il seme dei mix: cambia a ogni avvio e resta lo stesso per la sessione. Lo sceglie l'host perché
/// il riduttore non legge l'orologio.
fn mix_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0x5eed)
}

/// Il corpo del thread `psytify-core`. Il runtime proprio nasce e muore qui, mai nel contesto
/// asincrono di chi ha chiamato `start`.
fn host(options: CoreOptions, inbox: mpsc::UnboundedReceiver<Control>, ready: std_mpsc::SyncSender<Ready>) {
    let (downloads, updates) = download_dirs(&options);
    let (handle, owned) = match options.runtime {
        RuntimeChoice::Borrow(handle) => (handle, None),
        RuntimeChoice::Own { workers } => {
            match Builder::new_multi_thread()
                .worker_threads(workers.max(1))
                .thread_name("psytify-core-io")
                .enable_all()
                .build()
            {
                Ok(runtime) => (runtime.handle().clone(), Some(runtime)),
                Err(err) => {
                    let _ = ready.send(Err(err));
                    return;
                }
            }
        }
    };
    // `config.json` si legge qui, una volta, prima della prima istantanea: al ritorno di `start` il
    // frontend ha già la configurazione, come quando la leggeva da sé.
    let path = options.data_dir.as_deref().map(config::file_in);
    let config = config::load(path.as_deref());
    // L'uscita audio nasce, vive e muore su questo thread (la scheda audio non va spostata altrove).
    let (mut audio, device_error) = open_audio(options.audio);
    if let Some(audio) = &audio {
        audio.set_equalizer(config.core.equalizer);
    }
    let mut system = Integrations::open(options.system);
    // La sessione del login sta accanto a `config.json`, nella cartella di questo core; se c'è, il
    // dominio sociale parte già su «accesso in corso» e la riprende all'avvio.
    let session_file = options.data_dir.as_ref().map(|dir| dir.join("session.json"));
    let session_stored = session_file.as_deref().is_some_and(std::path::Path::exists);
    let state = CoreState::new(config, path, mix_seed())
        .with_device_error(device_error)
        .with_dirs(downloads, updates)
        .with_session(session_file, session_stored);
    let (publish, changes) = watch::channel(Arc::new(state.snapshot()));
    if ready.send(Ok((changes, handle.clone()))).is_err() {
        return;
    }
    let stopped = handle.block_on(actor(state, inbox, publish, &mut audio, &mut system));
    drop(system);
    drop(audio);
    if let Some(runtime) = owned {
        runtime.shutdown_timeout(RUNTIME_GRACE);
    }
    if let Some((stopped, saved)) = stopped {
        let _ = stopped.send(saved.map_err(CoreError::Save));
    }
}

/// Le cartelle dei download e degli aggiornamenti di questo core (vedi [`CoreOptions::downloads_dir`]).
fn download_dirs(options: &CoreOptions) -> (PathBuf, PathBuf) {
    let temp = std::env::temp_dir();
    let downloads = options
        .downloads_dir
        .clone()
        .or_else(|| options.data_dir.as_ref().map(|dir| dir.join("Download")))
        .unwrap_or_else(|| temp.join("psytify-downloads"));
    let updates =
        options.data_dir.as_ref().map_or_else(|| temp.join("psytify-updates"), |dir| dir.join("updates"));
    (downloads, updates)
}

/// Apre l'uscita audio scelta: l'uscita (se c'è) e perché non si è aperta (se non si è aperta).
fn open_audio(choice: AudioChoice) -> (Option<AudioOutput>, Option<AppError>) {
    match choice {
        AudioChoice::System => match AudioOutput::open() {
            Ok(output) => (Some(output), None),
            Err(err) => (None, Some(err)),
        },
        AudioChoice::None => (None, None),
        #[cfg(any(test, feature = "test-support"))]
        AudioChoice::Detached => (Some(AudioOutput::detached().0), None),
    }
}

/// I collegamenti col sistema: i comandi che arrivano (tasti multimediali), il capo per mostrare il
/// brano, Discord e la stanza della jam. Nascono, vivono e muoiono sul thread del core, come
/// l'uscita audio.
struct Integrations {
    /// Con `SystemChoice::None` niente si esegue: un core di prova non deve accendere Discord né
    /// mostrare un brano nel widget di chi lo prova.
    desktop: bool,
    /// I comandi del sistema; `None` se non c'è collegamento o il flusso è finito.
    commands: Option<Pin<Box<dyn Stream<Item = media::Event> + Send>>>,
    /// Il collegamento multimediale, dopo `Event::Ready`.
    media: Option<media::Handle>,
    #[cfg(not(target_os = "android"))]
    discord: Option<crate::discord::Presence>,
    /// Il collegamento alla stanza della jam, finché c'è una jam. A differenza degli altri non
    /// dipende da `SystemChoice`: non è un widget del sistema, è rete, e un test lo prova con una
    /// stanza finta.
    jam: Option<jam::Link>,
    /// Il collegamento che ascolta il movimento chiesto dal sistema, mentre un frontend lo vuole
    /// (`Effect::WatchMotion`); `None` se spento o se il flusso è finito (nessun bus).
    motion: Option<Pin<Box<dyn Stream<Item = SystemMotion> + Send>>>,
}

impl Integrations {
    fn open(choice: SystemChoice) -> Self {
        let commands = match choice {
            SystemChoice::Desktop => {
                Some(Box::pin(media::events()) as Pin<Box<dyn Stream<Item = media::Event> + Send>>)
            }
            SystemChoice::None => None,
        };
        Self {
            desktop: choice == SystemChoice::Desktop,
            commands,
            media: None,
            #[cfg(not(target_os = "android"))]
            discord: None,
            jam: None,
            motion: None,
        }
    }

    /// Apre o chiude il collegamento del movimento. Un core senza collegamenti col sistema (i
    /// test) non si iscrive al bus della sessione.
    fn watch_motion(&mut self, on: bool) {
        self.motion = (on && self.desktop).then(|| {
            Box::pin(crate::system_motion::events()) as Pin<Box<dyn Stream<Item = SystemMotion> + Send>>
        });
    }

    /// Apre un indirizzo nel browser del sistema. `false` se non si è aperto. Un core senza
    /// collegamenti col sistema (i test) non apre niente e lo dice come riuscito: nessun browser
    /// compare a chi prova il core.
    fn open_url(&self, url: &crate::auth::Secret) -> bool {
        !self.desktop || crate::platform::open_url(url.expose())
    }

    /// Il prossimo comando dal sistema o evento della stanza; senza collegamenti non arriva mai.
    /// Le due attese stanno insieme, su campi diversi, per poterle aspettare con una sola borrow.
    async fn next(&mut self) -> Heard {
        let Self { commands, jam, motion, .. } = self;
        tokio::select! {
            event = async {
                match commands {
                    Some(commands) => commands.next().await,
                    None => std::future::pending().await,
                }
            } => Heard::System(event),
            event = async {
                match jam {
                    Some(link) => link.events.recv().await,
                    None => std::future::pending().await,
                }
            } => Heard::Jam(event),
            reading = async {
                match motion {
                    Some(motion) => motion.next().await,
                    None => std::future::pending().await,
                }
            } => Heard::Motion(reading),
        }
    }

    /// Ciò che si è sentito: lo smista.
    fn heard(&mut self, heard: Heard, inputs: &mut Vec<Input>) {
        match heard {
            Heard::System(event) => self.received(event, inputs),
            Heard::Jam(event) => self.jam_received(event, inputs),
            Heard::Motion(Some(reading)) => inputs.push(Input::SystemMotion(reading)),
            // Il flusso è finito (nessun bus della sessione): la vista resta senza preferenza.
            Heard::Motion(None) => self.motion = None,
        }
    }

    /// Un comando arrivato: il capo del collegamento si tiene qui, il resto va al riduttore.
    fn received(&mut self, event: Option<media::Event>, inputs: &mut Vec<Input>) {
        match event {
            Some(event) => {
                if let media::Event::Ready(handle) = &event {
                    self.media = Some(handle.clone());
                }
                inputs.push(Input::Media(event));
            }
            // Il flusso è finito (nessun bus della sessione): non lo si interroga più.
            None => self.commands = None,
        }
    }

    /// Un evento della stanza arrivato: va al riduttore. Una chiusura, o un canale che si è svuotato
    /// senza dirla (il thread del collegamento è caduto), toglie il collegamento: non lo si
    /// interroga più, e il riduttore chiude la jam.
    fn jam_received(&mut self, event: Option<jam::Event>, inputs: &mut Vec<Input>) {
        let event = event.unwrap_or_else(|| jam::Event::Closed(jam::LOST.into()));
        if matches!(event, jam::Event::Closed(_)) {
            self.jam = None;
        }
        inputs.push(Input::Jam(event));
    }

    /// Esegue un'operazione sul collegamento alla stanza.
    fn jam(&mut self, op: JamOp) {
        match op {
            // Il collegamento che c'era cade con il suo thread: la stanza libera il posto.
            JamOp::Start { base, target, name } => self.jam = Some(jam::Link::start(&base, target, name)),
            JamOp::Send(snapshot) => {
                if let Some(link) = &self.jam {
                    link.send(snapshot);
                }
            }
            JamOp::Leave => self.jam = None,
        }
    }

    fn media(&self, op: MediaOp) {
        let Some(handle) = self.media.as_ref().filter(|_| self.desktop) else { return };
        match op {
            MediaOp::Show(now) => handle.show(now.as_deref().map(media::Now::borrowed).as_ref()),
            MediaOp::Volume(volume) => handle.set_volume(volume),
        }
    }

    #[cfg(not(target_os = "android"))]
    fn discord(&mut self, op: DiscordOp) {
        if !self.desktop {
            return;
        }
        match op {
            DiscordOp::Start(app_id) => self.discord = crate::discord::Presence::start(&app_id),
            DiscordOp::Stop => self.discord = None,
            DiscordOp::Set(now) => {
                let Some(presence) = &self.discord else { return };
                presence.set(now.map(|now| {
                    crate::discord::Activity::playing(
                        &now.title,
                        &now.artist,
                        now.album.as_deref(),
                        now.cover_url.as_deref(),
                        now.position,
                        now.duration,
                    )
                }));
            }
        }
    }

    /// Discord non c'è su Android: il riduttore non chiede niente.
    #[cfg(target_os = "android")]
    fn discord(&mut self, _op: DiscordOp) {}
}

/// Ciò che i collegamenti dell'host hanno da dire: un comando del sistema (`None`: il flusso è
/// finito) o un evento della stanza della jam (`None`: il canale si è svuotato).
enum Heard {
    System(Option<media::Event>),
    Jam(Option<jam::Event>),
    /// Una lettura del movimento chiesto dal sistema (`None`: il flusso è finito).
    Motion(Option<SystemMotion>),
}

/// Il ciclo del core: aspetta un comando, l'esito di un lavoro o un timer, lo passa al riduttore,
/// esegue gli effetti e pubblica se qualcosa è cambiato. Dopo `shutdown` non legge più comandi e
/// gira finché i domini non hanno salvato ([`Effect::Flushed`]); allora restituisce chi aspetta la
/// fine e l'esito del salvataggio. Se il `Core` cade senza `shutdown`, si ferma subito.
async fn actor(
    mut state: CoreState,
    mut inbox: mpsc::UnboundedReceiver<Control>,
    publish: watch::Sender<Arc<Snapshot>>,
    audio: &mut Option<AudioOutput>,
    system: &mut Integrations,
) -> Option<(Stopped, Result<(), AppError>)> {
    let mut timers = Timers::default();
    let mut jobs = JoinSet::new();
    let mut closing: Option<Stopped> = None;
    // Il primo giro è l'avvio: nessuno ha ancora scritto, ma i domini hanno letto il file.
    let mut starting = Some(Input::Start);
    loop {
        let mut inputs = Vec::new();
        if let Some(start) = starting.take() {
            inputs.push(start);
        } else {
            tokio::select! {
            control = inbox.recv(), if closing.is_none() => match control {
                Some(control) => receive(control, &mut inbox, &mut inputs, &mut closing),
                // Il `Core` è caduto senza `shutdown`: nessuno aspetta.
                None => return None,
            },
            // Un lavoro annullato non ha esito: succede solo se il runtime si chiude prima del core.
            Some(done) = jobs.join_next() => inputs.extend(done.ok().map(Input::Done)),
            () = deadline(timers.next()) => inputs.extend(timers.expired(Instant::now()).into_iter().map(Input::Timer)),
            // I comandi del sistema e gli eventi della stanza della jam: entrano nel core da qui,
            // senza che nessun battito li vada a leggere.
            heard = system.next() => system.heard(heard, &mut inputs),
            }
        }
        let mut flushed = None;
        for input in inputs {
            let now = Instant::now();
            // Ciò che l'audio risponde a un'operazione rientra nello stesso giro, subito dopo gli
            // effetti che l'hanno chiesta.
            let mut pending = VecDeque::from([input]);
            while let Some(input) = pending.pop_front() {
                for effect in state.apply(input, now.into_std()) {
                    match effect {
                        Effect::Audio(op) => pending.extend(run_audio(audio, op).map(Input::Audio)),
                        Effect::Media(op) => system.media(op),
                        Effect::Discord(op) => system.discord(op),
                        Effect::Jam(op) => system.jam(op),
                        Effect::WatchMotion(on) => system.watch_motion(on),
                        Effect::OpenUrl(url) => {
                            // Se il browser non si apre, il dominio smette di aspettarlo.
                            if !system.open_url(&url) {
                                pending.push_back(Input::Done(Done::Social(
                                    crate::state::social::Done::BrowserFailed,
                                )));
                            }
                        }
                        effect => flushed = run(effect, &mut timers, &mut jobs, now).or(flushed),
                    }
                }
            }
        }
        publish_changes(&state, &publish);
        if let Some(saved) = flushed {
            return closing.map(|stopped| (stopped, saved));
        }
    }
}

/// Mette in `inputs` il comando appena arrivato e quelli già in coda dietro di lui (al più
/// [`BURST`]): una raffica diventa un solo giro e una sola istantanea. Si ferma a `shutdown`, dopo
/// il quale nessuno manda più niente.
fn receive(
    first: Control,
    inbox: &mut mpsc::UnboundedReceiver<Control>,
    inputs: &mut Vec<Input>,
    closing: &mut Option<Stopped>,
) {
    let queued = std::iter::from_fn(|| inbox.try_recv().ok());
    for control in std::iter::once(first).chain(queued).take(BURST) {
        match control {
            Control::Command(command) => inputs.push(Input::Command(command)),
            Control::Shutdown(stopped) => {
                *closing = Some(stopped);
                inputs.push(Input::Shutdown);
                return;
            }
        }
    }
}

/// Esegue un effetto. Restituisce l'esito del salvataggio quando l'effetto chiude il core.
fn run(
    effect: Effect,
    timers: &mut Timers<TimerKey>,
    jobs: &mut JoinSet<Done>,
    now: Instant,
) -> Option<Result<(), AppError>> {
    match effect {
        Effect::Timer { key, after } => match now.checked_add(after) {
            Some(at) => timers.arm(key, at),
            // Una scadenza oltre la fine del tempo non arriva mai: basta togliere il timer.
            None => timers.cancel(key),
        },
        Effect::CancelTimer(key) => timers.cancel(key),
        // Un panic nel lavoro diventa l'esito `lost`: il dominio non resta ad aspettare.
        Effect::Run(job) => {
            let lost = job.lost();
            jobs.spawn(async move { AssertUnwindSafe(job.run()).catch_unwind().await.unwrap_or(lost) });
        }
        Effect::Flushed(saved) => return Some(saved),
        // L'audio e i collegamenti col sistema li esegue chi li tiene (`actor`), nello stesso giro.
        Effect::Audio(_)
        | Effect::Media(_)
        | Effect::Discord(_)
        | Effect::Jam(_)
        | Effect::OpenUrl(_)
        | Effect::WatchMotion(_) => {}
    }
    None
}

/// Pubblica l'istantanea successiva se un dominio è cambiato.
fn publish_changes(state: &CoreState, publish: &watch::Sender<Arc<Snapshot>>) {
    let previous = Arc::clone(&publish.borrow());
    if let Some(next) = state.publish(&previous) {
        publish.send_replace(Arc::new(next));
    }
}

/// Aspetta fino a `at`; senza scadenza, per sempre.
async fn deadline(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// I timer armati, in ordine di scadenza (a pari scadenza, in ordine di arrivo). Sono pochi, uno
/// per chiave: basta una lista.
#[derive(Debug)]
struct Timers<K> {
    armed: Vec<(K, Instant)>,
}

impl<K> Default for Timers<K> {
    fn default() -> Self {
        Self { armed: Vec::new() }
    }
}

impl<K: Copy + PartialEq> Timers<K> {
    /// Arma `key` per `at`; se era già armato, il timer precedente si annulla.
    fn arm(&mut self, key: K, at: Instant) {
        self.cancel(key);
        let index = self.armed.partition_point(|(_, other)| *other <= at);
        self.armed.insert(index, (key, at));
    }

    fn cancel(&mut self, key: K) {
        self.armed.retain(|(armed, _)| *armed != key);
    }

    /// La prossima scadenza.
    fn next(&self) -> Option<Instant> {
        self.armed.first().map(|(_, at)| *at)
    }

    /// Toglie i timer scaduti a `now` e li restituisce nell'ordine in cui scadono.
    fn expired(&mut self, now: Instant) -> Vec<K> {
        let due = self.armed.partition_point(|(_, at)| *at <= now);
        self.armed.drain(..due).map(|(key, _)| key).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::Value;

    use crate::command::SettingsCmd;
    use crate::config::{Config, CoreConfig};
    use crate::state::settings::SAVE_DELAY;

    /// Un core sul runtime `handle`, senza cartella dei dati: non legge e non scrive file.
    fn borrowing(handle: Handle) -> CoreOptions {
        CoreOptions {
            runtime: RuntimeChoice::Borrow(handle),
            audio: AudioChoice::Detached,
            system: SystemChoice::None,
            data_dir: None,
            downloads_dir: None,
        }
    }

    /// Una cartella dei dati nuova, solo per questo test.
    fn data_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("psytify-core-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn store(core: CoreConfig) -> Command {
        Command::Settings(SettingsCmd::Store(Box::new(core)))
    }

    /// L'audio è del core dal passo 4.3: questo test lo apre staccato (`AudioChoice::Detached`),
    /// mai sulla scheda audio, e lo chiude sul thread che l'ha aperto.
    #[test]
    fn core_starts_and_stops_100_times() {
        let runtime = Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        for round in 0..100 {
            let core = Core::start(borrowing(runtime.handle().clone())).unwrap();
            assert_eq!(core.snapshot().version, 0, "giro {round}");
            runtime.block_on(core.shutdown()).unwrap().unwrap();
        }
        for _ in 0..100 {
            let options = CoreOptions {
                runtime: RuntimeChoice::Own { workers: 1 },
                audio: AudioChoice::Detached,
                system: SystemChoice::None,
                data_dir: None,
                downloads_dir: None,
            };
            Core::start(options).unwrap().shutdown().blocking_recv().unwrap().unwrap();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn idle_core_publishes_nothing() {
        let core = Core::start(borrowing(Handle::current())).unwrap();
        let first = core.snapshot();
        let changes = core.changes();
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(!changes.has_changed().unwrap(), "5 s senza comandi: nessuna istantanea");
        assert!(Arc::ptr_eq(&core.snapshot(), &first));
        core.shutdown().await.unwrap().unwrap();
    }

    #[test]
    fn a_core_without_workers_still_gets_one() {
        let options = CoreOptions {
            runtime: RuntimeChoice::Own { workers: 0 },
            audio: AudioChoice::Detached,
            system: SystemChoice::None,
            data_dir: None,
            downloads_dir: None,
        };
        Core::start(options).unwrap().shutdown().blocking_recv().unwrap().unwrap();
    }

    /// Il `config.json` di una release, nella cartella di questo core (nessun lucchetto fra i test,
    /// `docs/transformation/BACKLOG.md`, 11): il core lo legge all'avvio e alla chiusura lo riscrive
    /// con l'ultimo stato, senza aspettare il ritardo e senza perdere le chiavi che non conosce.
    #[tokio::test(start_paused = true)]
    async fn the_core_loads_a_release_config_and_close_writes_the_last_state() {
        let dir = data_dir();
        let fixture = include_str!("../tests/fixtures/config.golden.json");
        std::fs::write(dir.join("config.json"), fixture).unwrap();
        let core =
            Core::start(CoreOptions { data_dir: Some(dir.clone()), ..borrowing(Handle::current()) }).unwrap();
        let settings = Arc::clone(&core.snapshot().settings);
        assert_eq!(settings.config.volume, 0.5);
        assert_eq!(settings.ui["last_screen"]["kind"], "album");

        core.send(store(CoreConfig { clerk_client_id: "rock".into(), ..(*settings.config).clone() }));
        core.shutdown().await.unwrap().unwrap();

        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).unwrap()).unwrap();
        assert_eq!(saved["clerk_client_id"], "rock");
        assert_eq!(saved["unknown_future_key"], serde_json::json!({"a": 1}));
        assert_eq!(saved["panel_open"], true);
        assert!(!dir.join("config.json.tmp").exists());
    }

    /// Senza chiusura, una modifica arriva sul disco da sola dopo [`SAVE_DELAY`]. Qui il disco la
    /// rifiuta (al posto della cartella c'è un file): l'errore si pubblica, e la chiusura, dopo
    /// un ultimo tentativo, lo riporta. Il core si ferma comunque.
    #[tokio::test(start_paused = true)]
    async fn a_change_is_saved_after_the_delay_and_a_failed_save_is_reported() {
        let blocked = data_dir().join("not-a-dir");
        std::fs::write(&blocked, "").unwrap();
        let core =
            Core::start(CoreOptions { data_dir: Some(blocked), ..borrowing(Handle::current()) }).unwrap();
        let mut changes = core.changes();
        let sent = Instant::now();
        core.send(store(CoreConfig { clerk_client_id: "jazz".into(), ..CoreConfig::default() }));
        let error = loop {
            changes.changed().await.unwrap();
            if let Some(error) = changes.borrow_and_update().settings.save_error.clone() {
                break error;
            }
        };
        assert!(sent.elapsed() >= SAVE_DELAY, "the write waits for the delay");
        assert!(matches!(error, AppError::Io(_)), "{error:?}");
        let stopped = core.shutdown().await.unwrap();
        assert!(matches!(stopped, Err(CoreError::Save(AppError::Io(_)))), "{stopped:?}");
    }

    /// Passo 4.2: la libreria passa dal core, che la scrive in `config.json` dopo [`SAVE_DELAY`]
    /// insieme agli altri campi, e la ritrova al riavvio. Una chiusura subito dopo il comando non
    /// perde niente.
    #[tokio::test(start_paused = true)]
    async fn library_changes_are_saved_and_come_back_after_a_restart() {
        use crate::api::models::fixtures::track;
        use crate::command::LibraryCmd;

        let dir = data_dir();
        let core =
            Core::start(CoreOptions { data_dir: Some(dir.clone()), ..borrowing(Handle::current()) }).unwrap();
        let mut changes = core.changes();
        core.send(Command::library(LibraryCmd::ToggleLike(track(1))));
        core.send(Command::library(LibraryCmd::RememberSearch("eminem".into())));
        while changes.borrow_and_update().library.search_history.is_empty() {
            changes.changed().await.unwrap();
        }
        assert_eq!(changes.borrow().library.liked.len(), 1);
        assert!(!config::file_in(&dir).exists(), "nothing on disk before the delay");
        // Lo scrive il core quando vuole lui, sul runtime del test: si guarda il disco finché non arriva.
        let give_up = std::time::Instant::now() + Duration::from_secs(10);
        let saved = loop {
            let saved = config::load(Some(&config::file_in(&dir)));
            if !saved.core.search_history.is_empty() || std::time::Instant::now() > give_up {
                break saved;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(saved.core.liked.len(), 1);
        assert_eq!(*saved.core.search_history, ["eminem"]);

        // Un cambio e subito la chiusura: la scrittura si fa lo stesso.
        core.send(Command::library(LibraryCmd::ToggleLike(track(2))));
        core.shutdown().await.unwrap().unwrap();
        assert_eq!(config::load(Some(&config::file_in(&dir))).core.liked.len(), 2);

        let core = Core::start(CoreOptions { data_dir: Some(dir), ..borrowing(Handle::current()) }).unwrap();
        let liked = core.snapshot().library.liked.iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(liked, [track(2).id, track(1).id]);
        core.shutdown().await.unwrap().unwrap();
    }

    /// Passo 5: un frontend chiede una copertina al core, che la scarica dal backend scelto nelle
    /// impostazioni (`/v1/cover`) sul proprio runtime e la restituisce decodificata. Un indirizzo non
    /// ammesso (non https, o un host che non è archive.org né il CDN di Deezer) non esce dal core: `None`
    /// senza nessuna richiesta.
    #[tokio::test]
    async fn a_cover_is_fetched_through_the_backend_and_a_foreign_address_is_refused() {
        use crate::testing::fake_backend;

        let backend = fake_backend::start(Vec::new(), "flac").await;
        let dir = data_dir();
        std::fs::write(config::file_in(&dir), format!(r#"{{"server":"{}"}}"#, backend.url)).unwrap();
        let core =
            Core::start(CoreOptions { data_dir: Some(dir.clone()), ..borrowing(Handle::current()) }).unwrap();

        let url = "https://cdn-images.dzcdn.net/images/cover/abc/1000x1000.jpg".to_owned();
        let pixels = core.artwork(url.clone()).await.unwrap().expect("la copertina del backend");
        assert_eq!((pixels.side, pixels.rgba.len()), (8, 8 * 8 * 4));
        assert_eq!(backend.count("/v1/cover"), 1);

        let refused = core.artwork("http://evil.example/cover.png".into()).await.unwrap();
        assert!(refused.is_none());
        let other = core.artwork("https://example.com/cover.png".into()).await.unwrap();
        assert!(other.is_none());
        assert_eq!(backend.count("/v1/cover"), 1, "gli indirizzi non ammessi non arrivano al backend");

        // Il client delle copertine si riusa finché il backend è lo stesso.
        core.artwork(url).await.unwrap().expect("ancora una");
        assert_eq!(backend.count("/v1/cover"), 2);
        core.shutdown().await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Passo 4.5 (BACKLOG 24): un solo comando, `ExternalCmd::PlaySource`, porta un risultato del
    /// catalogo ufficiale nel catalogo del server e lo fa suonare, tutto sul runtime del core: il
    /// frontend non importa, non accoda e non rilegge niente. La riga «in importazione» c'è finché
    /// il server lavora, poi il brano è quello in corso.
    #[tokio::test]
    async fn playing_an_official_result_imports_it_and_starts_it_from_one_command() {
        use crate::api::SourceKind;
        use crate::api::models::fixtures::track;
        use crate::command::ExternalCmd;
        use crate::testing::fake_backend;

        let audio = include_bytes!("../tests/fixtures/tone-4s.flac").to_vec();
        let imported = track(7);
        let resolve = serde_json::json!({ "album": null, "tracks": [imported] }).to_string();
        let backend = fake_backend::start_with(audio, "flac", resolve).await;
        let dir = data_dir();
        std::fs::write(config::file_in(&dir), format!(r#"{{"server":"{}"}}"#, backend.url)).unwrap();
        let core =
            Core::start(CoreOptions { data_dir: Some(dir.clone()), ..borrowing(Handle::current()) }).unwrap();
        let mut changes = core.changes();

        core.send(Command::External(ExternalCmd::PlaySource {
            source: SourceKind::Deezer,
            id: "7".into(),
            video: false,
        }));
        let playing = until(&mut changes, |snapshot| snapshot.playback.is_playing()).await;
        assert_eq!(playing.playback.queue.current().map(|t| t.id), Some(imported.id));
        assert_eq!(backend.hits().iter().filter(|hit| *hit == "POST /v1/meta/tracks/7/resolve").count(), 1);
        let settled = until(&mut changes, |snapshot| snapshot.external.imports.play.is_none()).await;
        assert!(!settled.external.importing(SourceKind::Deezer, "7"));
        core.shutdown().await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Passo 4.9: il videoclip passa dal core, sul runtime. Finché il video è nascosto il core non
    /// chiede l'indirizzo; con il video mostrato lo chiede al backend e pubblica quello a 360p; sei
    /// secondi dopo (il timer del core, armato da lui) lo richiede e pubblica il 720p, una volta, e
    /// poi non chiede più niente. Il brano che parte fa preparare il suo videoclip una volta sola.
    /// Nascosto il video, il dominio torna a riposo. Tempo vero: il timer è di sei secondi e il backend è una porta
    /// vera, su cui un orologio fermo salterebbe dritto ai timeout.
    #[cfg(feature = "video")]
    #[tokio::test]
    async fn a_shown_clip_is_asked_of_the_backend_and_upgraded_on_the_core_timer() {
        use crate::api::Track;
        use crate::api::models::fixtures::track;
        use crate::command::{PlaybackCmd, VideoCmd};
        use crate::snapshot::ClipState;
        use crate::testing::fake_backend;

        let audio = include_bytes!("../tests/fixtures/tone-4s.flac").to_vec();
        let backend = fake_backend::start(audio, "flac").await;
        backend.serve_video_heights(&[360, 720]);
        let dir = data_dir();
        std::fs::write(config::file_in(&dir), format!(r#"{{"server":"{}"}}"#, backend.url)).unwrap();
        let core =
            Core::start(CoreOptions { data_dir: Some(dir.clone()), ..borrowing(Handle::current()) }).unwrap();
        let mut changes = core.changes();
        let clip = Track { has_video: true, ..track(1) };
        let video_url = format!("GET /v1/tracks/{}/video-url", clip.id);

        // Il brano parte con i videoclip voluti ma nascosti: il server prepara il suo, una volta, e
        // nessuno chiede l'indirizzo.
        core.send(Command::Video(VideoCmd::PrepareAhead(true)));
        core.send(Command::Playback(PlaybackCmd::Play { tracks: vec![clip.clone()].into(), start: 0 }));
        until(&mut changes, |snapshot| snapshot.playback.is_playing()).await;
        let prepare = format!("POST /v1/tracks/{}/video/prepare", clip.id);
        tokio::time::timeout(Duration::from_secs(20), async {
            while !backend.hits().contains(&prepare) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("il brano che parte fa preparare il suo videoclip");
        assert!(!backend.hits().contains(&video_url), "nascosto: nessun indirizzo chiesto");
        assert_eq!(changes.borrow().video.state, ClipState::Idle);

        // Mostrato: l'indirizzo, a 360p, con l'origine del backend davanti all'indirizzo del proxy.
        let shown = Instant::now();
        core.send(Command::Video(VideoCmd::Show(true)));
        let ready = until(&mut changes, |snapshot| matches!(snapshot.video.state, ClipState::Ready(_))).await;
        let ClipState::Ready(first) = &ready.video.state else { unreachable!("appena controllato") };
        assert_eq!((ready.video.track, first.height), (Some(clip.id), Some(360)));
        assert_eq!(first.url(), format!("{}/v1/media?t=clip", backend.url));

        // Sei secondi dopo, da sola, la versione completa; poi il core non chiede più niente.
        let full = until(
            &mut changes,
            |snapshot| matches!(&snapshot.video.state, ClipState::Ready(clip) if clip.height == Some(720)),
        )
        .await;
        assert!(shown.elapsed() >= crate::state::video::UPGRADE_WAIT, "{:?}", shown.elapsed());
        let ClipState::Ready(second) = &full.video.state else { unreachable!("appena controllato") };
        assert!(second.seq > first.seq && second.is_full());
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(backend.count(&video_url["GET ".len()..]), 2, "360p e 720p, non una richiesta in più");
        assert_eq!(backend.hits().iter().filter(|hit| **hit == prepare).count(), 1);

        // Nascosto di nuovo, il videoclip se ne va e il core torna a riposo.
        core.send(Command::Video(VideoCmd::Show(false)));
        let idle = until(&mut changes, |snapshot| snapshot.video.state == ClipState::Idle).await;
        tokio::time::sleep(Duration::from_millis(900)).await;
        assert_eq!(core.snapshot().versions.video, idle.versions.video, "il video non pubblica più niente");
        assert_eq!(backend.count(&video_url["GET ".len()..]), 2, "e non chiede più niente");
        core.shutdown().await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Passo 4.3: un `Play` fa partire il caricamento sul runtime del core e la risposta (qui un
    /// errore: il backend è una porta chiusa) rientra nel dominio nello stesso modo che a un brano
    /// vero. Il core tiene l'uscita audio (staccata) e la chiude con sé.
    #[tokio::test]
    async fn a_play_runs_its_load_on_the_runtime_and_the_answer_comes_back() {
        use crate::api::models::fixtures::track;
        use crate::command::PlaybackCmd;
        use crate::player::PlaybackState;

        let dir = data_dir();
        std::fs::write(config::file_in(&dir), r#"{"server":"http://127.0.0.1:9"}"#).unwrap();
        let core = Core::start(CoreOptions { data_dir: Some(dir), ..borrowing(Handle::current()) }).unwrap();
        let mut changes = core.changes();
        core.send(Command::Playback(PlaybackCmd::Play { tracks: vec![track(1)].into(), start: 0 }));
        let seen = tokio::time::timeout(Duration::from_secs(20), async {
            let mut states = Vec::new();
            loop {
                changes.changed().await.unwrap();
                let state = changes.borrow_and_update().playback.state.clone();
                let failed = matches!(state, PlaybackState::Error(_));
                states.push(state);
                if failed {
                    return states;
                }
            }
        })
        .await
        .expect("il caricamento finisce, anche se fallisce");
        assert!(matches!(seen.first(), Some(PlaybackState::Loading { .. }) | Some(PlaybackState::Error(_))));
        assert!(matches!(seen.last(), Some(PlaybackState::Error(_))), "{seen:?}");
        core.shutdown().await.unwrap().unwrap();
    }

    /// Passo 4.4: il tasto arma l'attesa nel core, la ricerca parte sul runtime allo scadere e le
    /// risposte (qui errori: il backend è una porta chiusa) rientrano nel dominio; poi il core è fermo.
    #[tokio::test]
    async fn the_debounced_search_runs_on_the_runtime_and_the_core_goes_quiet() {
        use crate::catalog::Load;
        use crate::command::CatalogCmd;

        let dir = data_dir();
        std::fs::write(config::file_in(&dir), r#"{"server":"http://127.0.0.1:9"}"#).unwrap();
        let core = Core::start(CoreOptions { data_dir: Some(dir), ..borrowing(Handle::current()) }).unwrap();
        let mut changes = core.changes();
        core.send(Command::Catalog(CatalogCmd::SearchInput("low".into())));
        // Scrivere non pubblica niente: la ricerca parte solo allo scadere dei 300 ms.
        let waited = tokio::time::timeout(Duration::from_millis(150), changes.changed()).await;
        assert!(waited.is_err(), "nothing is published while the debounce waits");
        let settled = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                changes.changed().await.unwrap();
                let search = changes.borrow_and_update().catalog.search.clone();
                let done =
                    [matches!(search.results, Load::Failed(_)), matches!(search.discover, Load::Failed(_))];
                if search.submitted == "low"
                    && done == [true, true]
                    && matches!(search.sources, Load::Failed(_))
                {
                    return search;
                }
            }
        })
        .await
        .expect("the three searches end, even if they fail");
        assert_eq!(settled.submitted, "low");
        // Fermo: dopo le risposte, e dopo il salvataggio che ne segue, niente più pubblicazioni.
        tokio::time::sleep(Duration::from_millis(900)).await;
        changes.borrow_and_update();
        let quiet = tokio::time::timeout(Duration::from_millis(700), changes.changed()).await;
        assert!(quiet.is_err(), "an idle core publishes nothing");
        core.shutdown().await.unwrap().unwrap();
    }

    /// Una raffica di comandi già in coda è un solo giro: una sola istantanea, con l'ultimo stato.
    #[tokio::test(start_paused = true)]
    async fn a_burst_of_commands_publishes_once() {
        let (commands, inbox) = mpsc::unbounded_channel();
        for query in ["a", "b", "c"] {
            commands
                .send(Control::Command(store(CoreConfig {
                    clerk_client_id: query.into(),
                    ..CoreConfig::default()
                })))
                .unwrap();
        }
        let (stopped, _) = oneshot::channel();
        commands.send(Control::Shutdown(stopped)).unwrap();
        let state = CoreState::new(Config::default(), None, 1);
        let (publish, changes) = watch::channel(Arc::new(state.snapshot()));

        let (_, saved) = actor(state, inbox, publish, &mut None, &mut Integrations::open(SystemChoice::None))
            .await
            .unwrap();

        assert_eq!(saved, Ok(()));
        let last = changes.borrow();
        assert_eq!(
            (last.version, last.settings.config.clerk_client_id.as_str()),
            (1, "c"),
            "one snapshot for the burst"
        );
    }

    #[test]
    fn rearming_a_timer_replaces_it_and_expired_timers_come_in_order() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut timers = Timers::default();
        timers.arm('a', at(30));
        timers.arm('b', at(10));
        timers.arm('c', at(10));
        timers.arm('a', at(20));
        assert_eq!(timers.next(), Some(at(10)));
        assert_eq!(timers.expired(at(5)), Vec::<char>::new());
        assert_eq!(timers.expired(at(20)), ['b', 'c', 'a'], "a pari scadenza vale l'ordine di arrivo");
        assert_eq!(timers.next(), None);
    }

    #[test]
    fn a_cancelled_timer_never_fires() {
        let start = Instant::now();
        let mut timers = Timers::default();
        timers.arm(1u8, start + Duration::from_millis(10));
        timers.cancel(1);
        timers.cancel(2);
        assert_eq!(timers.expired(start + Duration::from_secs(1)), Vec::<u8>::new());
    }

    /// Aspetta, al massimo 20 s, che l'istantanea del core soddisfi `ready`.
    async fn until(
        changes: &mut watch::Receiver<Arc<Snapshot>>,
        ready: impl Fn(&Snapshot) -> bool,
    ) -> Arc<Snapshot> {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let snapshot = Arc::clone(&changes.borrow_and_update());
                if ready(&snapshot) {
                    return snapshot;
                }
                changes.changed().await.unwrap();
            }
        })
        .await
        .expect("the core reaches the expected state")
    }

    /// Un'istanza Clerk che non risponde (https su una porta chiusa di localhost): nessun test
    /// parla con Clerk, e nessun test usa una sessione vera.
    fn dead_clerk(session_file: Option<PathBuf>) -> crate::auth::ClerkApp {
        crate::auth::ClerkApp { issuer: "https://127.0.0.1:9".into(), port: 53682, session_file }
    }

    /// Una cartella dei dati con la configurazione che punta a Clerk e al backend finti.
    fn social_dir() -> PathBuf {
        let dir = data_dir();
        std::fs::write(
            dir.join("config.json"),
            r#"{"clerk_issuer":"https://127.0.0.1:9","server":"http://127.0.0.1:9"}"#,
        )
        .unwrap();
        dir
    }

    /// Passo 4.7 e BACKLOG 11: la sessione salvata sta nella cartella dei dati di questo core, senza
    /// un percorso per processo. Un core con la sua `session.json` parte già su «accesso in corso» e
    /// la riprende da sé; un altro, in un'altra cartella, nello stesso processo, non la vede. Se
    /// Clerk non risponde la sessione non si perde: il file resta per il prossimo avvio.
    #[tokio::test]
    async fn the_session_file_belongs_to_each_cores_own_data_dir() {
        use crate::state::social::{Account, SocialEvent};

        let with = social_dir();
        std::fs::write(with.join("session.json"), r#"{"client":"client-finto","session":"sess_finta"}"#)
            .unwrap();
        let without = social_dir();

        let alone =
            Core::start(CoreOptions { data_dir: Some(without.clone()), ..borrowing(Handle::current()) })
                .unwrap();
        assert_eq!(alone.snapshot().social.account, Account::SignedOut, "nessun file in questa cartella");
        let core = Core::start(CoreOptions { data_dir: Some(with.clone()), ..borrowing(Handle::current()) })
            .unwrap();
        assert_eq!(
            core.snapshot().social.account,
            Account::Working,
            "la prima istantanea sa già della sessione"
        );

        let mut changes = core.changes();
        let settled = until(&mut changes, |s| s.social.account != Account::Working).await;
        assert_eq!(settled.social.account, Account::SignedOut);
        assert!(
            settled.social.events.iter().any(|raised| matches!(raised.event, SocialEvent::Failed(_))),
            "il ripristino mancato si dice: {:?}",
            settled.social.events
        );
        assert!(with.join("session.json").exists(), "un errore di rete non cancella la sessione");
        assert_eq!(alone.snapshot().social.account, Account::SignedOut);
        assert!(!without.join("session.json").exists());
        core.shutdown().await.unwrap().unwrap();
        alone.shutdown().await.unwrap().unwrap();
    }

    /// Uscire toglie `session.json` della cartella di questo core, anche con Clerk e il backend che
    /// non rispondono, e il core si ferma: nessun timer del sondaggio resta armato.
    #[tokio::test]
    async fn signing_out_removes_the_session_file_of_that_core() {
        use crate::state::social::{Account, SocialCmd};

        let dir = social_dir();
        let file = dir.join("session.json");
        let other = social_dir();
        let other_file = other.join("session.json");
        std::fs::write(&other_file, r#"{"client":"altro","session":"altra"}"#).unwrap();
        let spare =
            Core::start(CoreOptions { data_dir: Some(other), ..borrowing(Handle::current()) }).unwrap();

        // Un accesso già fatto, come dopo l'ultimo passo: nessuna chiamata di rete all'avvio.
        let core = Core::start(CoreOptions { data_dir: Some(dir), ..borrowing(Handle::current()) }).unwrap();
        let mut changes = core.changes();
        let session = crate::auth::Session::for_tests(dead_clerk(Some(file.clone())));
        let me = crate::api::Me { user_id: "user_prova".into(), admin: false, profile: None };
        core.send(Command::Social(SocialCmd::Adopt { session, me }));
        let _ = until(&mut changes, |s| matches!(s.social.account, Account::SignedIn(_))).await;
        std::fs::write(&file, r#"{"client":"client-finto","session":"sess_finta"}"#).unwrap();

        core.send(Command::Social(SocialCmd::SignOut));
        let _ = until(&mut changes, |s| s.social.account == Account::SignedOut).await;
        tokio::time::timeout(Duration::from_secs(20), async {
            while file.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("session.json si toglie");
        core.shutdown().await.unwrap().unwrap();
        assert!(other_file.exists(), "la sessione di un altro core non si tocca");
        spare.shutdown().await.unwrap().unwrap();
    }

    fn play_one(core: &Core, track: &crate::api::Track) {
        core.send(Command::Playback(crate::command::PlaybackCmd::Play {
            tracks: Arc::from(vec![track.clone()]),
            start: 0,
        }));
    }

    /// Passo 4.6b, su un core vero: un brano scaricato (da un backend finto, in una cartella dei dati
    /// temporanea) entra nell'indice con peso e SHA-256, e si suona dal disco con il backend spento:
    /// nessuna richiesta di ascolto, niente rete. Il file è quello scaricato, byte per byte.
    #[tokio::test]
    async fn a_downloaded_track_plays_from_disk_with_the_backend_gone() {
        use sha2::{Digest, Sha256};

        use crate::api::models::fixtures;
        use crate::command::DownloadsCmd;
        use crate::snapshot::DownloadsEvent;
        use crate::testing::fake_backend;

        let audio = include_bytes!("../tests/fixtures/tone-4s.flac").to_vec();
        let backend = fake_backend::start(audio.clone(), "flac").await;
        let dir = data_dir();
        std::fs::write(config::file_in(&dir), format!(r#"{{"server":"{}"}}"#, backend.url)).unwrap();
        let core =
            Core::start(CoreOptions { data_dir: Some(dir.clone()), ..borrowing(Handle::current()) }).unwrap();
        let mut changes = core.changes();
        let track = crate::api::Track { downloadable: true, ..fixtures::track(1) };

        core.send(Command::Downloads(DownloadsCmd::Download(Box::new(track.clone()))));
        let done = until(&mut changes, |snapshot| snapshot.downloads.saved.len() == 1).await;
        let entry = done.downloads.saved_track(track.id).expect("the entry").clone();
        assert_eq!(entry.size, audio.len() as u64);
        assert_eq!(entry.sha256, crate::offline::hex(&Sha256::digest(&audio)));
        assert_eq!(entry.format, "flac");
        assert_eq!(entry.path.parent(), Some(dir.join("Download").as_path()));
        assert_eq!(std::fs::read(&entry.path).unwrap(), audio, "il file è quello che il server ha dato");
        assert_eq!(done.downloads.used_bytes, entry.size);
        assert!(done.downloads.events.iter().any(|raised| matches!(raised.event, DownloadsEvent::Saved(_))));
        let leftovers: Vec<_> = std::fs::read_dir(dir.join("Download"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|file| file.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(leftovers.is_empty(), "nessun temporaneo rimasto: {leftovers:?}");

        // Senza backend: il brano parte lo stesso, dal file.
        let media_before = backend.count("/v1/media");
        backend.stop();
        play_one(&core, &track);
        let playing = until(&mut changes, |snapshot| snapshot.playback.is_playing()).await;
        assert_eq!(playing.playback.queue.current().map(|t| t.id), Some(track.id));
        assert_eq!(backend.hits().iter().filter(|hit| hit.ends_with("/stream-url")).count(), 0);
        assert_eq!(backend.count("/v1/media"), media_before, "nessun byte dalla rete");
        assert!(playing.downloads.lost.is_empty());
        core.shutdown().await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Il file sparisce: il brano si suona dalla rete, l'indice lo segna mancante e non lo toglie; a un
    /// nuovo avvio il controllo veloce lo segna da sé; «Rimuovi» toglie la voce e non trova niente da
    /// cancellare. L'indice sopravvive al riavvio (sta in `config.json`).
    #[tokio::test]
    async fn a_deleted_file_falls_back_to_streaming_and_the_index_survives_a_restart() {
        use crate::api::models::fixtures;
        use crate::command::DownloadsCmd;
        use crate::offline::LostReason;
        use crate::snapshot::DownloadsEvent;
        use crate::testing::fake_backend;

        let audio = include_bytes!("../tests/fixtures/tone-4s.flac").to_vec();
        let backend = fake_backend::start(audio.clone(), "flac").await;
        let dir = data_dir();
        std::fs::write(config::file_in(&dir), format!(r#"{{"server":"{}"}}"#, backend.url)).unwrap();
        let options = || CoreOptions { data_dir: Some(dir.clone()), ..borrowing(Handle::current()) };
        let core = Core::start(options()).unwrap();
        let mut changes = core.changes();
        let track = crate::api::Track { downloadable: true, ..fixtures::track(1) };
        let streams = || backend.hits().iter().filter(|hit| hit.ends_with("/stream-url")).count();

        core.send(Command::Downloads(DownloadsCmd::Download(Box::new(track.clone()))));
        let done = until(&mut changes, |snapshot| snapshot.downloads.saved.len() == 1).await;
        let path = done.downloads.saved[0].path.clone();

        // Il file sparisce fuori dall'app, il core non lo sa: il caricamento lo scopre.
        std::fs::remove_file(&path).unwrap();
        play_one(&core, &track);
        let playing = until(&mut changes, |snapshot| {
            snapshot.playback.is_playing() && snapshot.downloads.lost_reason(track.id).is_some()
        })
        .await;
        assert_eq!(playing.downloads.lost_reason(track.id), Some(LostReason::Missing));
        assert_eq!(streams(), 1, "ripiegato sulla rete, una volta");
        assert!(playing.downloads.is_saved(track.id), "la voce resta");
        assert!(!playing.downloads.is_offline_ready(track.id));

        // Riavvio: l'indice è in `config.json`, il controllo veloce segna il file mancante.
        core.shutdown().await.unwrap().unwrap();
        let core = Core::start(options()).unwrap();
        let mut changes = core.changes();
        assert_eq!(core.snapshot().downloads.saved.len(), 1, "l'indice sopravvive");
        let marked = until(&mut changes, |snapshot| snapshot.downloads.lost_reason(track.id).is_some()).await;
        assert_eq!(marked.downloads.lost_reason(track.id), Some(LostReason::Missing));
        assert_eq!(marked.downloads.saved.len(), 1, "non si cancella niente da soli");

        core.send(Command::Downloads(DownloadsCmd::Remove(track.id)));
        let removed = until(&mut changes, |snapshot| {
            snapshot
                .downloads
                .events
                .iter()
                .any(|raised| matches!(&raised.event, DownloadsEvent::Removed { kept_file: false, .. }))
        })
        .await;
        assert!(removed.downloads.saved.is_empty() && removed.downloads.lost.is_empty());
        core.shutdown().await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Aspetta, al massimo 20 s, che `ready` sia vero (per ciò che sta fuori dal core: la stanza finta).
    async fn eventually(ready: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(20), async {
            while !ready() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the outside world reaches the expected state");
    }

    /// Passo 4.8: la jam gira sugli eventi del collegamento, che l'host aspetta insieme a tutto il
    /// resto. Il core crea la stanza (con il nome del comando), gli eventi della stanza lo svegliano
    /// da soli (i membri, uno stato degli altri con i brani da chiedere al catalogo), una jam ferma
    /// non pubblica niente e non manda niente alla stanza, e uscire chiude il socket.
    #[tokio::test]
    async fn a_jam_runs_on_the_rooms_events_and_an_idle_jam_wakes_nothing() {
        use crate::api::models::fixtures::track;
        use crate::command::JamCmd;
        use crate::snapshot::JamPhase;
        use crate::testing::fake_room;

        let (first, second) = (track(1), track(2));
        let room = fake_room::start("K7QXM3", None, vec![first.clone(), second.clone()]).await;
        let dir = data_dir();
        std::fs::write(config::file_in(&dir), format!(r#"{{"server":"{}"}}"#, room.url)).unwrap();
        let core =
            Core::start(CoreOptions { data_dir: Some(dir.clone()), ..borrowing(Handle::current()) }).unwrap();
        let mut changes = core.changes();

        core.send(Command::Jam(JamCmd::Create { name: "Luca".into() }));
        let open = until(&mut changes, |snapshot| snapshot.jam.phase == JamPhase::Open).await;
        assert_eq!(open.jam.code.as_deref(), Some("K7QXM3"));
        assert_eq!(open.jam.members.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Luca"]);
        assert_eq!(room.names(), ["Luca"]);
        assert_eq!(open.jam.events.len(), 1, "«Sei nella jam»");

        // Ferma: nessuna istantanea, nessuno stato alla stanza (niente in coda, niente da dire).
        let quiet = tokio::time::timeout(Duration::from_millis(400), changes.changed()).await;
        assert!(quiet.is_err(), "una jam ferma non sveglia il core");
        assert!(room.received().is_empty());

        // Un membro entra: la stanza lo dice, il core pubblica la sola jam.
        let before = core.snapshot();
        room.say(&serde_json::json!({
            "type": "members",
            "members": [
                { "id": uuid::Uuid::from_u128(500), "name": "Luca" },
                { "id": uuid::Uuid::from_u128(501), "name": "Sara" },
            ],
        }));
        let more = until(&mut changes, |snapshot| snapshot.jam.members.len() == 2).await;
        assert_eq!(more.versions.jam, before.versions.jam + 1);
        assert!(Arc::ptr_eq(&more.playback, &before.playback) && Arc::ptr_eq(&more.social, &before.social));

        // Lo stato degli altri: i brani non ci sono, si chiedono al catalogo (sul runtime del core) e
        // la coda diventa quella del gruppo, sul secondo brano.
        room.say(&serde_json::json!({
            "type": "state", "queue": [first.id, second.id], "index": 1, "position_ms": 9000,
            "playing": true, "from": "Sara", "at_ms": 2,
        }));
        let aligned = until(&mut changes, |snapshot| {
            snapshot.playback.queue.len() == 2 && snapshot.playback.queue.position() == Some(1)
        })
        .await;
        assert_eq!(
            aligned.playback.queue.entries().map(|(_, t)| t.id).collect::<Vec<_>>(),
            [first.id, second.id]
        );
        assert!(
            aligned.jam.events.iter().any(|raised| matches!(
                &raised.event,
                crate::snapshot::JamEvent::TrackChanged { from } if from == "Sara"
            )),
            "{:?}",
            aligned.jam.events
        );

        // Uscire: il socket si chiude, la vista torna vuota.
        core.send(Command::Jam(JamCmd::Leave));
        let left = until(&mut changes, |snapshot| !snapshot.jam.is_active()).await;
        assert!(left.jam.code.is_none() && left.jam.members.is_empty());
        eventually(|| room.closed() == 1).await;
        core.shutdown().await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Un server che non risponde chiude la jam con il suo motivo, e si può ripartire.
    #[tokio::test]
    async fn a_jam_whose_server_is_down_ends_with_the_reason() {
        use crate::command::JamCmd;
        use crate::snapshot::JamEvent;

        let dir = data_dir();
        std::fs::write(config::file_in(&dir), r#"{"server":"http://127.0.0.1:9"}"#).unwrap();
        let core =
            Core::start(CoreOptions { data_dir: Some(dir.clone()), ..borrowing(Handle::current()) }).unwrap();
        let mut changes = core.changes();
        core.send(Command::Jam(JamCmd::Create { name: String::new() }));
        let closed = until(&mut changes, |snapshot| {
            snapshot.jam.events.iter().any(|raised| matches!(raised.event, JamEvent::Closed(_)))
        })
        .await;
        assert!(!closed.jam.is_active());
        let last = closed.jam.events.back().expect("an event");
        assert!(last.event.is_error());
        assert_eq!(last.event.message(), "Il server non risponde");
        core.send(Command::Jam(JamCmd::Create { name: String::new() }));
        until(&mut changes, |snapshot| snapshot.jam.events.len() == 2).await;
        core.shutdown().await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
