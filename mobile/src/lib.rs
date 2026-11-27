//! Psytify for the Kotlin Android app: the core (`psytify-core`) behind a UniFFI object. Like the
//! GPUI desktop's `CoreModel`, the app sends commands and receives the core's snapshots, one domain
//! at a time and only when that domain's version changed; nothing here decides anything the core
//! does not already decide, except how a page's tap maps to a core command (`home.rs`), which the
//! desktop also does in its pages.
//!
//! Plan and stages: `docs/transformation/android-kotlin.md`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use psytify_core::api::Track;
use psytify_core::auth::Secret;
use psytify_core::command::{CatalogCmd, JamCmd, PlaybackCmd, Resource, SettingsCmd, SocialCmd, VideoCmd};
use psytify_core::{AudioChoice, Command, Core, CoreOptions, RuntimeChoice, SystemChoice, config};
use tokio::sync::{oneshot, watch};
use uuid::Uuid;

mod account;
#[cfg(target_os = "android")]
mod android;
mod dialog;
mod downloads;
#[cfg(feature = "fixtures")]
mod fixtures;
mod format;
mod home;
mod lyrics;
mod media;
mod menu;
mod pages;
mod playback;
mod pump;
mod queue;
mod settings;
mod social;
#[cfg(test)]
mod test_support;
mod update;
mod video;

pub use account::{AccountSnapshot, AccountState, SignInMode, SignInProvider, SignInStep};
pub use dialog::DialogView;
pub use downloads::{DownloadMark, DownloadsSnapshot, Quality};
use home::HomeModel;
pub use home::{HomeAction, HomeSnapshot, HomeStatus, Shelf, ShelfKind, Tile};
pub use lyrics::{LyricLine, LyricsCursor, LyricsSnapshot, LyricsStatus};
pub use media::{Cover, MediaCommand, NowPlaying};
pub use menu::{MenuChoice, MenuItem, MenuPlaylist, MenuQuality, MenuView};
pub use pages::{
    Art, ButtonKind, Chip, Header, HeaderButton, PageAction, PageHandle, PageListener, PageStatus, PageView,
    Route, Row, RowAction, RowButton, RowPage, RowState, Source, SourceRef, Tap, Tone,
};
pub use playback::{PlaybackAction, PlaybackSnapshot, PlayerState, RepeatMode, TrackItem};
use queue::QueueModel;
pub use queue::{QueueEntry, QueuePage};
pub use settings::{AccountLine, Band, Credit, MotionChoice, Select, SettingsAction, SettingsSnapshot};
pub use social::SocialMarks;
pub use update::{UpdateControl, UpdateLine};
pub use video::{ClipStatus, VideoAdjust, VideoMeasure, VideoSnapshot, VideoSync};

uniffi::setup_scaffolding!();

/// Threads of the core's own runtime (network, covers, downloads), as on the desktop.
const CORE_WORKERS: usize = 2;

/// How long the core keeps running while the app is hidden and nothing plays. Then it stops (after
/// its last write of `config.json`) and closes the audio output; the next command starts it again.
const IDLE_STOP: Duration = Duration::from_secs(120);

/// The backend of a new configuration. `PSYTIFY_DEFAULT_API_URL` at build time (from `.env.build`,
/// see `scripts/android-kotlin.sh`) wins; a debug build without it uses the PC that hosts the
/// emulator; otherwise the core's own default.
fn default_backend() -> &'static str {
    match option_env!("PSYTIFY_DEFAULT_API_URL") {
        Some(url) if !url.is_empty() => url,
        _ if cfg!(debug_assertions) => "http://10.0.2.2:8080",
        _ => config::DEFAULT_API_URL,
    }
}

