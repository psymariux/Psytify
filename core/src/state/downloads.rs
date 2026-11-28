//! Downloads of tracks and app updates (step 4.6).
//!
//! The domain keeps the download queue: each requested track is an [`Item`] that goes from queued to
//! preparing (the server is choosing or re-encoding the file) to downloading; if it ends well it leaves
//! the queue, if it ends badly it stays as failed, with the error, to retry. "Scarica tutto"
//! puts its tracks in the same queue and downloads two at a time ([`BATCH_PARALLEL`]); a single
//! track requested by hand starts at once, as before, even if a batch is in progress. Each download is a
//! job ([`Job::Track`]) that writes to a unique temporary file and publishes it at the end of the run
//! (`api::transfer`): a cancellation stops it and removes the temporary file.
//!
//! Progress does not pass through the reducer at each block: the job counts the bytes in a shared
//! [`Progress`] and a core timer ([`Timer::Progress`], every 250 ms) reads it. The timer is armed only
//! while a download is in progress, and the view changes only when a percentage point changes (or, without
//! the file's length, every 256 KiB): idle, or between two points, the core publishes nothing.
//!
//! Each saved track enters the index of tracks on the device (`offline.rs`: path, format,
//! weight and SHA-256 computed while the file was being written), written to `config.json` with the other
//! settings. At startup a quick check (it exists and has the right weight) marks the files that are not
//! ([`Lost`]): it never deletes them nor removes them from the index, and the track plays from the network. The
//! playback receives the map of the files in order ([`Downloads::local`]) and plays from there; if a file
//! does not open it reports it and the domain marks it lost. "Rimuovi" removes the entry at once and the file
//! only if it still weighs what the index says (a file changed outside the app is not deleted).
//!
//! The domain also checks the app's updates (at startup, every six hours with a core timer and on request)
//! and downloads the package into the updates folder. What the frontend must tell the user (saved track, batch
//! finished, new release) is an event with its sequence number, as in the catalog.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::api::update::{self, Offer};
use crate::api::{self, ApiClient, Cancel, DownloadQuality, Downloaded, Progress, Track};
use crate::config::{self, DownloadsFields};
use crate::effect::{self, Effect, TimerKey};
use crate::error::AppError;
use crate::offline::{self, LocalFile, Lost, LostReason, SavedTrack};
use crate::shared::Shared;

/// How many "Scarica tutto" tracks download together: enough to keep the network busy, few
/// so as not to choke the phone.
pub const BATCH_PARALLEL: usize = 2;
/// How often the progress of downloads in progress is read.
pub const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// Tracks queued or in progress, at most: "Scarica tutto" on a huge library does not fill
/// memory, and beyond the limit the user repeats it when the queue has gone down.
pub const MAX_QUEUED: usize = 500;
/// Failed downloads kept in the queue, to retry them: beyond that, the oldest goes out.
pub const MAX_FAILED: usize = 20;
/// Without the file's length, the progress changes every so many bytes.
const UNKNOWN_STEP: u64 = 256 * 1024;
/// Events kept in the view: a frontend that reads at each publish does not lose any.
const EVENTS: usize = 32;
/// How often an app left open asks for the latest release. One request every six hours: the
/// backend keeps the list in cache for fifteen minutes and GitHub sees nothing.
pub const UPDATE_POLL: Duration = Duration::from_secs(6 * 60 * 60);

/// The domain's commands.
#[derive(Debug, Clone)]
pub enum DownloadsCmd {
    /// Downloads the track in the quality chosen in the settings, at once.
    Download(Box<Track>),
    /// Downloads the track in this quality, at once.
    DownloadAs(Box<Track>, DownloadQuality),
    /// "Scarica tutto": the tracks that can be downloaded enter the queue, two at a time.
    DownloadAll(Arc<[Track]>),
    /// Removes from the queue a track waiting, stops one in progress (its temporary file disappears),
    /// or removes one failed from the list.
    Cancel(Uuid),
    /// Retries a failed download.
    Retry(Uuid),
    /// Changes the download quality.
    SetQuality(DownloadQuality),
    /// Removes a track from the device: the index entry disappears at once, the file if it is still
    /// the downloaded one.
    Remove(Uuid),
    /// Checks the downloaded files thoroughly (existence, weight and SHA-256): those that are not in order
    /// are marked, those back in order are freed. At startup the core only does the quick check.
    Verify,
    /// Opens the downloads folder in the system file manager.
    OpenFolder,
    /// Asks the backend for the latest release.
    CheckUpdate(UpdateCheck),
    /// Downloads the offered update's package.
    DownloadUpdate,
    /// Closes the update banner: it comes back by itself with the next release.
    DismissUpdate,
    /// Opens the folder of the downloaded package in the file manager, to install it by hand
    /// (BACKLOG 39). Only with a package ready; not on Android, where the installation is asked by the
    /// system. Installing it and restarting from the app is not done: it is a decision of the owner (signature,
    /// permissions, files locked on Windows).
    RevealUpdate,
}

/// Whoever asked for the check: it changes what is said when there is nothing new.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateCheck {
    /// At startup.
    Startup,
    /// The timer, with the app open for hours.
    Poll,
    /// "Controlla ora" in Settings: it always answers, even "sei aggiornato" or an error.
    Manual,
}

/// How the last check went, for the line in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    /// The most recent release is this one.
    Current,
    Failed,
}

/// The update offered, and how far it has got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    Offered(Offer),
    /// This release's package is downloading: `received` bytes out of `total`, if the server has
    /// said the length. It changes at most once per percentage point (BACKLOG 38).
    Downloading {
        tag: String,
        received: u64,
        total: Option<u64>,
    },
    Downloaded(PathBuf),
}

impl UpdateState {
    /// The package's progress in percentage points, if it is downloading and its length is known.
    pub fn percent(&self) -> Option<u8> {
        match self {
            Self::Downloading { received, total: Some(total), .. } => Some(percent_of(*received, *total)),
            _ => None,
        }
    }
}

/// The update seen from outside.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateView {
    pub state: Option<UpdateState>,
    pub status: UpdateStatus,
    /// The release whose banner the user has closed.
    dismissed: Option<String>,
}

impl UpdateView {
    /// The banner to show, if there is one: an offer not closed, the download or the file ready.
    pub fn banner(&self) -> Option<&UpdateState> {
        match self.state.as_ref()? {
            UpdateState::Offered(offer) if self.dismissed.as_deref() == Some(offer.tag.as_str()) => None,
            state => Some(state),
        }
    }

    /// A package download is in progress or already done: a check does not touch it.
    fn busy(&self) -> bool {
        matches!(self.state, Some(UpdateState::Downloading { .. } | UpdateState::Downloaded(_)))
    }
}

/// How far a track in the queue has got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemState {
    /// Waits for its turn (only the "Scarica tutto" tracks).
    Queued,
    /// The server is preparing the file (choosing the version, re-encoding): the bytes have not
    /// started yet.
    Preparing,
    /// The bytes arrive; `total` is the announced length, if the server said it.
    Downloading { received: u64, total: Option<u64> },
    /// Did not succeed: the error, and "Riprova".
    Failed(AppError),
}

/// A track in the downloads queue.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub track: Arc<Track>,
    pub quality: DownloadQuality,
    pub state: ItemState,
    /// It comes from "Scarica tutto": the batch's final notice summarises it, not one per track.
    pub batched: bool,
}

impl Item {
    /// The track is queued or in progress: not failed.
    pub fn is_busy(&self) -> bool {
        !matches!(self.state, ItemState::Failed(_))
    }

    /// The progress in percentage points, if the file's length is known.
    pub fn percent(&self) -> Option<u8> {
        match self.state {
            ItemState::Downloading { received, total: Some(total) } => Some(percent_of(received, total)),
            _ => None,
        }
    }
}

fn percent_of(received: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    (u128::from(received.min(total)) * 100 / u128::from(total)) as u8
}

/// A "Scarica tutto" in progress: how many tracks it had, how many arrived and how many not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Batch {
    pub total: usize,
    pub done: usize,
    pub failed: usize,
}

/// Why a download command did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// This track cannot be downloaded: the licence does not allow it.
    NotDownloadable,
    /// None of the tracks can be downloaded.
    NoneDownloadable,
    /// The tracks are already queued or in progress.
    AlreadyQueued,
    /// The queue is full ([`MAX_QUEUED`]).
    QueueFull,
    /// The track is already on the device and its file is in order: to do it again, remove it first.
    AlreadySaved,
}

/// Something the frontend must make known or do.
#[derive(Debug, Clone, PartialEq)]
pub enum DownloadsEvent {
    /// A track requested on its own has been saved.
    Saved(Downloaded),
    /// A track requested on its own was not saved.
    Failed {
        id: Uuid,
        error: AppError,
    },
    /// The last track of "Scarica tutto" has finished; `last_error` is the last error, if any
    /// failed.
    BatchFinished {
        done: usize,
        failed: usize,
        last_error: Option<AppError>,
    },
    Refused(Refusal),
    /// A new release to offer. `notify` says to warn even outside the app (on the phone):
    /// only from the periodic check and once per release.
    UpdateOffered {
        tag: String,
        notify: bool,
    },
    /// The manual check found nothing new.
    UpToDate,
    /// The manual check failed.
    UpdateCheckFailed(AppError),
    /// The manual check did not start: the backend cannot be reached.
    Unreachable,
    /// The update package has been downloaded.
    UpdateSaved(PathBuf),
    /// The update package did not arrive.
    UpdateFailed(AppError),
    /// A track has been removed from the device. `kept_file`: the file no longer weighed what the index
    /// remembered (it was changed outside the app) and was not deleted.
    Removed {
        title: String,
        kept_file: bool,
    },
    /// The file of a track removed from the index could not be deleted.
    RemoveFailed {
        title: String,
        error: AppError,
    },
    /// The thorough check of the files has finished: how many tracks, and how many are not in order.
    Verified {
        checked: usize,
        lost: usize,
    },
    /// The downloads folder did not open.
    FolderNotOpened,
    /// The update package's folder did not open.
    UpdateFolderNotOpened,
}

/// A [`DownloadsEvent`] with its sequence number, which grows with each event.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadsRaised {
    pub seq: u64,
    pub event: DownloadsEvent,
}

/// What the frontends see of the domain.
#[derive(Debug, Clone, Default)]
pub struct DownloadsView {
    /// The download quality chosen in the settings.
    pub quality: DownloadQuality,
    /// The queue: waiting, in progress and failed, in the order they were requested.
    pub items: Shared<Item>,
    /// The "Scarica tutto" in progress, if there is one.
    pub batch: Option<Batch>,
    pub update: UpdateView,
    /// The tracks on the device, the most recent at the top. It has no limit: it is the user's collection,
    /// not a cache.
    pub saved: Shared<SavedTrack>,
    /// The ids of the tracks on the device, to ask "is it downloaded?" row by row without scanning
    /// the index.
    pub saved_ids: Arc<HashSet<Uuid>>,
    /// The tracks whose file is no longer in order (missing or changed): they stay in the index, and are
    /// listened to from the network.
    pub lost: Shared<Lost>,
    /// Bytes taken up by the index's files.
    pub used_bytes: u64,
    /// A thorough check of the files is in progress.
    pub verifying: bool,
    /// The folder where the downloads arrive.
    pub folder: PathBuf,
    /// The latest events, from the oldest: a frontend shows those with a new `seq`, in order. If it
    /// forgets at most [`EVENTS`] between one read and the next, that is, never.
    pub events: Arc<VecDeque<DownloadsRaised>>,
}

impl DownloadsView {
    /// The track in the queue, in whatever state.
    pub fn item(&self, id: Uuid) -> Option<&Item> {
        self.items.iter().find(|item| item.track.id == id)
    }

    /// The track is queued or in progress (not failed).
    pub fn is_busy(&self, id: Uuid) -> bool {
        self.item(id).is_some_and(Item::is_busy)
    }

