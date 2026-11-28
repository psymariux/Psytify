//! The core's host: the `psytify-core` thread that keeps the state, receives the commands, runs the effects
//! the reducer asks for and publishes the snapshots. The frontends see only [`Core`]: commands in,
//! snapshots out.
//!
//! When idle, the core does no work: it waits for a command, a job's outcome or the next armed timer,
//! and without timers it does not wake up. It publishes only when a domain changes, once per round: an
//! idle frontend receives nothing, and a burst of commands arrives as a single snapshot.

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

/// How long, at shutdown, it waits for the blocking job still running on the core's own runtime.
const RUNTIME_GRACE: Duration = Duration::from_secs(1);

/// At most this many commands already queued enter the same round (and the same snapshot).
const BURST: usize = 64;

/// On which tokio runtime the core's async work runs.
#[derive(Debug, Clone)]
pub enum RuntimeChoice {
    /// The frontend's own: there stays a single runtime in the process. The
    /// frontend waits for [`Core::shutdown`] before closing it.
    Borrow(Handle),
    /// One of the core's, with `workers` threads (at least one), for frontends that do not use tokio.
    Own { workers: usize },
}

/// Which audio output the core opens. The core holds it for its whole life, on its own thread: the
/// sound card is not touched by anyone else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioChoice {
    /// The system's default sound card. If it does not open, playback answers with the
    /// error on each track and the view says so (`PlaybackView::device_error`).
    #[default]
    System,
    /// No output (a frontend without sound).
    None,
    /// An output without a sound card, from which nobody reads the samples: the tests that start a core
    /// for real without touching the card.
    #[cfg(any(test, feature = "test-support"))]
    Detached,
}

/// The links with the system that the core holds on its own thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SystemChoice {
    /// The system's media controls (MPRIS on Linux, the media session on Android:
    /// the commands enter the core without going through the interface) and, if the user wants it, Discord.
    #[default]
    Desktop,
    /// No link: a frontend without system widgets, and the tests (which must not
    /// register on the session bus).
    None,
}

/// How the core is started.
#[derive(Debug, Clone)]
pub struct CoreOptions {
    pub runtime: RuntimeChoice,
    pub audio: AudioChoice,
    pub system: SystemChoice,
    /// The data folder of this core: `config.json` is read at startup and written there. Without it,
    /// the core starts from the defaults and writes nothing (the tests, a frontend that does not save).
    pub data_dir: Option<PathBuf>,
    /// Where the downloaded tracks end up. Without it, `data_dir/Download`; without even that, a
    /// temporary folder (a test core never writes to the Download folder of whoever
    /// tries it). The update packages sit in `data_dir/updates`.
    pub downloads_dir: Option<PathBuf>,
}

/// The core did not start, or did not save before stopping.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// The system did not give the thread or the runtime.
    #[error("il core non è partito: {0}")]
    Start(#[from] io::Error),
    /// The last write of `config.json`, at shutdown, did not succeed: the file stays the previous one,
    /// whole.
    #[error("impostazioni non salvate: {0}")]
    Save(AppError),
}

/// Who waits for the core's end, and how the last save went.
type Stopped = oneshot::Sender<Result<(), CoreError>>;

/// What arrives at the core's thread.
#[derive(Debug)]
enum Control {
    Command(Command),
    /// Stop after saving, and tell whoever waits when it is done.
    Shutdown(Stopped),
}

/// The first snapshot and the runtime the core's work runs on, which the core's thread sends to
/// `start` when it is ready.
type Ready = io::Result<(watch::Receiver<Arc<Snapshot>>, Handle)>;

/// The running core, seen from a frontend. Dropping it stops the core even without
/// [`Core::shutdown`], but without waiting and without writing the changes still pending.
#[derive(Debug)]
pub struct Core {
    commands: mpsc::UnboundedSender<Control>,
    changes: watch::Receiver<Arc<Snapshot>>,
    /// The core's runtime: the covers a frontend asks for ([`Core::artwork`]) run there.
    runtime: Handle,
    /// The cover HTTP client, rebuilt only if the backend changes (connections are reused).
    covers: Mutex<Option<ApiClient>>,
}