/// Receives the core's changes, on a thread of the bridge: the app must hand them to its own
/// threads (a `StateFlow` does). Each method is called only when its domain changed, and with the
/// first state each time the core starts.
#[uniffi::export(with_foreign)]
pub trait CoreListener: Send + Sync {
    fn playback_changed(&self, playback: PlaybackSnapshot);
    fn home_changed(&self, home: HomeSnapshot);
    /// What the system's media controls show (Android only); `None` when nothing plays or the core
    /// has stopped.
    fn now_playing(&self, now: Option<NowPlaying>);
    /// Something to tell the person, briefly (an import that failed, an album saved).
    fn notice(&self, text: String, error: bool);
    /// The core asks the app to open a page (a catalogue artist that is now on the server).
    fn open(&self, route: Route);
    /// Starting the core again failed (`Some`, with the reason), or a later start worked (`None`).
    /// Only after the first start, whose failure `PsytifyCore.start` returns.
    fn core_failed(&self, reason: Option<String>);
    /// The account or the sign-in form changed.
    fn account_changed(&self, account: AccountSnapshot);
    /// Open `url` in the browser (a sign-in provider's page); `false` if nothing could open it. The
    /// address carries a one-time state: do not log it.
    fn open_browser(&self, url: String) -> bool;
    /// What the notification of the downloads shows; only when it changed. While `running`, the
    /// app keeps a foreground service so the transfer is not frozen in the background.
    fn downloads_changed(&self, downloads: DownloadsSnapshot);
    /// The jam marker of the player bar and the counts of invites and requests; only when changed.
    fn marks_changed(&self, marks: SocialMarks);
    /// What the settings page shows; only when it changed.
    fn settings_changed(&self, settings: SettingsSnapshot);
    /// What «Testo» shows for the current track; only when it changed, and at each start.
    fn lyrics_changed(&self, lyrics: LyricsSnapshot);
    /// The clip of the current track as the media slot sees it; only when it changed.
    fn video_changed(&self, video: VideoSnapshot);
    /// The app's update package is downloaded at `path` (the app's files): hand it to the system's
    /// installer, which asks the person to confirm.
    fn install_update(&self, path: String);
    /// A new release `tag`, found by the six-hourly check while the app stayed open: worth a
    /// notification, once per release.
    fn update_offered(&self, tag: String);
}

/// Why the core did not start or stop cleanly.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum CoreFailure {
    /// `NativeContext.init` was not called before `PsytifyCore.start` (Android).
    NoContext,
    Start {
        reason: String,
    },
    /// The last write of `config.json` at shutdown failed; the file is the previous one, whole.
    Save {
        reason: String,
    },
}

impl std::fmt::Display for CoreFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoContext => f.write_str("NativeContext.init was not called before start"),
            Self::Start { reason } | Self::Save { reason } => f.write_str(reason),
        }
    }
}

impl std::error::Error for CoreFailure {}

/// A started core and which start it was, so a stop meant for an older one does not take it.
struct Running {
    core: Core,
    generation: u64,
}

/// What the bridge shares with its pumps. The core inside stops when the app is idle in the
/// background and starts again on the next command; everything else lives as long as the bridge.
pub(crate) struct Inner {
    running: Mutex<Option<Running>>,
    launch: Launch,
    listener: Arc<dyn CoreListener>,
    generations: AtomicU64,
    home: Mutex<HomeModel>,
    /// Home versions keep growing across restarts, so a tap on a Home of an older core is stale.
    home_version: AtomicU64,
    queue: Mutex<QueueModel>,
    /// The lyrics last told, for the cursor and the taps on a verse.
    lyrics: Mutex<lyrics::LyricsModel>,
    /// The pages the app has open.
    pages: Mutex<pages::Pages>,
    /// Page versions keep growing across pages and restarts.
    page_version: AtomicU64,
    /// What the system's media controls show, kept across stops.
    shown: Mutex<media::Shown>,
    /// The «Altro» sheet and the dialog open, one of each at most.
    menu: Mutex<Option<menu::OpenMenu>>,
    dialog: Mutex<Option<(u64, dialog::Request)>>,
    /// Identifiers of menus and dialogs.
    overlays: AtomicU64,
    /// The last start failed and the app was told.
    failed: Mutex<bool>,
    /// The jam form's last attempt and the last jam event left behind.
    jam_form: Mutex<pages::jam::Form>,
    /// Fake social and jam views (debug builds with the `fixtures` feature).
    #[cfg(feature = "fixtures")]
    fixture: Mutex<Option<fixtures::Fixture>>,
    /// Whether the app's UI is on screen.
    visible: watch::Sender<bool>,
    /// The bridge's thread, where the pumps run.
    bridge: tokio::runtime::Handle,
    /// Dropped with the bridge: ends its thread.
    _bridge_alive: oneshot::Sender<()>,
}

impl Inner {
    /// Sends `command`, starting the core again if it had stopped.
    pub(crate) fn send(self: &Arc<Self>, command: Command) {
        #[cfg(feature = "fixtures")]
        if self.fixture_takes(&command) {
            return;
        }
        let running = self.running();
        if let Some(running) = running.as_ref() {
            running.core.send(command);
        }
    }

