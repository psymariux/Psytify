//! Music video (step 4.9): the core half of the video. The GStreamer pipeline, the frames and the
//! widget stay in the frontend; here sits everything else.
//!
//! **Which music video.** A frontend says it shows the video ([`VideoCmd::Show`], like
//! `PlaybackCmd::Lyrics` for the text): until it says so the core asks the server for nothing. With
//! the video shown and a track that has one (`has_video`) the core asks for the address
//! (`GET /v1/tracks/{id}/video-url`: the first time the server downloads the music video, at 360p at once and
//! at 720p in the background) and publishes it in [`VideoView`]. The frontend opens the address
//! ([`Clip::url`]); a new [`Clip::seq`] is another address to open.
//!
//! **From 360p to 720p.** While the clip shown is below [`FULL_HEIGHT`] the core re-arms a
//! timer ([`Timer::Upgrade`], [`UPGRADE_WAIT`], at most [`MAX_UPGRADES`] times) and asks for the address again:
//! if the server has a higher version it publishes it with a new `seq`, and the frontend opens it
//! alongside and replaces the other without stopping. The timer is armed only with a clip shown
//! and not yet complete: an idle core, a track without video or a hidden video arm nothing.
//!
//! **When it does not open.** The frontend says [`VideoCmd::Failed`] if the address does not open or the stream
//! drops: the core retries once with a new address (it may have expired) and then stops the
//! music video of that track (`ClipState::Failed`) until the track changes or [`VideoCmd::Retry`]
//! arrives. An upgrade that does not open stops nothing: the version that was playing stays, and the core
//! retries later.
//!
//! **In advance.** With [`VideoCmd::PrepareAhead`] the server prepares the music videos of the track that starts and
//! of the next [`AHEAD`] (`POST /v1/tracks/{id}/video/prepare`, once per track, with a
//! limited list of the tracks already asked for).
//!
//! **The alignment to the audio.** The sync decisions are pure functions ([`Alignment`]): the
//! frontend measures its player ([`Measure`]) and applies what comes back ([`Adjust`]); the position
//! to reach is given by the playback clock (`clock.at(now)`), the drift threshold
//! ([`DRIFT`]), the pause between two realignments ([`RESYNC_PAUSE`]) and the lead ([`MAX_LEAD`]).
//! No command and no publish for this: the frontend asks for it on its Tick, which is already there
//! while it plays.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::api::{ApiClient, SignedUrl};
use crate::auth::Secret;
use crate::config;
use crate::effect::{self, Effect, TimerKey};
use crate::error::AppError;
use crate::state::playback::PlaybackView;

/// Tracks of the queue, after the current one, whose music video the server prepares.
pub const AHEAD: usize = 2;
/// Height of the full version: below it, one waits for the server to prepare it.
pub const FULL_HEIGHT: u32 = 720;
/// How often it is asked whether the full version is ready, and how many times at most.
pub const UPGRADE_WAIT: Duration = Duration::from_secs(6);
pub const MAX_UPGRADES: u32 = 10;
/// Difference tolerated between video and audio before the video is realigned.
pub const DRIFT: Duration = Duration::from_millis(200);
/// After a realignment the video is given time to start again.
pub const RESYNC_PAUSE: Duration = Duration::from_secs(2);
/// Maximum lead with which a realignment is aimed (see [`Alignment`]).
pub const MAX_LEAD: Duration = Duration::from_millis(1500);
/// How long, on the thread that opens the video, the initial seek is waited for to reach its target.
pub const SETTLE: Duration = Duration::from_millis(600);
/// Tracks of which it is remembered that the preparation was asked (discarded: the least recent).
const PREPARED: usize = 128;

// ---------- Alignment to the audio: pure functions, of the frontend ----------

/// What the frontend measures on its player at each tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measure {
    /// Where the video is.
    pub position: Duration,
    /// The video's duration; zero if it does not know it yet.
    pub duration: Duration,
    /// The player is paused.
    pub paused: bool,
    /// The video has reached the end.
    pub ended: bool,
}

/// What the frontend does to its player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Adjust {
    /// `Some(true)`: pause; `Some(false)`: start again.
    pub pause: Option<bool>,
    /// Brings the video to this position.
    pub seek: Option<Duration>,
}

impl Adjust {
    /// There is nothing to do.
    pub fn is_none(&self) -> bool {
        self.pause.is_none() && self.seek.is_none()
    }
}

/// The realignment state of a player: the last realignment and the lead with which it is aimed.
/// The frontend keeps it together with its player (one per open music video); the rules are here.
#[derive(Debug, Clone, Copy)]
pub struct Alignment {
    /// Last realignment to the audio: it is not chased at each tick.
    synced_at: Instant,
    /// The seek itself costs time (the video stops to buffer while the audio goes on): one
    /// aims ahead by how far the video has fallen behind the previous times, or one chases forever.
    lead: Duration,
}

impl Alignment {
    /// A player just opened, now.
    pub fn new(now: Instant) -> Self {
        Self { synced_at: now, lead: Duration::ZERO }
    }

    /// The lead with which it is aimed now.
    pub fn lead(&self) -> Duration {
        self.lead
    }

    /// Playback has changed from `previous` to `view`: what the video must do to follow it.
    /// A track that starts or a jump within the track brings the video to the audio's position; a
    /// pause stops it; the resume starts it again from the audio's position. `None` if nothing
    /// that concerns it has changed.
    pub fn follow(&mut self, previous: &PlaybackView, view: &PlaybackView, now: Instant) -> Option<Adjust> {
        let playing = view.is_playing();
        let moved = view.started != previous.started || view.seeks != previous.seeks;
        if !moved && view.state == previous.state {
            return None;
        }
        // A jump, or the resume after a pause: the video goes where the audio is. A pause stops it and
        // that is all.
        let seek = (moved || playing).then(|| {
            self.synced_at = now;
            view.position(now)
        });
        Some(Adjust { pause: Some(!playing), seek })
    }

    /// At the tick, or with `force` just after a player opens: if the video has drifted from the audio
    /// (slow network, decoder late) it is brought back to where the audio is, at most every
    /// [`RESYNC_PAUSE`]. Pause and resume always follow playback. A video shorter
    /// than the audio stays on the last frame.
    pub fn align(&mut self, playback: &PlaybackView, video: Measure, force: bool, now: Instant) -> Adjust {
        let playing = playback.is_playing();
        if !force && (!playing || now.saturating_duration_since(self.synced_at) < RESYNC_PAUSE) {
            return Adjust::default();
        }
        let mut adjust = Adjust::default();
        if video.paused == playing {
            adjust.pause = Some(!playing);
        }
        let audio = playback.position(now);
        if (!video.duration.is_zero() && audio >= video.duration) || video.ended {
            return adjust;
        }
        if video.position.abs_diff(audio) > DRIFT {
            self.lead = if video.position < audio {
                (self.lead + (audio - video.position)).min(MAX_LEAD)
            } else {
                self.lead.saturating_sub(video.position - audio)
            };
            adjust.seek = Some(audio + self.lead);
            self.synced_at = now;
        }
        adjust
    }
}