impl Core {
    /// Starts the core's thread and waits until it is ready: on return the first snapshot is already there.
    pub fn start(options: CoreOptions) -> Result<Core, CoreError> {
        let (commands, inbox) = mpsc::unbounded_channel();
        let (ready, started) = std_mpsc::sync_channel(1);
        std::thread::Builder::new().name("psytify-core".into()).spawn(move || host(options, inbox, ready))?;
        let (changes, runtime) =
            started.recv().map_err(|_| io::Error::other("il thread del core si è fermato all'avvio"))??;
        Ok(Core { commands, changes, runtime, covers: Mutex::new(None) })
    }

    /// Sends a command to the core. Never blocks; with the core stopped the command is lost.
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(Control::Command(command));
    }

    /// The last published snapshot.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.changes.borrow())
    }

    /// The snapshots to come: `changed().await` wakes up only when the core publishes one.
    /// A receiver taken right after `start` loses nothing of what follows the first.
    pub fn changes(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.changes.clone()
    }

    /// Downloads and decodes a cover, off the frontend's thread: the address goes to the backend
    /// (`/v1/cover`, never to the source) and the reply is a square RGBA already reduced
    /// ([`artwork::CoverPixels`]). `None` if the address is not allowed, if the backend does not answer,
    /// if the image is not valid or if the core stops first: the frontend stays on the drawn cover.
    /// The frontend keeps its own texture cache and asks for each address only once.
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

    /// The cover client for the backend chosen now in the settings.
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

    /// Stops the core after saving: waits for the `config.json` write in progress and writes at once
    /// what is missing. The receiver resolves when the thread has closed everything (the state and
    /// its own runtime, if any): a frontend that lent its runtime waits for it before
    /// closing it. Inside there is [`CoreError::Save`] if the last write did not succeed; a receive
    /// error means the thread had already stopped.
    pub fn shutdown(self) -> oneshot::Receiver<Result<(), CoreError>> {
        let (done, stopped) = oneshot::channel();
        let _ = self.commands.send(Control::Shutdown(done));
        stopped
    }
}

/// The seed of the mixes: it changes at each startup and stays the same for the session. The host picks it because
/// the reducer does not read the clock.
fn mix_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0x5eed)
}

/// The body of the `psytify-core` thread. Its own runtime is born and dies here, never in the async
/// context of whoever called `start`.
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
    // `config.json` is read here, once, before the first snapshot: on return from `start` the
    // frontend already has the configuration, as when it read it itself.
    let path = options.data_dir.as_deref().map(config::file_in);
    let config = config::load(path.as_deref());
    // The audio output is born, lives and dies on this thread (the sound card must not be moved elsewhere).
    let (mut audio, device_error) = open_audio(options.audio);
    if let Some(audio) = &audio {
        audio.set_equalizer(config.core.equalizer);
    }
    let mut system = Integrations::open(options.system);
    // The login session sits next to `config.json`, in this core's folder; if there is one, the
    // social domain already starts on "sign-in in progress" and resumes it at startup.
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

/// The download and update folders of this core (see [`CoreOptions::downloads_dir`]).
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

/// Opens the chosen audio output: the output (if there is one) and why it did not open (if it did not).
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

/// The system links: the commands that arrive (media keys), the handle to show the
/// track, Discord and the jam room. They are born, live and die on the core's thread, like
/// the audio output.
struct Integrations {
    /// With `SystemChoice::None` nothing runs: a test core must not turn on Discord nor
    /// show a track in the widget of whoever tries it.
    desktop: bool,
    /// The system's commands; `None` if there is no link or the stream has ended.
    commands: Option<Pin<Box<dyn Stream<Item = media::Event> + Send>>>,
    /// The media link, after `Event::Ready`.
    media: Option<media::Handle>,
    #[cfg(not(target_os = "android"))]
    discord: Option<crate::discord::Presence>,
    /// The link to the jam room, as long as there is a jam. Unlike the others it does not
    /// depend on `SystemChoice`: it is not a system widget, it is network, and a test checks it with a
    /// fake room.
    jam: Option<jam::Link>,
    /// The link that listens to the motion requested by the system, while a frontend wants it
    /// (`Effect::WatchMotion`); `None` if it is off or if the stream has ended (no bus).
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