    /// How many "Scarica tutto" tracks have finished and how many there were in all.
    pub fn batch_progress(&self) -> Option<(usize, usize)> {
        self.batch.map(|batch| (batch.done + batch.failed, batch.total))
    }

    /// How many tracks are queued or in progress.
    pub fn busy_count(&self) -> usize {
        self.items.iter().filter(|item| item.is_busy()).count()
    }

    /// The track is in the index of tracks on the device (even if its file is not in order).
    pub fn is_saved(&self, id: Uuid) -> bool {
        self.saved_ids.contains(&id)
    }

    /// The track's index entry.
    pub fn saved_track(&self, id: Uuid) -> Option<&SavedTrack> {
        self.is_saved(id).then(|| self.saved.iter().find(|saved| saved.track.id == id)).flatten()
    }

    /// Why the track's file is not in order, if it is not.
    pub fn lost_reason(&self, id: Uuid) -> Option<LostReason> {
        self.lost.iter().find(|lost| lost.id == id).map(|lost| lost.reason)
    }

    /// The track plays without network: it is on the device and its file is in order (as far as is known).
    pub fn is_offline_ready(&self, id: Uuid) -> bool {
        self.is_saved(id) && self.lost_reason(id).is_none()
    }
}

/// The folders where the domain writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dirs {
    /// The downloaded tracks.
    pub(crate) downloads: PathBuf,
    /// The update packages.
    pub(crate) updates: PathBuf,
}

impl Default for Dirs {
    /// Folders under the temporary one: a test reducer never writes (the jobs run only
    /// in the host, which sets them itself).
    fn default() -> Self {
        let base = std::env::temp_dir();
        Self { downloads: base.join("psytify-downloads"), updates: base.join("psytify-updates") }
    }
}

/// The domain's timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {
    /// Every [`PROGRESS_EVERY`], while a download is in progress: the progress is read.
    Progress,
    /// Every [`UPDATE_POLL`]: it checks whether there is a new release.
    UpdatePoll,
}

/// The domain's async jobs (network, files), which the host runs on the runtime.
#[derive(Debug)]
pub(crate) enum Job {
    /// Downloads a track; `ticket` tells it apart from an earlier download of the same track.
    Track {
        id: Uuid,
        ticket: u64,
        api: Result<ApiClient, AppError>,
        track: Arc<Track>,
        dir: PathBuf,
        quality: DownloadQuality,
        progress: Arc<Progress>,
        cancel: Cancel,
    },
    /// Removes from disk a file nobody wants any more (a download finished while it was being cancelled).
    Discard(PathBuf),
    /// Reads the latest release.
    CheckUpdate { api: Result<ApiClient, AppError>, kind: UpdateCheck, build: Option<&'static str> },
    /// Downloads the update package.
    DownloadUpdate { api: Result<ApiClient, AppError>, offer: Offer, dir: PathBuf, progress: Arc<Progress> },
    /// Checks the index's files (`deep`: the content too).
    Verify { entries: Vec<(Uuid, LocalFile, String)>, deep: bool },
    /// Removes from disk the file of a track removed from the index, if it still weighs `size`.
    Remove { path: PathBuf, size: u64, title: String },
    /// Opens the downloads folder in the file manager.
    OpenFolder(PathBuf),
    /// Opens the update package's folder in the file manager.
    RevealUpdate(PathBuf),
}

/// The outcome of a [`Job`], which goes back to the domain.
#[derive(Debug)]
pub(crate) enum Done {
    Track {
        id: Uuid,
        ticket: u64,
        result: Result<Downloaded, AppError>,
    },
    Discarded,
    UpdateChecked {
        kind: UpdateCheck,
        result: Result<Option<Offer>, AppError>,
    },
    UpdateDownloaded(Result<PathBuf, AppError>),
    Verified {
        deep: bool,
        checked: usize,
        lost: Vec<Lost>,
    },
    /// `Ok(true)`: the file stayed where it was because it is no longer the downloaded one.
    Removed {
        title: String,
        result: Result<bool, AppError>,
    },
    FolderOpened(bool),
    UpdateRevealed(bool),
}

/// A download the core is running: the number that tells it apart, and how to read its progress
/// and stop it.
#[derive(Debug)]
struct Slot {
    ticket: u64,
    progress: Arc<Progress>,
    cancel: Cancel,
}

/// The domain's state: the published view and what stays inside the core.
#[derive(Debug)]
pub(crate) struct Downloads {
    pub(crate) view: Arc<DownloadsView>,
    api: Result<ApiClient, AppError>,
    server: Option<String>,
    dirs: Dirs,
    /// This copy's release: without it, a local build does not check by itself.
    build: Option<&'static str>,
    /// The downloads in progress.
    slots: HashMap<Uuid, Slot>,
    /// The update package's progress, while it downloads: the same timer as the tracks reads it.
    /// brani.
    update_progress: Option<Arc<Progress>>,
    /// The number of the next download.
    tickets: u64,
    /// The number of the last event.
    events: u64,
    /// The progress timer is armed.
    ticking: bool,
    /// The last error of the batch in progress, for the final notice.
    batch_error: Option<AppError>,
    /// The release already warned about outside the app.
    notified: Option<String>,
    /// The quality or the index has changed and the settings have not yet taken it
    /// ([`Downloads::take_persist`]).
    persist: bool,
    /// The files playback can play from: the index's tracks whose file is not marked
    /// lost. It is made again at each change of the index; whoever receives it compares the pointer.
    local: Arc<HashMap<Uuid, LocalFile>>,
}

impl Default for Downloads {
    fn default() -> Self {
        Self::new(&DownloadsFields::default(), None, Dirs::default())
    }
}

impl Downloads {
    /// The domain with the quality read from `config.json`, the backend chosen in the settings and the
    /// folders decided by the host.
    pub(crate) fn new(fields: &DownloadsFields, server: Option<String>, dirs: Dirs) -> Self {
        let view = DownloadsView {
            quality: fields.download_quality,
            saved: fields.downloaded.clone(),
            folder: dirs.downloads.clone(),
            ..DownloadsView::default()
        };
        let mut this = Self {
            view: Arc::new(view),
            api: ApiClient::new(&config::api_url(server.as_deref())),
            server,
            dirs,
            build: update::BUILD_TAG,
            slots: HashMap::new(),
            update_progress: None,
            tickets: 0,
            events: 0,
            ticking: false,
            batch_error: None,
            notified: None,
            persist: false,
            local: Arc::default(),
        };
        this.reindex();
        this
    }

    /// The folders decided by the host.
    pub(crate) fn set_dirs(&mut self, dirs: Dirs) {
        if self.view.folder != dirs.downloads {
            self.view_mut().folder.clone_from(&dirs.downloads);
        }
        self.dirs = dirs;
    }

    /// The release this copy says it is (for the tests).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn with_build(mut self, build: Option<&'static str>) -> Self {
        self.build = build;
        self
    }

    fn view_mut(&mut self) -> &mut DownloadsView {
        Arc::make_mut(&mut self.view)
    }

    fn run(effects: &mut Vec<Effect>, job: Job) {
        effects.push(Effect::Run(effect::Job::Downloads(job)));
    }

    fn raise(&mut self, event: DownloadsEvent) {
        self.events += 1;
        let raised = DownloadsRaised { seq: self.events, event };
        let events = Arc::make_mut(&mut self.view_mut().events);
        events.push_back(raised);
        while events.len() > EVENTS {
            events.pop_front();
        }
    }

    // ---------- Passing between domains ----------

    /// The fields the domain owns, for the settings.
    pub(crate) fn fields(&self) -> DownloadsFields {
        DownloadsFields { download_quality: self.view.quality, downloaded: self.view.saved.clone() }
    }

    /// The files to play from, for playback.
    pub(crate) fn local(&self) -> &Arc<HashMap<Uuid, LocalFile>> {
        &self.local
    }

    /// Playback could not open these files: they are marked lost (they stay in the index).
    pub(crate) fn files_lost(&mut self, found: Vec<(Uuid, LostReason)>) {
        let fresh: Vec<Lost> = found
            .into_iter()
            .filter(|(id, _)| self.view.is_saved(*id) && self.view.lost_reason(*id).is_none())
            .map(|(id, reason)| Lost { id, reason })
            .collect();
        if fresh.is_empty() {
            return;
        }
        self.view_mut().lost.make_mut().extend(fresh);
        self.reindex();
    }

    /// The index or the lost files have changed: the data that derives from them is made again (the ids, the space
    /// taken, the map for playback).
    fn reindex(&mut self) {
        let ids: HashSet<Uuid> = self.view.saved.iter().map(|saved| saved.track.id).collect();
        let used = self.view.saved.iter().map(|saved| saved.size).sum();
        let local: HashMap<Uuid, LocalFile> = self
            .view
            .saved
            .iter()
            .filter(|saved| self.view.lost_reason(saved.track.id).is_none())
            .map(|saved| (saved.track.id, saved.local()))
            .collect();
        let view = self.view_mut();
        view.saved_ids = Arc::new(ids);
        view.used_bytes = used;
        self.local = Arc::new(local);
    }

    /// The settings must write the domain's fields: it says so once per change.
    pub(crate) fn take_persist(&mut self) -> bool {
        std::mem::take(&mut self.persist)
    }

    /// The backend chosen in the settings has changed: the HTTP client is made again.
    pub(crate) fn set_server(&mut self, server: &Option<String>) {
        if self.server == *server {
            return;
        }
        self.server.clone_from(server);
        self.api = ApiClient::new(&config::api_url(server.as_deref()));
    }

    /// The core has just started: a copy with its release checks whether there is a new one, and then
    /// every six hours.
    pub(crate) fn start(&mut self, effects: &mut Vec<Effect>) {
        // A quick check of the downloaded files: one `stat` per track, outside the reducer.
        self.verify(false, effects);
        if self.build.is_some() {
            self.check_update(UpdateCheck::Startup, effects);
            effects.push(Effect::Timer { key: TimerKey::Downloads(Timer::UpdatePoll), after: UPDATE_POLL });
        }
    }