// ---------- The domain ----------

/// The domain's commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoCmd {
    /// A frontend shows (or stops showing) the music video of the track playing: the "In riproduzione"
    /// panel on the video, the full-screen view. The core asks the server for it only while
    /// it is needed.
    Show(bool),
    /// The listener wants music videos: the server prepares those of the track that starts and of the next ones.
    /// When off nothing is prepared. The track that is already in the queue at startup is prepared only
    /// when a track starts to play.
    PrepareAhead(bool),
    /// The music video with that `seq` does not open or the stream has dropped. The first time the core asks for
    /// another address, the second stops that track's music video (a `seq` that is no longer the
    /// published one is ignored).
    Failed { seq: u64 },
    /// Removes the block of a music video that did not open ("Riprova", or whoever changes the choice of
    /// video): if it is still shown it is asked for again.
    Retry,
    /// Removes it and asks for it again, even if it was working: the frontend has changed the way
    /// it decodes it.
    Reopen,
}

/// A music video to open: the signed address, the height and the sequence number.
#[derive(Clone, PartialEq, Eq)]
pub struct Clip {
    /// Grows at each new address: the frontend opens the one it has not opened yet.
    pub seq: u64,
    /// The height of the version served (360 the first, 720 the complete one); `None` for a music video
    /// saved before the two versions, which is complete.
    pub height: Option<u32>,
    url: Secret,
}

impl Clip {
    /// The address to open. Signed and expiring: it does not go in logs (the `Debug` does not print it).
    pub fn url(&self) -> &str {
        self.url.expose()
    }

    /// This is the full version: there is nothing else to wait for.
    pub fn is_full(&self) -> bool {
        self.height.is_none_or(|height| height >= FULL_HEIGHT)
    }
}

impl std::fmt::Debug for Clip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clip").field("seq", &self.seq).field("height", &self.height).finish_non_exhaustive()
    }
}

/// Where the music video of [`VideoView::track`] is.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum ClipState {
    /// Nobody shows it, or the track has no music video.
    #[default]
    Idle,
    /// The server is preparing it (the first time it downloads it: it may take half a minute).
    Loading,
    /// The address to open; with a new `seq`, a better version to put alongside and replace.
    Ready(Clip),
    /// It did not open, not even at the second attempt (or the server did not give it): the frontend
    /// shows "Riprova" and sends [`VideoCmd::Retry`].
    Failed(AppError),
}

/// What the frontends see of the domain.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoView {
    /// The track being discussed (`None` at rest).
    pub track: Option<Uuid>,
    pub state: ClipState,
    /// Grows at each music video that stops: a frontend notifies the listener when the number is
    /// new (the error is in `state`).
    pub failures: u64,
}

/// The domain's timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {
    /// Time to ask whether the full version is ready. Armed only with a clip shown and
    /// still below the full version.
    Upgrade,
}

/// The domain's async jobs (network), which the host runs on the runtime.
#[derive(Debug)]
pub(crate) enum Job {
    /// The address of `track`'s music video for the request `ticket`. The first time the server
    /// downloads the music video and answers after (up to two minutes).
    Url { api: Result<ApiClient, AppError>, track: Uuid, ticket: u64 },
    /// The server prepares the music video of `id`, which will play soon: it answers at once.
    Prepare { api: Result<ApiClient, AppError>, id: Uuid },
}

/// The outcome of a [`Job`], which goes back to the domain.
#[derive(Debug)]
pub(crate) enum Done {
    Url {
        ticket: u64,
        result: Result<SignedUrl, AppError>,
    },
    /// How the preparation went changes nothing: the server does it or not.
    Prepared,
}

/// Where one is with the music video of the track playing.
#[derive(Debug, Default)]
enum Stage {
    #[default]
    Idle,
    Loading {
        track: Uuid,
    },
    Ready {
        track: Uuid,
        clip: Clip,
        /// The version this address replaced: if the frontend does not open it, that one stays.
        replaced: Option<Clip>,
    },
    Failed {
        track: Uuid,
        error: AppError,
    },
}

impl Stage {
    fn track(&self) -> Option<Uuid> {
        match self {
            Self::Idle => None,
            Self::Loading { track } | Self::Ready { track, .. } | Self::Failed { track, .. } => Some(*track),
        }
    }
}

/// The address request in flight.
#[derive(Debug, Clone, Copy)]
struct Awaiting {
    ticket: u64,
    /// Asks for a better version than the one shown, not the first address.
    upgrade: bool,
}

/// The domain's state: the published view and what stays inside the core.
#[derive(Debug)]
pub(crate) struct Video {
    pub(crate) view: Arc<VideoView>,
    /// The backend chosen in the settings and the client that comes from it.
    server: Option<String>,
    api: Result<ApiClient, AppError>,
    /// A frontend shows the music video.
    shown: bool,
    /// The listener wants music videos: the incoming ones are prepared.
    ahead: bool,
    /// `PrepareAhead(true)` has just arrived: it is prepared at the next passing.
    just_enabled: bool,
    /// The last `started` seen from playback: it changes when a track starts to play.
    started: u64,
    stage: Stage,
    /// The view is no longer the stage's.
    dirty: bool,
    awaiting: Option<Awaiting>,
    ticket: u64,
    /// The last `seq` given to an address.
    seq: u64,
    /// The new address after a failure has already been asked for once for this music video.
    retried: bool,
    /// Requests for the full version already made for this music video.
    upgrades: u32,
    /// The upgrade timer is armed.
    armed: bool,
    failures: u64,
    /// Tracks for which the preparation has already been asked, from the least recent.
    prepared: VecDeque<Uuid>,
}

impl Default for Video {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Video {
    /// The domain with the backend chosen in the settings.
    pub(crate) fn new(server: Option<String>) -> Self {
        Self {
            view: Arc::default(),
            api: ApiClient::new(&config::api_url(server.as_deref())),
            server,
            shown: false,
            ahead: false,
            just_enabled: false,
            started: 0,
            stage: Stage::Idle,
            dirty: false,
            awaiting: None,
            ticket: 0,
            seq: 0,
            retried: false,
            upgrades: 0,
            armed: false,
            failures: 0,
            prepared: VecDeque::new(),
        }
    }

    /// The backend chosen in the settings has changed: the client is made again.
    pub(crate) fn set_server(&mut self, server: &Option<String>) {
        if self.server != *server {
            self.server.clone_from(server);
            self.api = ApiClient::new(&config::api_url(server.as_deref()));
        }
    }