    /// Opens or closes the motion link. A core without system links (the tests)
    /// does not subscribe to the session bus.
    fn watch_motion(&mut self, on: bool) {
        self.motion = (on && self.desktop).then(|| {
            Box::pin(crate::system_motion::events()) as Pin<Box<dyn Stream<Item = SystemMotion> + Send>>
        });
    }

    /// Opens an address in the system browser. `false` if it did not open. A core without
    /// system links (the tests) opens nothing and reports it as a success: no browser
    /// appears for whoever tries the core.
    fn open_url(&self, url: &crate::auth::Secret) -> bool {
        !self.desktop || crate::platform::open_url(url.expose())
    }

    /// The next command from the system or event from the room; without links it never arrives.
    /// The two waits sit together, on different fields, so they can be awaited with a single borrow.
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

    /// What was heard: it sorts it out.
    fn heard(&mut self, heard: Heard, inputs: &mut Vec<Input>) {
        match heard {
            Heard::System(event) => self.received(event, inputs),
            Heard::Jam(event) => self.jam_received(event, inputs),
            Heard::Motion(Some(reading)) => inputs.push(Input::SystemMotion(reading)),
            // The stream has ended (no session bus): the view stays without a preference.
            Heard::Motion(None) => self.motion = None,
        }
    }

    /// A command arrived: the link's sender is kept here, the rest goes to the reducer.
    fn received(&mut self, event: Option<media::Event>, inputs: &mut Vec<Input>) {
        match event {
            Some(event) => {
                if let media::Event::Ready(handle) = &event {
                    self.media = Some(handle.clone());
                }
                inputs.push(Input::Media(event));
            }
            // The stream has ended (no session bus): it is no longer polled.
            None => self.commands = None,
        }
    }

    /// A room event arrived: it goes to the reducer. A close, or a channel that emptied
    /// without saying so (the link's thread fell), removes the link: it is no longer
    /// polled, and the reducer closes the jam.
    fn jam_received(&mut self, event: Option<jam::Event>, inputs: &mut Vec<Input>) {
        let event = event.unwrap_or_else(|| jam::Event::Closed(jam::LOST.into()));
        if matches!(event, jam::Event::Closed(_)) {
            self.jam = None;
        }
        inputs.push(Input::Jam(event));
    }