    /// The core, started if needed. `None` only if it cannot start: the app is told why, and the
    /// next command tries again.
    fn running(self: &Arc<Self>) -> MutexGuard<'_, Option<Running>> {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if running.is_none() {
            let started = self.start_core();
            let failure = started.as_ref().err().map(ToString::to_string);
            *running = started.ok();
            let mut failed = self.failed.lock().unwrap_or_else(PoisonError::into_inner);
            if failure.is_some() || *failed {
                *failed = failure.is_some();
                self.listener.core_failed(failure);
            }
        }
        running
    }

    fn start_core(self: &Arc<Self>) -> Result<Running, CoreFailure> {
        #[cfg(target_os = "android")]
        if self.launch.system_media {
            media::attach(Arc::downgrade(self));
        }
        let core = (self.launch.start)(self.launch.options.clone())
            .map_err(|err| CoreFailure::Start { reason: err.to_string() })?;
        let generation = self.generations.fetch_add(1, Ordering::Relaxed) + 1;
        self.bridge.spawn(pump::run(
            Arc::downgrade(self),
            generation,
            core.changes(),
            self.visible.subscribe(),
            self.launch.idle_stop,
        ));
        Ok(Running { core, generation })
    }

    /// Calls `with` on the running core, without starting it.
    fn current<T>(&self, with: impl FnOnce(&Core) -> T) -> Option<T> {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|running| with(&running.core))
    }

    pub(crate) fn artwork(
        &self,
        url: String,
    ) -> Option<tokio::sync::oneshot::Receiver<Option<psytify_core::artwork::CoverPixels>>> {
        self.current(|core| core.artwork(url))
    }

    /// Stops the core after its last write of `config.json`: start `generation` only, or whichever
    /// runs with `None`. The system's media controls are told nothing plays.
    pub(crate) async fn stop(&self, generation: Option<u64>) -> Result<(), CoreFailure> {
        let running = {
            let mut guard = self.running.lock().unwrap_or_else(PoisonError::into_inner);
            match guard.as_ref() {
                Some(running) if generation.is_none_or(|g| g == running.generation) => guard.take(),
                _ => None,
            }
        };
        let Some(running) = running else { return Ok(()) };
        #[cfg(target_os = "android")]
        media::detach();
        let result = match running.core.shutdown().await {
            Ok(Ok(())) | Err(_) => Ok(()),
            Ok(Err(err)) => Err(CoreFailure::Save { reason: err.to_string() }),
        };
        if self.launch.system_media {
            // The last track stays on the lock screen, paused, while the queue has it.
            self.show_media(None);
        }
        result
    }

    /// What the core shows to the system's media controls, through [`media::Shown`].
    pub(crate) fn show_media(&self, now: Option<NowPlaying>) {
        let shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner).core_shows(now);
        if let Some(shown) = shown {
            self.listener.now_playing(shown);
        }
    }

    /// Whether the queue the core published has a current entry, for [`media::Shown`].
    pub(crate) fn media_queue(&self, has_entry: bool) {
        let shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner).queue(has_entry);
        if let Some(shown) = shown {
            self.listener.now_playing(shown);
        }
    }

    pub(crate) fn listener(&self) -> &Arc<dyn CoreListener> {
        &self.listener
    }

    pub(crate) fn system_media(&self) -> bool {
        self.launch.system_media
    }

    pub(crate) fn home(&self) -> MutexGuard<'_, HomeModel> {
        self.home.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn next_home_version(&self) -> u64 {
        self.home_version.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub(crate) fn queue(&self) -> MutexGuard<'_, QueueModel> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn lyrics(&self) -> MutexGuard<'_, lyrics::LyricsModel> {
        self.lyrics.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn pages(&self) -> MutexGuard<'_, pages::Pages> {
        self.pages.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn next_page_version(&self) -> u64 {
        self.page_version.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// The core's latest snapshot, starting the core if needed. `None` only if it cannot start.
    pub(crate) fn snapshot(self: &Arc<Self>) -> Option<Arc<psytify_core::Snapshot>> {
        let snapshot = self.running().as_ref().map(|running| running.core.snapshot())?;
        Some(self.dressed(snapshot))
    }

    /// `snapshot` as the app sees it: with a fixture on, its social and jam views instead of the core's.
    pub(crate) fn dressed(&self, snapshot: Arc<psytify_core::Snapshot>) -> Arc<psytify_core::Snapshot> {
        #[cfg(feature = "fixtures")]
        if let Some(fixture) = self.fixture.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            let mut dressed = (*snapshot).clone();
            dressed.social = Arc::clone(&fixture.social);
            if let Some(jam) = &fixture.jam {
                dressed.jam = Arc::clone(jam);
            }
            return Arc::new(dressed);
        }
        snapshot
    }

    /// With a fixture on, a social or jam command stays here (answered by the fixture, if at all).
    #[cfg(feature = "fixtures")]
    fn fixture_takes(self: &Arc<Self>, command: &Command) -> bool {
        let changed = {
            let mut fixture = self.fixture.lock().unwrap_or_else(PoisonError::into_inner);
            match fixture.as_mut() {
                Some(fixture) if matches!(command, Command::Social(_) | Command::Jam(_)) => {
                    fixture.intercept(command)
                }
                _ => return false,
            }
        };
        if changed {
            self.republish();
        }
        true
    }

    /// Tells the app the social side again, from the latest snapshot (a fixture changed).
    #[cfg(feature = "fixtures")]
    fn republish(self: &Arc<Self>) {
        let Some(snapshot) = self.snapshot() else { return };
        self.listener.account_changed(AccountSnapshot::of(&snapshot.social));
        self.listener.marks_changed(SocialMarks::of(&snapshot));
        let free = downloads::free_bytes(&snapshot.downloads.folder);
        self.listener.settings_changed(settings::SettingsSnapshot::of(&snapshot, free));
        let changes = pages::Changes { pages: true, progress: false };
        for page in self.pages().live() {
            page.refresh(self, &snapshot, false, changes);
        }
    }

    /// The last jam event the person left behind (the Jam page's card shows newer errors only).
    pub(crate) fn jam_dismissed(&self) -> u64 {
        self.jam_form.lock().unwrap_or_else(PoisonError::into_inner).dismissed
    }

    /// Leaves the error on the Jam card behind; `true` if there was one.
    fn dismiss_jam_failure(self: &Arc<Self>) -> bool {
        let Some(snapshot) = self.snapshot() else { return false };
        let mut form = self.jam_form.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((seq, _, _)) = pages::jam::failure(&snapshot.jam, form.dismissed) else { return false };
        form.dismissed = seq;
        drop(form);
        let changes = pages::Changes { pages: true, progress: false };
        for page in self.pages().live() {
            page.refresh(self, &snapshot, false, changes);
        }
        true
    }

    /// Starts a jam as `attempt` with `command`, leaving the last error behind.
    fn start_jam(self: &Arc<Self>, attempt: pages::jam::Attempt, command: JamCmd) {
        self.jam_form.lock().unwrap_or_else(PoisonError::into_inner).attempt = Some(attempt);
        self.dismiss_jam_failure();
        self.send(Command::Jam(command));
    }

    /// «Entra» on a jam invite.
    pub(crate) fn join_invite(self: &Arc<Self>, code: &str) {
        let active = self.snapshot().is_some_and(|snapshot| snapshot.jam.is_active());
        match pages::jam::accept(code, active) {
            Ok(commands) => {
                self.jam_form.lock().unwrap_or_else(PoisonError::into_inner).attempt =
                    Some(pages::jam::Attempt::Join);
                self.dismiss_jam_failure();
                for command in commands {
                    self.send(command);
                }
            }
            Err(notice) => self.listener.notice(notice, false),
        }
    }

    fn next_overlay(&self) -> u64 {
        self.overlays.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Opens the «Altro» sheet of `track`, in place of any other.
    pub(crate) fn open_menu(
        self: &Arc<Self>,
        track: Track,
        from_playlist: Option<(Uuid, usize)>,
    ) -> MenuView {
        let open = menu::OpenMenu { id: self.next_overlay(), track, from_playlist };
        let snapshot = self.snapshot().unwrap_or_default();
        let view = menu::view(&open, &snapshot);
        *self.menu.lock().unwrap_or_else(PoisonError::into_inner) = Some(open);
        view
    }

    /// Opens a playlist dialog, in place of any other.
    pub(crate) fn open_dialog(self: &Arc<Self>, request: dialog::Request) -> DialogView {
        let id = self.next_overlay();
        let snapshot = self.snapshot().unwrap_or_default();
        let view = dialog::view(id, &request, &snapshot);
        *self.dialog.lock().unwrap_or_else(PoisonError::into_inner) = Some((id, request));
        view
    }

    /// Sends what `outcome` decided and tells the app what comes next.
    fn apply(self: &Arc<Self>, outcome: menu::Outcome) -> Tap {
        for command in outcome.commands {
            self.send(command);
        }
        if let Some((text, error)) = outcome.notice {
            self.listener.notice(text, error);
        }
        match outcome.next {
            menu::Next::Nothing | menu::Next::Stay => Tap::Nothing,
            menu::Next::Open(route) => Tap::Open { route },
            menu::Next::Copy(text) => Tap::Copy { text },
            menu::Next::Share(text) => Tap::Share { text },
            menu::Next::Dialog(request) => Tap::Dialog { dialog: self.open_dialog(request) },
            menu::Next::Leave(route) => Tap::Leave { route },
        }
    }
}

/// The running core, as the app holds it. One per process: the app keeps it in its `Application`.
#[derive(uniffi::Object)]
pub struct PsytifyCore {
    inner: Arc<Inner>,
}

/// How to start: the device's audio and media controls, or none (the host tests).
pub(crate) struct Launch {
    pub(crate) options: CoreOptions,
    /// `Core::start`; the tests make it fail.
    pub(crate) start: fn(CoreOptions) -> Result<Core, psytify_core::CoreError>,
    pub(crate) system_media: bool,
    pub(crate) idle_stop: Duration,
}

impl Launch {
    fn device(data_dir: PathBuf, downloads_dir: Option<PathBuf>) -> Self {
        let downloads_dir = downloads_dir.unwrap_or_else(|| data_dir.join("Download"));
        Self {
            start: Core::start,
            options: CoreOptions {
                runtime: RuntimeChoice::Own { workers: CORE_WORKERS },
                audio: AudioChoice::System,
                system: SystemChoice::Desktop,
                downloads_dir: Some(downloads_dir),
                data_dir: Some(data_dir),
            },
            system_media: cfg!(target_os = "android"),
            idle_stop: IDLE_STOP,
        }
    }
}

/// The bridge's thread: a current-thread runtime that runs the pumps until the bridge is dropped.
fn bridge_thread() -> std::io::Result<(tokio::runtime::Handle, oneshot::Sender<()>)> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build()?;
    let handle = runtime.handle().clone();
    let (alive, closed) = oneshot::channel::<()>();
    std::thread::Builder::new().name("psytify-mobile".into()).spawn(move || {
        let _ = runtime.block_on(closed);
    })?;
    Ok((handle, alive))
}

impl PsytifyCore {
    fn social(&self, command: SocialCmd) {
        self.inner.send(Command::Social(command));
    }

    pub(crate) fn launch(launch: Launch, listener: Arc<dyn CoreListener>) -> Result<Arc<Self>, CoreFailure> {
        config::set_default_api_url(default_backend().to_owned());
        let (bridge, alive) =
            bridge_thread().map_err(|err| CoreFailure::Start { reason: err.to_string() })?;
        account::install_browser(&listener);
        let inner = Arc::new(Inner {
            running: Mutex::new(None),
            launch,
            listener,
            generations: AtomicU64::new(0),
            home: Mutex::new(HomeModel::default()),
            home_version: AtomicU64::new(0),
            queue: Mutex::new(QueueModel::default()),
            lyrics: Mutex::new(lyrics::LyricsModel::default()),
            pages: Mutex::new(pages::Pages::default()),
            page_version: AtomicU64::new(0),
            shown: Mutex::new(media::Shown::default()),
            menu: Mutex::new(None),
            dialog: Mutex::new(None),
            overlays: AtomicU64::new(0),
            failed: Mutex::new(false),
            jam_form: Mutex::new(pages::jam::Form::default()),
            #[cfg(feature = "fixtures")]
            fixture: Mutex::new(None),
            visible: watch::Sender::new(false),
            bridge,
            _bridge_alive: alive,
        });
        let first = inner.start_core()?;
        *inner.running.lock().unwrap_or_else(PoisonError::into_inner) = Some(first);
        Ok(Arc::new(Self { inner }))
    }
}

#[uniffi::export]
impl PsytifyCore {
    /// Starts the core with `data_dir` (the app's private files directory) for `config.json` and
    /// the session, and `downloads_dir` for the downloaded tracks (`data_dir/Download` without it),
    /// and returns once the first snapshot exists. The listener gets the first state right after,
    /// from the bridge's thread.
    #[uniffi::constructor]
    pub fn start(
        data_dir: String,
        downloads_dir: Option<String>,
        listener: Arc<dyn CoreListener>,
    ) -> Result<Arc<Self>, CoreFailure> {
        #[cfg(target_os = "android")]
        if !android::ready() {
            return Err(CoreFailure::NoContext);
        }
        let launch = Launch::device(PathBuf::from(data_dir), downloads_dir.map(PathBuf::from));
        Self::launch(launch, listener)
    }

    /// The app's UI came on screen or left it. Hidden, the core writes what waits to be saved right
    /// away (Android may end the process at any time) and, if nothing plays for a while, stops.
    pub fn set_visible(&self, visible: bool) {
        self.inner.visible.send_replace(visible);
        if visible {
            drop(self.inner.running());
        } else {
            self.inner.current(|core| core.send(Command::Settings(SettingsCmd::SaveNow)));
        }
    }

    pub fn playback(&self, action: PlaybackAction) {
        self.inner.send(Command::Playback(action.command()));
    }

    pub fn home(&self, action: HomeAction) {
        let command = match action {
            HomeAction::Ensure => CatalogCmd::Ensure(Resource::Home),
            HomeAction::Retry => CatalogCmd::Retry(Resource::Home),
        };
        self.inner.send(Command::Catalog(command));
    }

    /// A tap on tile `tile` of shelf `shelf` of the Home with this `version`: it plays, or the app
    /// opens the page it returns. A tap on a Home that has been rebuilt since does nothing.
    pub fn activate_home(&self, version: u64, shelf: u32, tile: u32) -> Tap {
        let Some(playback) = self.inner.current(|core| Arc::clone(&core.snapshot().playback)) else {
            return Tap::Nothing;
        };
        match self.inner.home().activate(version, shelf as usize, tile as usize, &playback) {
            Some(home::HomeTap::Play(command)) => {
                self.inner.send(Command::Playback(command));
                Tap::Nothing
            }
            Some(home::HomeTap::Open(route)) => Tap::Open { route },
            None => Tap::Nothing,
        }
    }

    /// Opens the page at `route`; it stays open, and `listener` hears its changes, while the
    /// handle lives. The listener gets the first view before this returns.
    pub fn open_page(&self, route: Route, listener: Arc<dyn PageListener>) -> Arc<PageHandle> {
        PageHandle::open(&self.inner, &route, listener)
    }

    /// The search field changed: the core searches 300 ms after the last change.
    pub fn search_input(&self, text: String) {
        self.inner.send(Command::Catalog(CatalogCmd::SearchInput(text)));
    }

    /// Up to `limit` entries (at most [`queue::MAX_PAGE`]) of the queue in listening order, from
    /// `offset`, as last published.
    pub fn queue_page(&self, offset: u32, limit: u32) -> QueuePage {
        self.inner.queue().page(offset, limit)
    }

    /// The «Altro» sheet of the queue entry at `position` of the listening order, read from the queue
    /// at `version` (`PlaybackSnapshot::queue_version`); `None` if the queue changed since.
    pub fn queue_menu(&self, version: u64, position: u32) -> Option<MenuView> {
        let track = self.inner.queue().track(version, position as usize)?;
        Some(self.inner.open_menu(track, None))
    }

    /// A choice in the sheet `menu`. The sheet closes, except for «Aggiungi a una playlist», whose
    /// list the app shows in its place. A sheet that is no longer open does nothing.
    pub fn choose(&self, menu: u64, choice: MenuChoice) -> Tap {
        let open = {
            let mut guard = self.inner.menu.lock().unwrap_or_else(PoisonError::into_inner);
            match guard.as_ref() {
                Some(open) if open.id == menu => {
                    if matches!(choice, MenuChoice::ShowPlaylists | MenuChoice::ShowQualities) {
                        return Tap::Nothing;
                    }
                    guard.take()
                }
                _ => None,
            }
        };
        let Some(open) = open else { return Tap::Nothing };
        let snapshot = self.inner.snapshot().unwrap_or_default();
        let outcome = menu::choose(&open, &choice, &snapshot);
        self.inner.apply(outcome)
    }

    /// The sheet `menu` closed without a choice.
    pub fn close_menu(&self, menu: u64) {
        let mut guard = self.inner.menu.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.as_ref().is_some_and(|open| open.id == menu) {
            *guard = None;
        }
    }

    /// The dialog `dialog` confirmed, with the text of its field if it has one.
    pub fn confirm_dialog(&self, dialog: u64, text: Option<String>) -> Tap {
        let request = {
            let mut guard = self.inner.dialog.lock().unwrap_or_else(PoisonError::into_inner);
            match guard.take() {
                Some((id, request)) if id == dialog => request,
                other => {
                    *guard = other;
                    return Tap::Nothing;
                }
            }
        };
        let snapshot = self.inner.snapshot().unwrap_or_default();
        self.inner.apply(dialog::confirm(request, text, &snapshot))
    }

    /// The dialog `dialog` was cancelled.
    pub fn cancel_dialog(&self, dialog: u64) {
        let mut guard = self.inner.dialog.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.as_ref().is_some_and(|(id, _)| *id == dialog) {
            *guard = None;
        }
    }

    /// Signs in with an email or username and a password. The password becomes the core's `Secret` here
    /// and goes nowhere else.
    pub fn sign_in(&self, identifier: String, password: String) {
        let password = Secret::new(password);
        self.social(SocialCmd::SignIn { identifier: identifier.trim().to_owned(), password });
    }

    /// Creates an account; Clerk then sends a code to `email` (`SignInStep::Code`).
    pub fn sign_up(&self, email: String, username: String, password: String) {
        let password = Secret::new(password);
        let username = account::handle_chars(&username);
        self.social(SocialCmd::SignUp { email: email.trim().to_owned(), username, password });
    }

    /// The code Clerk sent, for the step on screen.
    pub fn submit_code(&self, code: String) {
        self.social(SocialCmd::SubmitCode(Secret::new(code)));
    }

    pub fn resend_code(&self) {
        self.social(SocialCmd::ResendCode);
    }

    /// Signs in through the browser: the core opens the provider (`CoreListener::open_browser`) and
    /// waits for its return on the loopback address.
    pub fn sign_in_with(&self, provider: SignInProvider) {
        self.social(SocialCmd::SignInWith(provider.provider()));
    }

    /// Back from the code to the fields, or stop waiting for the browser.
    pub fn cancel_sign_in(&self) {
        self.social(SocialCmd::CancelSignIn);
    }

    /// The person types again: the error under the button goes.
    pub fn dismiss_sign_in_error(&self) {
        self.social(SocialCmd::DismissError);
    }

    /// Chooses the username (and the name shown) of a new account.
    pub fn save_profile(&self, handle: String, display_name: String) {
        let handle = account::handle_chars(&handle);
        let display_name = display_name.trim().to_owned();
        self.social(SocialCmd::SaveProfile { handle, display_name, share_activity: None });
    }

    pub fn sign_out(&self) {
        self.social(SocialCmd::SignOut);
    }

    /// «Amici»: searches people by username (the field submitted); an empty text clears the results.
    pub fn search_users(&self, text: String) {
        self.social(SocialCmd::SearchUsers(pages::friends::search_query(&text)));
    }

    /// «Crea una jam» with the name to show in the room (empty: the profile's).
    pub fn jam_create(&self, name: String) {
        let (attempt, command) = pages::jam::retry(Some(pages::jam::Attempt::Create), &name, "");
        self.inner.start_jam(attempt, command);
    }

    /// «Entra» with a code: an incomplete one the core refuses, with its notice and the card's line.
    pub fn jam_join(&self, code: String, name: String) {
        let code = jam_code_chars_owned(&code);
        let (attempt, command) = pages::jam::retry(Some(pages::jam::Attempt::Join), &name, &code);
        self.inner.start_jam(attempt, command);
    }

    /// «Riprova» on the card: the last attempt again, with the fields as they are now.
    pub fn jam_retry(&self, name: String, code: String) {
        let last = self.inner.jam_form.lock().unwrap_or_else(PoisonError::into_inner).attempt;
        let (attempt, command) = pages::jam::retry(last, &name, &jam_code_chars_owned(&code));
        self.inner.start_jam(attempt, command);
    }

    /// The person typed in the card's fields: the error shown goes.
    pub fn jam_typed(&self) {
        self.inner.dismiss_jam_failure();
    }

    /// A change on the settings page.
    pub fn settings(&self, action: SettingsAction) {
        let snapshot = self.inner.snapshot().unwrap_or_default();
        let (commands, notice) = settings::apply(&action, &snapshot);
        for command in commands {
            self.inner.send(command);
        }
        if let Some(notice) = notice {
            self.inner.listener.notice(notice, false);
        }
    }

    /// The downloaded update package, if the core has one ready («Installa» again after the system's
    /// confirmation was dismissed).
    pub fn update_package(&self) -> Option<String> {
        let snapshot = self.inner.snapshot()?;
        update::package(&snapshot.downloads.update).map(|path| path.display().to_string())
    }

    /// «Testo» is on screen (`true`) or left it: the core loads the current track's lyrics only
    /// while it is. Sent again after a failure, it tries again («Riprova»).
    pub fn show_lyrics(&self, on: bool) {
        self.inner.send(Command::Playback(PlaybackCmd::Lyrics(on)));
    }

    /// Where the synced lyrics of `version` are at `position_ms`, and when to ask again.
    pub fn lyrics_cursor(&self, version: u64, position_ms: u64) -> LyricsCursor {
        self.inner.lyrics().cursor(version, Duration::from_millis(position_ms))
    }

    /// A tap on verse `line` of the lyrics of `version`: the playback goes there. Stale lyrics, or
    /// nothing active, do nothing.
    pub fn lyrics_seek(&self, version: u64, line: u32) {
        let Some(at) = self.inner.lyrics().line_at(version, line) else { return };
        let active = self.inner.current(|core| core.snapshot().playback.is_active()).unwrap_or(false);
        if active {
            self.inner.send(Command::Playback(PlaybackCmd::SeekTo(at)));
        }
    }

    /// The media slot shows the clip (`true`) or the cover: the core asks the server for the clip
    /// only while it shows, and has the next tracks' clips prepared (as the desktop's stage does).
    pub fn show_video(&self, on: bool) {
        self.inner.send(Command::Video(VideoCmd::PrepareAhead(on)));
        self.inner.send(Command::Video(VideoCmd::Show(on)));
    }

    /// The signed address of clip `seq` (`ClipStatus::Ready`), if it is still the published one.
    /// For ExoPlayer only: never log it.
    pub fn clip_url(&self, seq: u64) -> Option<String> {
        self.inner.current(|core| video::clip_url(&core.snapshot().video, seq)).flatten()
    }

    /// Clip `seq` did not open or its stream broke: the core asks for a new address once, then
    /// stops that track's clip.
    pub fn video_failed(&self, seq: u64) {
        self.inner.send(Command::Video(VideoCmd::Failed { seq }));
    }

    /// «Riprova» on a clip that did not open.
    pub fn retry_video(&self) {
        self.inner.send(Command::Video(VideoCmd::Retry));
    }

    /// The alignment of a new ExoPlayer to the audio: one per player opened.
    pub fn video_sync(&self) -> Arc<VideoSync> {
        VideoSync::new(&self.inner)
    }

    /// Debug builds with the `fixtures` feature: shows the fake social and jam views `name`
    /// (`fixtures::NAMES`) instead of the core's. `false` for an unknown name, or in any other build.
    pub fn use_fixture(&self, name: String) -> bool {
        #[cfg(feature = "fixtures")]
        {
            let Some(fixture) = fixtures::Fixture::named(&name) else { return false };
            *self.inner.fixture.lock().unwrap_or_else(PoisonError::into_inner) = Some(fixture);
            self.inner.republish();
            true
        }
        #[cfg(not(feature = "fixtures"))]
        {
            let _ = name;
            false
        }
    }

    /// Starts the core again after a failed restart («Riprova»); nothing if it runs.
    pub fn retry_start(&self) {
        drop(self.inner.running());
    }

    /// A button of the system's media controls (notification, lock screen, headset). A stopped core
    /// starts again and gets it.
    pub fn media(&self, command: MediaCommand) {
        drop(self.inner.running());
        media::send(command);
    }

    /// A cover from the backend (`/v1/cover`), decoded on the core's runtime. `None` if the address is
    /// not allowed, the backend does not answer, the image is not valid or the core is stopped.
    pub async fn artwork(&self, url: String) -> Option<Cover> {
        let answer = self.inner.artwork(url)?;
        let pixels = answer.await.ok().flatten()?;
        Some(Cover { side: pixels.side, rgba: pixels.rgba.to_vec() })
    }

    /// Stops the core now, after it has written `config.json`. The next command starts it again.
    pub async fn stop(&self) -> Result<(), CoreFailure> {
        self.inner.stop(None).await
    }
}

fn jam_code_chars_owned(code: &str) -> String {
    pages::jam::jam_code_chars(code)
}

/// Physical pixels per art pixel at this display density (`psytify_design::scale::art_px`): sprites
/// and pixel shapes are drawn at whole multiples of it, never filtered.
#[uniffi::export]
pub fn art_px(density: f32) -> u32 {
    psytify_design::scale::art_px(density)
}

#[cfg(test)]
mod tests;