    /// The number of the download in progress for `id` (for the tests).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn ticket(&self, id: Uuid) -> Option<u64> {
        self.slots.get(&id).map(|slot| slot.ticket)
    }

    /// The shared progress of the download in progress for `id` (for the tests).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn progress_of(&self, id: Uuid) -> Option<Arc<Progress>> {
        self.slots.get(&id).map(|slot| Arc::clone(&slot.progress))
    }

    // ---------- Commands ----------

    pub(crate) fn apply(&mut self, command: DownloadsCmd, _now: Instant, effects: &mut Vec<Effect>) {
        match command {
            DownloadsCmd::Download(track) => {
                let quality = self.view.quality;
                self.download_one(*track, quality, effects);
            }
            DownloadsCmd::DownloadAs(track, quality) => self.download_one(*track, quality, effects),
            DownloadsCmd::DownloadAll(tracks) => self.download_all(&tracks, effects),
            DownloadsCmd::Cancel(id) => self.cancel(id, effects),
            DownloadsCmd::Retry(id) => self.retry(id, effects),
            DownloadsCmd::SetQuality(quality) => {
                if self.view.quality != quality {
                    self.view_mut().quality = quality;
                    self.persist = true;
                }
            }
            DownloadsCmd::Remove(id) => self.remove(id, effects),
            DownloadsCmd::Verify => self.verify(true, effects),
            DownloadsCmd::OpenFolder => Self::run(effects, Job::OpenFolder(self.dirs.downloads.clone())),
            DownloadsCmd::CheckUpdate(kind) => self.check_update(kind, effects),
            DownloadsCmd::DownloadUpdate => self.download_update(effects),
            DownloadsCmd::DismissUpdate => self.dismiss_update(),
            DownloadsCmd::RevealUpdate => self.reveal_update(effects),
        }
    }

    /// A track requested on its own: it starts at once, even if a batch is in progress. If it is
    /// already in the queue or in progress it does nothing; if it had failed, it starts again.
    fn download_one(&mut self, track: Track, quality: DownloadQuality, effects: &mut Vec<Effect>) {
        // The check is here: the phone's player shows the button also for the tracks that cannot be
        // downloaded, and says why.
        if !track.downloadable {
            self.raise(DownloadsEvent::Refused(Refusal::NotDownloadable));
            return;
        }
        if self.view.is_busy(track.id) {
            return;
        }
        if self.view.is_offline_ready(track.id) {
            self.raise(DownloadsEvent::Refused(Refusal::AlreadySaved));
            return;
        }
        self.forget_failed(track.id);
        if self.view.items.len() >= MAX_QUEUED {
            self.raise(DownloadsEvent::Refused(Refusal::QueueFull));
            return;
        }
        let id = track.id;
        self.view_mut().items.make_mut().push(Item {
            track: Arc::new(track),
            quality,
            state: ItemState::Queued,
            batched: false,
        });
        self.launch(id, effects);
    }

    /// "Scarica tutto": the tracks that can be downloaded and are not already queued enter at the end.
    fn download_all(&mut self, tracks: &[Track], effects: &mut Vec<Effect>) {
        if !tracks.iter().any(|track| track.downloadable) {
            self.raise(DownloadsEvent::Refused(Refusal::NoneDownloadable));
            return;
        }
        let quality = self.view.quality;
        let mut added = 0;
        let mut already_saved = 0;
        let mut full = false;
        for track in tracks.iter().filter(|track| track.downloadable) {
            if self.view.is_busy(track.id) {
                continue;
            }
            if self.view.is_offline_ready(track.id) {
                already_saved += 1;
                continue;
            }
            self.forget_failed(track.id);
            if self.view.items.len() >= MAX_QUEUED {
                full = true;
                break;
            }
            self.view_mut().items.make_mut().push(Item {
                track: Arc::new(track.clone()),
                quality,
                state: ItemState::Queued,
                batched: true,
            });
            added += 1;
        }
        if added > 0 {
            let batch = self.view_mut().batch.get_or_insert_with(Batch::default);
            batch.total += added;
        }
        if full {
            self.raise(DownloadsEvent::Refused(Refusal::QueueFull));
        } else if added == 0 && self.view.batch.is_none() {
            let refusal = if already_saved > 0 { Refusal::AlreadySaved } else { Refusal::AlreadyQueued };
            self.raise(DownloadsEvent::Refused(refusal));
        }
        self.pump(effects);
    }

    /// Removes from the list a failed download of the same track, to put it back in the queue.
    fn forget_failed(&mut self, id: Uuid) {
        if self.view.item(id).is_some_and(|item| !item.is_busy()) {
            self.view_mut().items.make_mut().retain(|item| item.track.id != id);
        }
    }

    /// Starts the next tracks of the batch, up to [`BATCH_PARALLEL`] together; when all are finished, a single
    /// event summarises how it went.
    fn pump(&mut self, effects: &mut Vec<Effect>) {
        let mut active = self.slots.keys().filter(|id| self.is_batched(**id)).count();
        while active < BATCH_PARALLEL {
            let next = self
                .view
                .items
                .iter()
                .find(|item| item.batched && item.state == ItemState::Queued)
                .map(|item| item.track.id);
            let Some(id) = next else { break };
            self.launch(id, effects);
            active += 1;
        }
        let batch_busy = self.view.items.iter().any(|item| item.batched && item.is_busy());
        if let Some(batch) = self.view.batch.filter(|_| !batch_busy) {
            let last_error = self.batch_error.take();
            self.view_mut().batch = None;
            // A batch cancelled entirely has nothing to summarise.
            if batch.total > 0 {
                self.raise(DownloadsEvent::BatchFinished {
                    done: batch.done,
                    failed: batch.failed,
                    last_error,
                });
            }
        }
    }

    fn is_batched(&self, id: Uuid) -> bool {
        self.view.item(id).is_some_and(|item| item.batched)
    }

    /// Starts the job of the track `id`, which is already in the queue as waiting.
    fn launch(&mut self, id: Uuid, effects: &mut Vec<Effect>) {
        let Some(item) = self.view.item(id) else { return };
        let (track, quality) = (Arc::clone(&item.track), item.quality);
        self.tickets += 1;
        let slot = Slot { ticket: self.tickets, progress: Arc::default(), cancel: Cancel::default() };
        let job = Job::Track {
            id,
            ticket: slot.ticket,
            api: self.api.clone(),
            track,
            dir: self.dirs.downloads.clone(),
            quality,
            progress: Arc::clone(&slot.progress),
            cancel: slot.cancel.clone(),
        };
        self.slots.insert(id, slot);
        self.set_state(id, ItemState::Preparing);
        Self::run(effects, job);
        self.start_ticking(effects);
    }

    /// Arms the progress timer, if it is not already.
    fn start_ticking(&mut self, effects: &mut Vec<Effect>) {
        if !self.ticking {
            self.ticking = true;
            effects.push(Effect::Timer { key: TimerKey::Downloads(Timer::Progress), after: PROGRESS_EVERY });
        }
    }

    /// Something is downloading: a track or the update package.
    fn downloading(&self) -> bool {
        !self.slots.is_empty() || self.update_progress.is_some()
    }

    fn set_state(&mut self, id: Uuid, state: ItemState) {
        if let Some(item) = self.view_mut().items.make_mut().iter_mut().find(|item| item.track.id == id) {
            item.state = state;
        }
    }

    fn cancel(&mut self, id: Uuid, effects: &mut Vec<Effect>) {
        let Some(item) = self.view.item(id) else { return };
        let batched = item.batched && item.is_busy();
        if let Some(slot) = self.slots.remove(&id) {
            slot.cancel.cancel();
        }
        self.view_mut().items.make_mut().retain(|item| item.track.id != id);
        if batched && let Some(batch) = self.view_mut().batch.as_mut() {
            batch.total = batch.total.saturating_sub(1);
        }
        self.stop_ticking_if_idle(effects);
        self.pump(effects);
    }

    fn retry(&mut self, id: Uuid, effects: &mut Vec<Effect>) {
        let Some(item) = self.view.item(id).filter(|item| !item.is_busy()) else { return };
        let (track, quality) = (Arc::clone(&item.track), item.quality);
        self.view_mut().items.make_mut().retain(|item| item.track.id != id);
        self.view_mut().items.make_mut().push(Item {
            track,
            quality,
            state: ItemState::Queued,
            batched: false,
        });
        self.launch(id, effects);
    }

    fn stop_ticking_if_idle(&mut self, effects: &mut Vec<Effect>) {
        if !self.downloading() && self.ticking {
            self.ticking = false;
            effects.push(Effect::CancelTimer(TimerKey::Downloads(Timer::Progress)));
        }
    }

    // ---------- Timer ----------

    pub(crate) fn fire(&mut self, timer: Timer, _now: Instant, effects: &mut Vec<Effect>) {
        match timer {
            Timer::Progress => {
                self.ticking = false;
                self.read_progress();
                if self.downloading() {
                    self.ticking = true;
                    effects.push(Effect::Timer {
                        key: TimerKey::Downloads(Timer::Progress),
                        after: PROGRESS_EVERY,
                    });
                }
            }
            Timer::UpdatePoll => {
                self.check_update(UpdateCheck::Poll, effects);
                effects
                    .push(Effect::Timer { key: TimerKey::Downloads(Timer::UpdatePoll), after: UPDATE_POLL });
            }
        }
    }

    /// Reads the progress of the downloads in progress and changes the view only where a percentage point
    /// (or 256 KiB, without the length) has passed.
    fn read_progress(&mut self) {
        let changes: Vec<(Uuid, ItemState)> = self
            .slots
            .iter()
            .filter_map(|(id, slot)| {
                let item = self.view.item(*id)?;
                let next = if slot.progress.started() {
                    ItemState::Downloading {
                        received: slot.progress.received(),
                        total: slot.progress.total(),
                    }
                } else {
                    ItemState::Preparing
                };
                moved(&item.state, &next).then_some((*id, next))
            })
            .collect();
        for (id, state) in changes {
            self.set_state(id, state);
        }
        // The update package: like a track, one publish per percentage point.
        if let (Some(progress), Some(UpdateState::Downloading { tag, received, total })) =
            (&self.update_progress, &self.view.update.state)
            && progress.started()
        {
            let (now, length) = (progress.received(), progress.total());
            if bytes_moved((*received, *total), (now, length)) {
                let tag = tag.clone();
                self.view_mut().update.state =
                    Some(UpdateState::Downloading { tag, received: now, total: length });
            }
        }
    }

    // ---------- Outcomes ----------

    pub(crate) fn done(&mut self, done: Done, _now: Instant, effects: &mut Vec<Effect>) {
        match done {
            Done::Track { id, ticket, result } => self.track_done(id, ticket, result, effects),
            Done::Discarded => {}
            Done::UpdateChecked { kind, result } => self.update_checked(kind, result),
            Done::UpdateDownloaded(result) => self.update_downloaded(result, effects),
            Done::Verified { deep, checked, lost } => self.verified(deep, checked, lost),
            Done::Removed { title, result } => match result {
                Ok(kept_file) => self.raise(DownloadsEvent::Removed { title, kept_file }),
                Err(error) => self.raise(DownloadsEvent::RemoveFailed { title, error }),
            },
            Done::FolderOpened(opened) => {
                if !opened {
                    self.raise(DownloadsEvent::FolderNotOpened);
                }
            }
            Done::UpdateRevealed(opened) => {
                if !opened {
                    self.raise(DownloadsEvent::UpdateFolderNotOpened);
                }
            }
        }
    }

    fn track_done(
        &mut self,
        id: Uuid,
        ticket: u64,
        result: Result<Downloaded, AppError>,
        effects: &mut Vec<Effect>,
    ) {
        // The outcome of a download cancelled (or redone from scratch): its file, if it arrived, nobody
        // wants it.
        if self.slots.get(&id).is_none_or(|slot| slot.ticket != ticket) {
            if let Ok(saved) = result {
                Self::run(effects, Job::Discard(saved.path));
            }
            return;
        }
        self.slots.remove(&id);
        let batched = self.is_batched(id);
        match result {
            Ok(saved) => {
                let track = self.view.item(id).map(|item| Arc::clone(&item.track));
                self.view_mut().items.make_mut().retain(|item| item.track.id != id);
                if let Some(track) = track {
                    self.add_saved(&track, &saved);
                }
                if batched {
                    if let Some(batch) = self.view_mut().batch.as_mut() {
                        batch.done += 1;
                    }
                } else {
                    self.raise(DownloadsEvent::Saved(saved));
                }
            }
            Err(error) => {
                self.set_state(id, ItemState::Failed(error.clone()));
                self.trim_failed();
                if batched {
                    if let Some(batch) = self.view_mut().batch.as_mut() {
                        batch.failed += 1;
                    }
                    self.batch_error = Some(error);
                } else {
                    self.raise(DownloadsEvent::Failed { id, error });
                }
            }
        }
        self.stop_ticking_if_idle(effects);
        self.pump(effects);
    }

    /// Keeps at most [`MAX_FAILED`] failed downloads: beyond that, the oldest goes out.
    fn trim_failed(&mut self) {
        let failed = self.view.items.iter().filter(|item| !item.is_busy()).count();
        if failed <= MAX_FAILED {
            return;
        }
        let mut excess = failed - MAX_FAILED;
        self.view_mut().items.make_mut().retain(|item| {
            if excess > 0 && !item.is_busy() {
                excess -= 1;
                return false;
            }
            true
        });
    }

    // ---------- Index of tracks on the device ----------

    /// A downloaded track enters the head of the index, in place of an old entry of the same track
    /// (a lost file downloaded again).
    fn add_saved(&mut self, track: &Track, saved: &Downloaded) {
        let entry = SavedTrack {
            track: track.clone(),
            path: saved.path.clone(),
            format: saved.format.clone(),
            quality: saved.delivered,
            size: saved.size,
            sha256: saved.sha256.clone(),
        };
        let view = self.view_mut();
        let list = view.saved.make_mut();
        list.retain(|old| old.track.id != track.id);
        list.insert(0, entry);
        if view.lost.iter().any(|lost| lost.id == track.id) {
            view.lost.make_mut().retain(|lost| lost.id != track.id);
        }
        self.persist = true;
        self.reindex();
    }

    /// Removes a track from the device: the entry disappears at once, the file is removed by a job.
    fn remove(&mut self, id: Uuid, effects: &mut Vec<Effect>) {
        let Some(saved) = self.view.saved_track(id).cloned() else { return };
        let view = self.view_mut();
        view.saved.make_mut().retain(|entry| entry.track.id != id);
        if view.lost.iter().any(|lost| lost.id == id) {
            view.lost.make_mut().retain(|lost| lost.id != id);
        }
        self.persist = true;
        self.reindex();
        Self::run(
            effects,
            Job::Remove { path: saved.path, size: saved.size, title: saved.track.title.clone() },
        );
    }

    /// Checks the index's files outside the reducer. `deep` reads the content too: the user asks for it;
    /// at startup the quick check is enough.
    fn verify(&mut self, deep: bool, effects: &mut Vec<Effect>) {
        if self.view.verifying || self.view.saved.is_empty() {
            return;
        }
        let entries = self
            .view
            .saved
            .iter()
            .map(|saved| (saved.track.id, saved.local(), saved.sha256.clone()))
            .collect();
        if deep {
            self.view_mut().verifying = true;
        }
        Self::run(effects, Job::Verify { entries, deep });
    }

    /// The check has come back: the files not in order are marked (and those back in order are freed).
    fn verified(&mut self, deep: bool, checked: usize, mut lost: Vec<Lost>) {
        // An entry removed while it was being checked no longer matters.
        lost.retain(|entry| self.view.is_saved(entry.id));
        // A file marked lost after the last round (playback did not open it) stays so, if the
        // quick check does not see it: there is no proof it is back in order without reading it.
        if !deep {
            for known in self.view.lost.iter() {
                if !lost.iter().any(|entry| entry.id == known.id) {
                    lost.push(*known);
                }
            }
        }
        let count = lost.len();
        if deep {
            self.view_mut().verifying = false;
        }
        if self.view.lost.as_slice() != lost.as_slice() {
            self.view_mut().lost = lost.into();
            self.reindex();
        }
        if deep {
            self.raise(DownloadsEvent::Verified { checked, lost: count });
        }
    }

    // ---------- Updates ----------

    /// Asks the backend for the latest release. A local build (without a tag) does not check by
    /// itself, because it would not know what to compare; on request yes, and it offers the latest release.
    fn check_update(&mut self, kind: UpdateCheck, effects: &mut Vec<Effect>) {
        if self.build.is_none() && kind != UpdateCheck::Manual {
            return;
        }
        if self.api.is_err() {
            if kind == UpdateCheck::Manual {
                self.raise(DownloadsEvent::Unreachable);
            }
            return;
        }
        self.set_status(UpdateStatus::Checking);
        Self::run(
            effects,
            Job::CheckUpdate { api: self.api.clone(), kind, build: Some(self.build.unwrap_or("")) },
        );
    }

    fn set_status(&mut self, status: UpdateStatus) {
        if self.view.update.status != status {
            self.view_mut().update.status = status;
        }
    }

    /// The check has come back: a new offer appears in the banner and in a notice; without an offer it
    /// stays as it was, and whoever asked by hand is told how things stand.
    fn update_checked(&mut self, kind: UpdateCheck, result: Result<Option<Offer>, AppError>) {
        // A download in progress or already done is not touched.
        let busy = self.view.update.busy();
        match result {
            Ok(Some(offer)) => {
                self.set_status(UpdateStatus::Idle);
                if busy {
                    return;
                }
                let fresh = !matches!(&self.view.update.state, Some(UpdateState::Offered(known)) if known.tag == offer.tag);
                let tag = offer.tag.clone();
                let update = &mut self.view_mut().update;
                update.state = Some(UpdateState::Offered(offer));
                if fresh {
                    update.dismissed = None;
                    // The notice outside the app is for whoever does not look at the app: only from the timer, when
                    // the app has stayed open (or in the background) for hours, and once per release.
                    let notify = kind == UpdateCheck::Poll && self.notified.as_deref() != Some(tag.as_str());
                    if notify {
                        self.notified = Some(tag.clone());
                    }
                    self.raise(DownloadsEvent::UpdateOffered { tag, notify });
                }
            }
            Ok(None) => {
                self.set_status(UpdateStatus::Current);
                if !busy {
                    self.view_mut().update.state = None;
                }
                if kind == UpdateCheck::Manual {
                    self.raise(DownloadsEvent::UpToDate);
                }
            }
            Err(error) => {
                self.set_status(UpdateStatus::Failed);
                if kind == UpdateCheck::Manual {
                    self.raise(DownloadsEvent::UpdateCheckFailed(error));
                }
            }
        }
    }

    /// Downloads the offer's package, if there is one.
    fn download_update(&mut self, effects: &mut Vec<Effect>) {
        let (Some(UpdateState::Offered(offer)), Ok(_)) = (&self.view.update.state, &self.api) else {
            return;
        };
        let offer = offer.clone();
        let progress = Arc::new(Progress::default());
        self.update_progress = Some(Arc::clone(&progress));
        self.view_mut().update.state =
            Some(UpdateState::Downloading { tag: offer.tag.clone(), received: 0, total: None });
        Self::run(
            effects,
            Job::DownloadUpdate { api: self.api.clone(), offer, dir: self.dirs.updates.clone(), progress },
        );
        self.start_ticking(effects);
    }

    fn update_downloaded(&mut self, result: Result<PathBuf, AppError>, effects: &mut Vec<Effect>) {
        self.update_progress = None;
        self.stop_ticking_if_idle(effects);
        match result {
            Ok(path) => {
                self.view_mut().update.state = Some(UpdateState::Downloaded(path.clone()));
                self.raise(DownloadsEvent::UpdateSaved(path));
            }
            Err(error) => {
                self.view_mut().update.state = None;
                self.raise(DownloadsEvent::UpdateFailed(error));
            }
        }
    }

    /// The folder of the ready package, in the file manager: that of the file (the updates folder,
    /// if the path has none). Without a ready package it does nothing.
    fn reveal_update(&mut self, effects: &mut Vec<Effect>) {
        let Some(UpdateState::Downloaded(path)) = &self.view.update.state else { return };
        let dir = path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .map_or_else(|| self.dirs.updates.clone(), std::path::Path::to_path_buf);
        Self::run(effects, Job::RevealUpdate(dir));
    }

    /// The banner can be closed: it comes back by itself with the next release.
    fn dismiss_update(&mut self) {
        match &self.view.update.state {
            Some(UpdateState::Offered(offer)) => {
                let tag = offer.tag.clone();
                if self.view.update.dismissed.as_deref() != Some(tag.as_str()) {
                    self.view_mut().update.dismissed = Some(tag);
                }
            }
            Some(UpdateState::Downloaded(_)) => self.view_mut().update.state = None,
            _ => {}
        }
    }
}