    pub(crate) fn apply(&mut self, command: VideoCmd, _now: Instant, effects: &mut Vec<Effect>) {
        match command {
            VideoCmd::Show(on) => self.shown = on,
            VideoCmd::PrepareAhead(on) => {
                self.just_enabled = on && !self.ahead;
                self.ahead = on;
            }
            VideoCmd::Failed { seq } => self.failed(seq, effects),
            VideoCmd::Retry => {
                if matches!(self.stage, Stage::Failed { .. }) {
                    self.set(Stage::Idle);
                }
            }
            VideoCmd::Reopen => {
                if matches!(self.stage, Stage::Loading { .. } | Stage::Ready { .. }) {
                    self.drop_clip(effects);
                }
            }
        }
    }

    pub(crate) fn fire(&mut self, timer: Timer, _now: Instant, effects: &mut Vec<Effect>) {
        match timer {
            Timer::Upgrade => {
                self.armed = false;
                let Stage::Ready { track, clip, .. } = &self.stage else { return };
                if clip.is_full() || self.upgrades >= MAX_UPGRADES {
                    return;
                }
                let track = *track;
                self.upgrades += 1;
                self.request(track, true, effects);
            }
        }
    }

    pub(crate) fn done(&mut self, done: Done, _now: Instant, effects: &mut Vec<Effect>) {
        match done {
            Done::Url { ticket, result } => {
                // A reply that is not the one of the last request (another track, the video hidden, another
                // attempt) no longer counts.
                let Some(awaiting) = self.awaiting.filter(|awaiting| awaiting.ticket == ticket) else {
                    return;
                };
                self.awaiting = None;
                if awaiting.upgrade {
                    self.upgraded(result, effects);
                } else {
                    self.opened(result, effects);
                }
            }
            Done::Prepared => {}
        }
    }

    /// After each input: the music video the track playing needs, and what the server must
    /// prepare. Without news it compares and that is all: an idle core arms nothing and publishes nothing.
    pub(crate) fn settle(&mut self, playback: &PlaybackView, effects: &mut Vec<Effect>) {
        let current = playback.queue.current().filter(|track| track.has_video).map(|track| track.id);
        self.settle_clip(current, effects);
        self.settle_ahead(playback, effects);
        self.show();
    }

    // ---------- The music video of the track playing ----------

    fn settle_clip(&mut self, current: Option<Uuid>, effects: &mut Vec<Effect>) {
        let held = self.stage.track();
        let failed = matches!(self.stage, Stage::Failed { .. });
        // A music video that did not open stays stopped until the track changes.
        if failed && held == current {
            return;
        }
        let wanted = current.filter(|_| self.shown);
        if wanted == held && !failed {
            return;
        }
        self.drop_clip(effects);
        if let Some(track) = wanted {
            self.open(track, false, effects);
        }
    }

    /// Removes the music video: no timer, no reply to wait for.
    fn drop_clip(&mut self, effects: &mut Vec<Effect>) {
        self.disarm(effects);
        self.awaiting = None;
        self.retried = false;
        self.upgrades = 0;
        if !matches!(self.stage, Stage::Idle) {
            self.set(Stage::Idle);
        }
    }

    /// Asks for the address of `track`'s music video from the start.
    fn open(&mut self, track: Uuid, retried: bool, effects: &mut Vec<Effect>) {
        self.retried = retried;
        self.upgrades = 0;
        self.set(Stage::Loading { track });
        self.request(track, false, effects);
    }

    fn request(&mut self, track: Uuid, upgrade: bool, effects: &mut Vec<Effect>) {
        self.ticket += 1;
        self.awaiting = Some(Awaiting { ticket: self.ticket, upgrade });
        let job = Job::Url { api: self.api.clone(), track, ticket: self.ticket };
        effects.push(Effect::Run(effect::Job::Video(job)));
    }

    /// The first address (or the new one after a failure) has arrived.
    fn opened(&mut self, result: Result<SignedUrl, AppError>, effects: &mut Vec<Effect>) {
        let Stage::Loading { track } = self.stage else { return };
        match result {
            Ok(signed) => {
                let clip = self.clip(signed);
                self.set(Stage::Ready { track, clip, replaced: None });
                self.arm(effects);
            }
            Err(error) => self.fail(track, error, effects),
        }
    }

    /// The reply to the request for a better version. If it is not one, or the request failed,
    /// it is retried later, until the attempts run out.
    fn upgraded(&mut self, result: Result<SignedUrl, AppError>, effects: &mut Vec<Effect>) {
        let Stage::Ready { track, clip, .. } = &self.stage else { return };
        let (track, shown) = (*track, clip.height);
        // Without a height it is a music video saved before the two versions: it is complete.
        let better = |signed: &SignedUrl| match (signed.height, shown) {
            (Some(offered), Some(shown)) => offered > shown,
            _ => true,
        };
        if let Ok(signed) = result
            && better(&signed)
        {
            let replaced = match std::mem::take(&mut self.stage) {
                Stage::Ready { clip, .. } => Some(clip),
                _ => None,
            };
            let clip = self.clip(signed);
            self.set(Stage::Ready { track, clip, replaced });
        }
        self.arm(effects);
    }

    fn clip(&mut self, signed: SignedUrl) -> Clip {
        self.seq += 1;
        Clip { seq: self.seq, height: signed.height, url: Secret::new(signed.url) }
    }

    /// The frontend did not manage to open (or to keep open) the music video `seq`.
    fn failed(&mut self, seq: u64, effects: &mut Vec<Effect>) {
        let Stage::Ready { track, clip, replaced } = &mut self.stage else { return };
        if clip.seq != seq {
            return;
        }
        let track = *track;
        if let Some(previous) = replaced.take() {
            // The upgrade did not open: the version that was already playing stays.
            self.set(Stage::Ready { track, clip: previous, replaced: None });
            self.arm(effects);
        } else if self.retried {
            self.fail(track, AppError::Video, effects);
        } else {
            // An expired address or a dropped stream: once, a new one is asked for.
            self.disarm(effects);
            self.open(track, true, effects);
        }
    }

    fn fail(&mut self, track: Uuid, error: AppError, effects: &mut Vec<Effect>) {
        self.disarm(effects);
        self.awaiting = None;
        self.failures += 1;
        self.set(Stage::Failed { track, error });
    }

    /// Arms the upgrade timer if the music video shown is not yet complete.
    fn arm(&mut self, effects: &mut Vec<Effect>) {
        let Stage::Ready { clip, .. } = &self.stage else { return };
        if clip.is_full() || self.upgrades >= MAX_UPGRADES {
            return;
        }
        self.armed = true;
        effects.push(Effect::Timer { key: TimerKey::Video(Timer::Upgrade), after: UPGRADE_WAIT });
    }

    fn disarm(&mut self, effects: &mut Vec<Effect>) {
        if std::mem::take(&mut self.armed) {
            effects.push(Effect::CancelTimer(TimerKey::Video(Timer::Upgrade)));
        }
    }

    fn set(&mut self, stage: Stage) {
        self.stage = stage;
        self.dirty = true;
    }