    /// Runs an operation on the room link.
    fn jam(&mut self, op: JamOp) {
        match op {
            // The link that was there falls with its thread: the room frees the place.
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

    /// There is no Discord on Android: the reducer asks for nothing.
    #[cfg(target_os = "android")]
    fn discord(&mut self, _op: DiscordOp) {}
}

/// What the host's links have to say: a system command (`None`: the stream has
/// ended) or a jam room event (`None`: the channel has emptied).
enum Heard {
    System(Option<media::Event>),
    Jam(Option<jam::Event>),
    /// A reading of the motion requested by the system (`None`: the stream has ended).
    Motion(Option<SystemMotion>),
}

/// The core's loop: waits for a command, a job's outcome or a timer, passes it to the reducer,
/// runs the effects and publishes if anything changed. After `shutdown` it reads no more commands and
/// runs until the domains have saved ([`Effect::Flushed`]); then it returns whoever waits for the
/// end and the outcome of the save. If the `Core` drops without `shutdown`, it stops at once.
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
    // The first round is the startup: nobody has written yet, but the domains have read the file.
    let mut starting = Some(Input::Start);
    loop {
        let mut inputs = Vec::new();
        if let Some(start) = starting.take() {
            inputs.push(start);
        } else {
            tokio::select! {
            control = inbox.recv(), if closing.is_none() => match control {
                Some(control) => receive(control, &mut inbox, &mut inputs, &mut closing),
                // The `Core` dropped without `shutdown`: nobody is waiting.
                None => return None,
            },
            // A cancelled job has no outcome: it only happens if the runtime closes before the core.
            Some(done) = jobs.join_next() => inputs.extend(done.ok().map(Input::Done)),
            () = deadline(timers.next()) => inputs.extend(timers.expired(Instant::now()).into_iter().map(Input::Timer)),
            // The system's commands and the jam room's events: they enter the core from here,
            // without any tick going to read them.
            heard = system.next() => system.heard(heard, &mut inputs),
            }
        }
        let mut flushed = None;
        for input in inputs {
            let now = Instant::now();
            // What the audio answers to an operation re-enters in the same round, right after the
            // effects that asked for it.
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
                            // If the browser does not open, the domain stops waiting for it.
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

/// Puts in `inputs` the command that just arrived and those already queued behind it (at most
/// [`BURST`]): a burst becomes a single round and a single snapshot. It stops at `shutdown`, after
/// which nobody sends anything any more.
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

/// Runs an effect. Returns the outcome of the save when the effect closes the core.
fn run(
    effect: Effect,
    timers: &mut Timers<TimerKey>,
    jobs: &mut JoinSet<Done>,
    now: Instant,
) -> Option<Result<(), AppError>> {
    match effect {
        Effect::Timer { key, after } => match now.checked_add(after) {
            Some(at) => timers.arm(key, at),
            // A deadline beyond the end of time never arrives: it is enough to remove the timer.
            None => timers.cancel(key),
        },
        Effect::CancelTimer(key) => timers.cancel(key),
        // A panic in the job becomes the outcome `lost`: the domain does not keep waiting.
        Effect::Run(job) => {
            let lost = job.lost();
            jobs.spawn(async move { AssertUnwindSafe(job.run()).catch_unwind().await.unwrap_or(lost) });
        }
        Effect::Flushed(saved) => return Some(saved),
        // The audio and the system links are run by whoever holds them (`actor`), in the same round.
        Effect::Audio(_)
        | Effect::Media(_)
        | Effect::Discord(_)
        | Effect::Jam(_)
        | Effect::OpenUrl(_)
        | Effect::WatchMotion(_) => {}
    }
    None
}

/// Publishes the next snapshot if a domain has changed.
fn publish_changes(state: &CoreState, publish: &watch::Sender<Arc<Snapshot>>) {
    let previous = Arc::clone(&publish.borrow());
    if let Some(next) = state.publish(&previous) {
        publish.send_replace(Arc::new(next));
    }
}

/// Waits until `at`; without a deadline, forever.
async fn deadline(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// The armed timers, in deadline order (on equal deadlines, in arrival order). There are few, one
/// per key: a list is enough.
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
    /// Arms `key` for `at`; if it was already armed, the previous timer is cancelled.
    fn arm(&mut self, key: K, at: Instant) {
        self.cancel(key);
        let index = self.armed.partition_point(|(_, other)| *other <= at);
        self.armed.insert(index, (key, at));
    }

    fn cancel(&mut self, key: K) {
        self.armed.retain(|(armed, _)| *armed != key);
    }

    /// The next deadline.
    fn next(&self) -> Option<Instant> {
        self.armed.first().map(|(_, at)| *at)
    }

    /// Removes the timers expired by `now` and returns them in the order they expire.
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

    /// A core on the `handle` runtime, without a data folder: it reads and writes no files.
    fn borrowing(handle: Handle) -> CoreOptions {
        CoreOptions {
            runtime: RuntimeChoice::Borrow(handle),
            audio: AudioChoice::Detached,
            system: SystemChoice::None,
            data_dir: None,
            downloads_dir: None,
        }
    }

    /// A new data folder, just for this test.
    fn data_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("psytify-core-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn store(core: CoreConfig) -> Command {
        Command::Settings(SettingsCmd::Store(Box::new(core)))
    }

    /// The audio belongs to the core since step 4.3: this test opens it detached (`AudioChoice::Detached`),
    /// never on the sound card, and closes it on the thread that opened it.
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

    /// The `config.json` of a release, in this core's folder (no lock between the tests,
    /// `docs/transformation/BACKLOG.md`, 11): the core reads it at startup and at shutdown rewrites it
    /// with the last state, without waiting for the delay and without losing the keys it does not know.
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

    /// Without a shutdown, a change reaches the disk by itself after [`SAVE_DELAY`]. Here the disk
    /// refuses it (there is a file in place of the folder): the error is published, and the shutdown, after
    /// one last attempt, reports it. The core stops anyway.
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

    /// Step 4.2: the library goes through the core, which writes it to `config.json` after [`SAVE_DELAY`]
    /// together with the other fields, and finds it again on restart. A shutdown right after the command does
    /// not lose anything.
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
        // The core writes it when it wants to, on the test's runtime: the disk is watched until it arrives.
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

        // A change and then the shutdown at once: the write happens all the same.
        core.send(Command::library(LibraryCmd::ToggleLike(track(2))));
        core.shutdown().await.unwrap().unwrap();
        assert_eq!(config::load(Some(&config::file_in(&dir))).core.liked.len(), 2);

        let core = Core::start(CoreOptions { data_dir: Some(dir), ..borrowing(Handle::current()) }).unwrap();
        let liked = core.snapshot().library.liked.iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(liked, [track(2).id, track(1).id]);
        core.shutdown().await.unwrap().unwrap();
    }

    /// Step 5: a frontend asks the core for a cover, which downloads it from the backend chosen in the
    /// settings (`/v1/cover`) on its own runtime and returns it decoded. An address that is not
    /// Not allowed (not https, or a host that is neither archive.org nor Deezer's CDN) it does not leave the core:
    /// `None` with no request at all.
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

        // The cover client is reused as long as the backend is the same.
        core.artwork(url).await.unwrap().expect("ancora una");
        assert_eq!(backend.count("/v1/cover"), 2);
        core.shutdown().await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Step 4.5 (BACKLOG 24): a single command, `ExternalCmd::PlaySource`, brings an official catalog result
    /// into the server's catalog and plays it, all on the core's runtime: the frontend does not import, does not
    /// queue and does not reread anything. The "importing" row stays while the server works,
    /// then the track is the one playing.
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

    /// Step 4.9: the music video goes through the core, on the runtime. While the video is hidden the core
    /// does not ask for the address; with the video shown it asks the backend and publishes the 360p one; six
    /// seconds later (the core's timer, armed by it) it asks for and publishes the 720p, once, and then it
    /// no longer asks for anything. The track that starts has its video prepared only once.
    /// With the video hidden, the domain rests. Real time: the timer is six seconds and the backend is a real
    /// port, on which a stopped clock would jump straight to the timeouts.
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

        // The track starts with the videos wanted but hidden: the server prepares its own, once, and
        // nobody asks for the address.
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

        // Shown: the address, at 360p, with the backend's origin in front of the proxy address.
        let shown = Instant::now();
        core.send(Command::Video(VideoCmd::Show(true)));
        let ready = until(&mut changes, |snapshot| matches!(snapshot.video.state, ClipState::Ready(_))).await;
        let ClipState::Ready(first) = &ready.video.state else { unreachable!("appena controllato") };
        assert_eq!((ready.video.track, first.height), (Some(clip.id), Some(360)));
        assert_eq!(first.url(), format!("{}/v1/media?t=clip", backend.url));

        // Six seconds later, on its own, the full version; then the core no longer asks for anything.
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

        // Hidden again, the video goes away and the core goes back to rest.
        core.send(Command::Video(VideoCmd::Show(false)));
        let idle = until(&mut changes, |snapshot| snapshot.video.state == ClipState::Idle).await;
        tokio::time::sleep(Duration::from_millis(900)).await;
        assert_eq!(core.snapshot().versions.video, idle.versions.video, "il video non pubblica più niente");
        assert_eq!(backend.count(&video_url["GET ".len()..]), 2, "e non chiede più niente");
        core.shutdown().await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Step 4.3: a `Play` starts the loading on the core's runtime and the reply (here an
    /// error: the backend is a closed port) re-enters the domain the same way as for a real
    /// track. The core holds the audio output (detached) and closes it with itself.
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

    /// Step 4.4: the key arms the wait in the core, the search starts on the runtime when it expires and the
    /// replies (here errors: the backend is a closed port) re-enter the domain; then the core is idle.
    #[tokio::test]
    async fn the_debounced_search_runs_on_the_runtime_and_the_core_goes_quiet() {
        use crate::catalog::Load;
        use crate::command::CatalogCmd;

        let dir = data_dir();
        std::fs::write(config::file_in(&dir), r#"{"server":"http://127.0.0.1:9"}"#).unwrap();
        let core = Core::start(CoreOptions { data_dir: Some(dir), ..borrowing(Handle::current()) }).unwrap();
        let mut changes = core.changes();
        core.send(Command::Catalog(CatalogCmd::SearchInput("low".into())));
        // Writing publishes nothing: the search starts only when the 300 ms expire.
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
        // Idle: after the replies, and after the save that follows, no more publishing.
        tokio::time::sleep(Duration::from_millis(900)).await;
        changes.borrow_and_update();
        let quiet = tokio::time::timeout(Duration::from_millis(700), changes.changed()).await;
        assert!(quiet.is_err(), "an idle core publishes nothing");
        core.shutdown().await.unwrap().unwrap();
    }

    /// A burst of commands already queued is a single round: a single snapshot, with the last state.
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

    /// Waits, at most 20 s, for the core's snapshot to satisfy `ready`.
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

    /// A Clerk instance that does not answer (https on a closed localhost port): no test
    /// talks to Clerk, and no test uses a real session.
    fn dead_clerk(session_file: Option<PathBuf>) -> crate::auth::ClerkApp {
        crate::auth::ClerkApp { issuer: "https://127.0.0.1:9".into(), port: 53682, session_file }
    }

    /// A data folder with the configuration pointing at the fake Clerk and backend.
    fn social_dir() -> PathBuf {
        let dir = data_dir();
        std::fs::write(
            dir.join("config.json"),
            r#"{"clerk_issuer":"https://127.0.0.1:9","server":"http://127.0.0.1:9"}"#,
        )
        .unwrap();
        dir
    }

    /// Step 4.7 and BACKLOG 11: the saved session sits in this core's data folder, with no
    /// per-process path. A core with its own `session.json` starts already on "sign-in in progress" and
    /// resumes it by itself; another, in another folder, in the same process, does not see it. If
    /// Clerk does not answer the session is not lost: the file stays for the next startup.
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

    /// Signing out removes `session.json` from this core's folder, even with Clerk and the backend not
    /// answering, and the core stops: no polling timer stays armed.
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

        // A sign-in already done, as after the last step: no network call at startup.
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

    /// Step 4.6b, on a real core: a downloaded track (from a fake backend, in a temporary data folder)
    /// enters the index with weight and SHA-256, and plays from the disk with the backend off:
    /// no listen request, no network. The file is the downloaded one, byte for byte.
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

        // Without a backend: the track starts anyway, from the file.
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

    /// The file disappears: the track plays from the network, the index marks it missing and does not remove it; at a
    /// new startup the quick check marks it by itself; "Remove" removes the entry and finds nothing to
    /// delete. The index survives the restart (it sits in `config.json`).
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

        // The file disappears outside the app, the core does not know: the loading discovers it.
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

        // Restart: the index is in `config.json`, the quick check marks the missing file.
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

    /// Waits, at most 20 s, for `ready` to be true (for what is outside the core: the fake room).
    async fn eventually(ready: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(20), async {
            while !ready() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the outside world reaches the expected state");
    }

    /// Step 4.8: the jam runs on the link's events, which the host waits for together with everything
    /// else. The core creates the room (with the command's name), the room's events wake it
    /// by themselves (the members, a state of the others with the tracks to ask the catalog for), an idle jam
    /// publishes nothing and sends nothing to the room, and leaving closes the socket.
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

        // Idle: no snapshot, no state to the room (nothing queued, nothing to say).
        let quiet = tokio::time::timeout(Duration::from_millis(400), changes.changed()).await;
        assert!(quiet.is_err(), "una jam ferma non sveglia il core");
        assert!(room.received().is_empty());

        // A member joins: the room says so, the core publishes only the jam.
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

        // The others' state: the tracks are not there, they are asked of the catalog (on the core's runtime) and
        // the queue becomes the group's, on the second track.
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

        // Leaving: the socket closes, the view goes back to empty.
        core.send(Command::Jam(JamCmd::Leave));
        let left = until(&mut changes, |snapshot| !snapshot.jam.is_active()).await;
        assert!(left.jam.code.is_none() && left.jam.members.is_empty());
        eventually(|| room.closed() == 1).await;
        core.shutdown().await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A server that does not answer closes the jam with its reason, and it can be started again.
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