/// The progress has changed enough to publish it: the state is another one, or a percentage point
/// (without a length, 256 KiB) has passed.
fn moved(old: &ItemState, new: &ItemState) -> bool {
    match (old, new) {
        (
            ItemState::Downloading { received: before, total: a },
            ItemState::Downloading { received: now, total: b },
        ) => bytes_moved((*before, *a), (*now, *b)),
        (a, b) => a != b,
    }
}

/// Between two readings of bytes received and the length: the length is another one, or a
/// percentage point has passed (without a length, 256 KiB).
fn bytes_moved(before: (u64, Option<u64>), now: (u64, Option<u64>)) -> bool {
    match (before, now) {
        ((before, Some(a)), (now, Some(b))) => a != b || percent_of(before, a) != percent_of(now, b),
        ((before, None), (now, None)) => before / UNKNOWN_STEP != now / UNKNOWN_STEP,
        _ => true,
    }
}

impl Job {
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Track { id, ticket, api, track, dir, quality, progress, cancel } => {
                let download = async {
                    match api {
                        Ok(api) => api::download_track(api, (*track).clone(), dir, quality, progress).await,
                        Err(error) => Err(error),
                    }
                };
                let result = tokio::select! {
                    () = cancel.cancelled() => Err(AppError::Cancelled),
                    result = download => result,
                };
                // Cancelled an instant after the end: the file is already in its place, and it is not wanted.
                let result = match result {
                    Ok(saved) if cancel.is_cancelled() => {
                        let _ = tokio::fs::remove_file(&saved.path).await;
                        Err(AppError::Cancelled)
                    }
                    other => other,
                };
                Done::Track { id, ticket, result }
            }
            Self::Discard(path) => {
                let _ = tokio::fs::remove_file(&path).await;
                Done::Discarded
            }
            Self::CheckUpdate { api, kind, build } => {
                let result = match api {
                    Ok(api) => api.latest_release().await.map(|release| update::offer(release, build)),
                    Err(error) => Err(error),
                };
                Done::UpdateChecked { kind, result }
            }
            Self::DownloadUpdate { api, offer, dir, progress } => {
                let result = match api {
                    Ok(api) => api.download_release(&offer.asset, &dir, &progress).await,
                    Err(error) => Err(error),
                };
                Done::UpdateDownloaded(result)
            }
            Self::Verify { entries, deep } => {
                let checked = entries.len();
                // Reads the disk (and, thoroughly, every file): outside the runtime.
                let lost = tokio::task::spawn_blocking(move || offline::verify(&entries, deep))
                    .await
                    .unwrap_or_default();
                Done::Verified { deep, checked, lost }
            }
            Self::Remove { path, size, title } => {
                let result = remove_saved(&path, size).await;
                Done::Removed { title, result }
            }
            Self::OpenFolder(dir) => Done::FolderOpened(open_folder(dir).await),
            // The folder is already there (the package is in it): if it has gone meanwhile, it is
            // recreated empty and opened all the same, as the downloads one is.
            Self::RevealUpdate(dir) => Done::UpdateRevealed(open_folder(dir).await),
        }
    }

    /// If the job is lost, the outcome is a network error: a download does not stay "in progress" for
    /// ever, and the update check does not stay "in progress".
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Track { id, ticket, .. } => {
                Done::Track { id: *id, ticket: *ticket, result: Err(AppError::Network) }
            }
            Self::Discard(_) => Done::Discarded,
            Self::CheckUpdate { kind, .. } => {
                Done::UpdateChecked { kind: *kind, result: Err(AppError::Network) }
            }
            Self::DownloadUpdate { .. } => Done::UpdateDownloaded(Err(AppError::Network)),
            // A lost check marks nothing: no proof, no lost file.
            Self::Verify { entries, deep } => {
                Done::Verified { deep: *deep, checked: entries.len(), lost: Vec::new() }
            }
            Self::Remove { title, .. } => {
                Done::Removed { title: title.clone(), result: Err(AppError::Io("job lost".into())) }
            }
            Self::OpenFolder(_) => Done::FolderOpened(false),
            Self::RevealUpdate(_) => Done::UpdateRevealed(false),
        }
    }
}

/// Opens `dir` in the system file manager (`platform::open_path`, never on Android), creating it if it
/// is missing. Says whether it opened.
async fn open_folder(dir: PathBuf) -> bool {
    match tokio::fs::create_dir_all(&dir).await {
        Ok(()) => {
            tokio::task::spawn_blocking(move || crate::platform::open_path(&dir)).await.unwrap_or(false)
        }
        Err(_) => false,
    }
}