    /// The view follows the stage, but changes (and is published again) only if something is really different.
    fn show(&mut self) {
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        let (track, state) = match &self.stage {
            Stage::Idle => (None, ClipState::Idle),
            Stage::Loading { track } => (Some(*track), ClipState::Loading),
            Stage::Ready { track, clip, .. } => (Some(*track), ClipState::Ready(clip.clone())),
            Stage::Failed { track, error } => (Some(*track), ClipState::Failed(error.clone())),
        };
        if self.view.track != track || self.view.state != state || self.view.failures != self.failures {
            let view = Arc::make_mut(&mut self.view);
            view.track = track;
            view.state = state;
            view.failures = self.failures;
        }
    }

    // ---------- In advance ----------

    /// When a track starts to play (or whoever listens turns the music videos on after one has already
    /// started) the server prepares the current one and the next ones. Each track once.
    fn settle_ahead(&mut self, playback: &PlaybackView, effects: &mut Vec<Effect>) {
        let begun = std::mem::replace(&mut self.started, playback.started) != playback.started;
        let enabled = std::mem::take(&mut self.just_enabled);
        if !self.ahead || playback.started == 0 || !(begun || enabled) {
            return;
        }
        let queue = &playback.queue;
        let ahead = queue.current().into_iter().chain(queue.upcoming().take(AHEAD).map(|(_, track)| track));
        let ids: Vec<Uuid> =
            ahead.filter(|track| track.has_video && !track.video_ready).map(|track| track.id).collect();
        for id in ids {
            if self.remember(id) {
                effects.push(Effect::Run(effect::Job::Video(Job::Prepare { api: self.api.clone(), id })));
            }
        }
    }

    /// Remembers that `id` was asked; `false` if it was already there (and puts it back among the most recent).
    fn remember(&mut self, id: Uuid) -> bool {
        let known = self.prepared.iter().position(|prepared| *prepared == id);
        if let Some(index) = known {
            self.prepared.remove(index);
        } else if self.prepared.len() >= PREPARED {
            self.prepared.pop_front();
        }
        self.prepared.push_back(id);
        known.is_none()
    }
}

impl Job {
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Url { api, track, ticket } => {
                let result = match api {
                    Ok(api) => api.video_url(track).await,
                    Err(err) => Err(err),
                };
                Done::Url { ticket, result }
            }
            Self::Prepare { api, id } => {
                if let Ok(api) = api {
                    let _ = api.prepare_video(id).await;
                }
                Done::Prepared
            }
        }
    }

    /// If the job is lost, the domain does not keep waiting: the address did not arrive.
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Url { ticket, .. } => Done::Url { ticket: *ticket, result: Err(AppError::Network) },
            Self::Prepare { .. } => Done::Prepared,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::api::Track;
    use crate::api::models::fixtures::track;
    use crate::command::{Command, PlaybackCmd};
    use crate::effect::Done as AnyDone;
    use crate::player::stream::StreamBuffer;
    use crate::player::{Clock, PlaybackState};
    use crate::snapshot::{DomainVersions, Snapshot};
    use crate::state::playback::{AudioFact, AudioOp, Done as PlaybackDone, Job as PlaybackJob};
    use crate::state::{CoreState, Input};

    fn id(n: u128) -> Uuid {
        track(n).id
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// A catalog track with its music video, still to be downloaded to the server.
    fn clip_track(n: u128) -> Track {
        Track { has_video: true, ..track(n) }
    }

    fn signed(height: Option<u32>) -> SignedUrl {
        SignedUrl {
            kind: "video".into(),
            url: "https://media.test/v1/media?t=firmato".into(),
            expires_at: "2026-11-24T10:00:00Z".into(),
            height,
            format: None,
            quality: None,
        }
    }

    /// The whole core (the same passing between domains as the real one) with the time in the test's hands, a
    /// fake audio and the requests to the server collected: no network, no clock.
    struct Rig {
        state: CoreState,
        snapshot: Snapshot,
        now: Instant,
        /// The addresses asked of the server, since the last reading: (track, request number).
        urls: Vec<(Uuid, u64)>,
        /// The preparations asked, since the last reading.
        prepares: Vec<Uuid>,
        /// The upgrade timer is armed, and how many times it has been armed in all.
        upgrade: bool,
        arms: usize,
        loads: Vec<(Uuid, u64)>,
    }

    impl Rig {
        fn new() -> Self {
            let state = CoreState::default();
            let snapshot = state.snapshot();
            Self {
                state,
                snapshot,
                now: Instant::now(),
                urls: Vec::new(),
                prepares: Vec::new(),
                upgrade: false,
                arms: 0,
                loads: Vec::new(),
            }
        }

        fn feed(&mut self, input: Input) {
            let mut pending = std::collections::VecDeque::from([input]);
            while let Some(input) = pending.pop_front() {
                for effect in self.state.apply(input, self.now) {
                    match effect {
                        Effect::Run(effect::Job::Video(Job::Url { track, ticket, .. })) => {
                            self.urls.push((track, ticket));
                        }
                        Effect::Run(effect::Job::Video(Job::Prepare { id, .. })) => self.prepares.push(id),
                        Effect::Run(effect::Job::Playback(PlaybackJob::Load {
                            track_id,
                            generation,
                            ..
                        })) => self.loads.push((track_id, generation)),
                        Effect::Audio(op) => {
                            if matches!(op, AudioOp::Play { .. }) {
                                pending.push_back(Input::Audio(AudioFact::Started(Ok(()))));
                            }
                        }
                        Effect::Timer { key: TimerKey::Video(Timer::Upgrade), after } => {
                            assert_eq!(after, UPGRADE_WAIT);
                            self.upgrade = true;
                            self.arms += 1;
                        }
                        Effect::CancelTimer(TimerKey::Video(Timer::Upgrade)) => self.upgrade = false,
                        _ => {}
                    }
                }
            }
            if let Some(next) = self.state.publish(&self.snapshot) {
                self.snapshot = next;
            }
        }

        fn video(&mut self, command: VideoCmd) {
            self.feed(Input::Command(Command::Video(command)));
        }

        fn playback(&mut self, command: PlaybackCmd) {
            self.feed(Input::Command(Command::Playback(command)));
        }

        /// The tracks in the queue, the first plays.
        fn play(&mut self, tracks: Vec<Track>) {
            self.playback(PlaybackCmd::Play { tracks: tracks.into(), start: 0 });
            self.load_arrives();
        }

        fn next(&mut self) {
            self.playback(PlaybackCmd::Next);
            self.load_arrives();
        }

        fn load_arrives(&mut self) {
            let (_, generation) = self.loads.pop().expect("un caricamento chiesto");
            self.loads.clear();
            self.feed(Input::Done(AnyDone::Playback(PlaybackDone::Loaded {
                generation,
                result: Ok(StreamBuffer::new()),
                lost: None,
            })));
        }

        /// The server answers request number `ticket`.
        fn answer(&mut self, ticket: u64, result: Result<SignedUrl, AppError>) {
            self.feed(Input::Done(AnyDone::Video(Done::Url { ticket, result })));
        }

        /// The last request in flight receives the reply.
        fn answer_last(&mut self, result: Result<SignedUrl, AppError>) {
            let (_, ticket) = *self.urls.last().expect("una richiesta");
            self.answer(ticket, result);
        }

        /// The upgrade timer expires, armed by `UPGRADE_WAIT`.
        fn fire_upgrade(&mut self) {
            assert!(self.upgrade, "il timer deve essere armato");
            self.upgrade = false;
            self.now += UPGRADE_WAIT;
            self.feed(Input::Timer(TimerKey::Video(Timer::Upgrade)));
        }

        fn view(&self) -> Arc<VideoView> {
            Arc::clone(&self.snapshot.video)
        }

        fn state(&self) -> ClipState {
            self.view().state.clone()
        }

        fn clip(&self) -> Clip {
            match self.state() {
                ClipState::Ready(clip) => clip,
                other => panic!("un videoclip pronto, non {other:?}"),
            }
        }

        fn published(&self) -> u64 {
            self.snapshot.versions.video
        }

        /// A track with the music video plays and the video is shown; the address at 360p has arrived.
        fn showing_360() -> Self {
            let mut rig = Self::new();
            rig.play(vec![clip_track(1), clip_track(2)]);
            rig.video(VideoCmd::Show(true));
            rig.answer_last(Ok(signed(Some(360))));
            rig
        }
    }

    // ---------- Which music video ----------

    #[test]
    fn nothing_is_asked_while_the_video_is_hidden() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1), clip_track(2)]);
        rig.next();
        assert!(rig.urls.is_empty() && rig.prepares.is_empty());
        assert_eq!(rig.view().state, ClipState::Idle);
        assert_eq!((rig.published(), rig.arms), (0, 0));
    }

    #[test]
    fn showing_the_video_asks_for_the_clip_of_the_current_track() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1), clip_track(2)]);
        rig.video(VideoCmd::Show(true));
        assert_eq!(rig.urls, [(id(1), 1)]);
        let view = rig.view();
        assert_eq!((view.track, &view.state), (Some(id(1)), &ClipState::Loading));
        assert!(!rig.upgrade, "ancora niente da aggiornare");
        // Showing it again does not repeat the request.
        rig.video(VideoCmd::Show(true));
        assert_eq!(rig.urls.len(), 1);
    }

    #[test]
    fn a_track_without_a_clip_asks_for_nothing() {
        let mut rig = Rig::new();
        rig.play(vec![track(1), clip_track(2)]);
        rig.video(VideoCmd::Show(true));
        assert!(rig.urls.is_empty());
        assert_eq!((rig.published(), rig.view().state.clone()), (0, ClipState::Idle));
        // The next track has one: the request starts by itself, and when the track without video returns, the
        // music video goes away.
        rig.next();
        assert_eq!(rig.urls, [(id(2), 1)]);
        rig.playback(PlaybackCmd::Previous);
        rig.load_arrives();
        assert_eq!(rig.view().state, ClipState::Idle);
        assert!(rig.view().track.is_none());
    }

    #[test]
    fn the_clip_arrives_and_is_published_with_an_address_to_open() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1)]);
        rig.video(VideoCmd::Show(true));
        rig.answer_last(Ok(signed(Some(360))));
        let clip = rig.clip();
        assert_eq!((clip.seq, clip.height, clip.is_full()), (1, Some(360), false));
        assert_eq!(clip.url(), "https://media.test/v1/media?t=firmato");
        assert_eq!(rig.view().track, Some(id(1)));
        // The address is signed: it does not appear in logs.
        let debug = format!("{:?}", rig.view());
        assert!(!debug.contains("media.test") && !debug.contains("firmato"), "{debug}");
    }

    #[test]
    fn another_track_takes_the_clip_and_an_old_answer_is_dropped() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1), clip_track(2)]);
        rig.video(VideoCmd::Show(true));
        rig.next();
        assert_eq!(rig.urls, [(id(1), 1), (id(2), 2)]);
        // The reply of the first track arrives late: it no longer counts.
        rig.answer(1, Ok(signed(Some(360))));
        assert_eq!(rig.view().state, ClipState::Loading);
        rig.answer(2, Ok(signed(Some(720))));
        assert_eq!((rig.view().track, rig.clip().seq), (Some(id(2)), 1));
    }

    #[test]
    fn hiding_the_video_drops_the_clip_and_the_timer() {
        let mut rig = Rig::showing_360();
        assert!(rig.upgrade);
        rig.video(VideoCmd::Show(false));
        assert!(!rig.upgrade, "il timer si annulla");
        assert_eq!((rig.view().state.clone(), rig.view().track), (ClipState::Idle, None));
        // A reply in flight does not bring it back.
        rig.video(VideoCmd::Show(true));
        rig.video(VideoCmd::Show(false));
        let before = rig.published();
        rig.answer_last(Ok(signed(Some(360))));
        assert_eq!((rig.state(), rig.published()), (ClipState::Idle, before));
        assert!(!rig.upgrade);
    }

    // ---------- From 360p to 720p ----------

    #[test]
    fn the_full_version_is_asked_every_six_seconds_up_to_ten_times() {
        let mut rig = Rig::showing_360();
        assert_eq!((rig.urls.len(), rig.arms), (1, 1));
        let published = rig.published();
        // The server does not have the 720p yet: ten questions, and then no more.
        for asked in 1..=MAX_UPGRADES as usize {
            rig.fire_upgrade();
            assert_eq!(rig.urls.len(), 1 + asked);
            rig.answer_last(Ok(signed(Some(360))));
            assert_eq!(rig.upgrade, asked < MAX_UPGRADES as usize, "dopo {asked} domande");
        }
        assert_eq!(rig.urls.len(), 11);
        assert_eq!(rig.arms, 10, "all'apertura e dopo ogni risposta che non basta, tranne l'ultima");
        assert_eq!(rig.clip().seq, 1, "il videoclip è sempre lo stesso");
        assert_eq!(rig.published(), published, "nessuna risposta inutile si pubblica");
    }

    #[test]
    fn a_better_version_is_published_once_with_a_new_sequence_and_stops_the_timer() {
        let mut rig = Rig::showing_360();
        let published = rig.published();
        rig.fire_upgrade();
        rig.answer_last(Ok(signed(Some(720))));
        let clip = rig.clip();
        assert_eq!((clip.seq, clip.height, clip.is_full()), (2, Some(720), true));
        assert_eq!(rig.published(), published + 1, "una pubblicazione sola");
        assert!(!rig.upgrade, "la versione completa non aspetta altro");
        assert_eq!(rig.urls.len(), 2);
    }

    #[test]
    fn a_failed_upgrade_request_is_asked_again_and_a_clip_without_height_is_full() {
        let mut rig = Rig::showing_360();
        let published = rig.published();
        rig.fire_upgrade();
        rig.answer_last(Err(AppError::Network));
        assert!(rig.upgrade, "la rete è caduta: si riprova al prossimo giro");
        assert_eq!((rig.state(), rig.published()), (ClipState::Ready(rig.clip()), published));
        rig.fire_upgrade();
        // Without a height it is a music video saved before the two versions: it is complete.
        rig.answer_last(Ok(signed(None)));
        assert_eq!((rig.clip().seq, rig.clip().is_full()), (2, true));
        assert!(!rig.upgrade);
    }

    #[test]
    fn a_complete_clip_from_the_start_arms_nothing() {
        for height in [Some(720), Some(1080), None] {
            let mut rig = Rig::new();
            rig.play(vec![clip_track(1)]);
            rig.video(VideoCmd::Show(true));
            rig.answer_last(Ok(signed(height)));
            assert_eq!(rig.arms, 0, "{height:?}");
            assert!(rig.clip().is_full());
        }
    }

    #[test]
    fn leaving_the_track_while_the_upgrade_is_asked_drops_the_answer() {
        let mut rig = Rig::showing_360();
        rig.fire_upgrade();
        let (_, upgrade_ticket) = *rig.urls.last().unwrap();
        rig.next();
        assert!(!rig.upgrade, "il timer del brano di prima non c'è più");
        rig.answer(upgrade_ticket, Ok(signed(Some(720))));
        assert_eq!((rig.view().track, rig.view().state.clone()), (Some(id(2)), ClipState::Loading));
    }

    /// Performance (step 4.9): with the video hidden the core arms nothing and publishes nothing,
    /// even with commands that change nothing; a music video publishes only the version of its
    /// domain, and a better version once only.
    #[test]
    fn only_the_video_version_moves_and_an_idle_core_moves_nothing() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1), clip_track(2)]);
        let idle = rig.snapshot.versions;
        for command in [
            VideoCmd::Show(false),
            VideoCmd::Retry,
            VideoCmd::Reopen,
            VideoCmd::Failed { seq: 1 },
            VideoCmd::PrepareAhead(true),
            VideoCmd::PrepareAhead(false),
        ] {
            rig.video(command);
        }
        assert_eq!(rig.snapshot.versions, idle, "nessuna pubblicazione");
        assert_eq!((rig.arms, rig.urls.len()), (0, 0), "niente armato, niente chiesto");

        rig.video(VideoCmd::Show(true));
        let loading = rig.snapshot.versions;
        assert_eq!(loading, DomainVersions { video: idle.video + 1, ..idle });
        rig.answer_last(Ok(signed(Some(360))));
        let ready = rig.snapshot.versions;
        assert_eq!(ready, DomainVersions { video: loading.video + 1, ..idle });
        // A wait that brings nothing new does not publish; the 720p does, once, and by itself.
        rig.fire_upgrade();
        rig.answer_last(Ok(signed(Some(360))));
        assert_eq!(rig.snapshot.versions, ready);
        rig.fire_upgrade();
        rig.answer_last(Ok(signed(Some(720))));
        assert_eq!(rig.snapshot.versions, DomainVersions { video: ready.video + 1, ..idle });
        assert!(!rig.upgrade);
        // Hidden again: one publish, and the core goes back to rest.
        rig.video(VideoCmd::Show(false));
        assert_eq!(rig.snapshot.versions, DomainVersions { video: ready.video + 2, ..idle });
        assert_eq!(rig.view().state, ClipState::Idle);
    }

    // ---------- When it does not open ----------

    #[test]
    fn a_clip_that_does_not_open_is_asked_again_once_then_blocked() {
        let mut rig = Rig::showing_360();
        let first = rig.clip().seq;
        rig.video(VideoCmd::Failed { seq: first });
        assert_eq!(rig.state(), ClipState::Loading, "un indirizzo nuovo, può essere scaduto");
        assert!(!rig.upgrade);
        assert_eq!(rig.urls.len(), 2);
        rig.answer_last(Ok(signed(Some(360))));
        let second = rig.clip().seq;
        assert_ne!(first, second);
        assert!(rig.upgrade, "il videoclip nuovo riprende l'attesa del 720p");
        // The second failure stops that track's music video.
        rig.video(VideoCmd::Failed { seq: second });
        assert_eq!(rig.state(), ClipState::Failed(AppError::Video));
        assert_eq!((rig.view().track, rig.view().failures), (Some(id(1)), 1));
        assert!(!rig.upgrade);
        // Showing it again or hiding it does not retry it.
        rig.video(VideoCmd::Show(false));
        rig.video(VideoCmd::Show(true));
        assert_eq!((rig.urls.len(), rig.state()), (2, ClipState::Failed(AppError::Video)));
    }

    #[test]
    fn a_failure_of_a_clip_that_is_not_the_published_one_is_ignored() {
        let mut rig = Rig::showing_360();
        let before = (rig.published(), rig.urls.len());
        rig.video(VideoCmd::Failed { seq: 99 });
        assert_eq!((rig.published(), rig.urls.len()), before);
        // And without a music video there is nothing to fail.
        let mut idle = Rig::new();
        idle.video(VideoCmd::Failed { seq: 1 });
        assert_eq!(idle.published(), 0);
    }

    #[test]
    fn a_blocked_clip_is_tried_again_by_another_track_or_by_retry() {
        let mut rig = Rig::showing_360();
        rig.video(VideoCmd::Failed { seq: 1 });
        rig.answer_last(Ok(signed(Some(360))));
        rig.video(VideoCmd::Failed { seq: 2 });
        assert!(matches!(rig.state(), ClipState::Failed(_)));
        // "Riprova": it is asked from the start, with another chance to retry.
        rig.video(VideoCmd::Retry);
        assert_eq!(rig.state(), ClipState::Loading);
        assert_eq!(rig.urls.len(), 3);
        rig.answer_last(Ok(signed(Some(720))));
        assert_eq!(rig.clip().seq, 3);
        rig.video(VideoCmd::Failed { seq: 3 });
        assert_eq!(rig.state(), ClipState::Loading, "il riprovare è di nuovo da fare");
        rig.answer_last(Ok(signed(Some(720))));
        rig.video(VideoCmd::Failed { seq: 4 });
        assert!(matches!(rig.state(), ClipState::Failed(_)));
        // Another track has its music video.
        rig.next();
        assert_eq!((rig.view().track, rig.state()), (Some(id(2)), ClipState::Loading));
        assert_eq!(rig.view().failures, 2, "i guasti già contati restano contati");
    }

    #[test]
    fn retry_on_a_working_clip_changes_nothing() {
        let mut rig = Rig::showing_360();
        let before = (rig.published(), rig.urls.len(), rig.arms);
        rig.video(VideoCmd::Retry);
        assert_eq!((rig.published(), rig.urls.len(), rig.arms), before);
    }

    #[test]
    fn the_server_refusing_the_clip_blocks_it_with_its_error() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1)]);
        rig.video(VideoCmd::Show(true));
        let refusal = AppError::Api { status: 404, code: "not_found".into(), message: "Non trovato.".into() };
        rig.answer_last(Err(refusal.clone()));
        assert_eq!(rig.state(), ClipState::Failed(refusal));
        assert_eq!(rig.view().failures, 1);
        assert_eq!(rig.urls.len(), 1, "un rifiuto del server non si ripete da solo");
        // But "Riprova" asks from the start.
        rig.video(VideoCmd::Retry);
        assert_eq!(rig.state(), ClipState::Loading);
    }

    #[test]
    fn an_upgrade_that_does_not_open_leaves_the_old_clip_and_asks_again_later() {
        let mut rig = Rig::showing_360();
        rig.fire_upgrade();
        rig.answer_last(Ok(signed(Some(720))));
        assert_eq!(rig.clip().seq, 2);
        // The frontend does not open the 720p: it goes on with the 360p that was playing.
        rig.video(VideoCmd::Failed { seq: 2 });
        let clip = rig.clip();
        assert_eq!((clip.seq, clip.height), (1, Some(360)));
        assert!(rig.upgrade, "e il 720p si richiede più tardi");
        assert_eq!(rig.view().failures, 0, "un aggiornamento mancato non è un guasto");
        // The attempt after finds the 720p again with a new address.
        rig.fire_upgrade();
        rig.answer_last(Ok(signed(Some(720))));
        assert_eq!(rig.clip().seq, 3);
        assert!(!rig.upgrade);
    }

    #[test]
    fn reopen_asks_for_the_clip_again_but_only_while_it_is_there() {
        let mut idle = Rig::new();
        idle.play(vec![clip_track(1)]);
        idle.video(VideoCmd::Reopen);
        assert_eq!((idle.published(), idle.urls.len()), (0, 0));

        let mut rig = Rig::showing_360();
        rig.video(VideoCmd::Reopen);
        assert_eq!(rig.state(), ClipState::Loading);
        assert_eq!(rig.urls.len(), 2);
        assert!(!rig.upgrade);
        rig.answer_last(Ok(signed(Some(360))));
        assert!(rig.upgrade);
    }

    #[test]
    fn a_lost_job_is_a_network_failure_not_a_hang() {
        let mut rig = Rig::new();
        rig.play(vec![clip_track(1)]);
        rig.video(VideoCmd::Show(true));
        let (_, ticket) = rig.urls[0];
        let lost = Job::Url { api: Err(AppError::Network), track: id(1), ticket }.lost();
        rig.feed(Input::Done(AnyDone::Video(lost)));
        assert_eq!(rig.state(), ClipState::Failed(AppError::Network));
        assert!(matches!(Job::Prepare { api: Err(AppError::Network), id: id(1) }.lost(), Done::Prepared));
    }

    // ---------- In advance ----------

    fn queue_of_clips() -> Vec<Track> {
        // Track 3 has its music video already ready, 5 does not have one: they are not prepared.
        vec![
            clip_track(1),
            clip_track(2),
            Track { video_ready: true, ..clip_track(3) },
            clip_track(4),
            track(5),
            clip_track(6),
        ]
    }

    #[test]
    fn the_server_prepares_the_clips_of_the_track_and_the_next_two_once() {
        let mut rig = Rig::new();
        rig.video(VideoCmd::PrepareAhead(true));
        rig.play(queue_of_clips());
        // The current track and the two after: 3 is already ready, so 1 and 2.
        assert_eq!(std::mem::take(&mut rig.prepares), [id(1), id(2)]);
        rig.next();
        // Current 2 (already asked), then 3 (ready) and 4.
        assert_eq!(std::mem::take(&mut rig.prepares), [id(4)]);
        rig.next();
        assert!(rig.prepares.is_empty(), "3 pronto, 4 già chiesto, 5 senza video");
        rig.next();
        rig.next();
        assert_eq!(std::mem::take(&mut rig.prepares), [id(6)]);
        assert!(rig.urls.is_empty(), "preparare non è mostrare");
        assert_eq!(rig.published(), 0);
    }

    #[test]
    fn nothing_is_prepared_unless_the_listener_wants_clips() {
        let mut rig = Rig::new();
        rig.play(queue_of_clips());
        rig.next();
        assert!(rig.prepares.is_empty());
        // Turned on with a track already started: the current one and the next ones are prepared at once.
        rig.video(VideoCmd::PrepareAhead(true));
        assert_eq!(std::mem::take(&mut rig.prepares), [id(2), id(4)]);
        // Turned off again the same: nothing changes.
        rig.video(VideoCmd::PrepareAhead(true));
        assert!(rig.prepares.is_empty());
        // Off, another track that starts prepares nothing; on again, it resumes from the rest.
        rig.video(VideoCmd::PrepareAhead(false));
        rig.next();
        rig.next();
        assert!(rig.prepares.is_empty());
        rig.video(VideoCmd::PrepareAhead(true));
        assert_eq!(std::mem::take(&mut rig.prepares), [id(6)]);
    }

    #[test]
    fn the_restored_queue_is_not_prepared_until_something_plays() {
        let mut rig = Rig::new();
        // The queue resumed from the last session is there, but nothing has started: at startup the server
        // does not download three music videos that nobody has asked for.
        rig.playback(PlaybackCmd::Enqueue(Box::new(clip_track(1))));
        rig.playback(PlaybackCmd::Enqueue(Box::new(clip_track(2))));
        rig.video(VideoCmd::PrepareAhead(true));
        assert!(rig.prepares.is_empty());
        assert_eq!(rig.state.playback.view.started, 0);
    }

    #[test]
    fn the_prepared_tracks_are_a_bounded_list_that_forgets_the_oldest() {
        let mut video = Video::default();
        assert!(video.remember(id(1)));
        assert!(!video.remember(id(1)), "già chiesto");
        for n in 2..=PREPARED as u128 {
            assert!(video.remember(id(n)));
        }
        assert_eq!(video.prepared.len(), PREPARED);
        // One more: the least recent goes out (the 1, put back in the queue before the others arrived).
        assert!(video.remember(id(1000)));
        assert_eq!(video.prepared.len(), PREPARED);
        assert!(video.remember(id(1)), "il meno recente è stato dimenticato e si richiede");
        assert!(!video.remember(id(1000)));
        // Seeing a track again puts it back among the most recent: 3 stays, 2 (by now the oldest) goes out.
        assert!(!video.remember(id(3)));
        assert!(video.remember(id(2000)));
        assert!(video.remember(id(2)));
        assert!(!video.remember(id(3)));
    }

    // ---------- Alignment to the audio ----------

    fn view_at(position: Duration, playing: bool, at: Instant) -> PlaybackView {
        let (state, clock) = if playing {
            (PlaybackState::Playing, Clock::running(position, at))
        } else {
            (PlaybackState::Paused, Clock::paused(position, at))
        };
        PlaybackView { state, clock, ..PlaybackView::default() }
    }

    fn measure(position: Duration) -> Measure {
        Measure { position, duration: secs(200), paused: false, ended: false }
    }

    #[test]
    fn a_video_on_the_audio_is_left_alone() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start);
        let later = start + secs(3);
        let audio = view_at(secs(10), true, start);
        // Audio at 13 s: the video at 13.15 s is within the threshold.
        let adjust = alignment.align(&audio, measure(secs(13) + Duration::from_millis(150)), false, later);
        assert!(adjust.is_none());
        assert_eq!(alignment.lead(), Duration::ZERO);
    }

    #[test]
    fn a_video_behind_the_audio_is_moved_and_aims_further_ahead_each_time() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start);
        let audio = view_at(secs(10), true, start);
        // After 3 s the audio is at 13 s and the video at 12: behind by 1 s. One aims 1 s ahead.
        let at = start + secs(3);
        let adjust = alignment.align(&audio, measure(secs(12)), false, at);
        assert_eq!(adjust, Adjust { pause: None, seek: Some(secs(14)) });
        assert_eq!(alignment.lead(), secs(1));
        // Too early: the video has not started again yet, it is not chased.
        let adjust = alignment.align(&audio, measure(secs(12)), false, at + Duration::from_millis(1900));
        assert!(adjust.is_none());
        // After two seconds (audio at 15 s) it is still behind by 0.5 s: the lead grows.
        let at = at + RESYNC_PAUSE;
        let adjust = alignment.align(&audio, measure(secs(14) + Duration::from_millis(500)), false, at);
        assert_eq!(alignment.lead(), Duration::from_millis(1500));
        assert_eq!(adjust.seek, Some(secs(15) + Duration::from_millis(1500)));
    }

    #[test]
    fn the_lead_never_grows_past_the_cap() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start);
        let audio = view_at(secs(100), true, start);
        let mut at = start;
        for _ in 0..5 {
            at += secs(3);
            alignment.align(&audio, measure(secs(10)), false, at);
        }
        assert_eq!(alignment.lead(), MAX_LEAD);
    }

    #[test]
    fn a_video_ahead_of_the_audio_shortens_the_lead() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start);
        let audio = view_at(secs(10), true, start);
        let at = start + secs(3);
        alignment.align(&audio, measure(secs(12)), false, at);
        assert_eq!(alignment.lead(), secs(1));
        // Audio at 15 s; the video at 15.4 s is 400 ms ahead: the lead falls to 600 ms.
        let at = at + secs(2);
        let adjust = alignment.align(&audio, measure(secs(15) + Duration::from_millis(400)), false, at);
        assert_eq!(alignment.lead(), Duration::from_millis(600));
        assert_eq!(adjust.seek, Some(secs(15) + Duration::from_millis(600)));
        // And it does not fall below zero.
        let at = at + secs(2);
        alignment.align(&audio, measure(secs(17) + Duration::from_secs(3)), false, at);
        assert_eq!(alignment.lead(), Duration::ZERO);
    }

    #[test]
    fn the_beat_waits_while_paused_but_a_fresh_player_aligns_at_once() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start - secs(10));
        let paused = view_at(secs(10), false, start);
        let moving = Measure { paused: false, ..measure(secs(30)) };
        // At the tick, with the audio paused, nothing is done (the pause is followed by `follow`).
        assert!(alignment.align(&paused, moving, false, start).is_none());
        // A player just opened is set at once like the audio: paused and at the same position.
        let adjust = alignment.align(&paused, moving, true, start);
        assert_eq!(adjust, Adjust { pause: Some(true), seek: Some(secs(10)) });
        // While it plays, a stopped video starts again.
        let playing = view_at(secs(10), true, start);
        let stopped = Measure { paused: true, ..measure(secs(10)) };
        let adjust = alignment.align(&playing, stopped, true, start);
        assert_eq!(adjust, Adjust { pause: Some(false), seek: None });
    }

    #[test]
    fn a_video_shorter_than_the_audio_stays_on_its_last_frame() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start - secs(10));
        let audio = view_at(secs(190), true, start);
        let short = Measure { duration: secs(180), ..measure(secs(179)) };
        assert!(alignment.align(&audio, short, false, start).is_none());
        // A video that has reached the end is not brought back, but the pause stays that of the audio.
        let ended = Measure { ended: true, paused: true, ..measure(secs(20)) };
        let playing = view_at(secs(20), true, start);
        assert_eq!(alignment.align(&playing, ended, false, start).seek, None);
    }

    #[test]
    fn the_video_follows_the_playback_changes() {
        let start = Instant::now();
        let at = start + secs(5);
        let mut alignment = Alignment::new(start);
        let base = view_at(secs(10), true, start);

        // Nothing new: nothing to do.
        assert_eq!(alignment.follow(&base, &base.clone(), at), None);

        // A track starting: the video goes to the audio's position (15 s at `at`).
        let started = PlaybackView { started: 1, ..base.clone() };
        assert_eq!(
            alignment.follow(&base, &started, at),
            Some(Adjust { pause: Some(false), seek: Some(secs(15)) })
        );
        // A jump within the track, stopped or not, brings the video there.
        let sought = PlaybackView { seeks: 1, ..started.clone() };
        assert_eq!(
            alignment.follow(&started, &sought, at),
            Some(Adjust { pause: Some(false), seek: Some(secs(15)) })
        );
        let sought_paused = PlaybackView { seeks: 2, ..view_at(secs(40), false, at) };
        assert_eq!(
            alignment.follow(&sought, &sought_paused, at),
            Some(Adjust { pause: Some(true), seek: Some(secs(40)) })
        );
        // A pause stops the video and that is all; the resume puts it back where the audio is.
        let paused = PlaybackView { started: 1, ..view_at(secs(15), false, at) };
        assert_eq!(alignment.follow(&started, &paused, at), Some(Adjust { pause: Some(true), seek: None }));
        let resumed = PlaybackView { started: 1, ..view_at(secs(15), true, at) };
        assert_eq!(
            alignment.follow(&paused, &resumed, at),
            Some(Adjust { pause: Some(false), seek: Some(secs(15)) })
        );
    }

    #[test]
    fn a_move_starts_the_wait_before_the_next_chase() {
        let start = Instant::now();
        let mut alignment = Alignment::new(start - secs(60));
        let base = view_at(secs(10), true, start);
        let started = PlaybackView { started: 1, ..base.clone() };
        alignment.follow(&base, &started, start);
        // Just moved: the video is still behind, but it is awaited to start again.
        let behind = measure(secs(5));
        assert!(alignment.align(&started, behind, false, start + Duration::from_millis(500)).is_none());
        assert!(alignment.align(&started, behind, false, start + RESYNC_PAUSE).seek.is_some());
    }
}