/// Removes from disk the file of a track removed from the index, but only if it is still the downloaded one (same
/// weight): a file changed outside the app is not ours and stays. `Ok(true)`: it stayed; a file that
/// is no longer there is already as wanted.
async fn remove_saved(path: &std::path::Path, size: u64) -> Result<bool, AppError> {
    let io = |err: std::io::Error| AppError::Io(err.to_string());
    match tokio::fs::metadata(path).await {
        Ok(meta) if meta.is_file() && meta.len() == size => {
            tokio::fs::remove_file(path).await.map_err(io)?;
            Ok(false)
        }
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(io(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures;
    use crate::api::update::Asset;
    use crate::command::Command;
    use crate::snapshot::DomainVersions;
    use crate::state::{CoreState, Input};

    fn track(n: u128) -> Track {
        Track { downloadable: true, ..fixtures::track(n) }
    }

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn saved(n: u128) -> Downloaded {
        Downloaded {
            path: PathBuf::from(format!("{n}.flac")),
            size: 4096,
            sha256: format!("hash{n}"),
            format: "flac".into(),
            delivered: None,
            asked: DownloadQuality::Lossless,
        }
    }

    fn offer(tag: &str) -> Offer {
        Offer { tag: tag.into(), asset: Asset { name: format!("psytify-{tag}.bin"), size: 1 } }
    }

    /// The domain alone, with a fake clock and no release (a local build).
    /// BACKLOG 38: the update package has its progress, read by the same 250 ms timer as the tracks (armed only
    /// while downloading) and published once per percentage point; at the end of the run the timer turns off and
    /// another expiry publishes nothing.
    #[test]
    fn the_update_package_progress_is_read_on_the_timer_and_published_per_whole_percent() {
        let mut state = CoreState::default();
        let mut now = Instant::now();
        let send = |state: &mut CoreState, input: Input, now: Instant| state.apply(input, now);
        let done = |done| Input::Done(effect::Done::Downloads(done));
        send(
            &mut state,
            done(Done::UpdateChecked { kind: UpdateCheck::Manual, result: Ok(Some(offer("v2"))) }),
            now,
        );
        let offered = state.snapshot();
        assert!(
            timers(&send(&mut state, Input::Timer(TimerKey::Downloads(Timer::Progress)), now)).is_empty()
        );
        assert!(state.publish(&offered).is_none(), "nessun download: il timer non c'è e non pubblica");

        let effects = send(&mut state, Input::Command(Command::Downloads(DownloadsCmd::DownloadUpdate)), now);
        assert_eq!(timers(&effects), [(Timer::Progress, Some(PROGRESS_EVERY))]);
        let progress = match jobs(effects).as_slice() {
            [Job::DownloadUpdate { progress, offer, .. }] if offer.tag == "v2" => Arc::clone(progress),
            other => panic!("one update job, got {other:?}"),
        };
        let started = state.publish(&offered).expect("the download appears");
        assert_eq!(
            started.downloads.update.state,
            Some(UpdateState::Downloading { tag: "v2".into(), received: 0, total: None })
        );

        // The server has not answered yet: the reading changes nothing.
        now += PROGRESS_EVERY;
        let effects = send(&mut state, Input::Timer(TimerKey::Downloads(Timer::Progress)), now);
        assert_eq!(timers(&effects), [(Timer::Progress, Some(PROGRESS_EVERY))], "si riarma mentre scarica");
        assert!(state.publish(&started).is_none());

        let mut last = started;
        let mut published = 0;
        for step in 0..=1000u64 {
            progress.set(step, Some(1000));
            now += PROGRESS_EVERY;
            send(&mut state, Input::Timer(TimerKey::Downloads(Timer::Progress)), now);
            if let Some(next) = state.publish(&last) {
                published += 1;
                last = next;
            }
        }
        assert_eq!(published, 101, "lo 0 % e poi un punto alla volta");
        assert_eq!(last.downloads.update.state.as_ref().and_then(UpdateState::percent), Some(100));

        let path = PathBuf::from("/dati/updates/psytify-v2.bin");
        let effects = send(&mut state, done(Done::UpdateDownloaded(Ok(path.clone()))), now);
        assert_eq!(timers(&effects), [(Timer::Progress, None)], "finito, il timer si spegne");
        let finished = state.publish(&last).expect("the package is ready");
        assert_eq!(finished.downloads.update.state, Some(UpdateState::Downloaded(path)));
        let effects = send(&mut state, Input::Timer(TimerKey::Downloads(Timer::Progress)), now);
        assert!(effects.is_empty());
        assert!(state.publish(&finished).is_none());
    }

    /// A track and the package download together: the timer stays until one of the two is in progress, and
    /// without the length the package advances every 256 KiB.
    #[test]
    fn the_progress_timer_waits_for_both_a_track_and_the_update_package() {
        let mut rig = Rig::with_build("v1");
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Startup, result: Ok(Some(offer("v2"))) });
        assert_eq!(
            timers(&rig.cmd(DownloadsCmd::Download(Box::new(track(1))))),
            [(Timer::Progress, Some(PROGRESS_EVERY))]
        );
        let effects = rig.cmd(DownloadsCmd::DownloadUpdate);
        assert!(timers(&effects).is_empty(), "già armato dal brano");
        let Some(progress) = rig.downloads.update_progress.clone() else { panic!("the update progress") };

        progress.set(UNKNOWN_STEP - 1, None);
        rig.fire(Timer::Progress);
        assert_eq!(
            rig.view().update.state,
            Some(UpdateState::Downloading { tag: "v2".into(), received: 0, total: None }),
            "meno di 256 KiB: niente da pubblicare"
        );
        progress.set(UNKNOWN_STEP + 10, None);
        let before = rig.view();
        rig.fire(Timer::Progress);
        assert!(!Arc::ptr_eq(&before, &rig.view()), "256 KiB passati: si pubblica");
        progress.set(UNKNOWN_STEP + 20, None);
        let before = rig.view();
        rig.fire(Timer::Progress);
        assert!(Arc::ptr_eq(&before, &rig.view()), "fra due passi niente");
        assert_eq!(rig.view().update.state.as_ref().and_then(UpdateState::percent), None);

        assert!(timers(&rig.finish(1, Ok(saved(1)))).is_empty(), "il pacchetto scarica ancora");
        assert_eq!(timers(&rig.fire(Timer::Progress)), [(Timer::Progress, Some(PROGRESS_EVERY))]);
        let effects = rig.done(Done::UpdateDownloaded(Err(AppError::Network)));
        assert_eq!(timers(&effects), [(Timer::Progress, None)]);
        assert!(rig.downloads.update_progress.is_none());
    }

    struct Rig {
        downloads: Downloads,
        now: Instant,
    }

    impl Rig {
        fn new() -> Self {
            Self { downloads: Downloads::default().with_build(None), now: Instant::now() }
        }

        fn with_build(build: &'static str) -> Self {
            Self { downloads: Downloads::default().with_build(Some(build)), now: Instant::now() }
        }

        fn cmd(&mut self, command: DownloadsCmd) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.downloads.apply(command, self.now, &mut effects);
            effects
        }

        fn done(&mut self, done: Done) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.downloads.done(done, self.now, &mut effects);
            effects
        }

        fn fire(&mut self, timer: Timer) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.downloads.fire(timer, self.now, &mut effects);
            effects
        }

        /// The download in progress of track `n` ends with `result`.
        fn finish(&mut self, n: u128, result: Result<Downloaded, AppError>) -> Vec<Effect> {
            let ticket = self.downloads.ticket(id(n)).expect("a running download");
            self.done(Done::Track { id: id(n), ticket, result })
        }

        fn view(&self) -> Arc<DownloadsView> {
            Arc::clone(&self.downloads.view)
        }

        /// The tracks queued or in progress, in order.
        fn busy(&self) -> Vec<Uuid> {
            let mut ids: Vec<Uuid> =
                self.view().items.iter().filter(|item| item.is_busy()).map(|item| item.track.id).collect();
            ids.sort();
            ids
        }

        /// The tracks whose job has started, in order.
        fn running(&self) -> Vec<Uuid> {
            let mut ids: Vec<Uuid> = self.downloads.slots.keys().copied().collect();
            ids.sort();
            ids
        }

        fn events(&self) -> Vec<DownloadsEvent> {
            self.view().events.iter().map(|raised| raised.event.clone()).collect()
        }
    }

    fn jobs(effects: Vec<Effect>) -> Vec<Job> {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Run(effect::Job::Downloads(job)) => Some(job),
                _ => None,
            })
            .collect()
    }

    fn timers(effects: &[Effect]) -> Vec<(Timer, Option<Duration>)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Timer { key: TimerKey::Downloads(timer), after } => Some((*timer, Some(*after))),
                Effect::CancelTimer(TimerKey::Downloads(timer)) => Some((*timer, None)),
                _ => None,
            })
            .collect()
    }

    // ---------- Queue and batches ----------

    /// `download_all_runs_two_at_a_time_and_reports_once` (it was in the client): "Scarica tutto" downloads
    /// two tracks at a time, the one that cannot be downloaded does not count, and a single event summarises.
    #[test]
    fn download_all_runs_two_at_a_time_and_reports_once() {
        let mut rig = Rig::new();
        let mut tracks: Vec<Track> = (1..=5).map(track).collect();
        tracks[4].downloadable = false;

        let effects = rig.cmd(DownloadsCmd::DownloadAll(Arc::from(tracks)));
        assert_eq!(jobs(effects).len(), 2, "due alla volta");
        assert_eq!(rig.view().batch_progress(), Some((0, 4)), "il brano che non si può scaricare non conta");
        assert_eq!(rig.running(), [id(1), id(2)]);
        assert_eq!(rig.busy(), [id(1), id(2), id(3), id(4)]);

        rig.finish(1, Ok(saved(1)));
        assert_eq!((rig.view().batch_progress(), rig.running()), (Some((1, 4)), vec![id(2), id(3)]));
        rig.finish(2, Err(AppError::Network));
        assert_eq!((rig.view().batch_progress(), rig.running()), (Some((2, 4)), vec![id(3), id(4)]));
        rig.finish(3, Ok(saved(3)));
        assert_eq!((rig.view().batch_progress(), rig.running()), (Some((3, 4)), vec![id(4)]));
        assert!(rig.events().is_empty(), "nessun avviso brano per brano");

        // A single event at the end, with the last error.
        let last = rig.finish(4, Ok(saved(4)));
        assert_eq!((rig.view().batch_progress(), rig.running()), (None, Vec::new()));
        assert_eq!(
            rig.events(),
            [DownloadsEvent::BatchFinished { done: 3, failed: 1, last_error: Some(AppError::Network) }]
        );
        assert_eq!(timers(&last), [(Timer::Progress, None)], "fermo, il timer dell'avanzamento si spegne");
        // The failed track stays in the list to retry, and does not count as in progress.
        assert!(matches!(rig.view().item(id(2)).map(|item| &item.state), Some(ItemState::Failed(_))));
        assert!(!rig.view().is_busy(id(2)));
    }

    #[test]
    fn a_single_download_starts_at_once_even_during_a_batch_and_is_reported_alone() {
        let mut rig = Rig::new();
        let tracks: Vec<Track> = (1..=3).map(track).collect();
        rig.cmd(DownloadsCmd::DownloadAll(Arc::from(tracks)));
        assert_eq!(rig.running(), [id(1), id(2)]);

        let effects = rig.cmd(DownloadsCmd::Download(Box::new(track(9))));
        assert_eq!(jobs(effects).len(), 1, "un brano chiesto a mano non aspetta il lotto");
        assert_eq!(rig.running(), [id(1), id(2), id(9)]);
        assert_eq!(rig.view().batch_progress(), Some((0, 3)), "e non entra nel conto del lotto");

        rig.finish(9, Ok(saved(9)));
        assert_eq!(rig.events(), [DownloadsEvent::Saved(saved(9))]);
        assert!(!rig.view().is_busy(id(9)));
        assert_eq!(rig.running(), [id(1), id(2)], "il lotto non ha preso il posto lasciato libero");
    }

    #[test]
    fn download_as_asks_for_that_quality_and_download_for_the_settings_one() {
        let mut rig = Rig::new();
        let ask = |effects: Vec<Effect>| match jobs(effects).as_slice() {
            [Job::Track { quality, .. }] => *quality,
            other => panic!("{other:?}"),
        };
        rig.cmd(DownloadsCmd::SetQuality(DownloadQuality::Low));
        assert_eq!(ask(rig.cmd(DownloadsCmd::Download(Box::new(track(1))))), DownloadQuality::Low);
        assert_eq!(
            ask(rig.cmd(DownloadsCmd::DownloadAs(Box::new(track(2)), DownloadQuality::Lossless))),
            DownloadQuality::Lossless
        );
    }

    #[test]
    fn a_track_that_cannot_be_downloaded_is_refused_and_a_busy_one_is_ignored() {
        let mut rig = Rig::new();
        let locked = fixtures::track(1);
        assert!(jobs(rig.cmd(DownloadsCmd::Download(Box::new(locked.clone())))).is_empty());
        assert!(jobs(rig.cmd(DownloadsCmd::DownloadAll(Arc::from(vec![locked])))).is_empty());
        assert_eq!(
            rig.events(),
            [
                DownloadsEvent::Refused(Refusal::NotDownloadable),
                DownloadsEvent::Refused(Refusal::NoneDownloadable)
            ]
        );
        assert!(rig.view().items.is_empty() && rig.view().batch.is_none());

        // Already in progress: neither a second job nor a notice.
        assert_eq!(jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(2))))).len(), 1);
        assert!(jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(2))))).is_empty());
        assert_eq!(rig.events().len(), 2);
        // "Scarica tutto" of tracks already in progress and with no batch: it says so.
        assert!(jobs(rig.cmd(DownloadsCmd::DownloadAll(Arc::from(vec![track(2)])))).is_empty());
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::Refused(Refusal::AlreadyQueued)));
        assert!(rig.view().batch.is_none());
    }

    /// A second "Scarica tutto" adds only the new tracks to the batch in progress.
    #[test]
    fn a_second_download_all_extends_the_running_batch() {
        let mut rig = Rig::new();
        rig.cmd(DownloadsCmd::DownloadAll(Arc::from((1..=3).map(track).collect::<Vec<_>>())));
        rig.cmd(DownloadsCmd::DownloadAll(Arc::from((2..=5).map(track).collect::<Vec<_>>())));
        assert_eq!(rig.view().batch_progress(), Some((0, 5)));
        assert_eq!(rig.view().busy_count(), 5);
        assert_eq!(rig.running(), [id(1), id(2)]);
    }

    #[test]
    fn the_queue_is_bounded() {
        let mut rig = Rig::new();
        let many: Vec<Track> = (1..=(MAX_QUEUED as u128 + 50)).map(track).collect();
        rig.cmd(DownloadsCmd::DownloadAll(Arc::from(many)));
        assert_eq!(rig.view().items.len(), MAX_QUEUED);
        assert_eq!(rig.view().batch_progress(), Some((0, MAX_QUEUED)));
        assert_eq!(rig.events(), [DownloadsEvent::Refused(Refusal::QueueFull)]);
        assert!(jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(9999))))).is_empty());
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::Refused(Refusal::QueueFull)));
    }

    // ---------- Cancel, errors, retry ----------

    #[test]
    fn a_cancelled_download_is_stopped_removed_and_its_late_file_discarded() {
        let mut rig = Rig::new();
        let effects = rig.cmd(DownloadsCmd::DownloadAll(Arc::from((1..=3).map(track).collect::<Vec<_>>())));
        let (cancel, ticket) = match jobs(effects).remove(0) {
            Job::Track { cancel, ticket, .. } => (cancel, ticket),
            other => panic!("{other:?}"),
        };
        assert!(!cancel.is_cancelled());

        let effects = rig.cmd(DownloadsCmd::Cancel(id(1)));
        assert!(cancel.is_cancelled(), "il lavoro lo vede");
        assert!(!rig.view().is_busy(id(1)) && rig.view().item(id(1)).is_none());
        assert_eq!(rig.view().batch_progress(), Some((0, 2)), "il brano tolto non conta più");
        assert_eq!(jobs(effects).len(), 1, "il posto libero va al brano successivo");
        assert_eq!(rig.running(), [id(2), id(3)]);

        // The job was already at the end: nobody wants its file.
        let late = rig.done(Done::Track { id: id(1), ticket, result: Ok(saved(1)) });
        assert!(matches!(jobs(late).as_slice(), [Job::Discard(path)] if path == &PathBuf::from("1.flac")));
        assert!(rig.events().is_empty());
        // An error outcome of the same job does nothing.
        assert!(rig.done(Done::Track { id: id(1), ticket, result: Err(AppError::Cancelled) }).is_empty());
    }

    #[test]
    fn a_stale_result_does_not_touch_a_download_started_again() {
        let mut rig = Rig::new();
        let first = match jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(1))))).remove(0) {
            Job::Track { ticket, .. } => ticket,
            other => panic!("{other:?}"),
        };
        rig.cmd(DownloadsCmd::Cancel(id(1)));
        rig.cmd(DownloadsCmd::Download(Box::new(track(1))));
        assert!(rig.view().is_busy(id(1)));
        let stale = rig.done(Done::Track { id: id(1), ticket: first, result: Err(AppError::Network) });
        assert!(stale.is_empty());
        assert!(rig.view().is_busy(id(1)), "il nuovo download continua");
        assert!(rig.events().is_empty());
    }

    #[test]
    fn cancelling_every_item_of_a_batch_ends_it_silently() {
        let mut rig = Rig::new();
        rig.cmd(DownloadsCmd::DownloadAll(Arc::from((1..=2).map(track).collect::<Vec<_>>())));
        rig.cmd(DownloadsCmd::Cancel(id(1)));
        let effects = rig.cmd(DownloadsCmd::Cancel(id(2)));
        assert!(rig.view().batch.is_none() && rig.view().items.is_empty());
        assert!(rig.events().is_empty(), "niente da riassumere");
        assert_eq!(timers(&effects), [(Timer::Progress, None)]);
    }

    #[test]
    fn a_failed_single_download_stays_to_be_retried() {
        let mut rig = Rig::new();
        rig.cmd(DownloadsCmd::Download(Box::new(track(1))));
        rig.finish(1, Err(AppError::Timeout));
        assert_eq!(rig.events(), [DownloadsEvent::Failed { id: id(1), error: AppError::Timeout }]);
        assert!(matches!(
            rig.view().item(id(1)).map(|item| &item.state),
            Some(ItemState::Failed(AppError::Timeout))
        ));

        let effects = rig.cmd(DownloadsCmd::Retry(id(1)));
        assert_eq!(jobs(effects).len(), 1);
        assert!(rig.view().is_busy(id(1)));
        rig.finish(1, Ok(saved(1)));
        assert!(rig.view().items.is_empty());
        // Retrying what has not failed does nothing.
        assert!(rig.cmd(DownloadsCmd::Retry(id(1))).is_empty());
    }

    #[test]
    fn a_failed_item_can_be_dismissed_and_failed_items_are_bounded() {
        let mut rig = Rig::new();
        for n in 1..=(MAX_FAILED as u128 + 5) {
            rig.cmd(DownloadsCmd::Download(Box::new(track(n))));
            rig.finish(n, Err(AppError::Network));
        }
        assert_eq!(rig.view().items.len(), MAX_FAILED);
        assert!(rig.view().item(id(1)).is_none(), "esce il più vecchio");
        assert!(rig.view().item(id(MAX_FAILED as u128 + 5)).is_some());
        let last = id(MAX_FAILED as u128 + 5);
        rig.cmd(DownloadsCmd::Cancel(last));
        assert!(rig.view().item(last).is_none());
    }

    #[test]
    fn a_lost_job_fails_the_download_instead_of_leaving_it_running() {
        let mut rig = Rig::new();
        let job = jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(1))))).remove(0);
        rig.done(job.lost());
        assert!(!rig.view().is_busy(id(1)));
        assert!(matches!(rig.events().as_slice(), [DownloadsEvent::Failed { error: AppError::Network, .. }]));
    }

    // ---------- Progress ----------

    #[test]
    fn progress_is_read_on_the_timer_and_published_per_whole_percent() {
        let mut rig = Rig::new();
        let effects = rig.cmd(DownloadsCmd::Download(Box::new(track(1))));
        assert_eq!(timers(&effects), [(Timer::Progress, Some(PROGRESS_EVERY))]);
        let progress = rig.downloads.progress_of(id(1)).unwrap();
        let state = |rig: &Rig| rig.view().item(id(1)).unwrap().state.clone();
        assert_eq!(state(&rig), ItemState::Preparing);

        // The server is still preparing: nothing changes, but the timer stays armed.
        let first = rig.view();
        assert_eq!(timers(&rig.fire(Timer::Progress)), [(Timer::Progress, Some(PROGRESS_EVERY))]);
        assert!(Arc::ptr_eq(&first, &rig.downloads.view), "nessuna pubblicazione");

        progress.set(0, Some(1000));
        rig.fire(Timer::Progress);
        assert_eq!(state(&rig), ItemState::Downloading { received: 0, total: Some(1000) });
        let at_zero = rig.view();
        // Between two percentage points the view does not move.
        progress.set(9, Some(1000));
        rig.fire(Timer::Progress);
        assert!(Arc::ptr_eq(&at_zero, &rig.downloads.view), "0,9%: sempre lo 0%");
        progress.set(10, Some(1000));
        rig.fire(Timer::Progress);
        assert!(!Arc::ptr_eq(&at_zero, &rig.downloads.view));
        assert_eq!(rig.view().item(id(1)).unwrap().percent(), Some(1));
    }

    #[test]
    fn without_a_length_progress_moves_every_256_kib() {
        let mut rig = Rig::new();
        rig.cmd(DownloadsCmd::Download(Box::new(track(1))));
        let progress = rig.downloads.progress_of(id(1)).unwrap();
        progress.set(1000, None);
        rig.fire(Timer::Progress);
        let view = rig.view();
        progress.set(200_000, None);
        rig.fire(Timer::Progress);
        assert!(Arc::ptr_eq(&view, &rig.downloads.view));
        progress.set(300_000, None);
        rig.fire(Timer::Progress);
        assert!(!Arc::ptr_eq(&view, &rig.downloads.view));
        assert_eq!(rig.view().item(id(1)).unwrap().percent(), None);
    }

    /// Performance: idle, the core does not publish; a download publishes only when something is seen
    /// (its start, a percentage point, the end), and at the end of the run the timer turns off.
    #[test]
    fn idle_publishes_nothing_progress_is_throttled_and_done_publishes_once() {
        let mut state = CoreState::default();
        let mut now = Instant::now();
        let start = state.snapshot();
        assert!(state.publish(&start).is_none(), "idle");
        let send = |state: &mut CoreState, input: Input, now: Instant| state.apply(input, now);
        let command = |cmd| Input::Command(Command::Downloads(cmd));

        // Commands that change nothing (a track without a licence is an event: this is not).
        send(&mut state, command(DownloadsCmd::Cancel(id(404))), now);
        send(&mut state, command(DownloadsCmd::Retry(id(404))), now);
        send(&mut state, command(DownloadsCmd::DismissUpdate), now);
        send(&mut state, command(DownloadsCmd::SetQuality(DownloadQuality::default())), now);
        assert!(state.publish(&start).is_none(), "commands that change nothing");

        // The track starts: one publish, only of the downloads domain.
        let effects = send(&mut state, command(DownloadsCmd::Download(Box::new(track(1)))), now);
        assert!(effects.iter().any(|e| matches!(e, Effect::Timer { key: TimerKey::Downloads(_), .. })));
        let started = state.publish(&start).expect("the download appears");
        assert_eq!(started.versions, DomainVersions { downloads: 1, ..DomainVersions::default() });
        assert!(state.publish(&started).is_none(), "published once");

        let progress = state.downloads.progress_of(id(1)).unwrap();
        let mut last = started;
        let mut published = 0;
        // 100 readings of the timer while the bytes arrive from 0 to 100%, in steps of a point: each reading
        // publishes once; with steps of a tenth of a point, one reading in ten.
        for step in 1..=1000u64 {
            progress.set(step, Some(1000));
            now += PROGRESS_EVERY;
            let effects = send(&mut state, Input::Timer(TimerKey::Downloads(Timer::Progress)), now);
            assert!(
                effects.iter().any(|e| matches!(e, Effect::Timer { key: TimerKey::Downloads(_), .. })),
                "il timer resta armato finché si scarica"
            );
            if let Some(next) = state.publish(&last) {
                published += 1;
                assert_eq!(next.versions.downloads, last.versions.downloads + 1);
                last = next;
            }
        }
        assert_eq!(published, 101, "lo 0 % (inizio dei byte) e poi un punto alla volta, mai di più");

        // The end: a single publish (the track leaves the queue, the event is born), and the timer turns off.
        let ticket = state.downloads.ticket(id(1)).unwrap();
        let effects = send(
            &mut state,
            Input::Done(effect::Done::Downloads(Done::Track { id: id(1), ticket, result: Ok(saved(1)) })),
            now,
        );
        assert_eq!(timers(&effects), [(Timer::Progress, None)]);
        let finished = state.publish(&last).expect("the download is done");
        assert!(finished.downloads.items.is_empty());
        assert!(matches!(finished.downloads.events.back().map(|e| &e.event), Some(DownloadsEvent::Saved(_))));
        assert!(state.publish(&finished).is_none(), "done publishes once");
        // Idle again: the timer that expired by mistake publishes nothing and does not re-arm.
        let effects = send(&mut state, Input::Timer(TimerKey::Downloads(Timer::Progress)), now);
        assert!(effects.is_empty());
        assert!(state.publish(&finished).is_none());
    }

    #[test]
    fn the_quality_is_a_setting_the_domain_hands_to_the_settings() {
        let mut state = CoreState::new(crate::config::Config::default(), None, 1);
        let command = |cmd| Input::Command(Command::Downloads(cmd));
        state.apply(command(DownloadsCmd::SetQuality(DownloadQuality::Low)), Instant::now());
        assert_eq!(state.settings.view.config.download_quality, DownloadQuality::Low);
        assert_eq!(state.downloads.view.quality, DownloadQuality::Low);
        // The frontend that still sends `Store` does not change the quality the domain owns.
        let stale =
            crate::config::CoreConfig { download_quality: DownloadQuality::High, ..Default::default() };
        state.apply(
            Input::Command(Command::Settings(crate::command::SettingsCmd::Store(Box::new(stale)))),
            Instant::now(),
        );
        assert_eq!(state.settings.view.config.download_quality, DownloadQuality::Low);
    }

    // ---------- Updates ----------

    #[test]
    fn a_local_build_checks_nothing_by_itself_but_a_release_checks_at_start_and_every_six_hours() {
        let mut local = Rig::new();
        let mut effects = Vec::new();
        local.downloads.start(&mut effects);
        assert!(effects.is_empty(), "senza release non si controlla da soli");
        assert!(local.cmd(DownloadsCmd::CheckUpdate(UpdateCheck::Poll)).is_empty());
        assert_eq!(
            jobs(local.cmd(DownloadsCmd::CheckUpdate(UpdateCheck::Manual))).len(),
            1,
            "a richiesta sì"
        );

        let mut rig = Rig::with_build("v1");
        let mut effects = Vec::new();
        rig.downloads.start(&mut effects);
        assert_eq!(timers(&effects), [(Timer::UpdatePoll, Some(UPDATE_POLL))]);
        match jobs(effects).as_slice() {
            [Job::CheckUpdate { kind: UpdateCheck::Startup, build: Some("v1"), .. }] => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(rig.view().update.status, UpdateStatus::Checking);

        let tick = rig.fire(Timer::UpdatePoll);
        assert_eq!(timers(&tick), [(Timer::UpdatePoll, Some(UPDATE_POLL))], "si riarma");
        assert!(matches!(jobs(tick).as_slice(), [Job::CheckUpdate { kind: UpdateCheck::Poll, .. }]));
    }

    #[test]
    fn a_new_release_is_offered_announced_once_and_can_be_dismissed() {
        let mut rig = Rig::with_build("v1");
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Startup, result: Ok(Some(offer("v2"))) });
        assert_eq!(rig.events(), [DownloadsEvent::UpdateOffered { tag: "v2".into(), notify: false }]);
        assert!(rig.view().update.banner().is_some());

        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(Some(offer("v2"))) });
        assert_eq!(rig.events().len(), 1, "la stessa release non si annuncia di nuovo");

        rig.cmd(DownloadsCmd::DismissUpdate);
        assert!(rig.view().update.banner().is_none());
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(Some(offer("v3"))) });
        assert!(rig.view().update.banner().is_some(), "una release più nuova riporta il banner");
        assert_eq!(
            rig.events().last(),
            Some(&DownloadsEvent::UpdateOffered { tag: "v3".into(), notify: true }),
            "dal timer, una release nuova avvisa anche fuori dall'app"
        );

        // A release already announced outside the app is not announced twice.
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(None) });
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(Some(offer("v3"))) });
        assert_eq!(
            rig.events().last(),
            Some(&DownloadsEvent::UpdateOffered { tag: "v3".into(), notify: false })
        );
    }

    #[test]
    fn a_manual_check_always_answers_and_a_poll_stays_quiet() {
        let mut rig = Rig::with_build("v1");
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(None) });
        assert!(rig.events().is_empty());
        assert_eq!(rig.view().update.status, UpdateStatus::Current);

        rig.done(Done::UpdateChecked { kind: UpdateCheck::Manual, result: Ok(None) });
        assert_eq!(rig.events(), [DownloadsEvent::UpToDate]);

        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Err(AppError::Network) });
        assert_eq!(rig.events().len(), 1);
        assert_eq!(rig.view().update.status, UpdateStatus::Failed);
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Manual, result: Err(AppError::Network) });
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::UpdateCheckFailed(AppError::Network)));
    }

    #[test]
    fn a_download_in_progress_is_not_replaced_by_a_check() {
        let mut rig = Rig::with_build("v1");
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(Some(offer("v2"))) });
        let effects = rig.cmd(DownloadsCmd::DownloadUpdate);
        assert!(matches!(jobs(effects).as_slice(), [Job::DownloadUpdate { offer, .. }] if offer.tag == "v2"));
        assert_eq!(
            rig.view().update.state,
            Some(UpdateState::Downloading { tag: "v2".into(), received: 0, total: None })
        );

        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(Some(offer("v3"))) });
        assert!(matches!(rig.view().update.state, Some(UpdateState::Downloading { .. })));
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Manual, result: Ok(None) });
        assert!(matches!(rig.view().update.state, Some(UpdateState::Downloading { .. })));
    }

    #[test]
    fn the_update_package_lands_or_fails() {
        let mut rig = Rig::with_build("v1");
        assert!(
            rig.cmd(DownloadsCmd::DownloadUpdate).is_empty(),
            "senza un'offerta non c'è niente da scaricare"
        );
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Startup, result: Ok(Some(offer("v2"))) });
        rig.cmd(DownloadsCmd::DownloadUpdate);
        rig.done(Done::UpdateDownloaded(Ok(PathBuf::from("/dati/updates/psytify-v2.bin"))));
        assert_eq!(
            rig.view().update.state,
            Some(UpdateState::Downloaded(PathBuf::from("/dati/updates/psytify-v2.bin")))
        );
        assert_eq!(
            rig.events().last(),
            Some(&DownloadsEvent::UpdateSaved(PathBuf::from("/dati/updates/psytify-v2.bin")))
        );
        // The ready file is closed with the banner.
        rig.cmd(DownloadsCmd::DismissUpdate);
        assert!(rig.view().update.state.is_none());

        rig.done(Done::UpdateChecked { kind: UpdateCheck::Startup, result: Ok(Some(offer("v3"))) });
        rig.cmd(DownloadsCmd::DownloadUpdate);
        rig.done(Done::UpdateDownloaded(Err(AppError::Network)));
        assert!(rig.view().update.state.is_none());
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::UpdateFailed(AppError::Network)));
    }

    #[test]
    fn events_are_numbered_and_bounded() {
        let mut rig = Rig::new();
        let locked = fixtures::track(1);
        for _ in 0..(EVENTS + 10) {
            rig.cmd(DownloadsCmd::Download(Box::new(locked.clone())));
        }
        let view = rig.view();
        assert_eq!(view.events.len(), EVENTS);
        assert_eq!(view.events.back().map(|e| e.seq), Some(EVENTS as u64 + 10));
        assert_eq!(view.events.front().map(|e| e.seq), Some(11));
    }

    // ---------- Tracks on the device ----------

    /// An index with the tracks `ns`, as `config.json` reads it at startup.
    fn rig_with_saved(ns: &[u128]) -> Rig {
        let entries: Vec<SavedTrack> = ns
            .iter()
            .map(|n| {
                let downloaded = saved(*n);
                SavedTrack {
                    track: track(*n),
                    path: downloaded.path,
                    format: downloaded.format,
                    quality: None,
                    size: downloaded.size,
                    sha256: downloaded.sha256,
                }
            })
            .collect();
        let fields = DownloadsFields { downloaded: entries.into(), ..DownloadsFields::default() };
        Rig {
            downloads: Downloads::new(&fields, None, Dirs::default()).with_build(None),
            now: Instant::now(),
        }
    }

    #[test]
    fn a_downloaded_track_enters_the_index_newest_first_and_is_handed_to_the_settings() {
        let mut rig = Rig::new();
        rig.cmd(DownloadsCmd::Download(Box::new(track(1))));
        rig.finish(1, Ok(Downloaded { delivered: Some(DownloadQuality::Lossless), ..saved(1) }));
        rig.cmd(DownloadsCmd::Download(Box::new(track(2))));
        rig.finish(2, Ok(saved(2)));

        let view = rig.view();
        let order: Vec<Uuid> = view.saved.iter().map(|entry| entry.track.id).collect();
        assert_eq!(order, [id(2), id(1)], "il più recente in testa");
        assert!(view.is_saved(id(1)) && view.is_offline_ready(id(2)) && !view.is_saved(id(3)));
        let first = view.saved_track(id(1)).expect("its entry");
        assert_eq!(
            (first.size, first.sha256.as_str(), first.format.as_str(), first.quality),
            (4096, "hash1", "flac", Some(DownloadQuality::Lossless))
        );
        assert_eq!(view.used_bytes, 8192);
        assert_eq!(rig.downloads.local().len(), 2);
        assert_eq!(rig.downloads.local()[&id(1)], LocalFile { path: PathBuf::from("1.flac"), size: 4096 });

        // The settings receive the index, once; and a new core rereads it the same.
        assert!(rig.downloads.take_persist());
        assert!(!rig.downloads.take_persist());
        let restarted = Downloads::new(&rig.downloads.fields(), None, Dirs::default());
        assert_eq!(restarted.view.saved, view.saved);
        assert_eq!(restarted.view.used_bytes, 8192);
        assert_eq!(restarted.local().len(), 2);
    }

    #[test]
    fn a_saved_track_is_refused_unless_its_file_was_lost_and_a_new_copy_replaces_the_old_entry() {
        let mut rig = rig_with_saved(&[1, 2]);
        assert!(jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(1))))).is_empty());
        assert_eq!(rig.events(), [DownloadsEvent::Refused(Refusal::AlreadySaved)]);

        // "Scarica tutto" skips the tracks already on the device, and if nothing remains it says so.
        let batch: Arc<[Track]> = Arc::from(vec![track(1), track(3)]);
        assert_eq!(jobs(rig.cmd(DownloadsCmd::DownloadAll(batch))).len(), 1);
        assert_eq!(rig.busy(), [id(3)]);
        rig.cmd(DownloadsCmd::Cancel(id(3)));
        let only_saved: Arc<[Track]> = Arc::from(vec![track(1), track(2)]);
        assert!(jobs(rig.cmd(DownloadsCmd::DownloadAll(only_saved))).is_empty());
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::Refused(Refusal::AlreadySaved)));

        // A lost file is downloaded again: the old entry gives way to the new one.
        rig.downloads.files_lost(vec![(id(1), LostReason::Missing)]);
        assert_eq!(jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(1))))).len(), 1);
        rig.finish(1, Ok(Downloaded { path: PathBuf::from("1 (1).flac"), ..saved(1) }));
        let view = rig.view();
        assert_eq!(view.saved.iter().filter(|entry| entry.track.id == id(1)).count(), 1);
        assert_eq!(view.saved[0].path, PathBuf::from("1 (1).flac"));
        assert!(view.lost.is_empty() && view.is_offline_ready(id(1)));
        assert_eq!(rig.downloads.local()[&id(1)].path, PathBuf::from("1 (1).flac"));
    }

    #[test]
    fn the_startup_check_marks_a_missing_file_but_never_deletes_its_entry() {
        let mut rig = rig_with_saved(&[1, 2]);
        let mut effects = Vec::new();
        rig.downloads.start(&mut effects);
        let verify = jobs(effects);
        assert!(
            matches!(verify.as_slice(), [Job::Verify { entries, deep: false }] if entries.len() == 2),
            "{verify:?}"
        );
        assert!(!rig.view().verifying, "il controllo veloce non mostra un'attesa");
        assert_eq!(rig.downloads.local().len(), 2, "finché non si sa, i file si suonano");

        let before = rig.view();
        rig.done(Done::Verified { deep: false, checked: 2, lost: Vec::new() });
        assert!(Arc::ptr_eq(&before, &rig.view()), "tutto a posto: niente cambia, niente si pubblica");

        rig.done(Done::Verified {
            deep: false,
            checked: 2,
            lost: vec![Lost { id: id(2), reason: LostReason::Missing }],
        });
        let view = rig.view();
        assert_eq!(view.saved.len(), 2, "la voce resta");
        assert_eq!(view.lost_reason(id(2)), Some(LostReason::Missing));
        assert!(view.is_saved(id(2)) && !view.is_offline_ready(id(2)) && view.is_offline_ready(id(1)));
        assert!(!rig.downloads.local().contains_key(&id(2)), "un file perso non si suona dal disco");
        assert!(rig.events().is_empty());
        assert!(!rig.downloads.take_persist(), "un file perso non si scrive: si ricontrolla a ogni avvio");

        // Without tracks on the device there is nothing to check.
        let mut empty = Rig::new();
        let mut effects = Vec::new();
        empty.downloads.start(&mut effects);
        assert!(jobs(effects).is_empty());
    }

    #[test]
    fn a_deep_check_reports_once_and_frees_the_files_that_came_back() {
        let mut rig = rig_with_saved(&[1, 2]);
        let first = jobs(rig.cmd(DownloadsCmd::Verify));
        assert!(matches!(first.as_slice(), [Job::Verify { deep: true, .. }]));
        assert!(rig.view().verifying);
        assert!(jobs(rig.cmd(DownloadsCmd::Verify)).is_empty(), "uno alla volta");

        rig.done(Done::Verified {
            deep: true,
            checked: 2,
            lost: vec![Lost { id: id(1), reason: LostReason::Changed }],
        });
        assert!(!rig.view().verifying);
        assert_eq!(rig.view().lost_reason(id(1)), Some(LostReason::Changed));
        assert_eq!(rig.events(), [DownloadsEvent::Verified { checked: 2, lost: 1 }]);

        // The file is back: the thorough check frees it, the quick one does not.
        rig.done(Done::Verified { deep: false, checked: 2, lost: Vec::new() });
        assert_eq!(rig.view().lost_reason(id(1)), Some(LostReason::Changed));
        rig.cmd(DownloadsCmd::Verify);
        rig.done(Done::Verified { deep: true, checked: 2, lost: Vec::new() });
        assert!(rig.view().lost.is_empty());
        assert_eq!(rig.downloads.local().len(), 2);
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::Verified { checked: 2, lost: 0 }));
    }

    #[test]
    fn playback_marks_a_file_lost_once_and_only_for_tracks_in_the_index() {
        let mut rig = rig_with_saved(&[1]);
        rig.downloads.files_lost(vec![(id(9), LostReason::Missing)]);
        assert!(rig.view().lost.is_empty(), "un brano che non è nell'indice non c'entra");
        rig.downloads.files_lost(vec![(id(1), LostReason::Missing)]);
        let marked = rig.view();
        rig.downloads.files_lost(vec![(id(1), LostReason::Changed)]);
        assert!(Arc::ptr_eq(&marked, &rig.view()), "già segnato: niente cambia");
        assert_eq!(marked.lost_reason(id(1)), Some(LostReason::Missing));
        assert!(rig.downloads.local().is_empty());
    }

    #[test]
    fn removing_a_track_forgets_it_at_once_and_a_job_deletes_the_file() {
        let mut rig = rig_with_saved(&[1, 2]);
        rig.downloads.files_lost(vec![(id(1), LostReason::Missing)]);
        assert!(jobs(rig.cmd(DownloadsCmd::Remove(id(404)))).is_empty());
        assert!(!rig.downloads.take_persist());

        let removal = jobs(rig.cmd(DownloadsCmd::Remove(id(1))));
        assert!(matches!(
            removal.as_slice(),
            [Job::Remove { path, size: 4096, title }] if path == &PathBuf::from("1.flac") && title == "Brano 1"
        ));
        let view = rig.view();
        assert!(!view.is_saved(id(1)) && view.lost.is_empty(), "la voce e il suo segno spariscono subito");
        assert_eq!((view.saved.len(), view.used_bytes), (1, 4096));
        assert!(!rig.downloads.local().contains_key(&id(1)));
        assert!(rig.downloads.take_persist());

        rig.done(Done::Removed { title: "Brano 1".into(), result: Ok(false) });
        rig.done(Done::Removed { title: "Brano 2".into(), result: Ok(true) });
        rig.done(Done::Removed { title: "Brano 3".into(), result: Err(AppError::Io("negato".into())) });
        assert_eq!(
            rig.events(),
            [
                DownloadsEvent::Removed { title: "Brano 1".into(), kept_file: false },
                DownloadsEvent::Removed { title: "Brano 2".into(), kept_file: true },
                DownloadsEvent::RemoveFailed {
                    title: "Brano 3".into(),
                    error: AppError::Io("negato".into())
                },
            ]
        );
    }

    /// BACKLOG 39: "Apri cartella" of the ready package opens its folder with a job; without a
    /// ready package (offered, downloading, closed) it does nothing, and a failed opening is an event
    /// of its own, not that of the downloads folder.
    #[test]
    fn the_downloaded_update_folder_opens_only_when_a_package_is_ready() {
        let mut rig = Rig::with_build("v1");
        assert!(rig.cmd(DownloadsCmd::RevealUpdate).is_empty(), "niente da aprire");
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Startup, result: Ok(Some(offer("v2"))) });
        assert!(rig.cmd(DownloadsCmd::RevealUpdate).is_empty(), "offerto, non ancora scaricato");
        rig.cmd(DownloadsCmd::DownloadUpdate);
        assert!(jobs(rig.cmd(DownloadsCmd::RevealUpdate)).is_empty(), "in download");

        rig.done(Done::UpdateDownloaded(Ok(PathBuf::from("/dati/updates/psytify-v2.bin"))));
        let open = jobs(rig.cmd(DownloadsCmd::RevealUpdate));
        assert!(
            matches!(open.as_slice(), [Job::RevealUpdate(dir)] if dir.as_path() == std::path::Path::new("/dati/updates")),
            "{open:?}"
        );
        let before = rig.view();
        rig.done(Done::UpdateRevealed(true));
        assert!(Arc::ptr_eq(&before, &rig.view()), "aperta: niente da dire");
        rig.done(Done::UpdateRevealed(false));
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::UpdateFolderNotOpened));
        assert!(matches!(rig.view().update.state, Some(UpdateState::Downloaded(_))), "il pacchetto resta");

        // A path with no folder: that of the updates.
        rig.downloads.view_mut().update.state =
            Some(UpdateState::Downloaded(PathBuf::from("psytify-v2.bin")));
        let updates = rig.downloads.dirs.updates.clone();
        assert!(
            matches!(jobs(rig.cmd(DownloadsCmd::RevealUpdate)).as_slice(), [Job::RevealUpdate(dir)] if *dir == updates)
        );

        rig.cmd(DownloadsCmd::DismissUpdate);
        assert!(rig.cmd(DownloadsCmd::RevealUpdate).is_empty(), "chiuso, il pacchetto non è più nel banner");
    }

    #[test]
    fn opening_the_folder_is_a_job_and_a_failure_is_an_event() {
        let mut rig = Rig::new();
        let open = jobs(rig.cmd(DownloadsCmd::OpenFolder));
        assert!(matches!(open.as_slice(), [Job::OpenFolder(dir)] if *dir == rig.view().folder));
        rig.done(Done::FolderOpened(true));
        assert!(rig.events().is_empty());
        rig.done(Done::FolderOpened(false));
        assert_eq!(rig.events(), [DownloadsEvent::FolderNotOpened]);
    }

    /// A new folder for the test, with a file of `size` bytes.
    fn file_of(size: usize) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("psytify-remove-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("brano.flac");
        std::fs::write(&path, vec![1u8; size]).unwrap();
        (dir, path)
    }

    /// `BACKLOG` 3, the removal part: only the file the index describes is deleted.
    #[tokio::test]
    async fn remove_deletes_the_saved_file_but_never_one_that_changed() {
        let (dir, path) = file_of(100);
        assert_eq!(remove_saved(&path, 100).await, Ok(false));
        assert!(!path.exists(), "il file scaricato è stato cancellato");
        assert_eq!(remove_saved(&path, 100).await, Ok(false), "già sparito: com'era voluto");

        std::fs::write(&path, vec![2u8; 150]).unwrap();
        assert_eq!(remove_saved(&path, 100).await, Ok(true));
        assert_eq!(std::fs::read(&path).unwrap(), vec![2u8; 150], "un file cambiato fuori dall'app resta");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_jobs_check_remove_and_open_with_the_real_disk() {
        let (dir, path) = file_of(64);
        let entries =
            vec![(id(1), LocalFile { path: path.clone(), size: 64 }, offline::hash_file(&path).unwrap())];
        let Done::Verified { deep: true, checked: 1, lost } = Job::Verify { entries, deep: true }.run().await
        else {
            panic!("a verification")
        };
        assert!(lost.is_empty());
        let gone = Job::Remove { path: path.clone(), size: 64, title: "x".into() }.run().await;
        assert!(matches!(gone, Done::Removed { result: Ok(false), .. }) && !path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The whole core: the downloaded track leaves the queue, enters the index and the settings
    /// write it (`config.json`); whoever still sends `Store` does not delete it.
    #[test]
    fn the_index_is_written_to_the_settings_and_a_stale_store_does_not_erase_it() {
        let mut state = CoreState::new(crate::config::Config::default(), None, 1);
        let command = |cmd| Input::Command(Command::Downloads(cmd));
        let now = Instant::now();
        state.apply(command(DownloadsCmd::Download(Box::new(track(1)))), now);
        let ticket = state.downloads.ticket(id(1)).unwrap();
        state.apply(
            Input::Done(effect::Done::Downloads(Done::Track { id: id(1), ticket, result: Ok(saved(1)) })),
            now,
        );
        assert_eq!(state.settings.view.config.downloaded.len(), 1);
        assert_eq!(state.settings.view.config.downloaded[0].sha256, "hash1");

        let stale = crate::config::CoreConfig::default();
        state.apply(
            Input::Command(Command::Settings(crate::command::SettingsCmd::Store(Box::new(stale)))),
            now,
        );
        assert_eq!(state.settings.view.config.downloaded.len(), 1);
        assert!(state.downloads.view.is_saved(id(1)));
    }
}
