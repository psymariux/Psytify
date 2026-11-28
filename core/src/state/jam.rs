//! Jam: listening together with friends (step 4.8).
//!
//! **The link.** The room is a WebSocket that the core's host holds (`jam::Link`, on a thread of its
//! own): the reducer does not open the network, it asks the host to start it, to send it the state and to
//! close it (`Effect::Jam`, [`JamOp`]), and the room's events enter the core as
//! `Input::Jam`, awaited by the host together with everything else. There is no timer: an idle jam
//! does not wake the core, and the playback's 250 ms tick (which runs only while playing) is the
//! only cadence needed to send the position again every five seconds.
//!
//! **Who plays and who listens.** There are no fixed roles: each member sends the room its own state
//! when it really changes (another track, the queue, pause and resume, a position jump, the tick
//! every five seconds while playing: `jam::should_send_state`) and aligns its own to the first state
//! that arrives from the others. The state received becomes the reference: as long as local playback
//! follows it, it is not sent back.
//!
//! **The passing between domains.** After each input [`CoreState::handoff`](super::CoreState) calls
//! [`Jam::settle`] (the states that arrived from the room, with the local queue in view: the missing tracks
//! are asked of the catalog, then `PlaybackCmd::Align`), applies to playback what the domain asks
//! ([`Jam::take_playback`]: the group, the alignment, the emptied queue) and only afterwards
//! calls [`Jam::publish`], so the state sent to the room is the already aligned one. To the social domain
//! the room's code arrives from the view ([`Social::set_jam`](super::social::Social)), and
//! the name to show in the room comes from the profile ([`Jam::set_profile`]): the frontend does not act as
//! go-between.
//!
//! Change the view only if something really changes: the same members, an event from a room that is no
//! longer there, or a state that changes nothing publish nothing.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{StreamExt, TryStreamExt};
use uuid::Uuid;

use crate::api::{ApiClient, Track};
use crate::config;
use crate::effect::{self, Effect};
use crate::error::AppError;
use crate::jam::{self, Member, Remote, Sent, Target};
use crate::player::PlaybackState;
use crate::state::playback::{Align, PlaybackCmd, PlaybackView};

/// Events kept in the view: a frontend that reads at each publish does not lose any.
const EVENTS: usize = 32;
/// Catalog tracks asked together, for the missing shared queue.
const FETCH_PARALLEL: usize = 8;

/// The domain's commands.
#[derive(Debug, Clone)]
pub enum JamCmd {
    /// Creates a new room. `name` is the name to show in the room: empty, the profile's one is used
    /// (if there is a sign-in); at most [`jam::MAX_NAME_CHARS`] characters.
    Create { name: String },
    /// Joins a room with its code ([`jam::CODE_LEN`] letters or digits).
    Join { code: String, name: String },
    /// Leaves the jam: the room frees the place and tells the others.
    Leave,
}

/// Where one is with the jam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JamPhase {
    /// No jam.
    #[default]
    Idle,
    /// The link has started and the room has not answered yet.
    Connecting,
    /// In the room: it has said its code and who is there.
    Open,
}

/// What the frontends see of the domain.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JamView {
    pub phase: JamPhase,
    /// The code to share, as soon as the room says it.
    pub code: Option<String>,
    /// This client's identity in the room.
    pub you: Option<Uuid>,
    pub members: Arc<[Member]>,
    /// The latest events, from the oldest: a frontend shows those with a new `seq`, in order.
    pub events: Arc<VecDeque<JamRaised>>,
}

impl JamView {
    /// A jam exists: connecting or open.
    pub fn is_active(&self) -> bool {
        self.phase != JamPhase::Idle
    }
}

/// Something to tell the user of the jam, with the usual texts.
#[derive(Debug, Clone, PartialEq)]
pub enum JamEvent {
    /// You have joined the room with this code.
    Joined(String),
    /// You have left.
    Left,
    /// A jam already exists: leave it first.
    AlreadyIn,
    /// The code does not have [`jam::CODE_LEN`] letters or digits.
    BadCode,
    /// Someone else changed track (it does not fire for the state that arrives with the entry).
    TrackChanged { from: String },
    /// The shared queue's tracks did not load.
    TracksFailed(AppError),
    /// The catalog does not have some of the shared queue's tracks.
    TracksMissing,
    /// The room refused a message; the link stays open.
    Rejected(String),
    /// The link has closed (the text is already ready).
    Closed(String),
}

impl JamEvent {
    /// The text to show.
    pub fn message(&self) -> String {
        match self {
            Self::Joined(code) => format!("Sei nella jam {code}"),
            Self::Left => "Sei uscito dalla jam".into(),
            Self::AlreadyIn => "Sei già in una jam".into(),
            Self::BadCode => format!("Il codice ha {} lettere o cifre", jam::CODE_LEN),
            Self::TrackChanged { from } => format!("{from} ha cambiato brano"),
            Self::TracksFailed(err) => format!("Brani della jam non caricati: {err}"),
            Self::TracksMissing => "Alcuni brani della jam non sono disponibili".into(),
            Self::Rejected(reason) | Self::Closed(reason) => reason.clone(),
        }
    }

    /// The notice is an error, not information.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            Self::BadCode | Self::TracksFailed(_) | Self::TracksMissing | Self::Rejected(_) | Self::Closed(_)
        )
    }
}

/// A [`JamEvent`] with its sequence number, which grows with each event.
#[derive(Debug, Clone, PartialEq)]
pub struct JamRaised {
    pub seq: u64,
    pub event: JamEvent,
}

/// What the host does with the room link, which it holds on its own thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JamOp {
    /// Starts the link: creates the room or joins the one of the code. A link that was there
    /// closes.
    Start { base: String, target: Target, name: String },
    /// Sends the local state to the room.
    Send(jam::Snapshot),
    /// Closes the link (if there is one): the room frees the place.
    Leave,
}

/// The domain's async jobs (network), which the host runs on the runtime.
#[derive(Debug)]
pub(crate) enum Job {
    /// The catalog tracks that the shared queue has and the local one does not, for the request `ticket`.
    Tracks { api: Result<ApiClient, AppError>, ids: Vec<Uuid>, ticket: u64 },
}

/// The outcome of a [`Job`], which goes back to the domain.
#[derive(Debug)]
pub(crate) enum Done {
    Tracks { ticket: u64, result: Result<Vec<Track>, AppError> },
}

/// Something that arrived from the room, which waits for the passing between domains to read the local queue.
#[derive(Debug)]
enum Arrived {
    /// A state of the group. `first` is the one the room sends at entry: it is not "someone else
    /// changed track".
    State { remote: Remote, first: bool },
    /// The tracks asked for `remote` have arrived (or not).
    Tracks { remote: Remote, result: Result<Vec<Track>, AppError> },
}

/// The tracks expected from the catalog, and the group's state they are needed for.
#[derive(Debug)]
struct Fetching {
    ticket: u64,
    remote: Remote,
}

/// The domain's state: the published view and what stays inside the core.
#[derive(Debug)]
pub(crate) struct Jam {
    pub(crate) view: Arc<JamView>,
    /// The backend chosen in the settings and the client that comes from it (the tracks of the queue).
    server: Option<String>,
    api: Result<ApiClient, AppError>,
    /// The profile's name, if there is a sign-in: the in-room name of whoever does not write one.
    profile: Option<String>,
    /// The room has said its state, or has none: only then is one's own sent.
    synced: bool,
    /// Last state exchanged with the room: the reference to decide whether to send again.
    last_sent: Option<Sent>,
    /// States and tracks arrived, in order, to be handled at the next passing.
    arrived: VecDeque<Arrived>,
    /// Shared queue tracks arriving: meanwhile nothing is sent.
    fetching: Option<Fetching>,
    tickets: u64,
    /// What is asked of playback at the next passing.
    to_playback: Vec<PlaybackCmd>,
    /// The core closes: no more states to the room (playback stops, and must not
    /// stop the others too).
    closing: bool,
    /// The number of the last event.
    events: u64,
}

impl Default for Jam {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Jam {
    /// The domain with the backend chosen in the settings.
    pub(crate) fn new(server: Option<String>) -> Self {
        Self {
            view: Arc::default(),
            api: ApiClient::new(&config::api_url(server.as_deref())),
            server,
            profile: None,
            synced: false,
            last_sent: None,
            arrived: VecDeque::new(),
            fetching: None,
            tickets: 0,
            to_playback: Vec::new(),
            closing: false,
            events: 0,
        }
    }

    fn view_mut(&mut self) -> &mut JamView {
        Arc::make_mut(&mut self.view)
    }

    /// The code of the room one is in, if the room has said it: the social domain reads it.
    pub(crate) fn code(&self) -> Option<&str> {
        self.view.code.as_deref()
    }

    /// The backend chosen in the settings has changed: the tracks' client is made again. A jam in
    /// progress stays on the server it started with.
    pub(crate) fn set_server(&mut self, server: &Option<String>) {
        if self.server != *server {
            self.server.clone_from(server);
            self.api = ApiClient::new(&config::api_url(server.as_deref()));
        }
    }

    /// The profile's name (if there is a sign-in): it is copied only when it changes.
    pub(crate) fn set_profile(&mut self, name: Option<&str>) {
        if self.profile.as_deref() != name {
            self.profile = name.map(str::to_owned);
        }
    }

    /// The core closes: from here the jam sends nothing more.
    pub(crate) fn close(&mut self) {
        self.closing = true;
    }

    fn raise(&mut self, event: JamEvent) {
        self.events += 1;
        let raised = JamRaised { seq: self.events, event };
        let events = Arc::make_mut(&mut self.view_mut().events);
        events.push_back(raised);
        while events.len() > EVENTS {
            events.pop_front();
        }
    }

    // ---------- Commands ----------

    pub(crate) fn apply(&mut self, command: JamCmd, _now: Instant, effects: &mut Vec<Effect>) {
        match command {
            JamCmd::Create { name } => self.start(Target::Create, &name, effects),
            JamCmd::Join { code, name } => self.start(Target::Join(code), &name, effects),
            JamCmd::Leave => {
                if self.view.is_active() {
                    // Closing the link is enough: the room frees the place and tells the others.
                    self.reset();
                    effects.push(Effect::Jam(JamOp::Leave));
                    self.to_playback.push(PlaybackCmd::Group(false));
                    self.raise(JamEvent::Left);
                }
            }
        }
    }

    /// Starts a jam: creates a room or joins an existing one. The link runs on a thread of its
    /// own, so the request returns at once; the room answers with an event.
    fn start(&mut self, target: Target, name: &str, effects: &mut Vec<Effect>) {
        if self.view.is_active() {
            self.raise(JamEvent::AlreadyIn);
            return;
        }
        if let Target::Join(code) = &target
            && !jam::valid_code(code)
        {
            self.raise(JamEvent::BadCode);
            return;
        }
        // Without a name chosen for the room, with a profile one introduces oneself with one's own.
        let typed = name.trim();
        let name = match &self.profile {
            Some(profile) if typed.is_empty() => profile.as_str(),
            _ => typed,
        };
        let name: String = name.chars().take(jam::MAX_NAME_CHARS).collect();
        self.reset();
        self.view_mut().phase = JamPhase::Connecting;
        // The queue belongs to the group: automatic playback adds nothing by itself.
        self.to_playback.push(PlaybackCmd::Group(true));
        effects.push(Effect::Jam(JamOp::Start {
            base: config::api_url(self.server.as_deref()),
            target,
            name,
        }));
    }

    /// Forgets the room: the view goes back to "no jam" (the events stay) and every wait falls.
    fn reset(&mut self) {
        if self.view.is_active() || self.view.code.is_some() || !self.view.members.is_empty() {
            let view = self.view_mut();
            view.phase = JamPhase::Idle;
            view.code = None;
            view.you = None;
            view.members = Arc::default();
        }
        self.synced = false;
        self.last_sent = None;
        self.arrived.clear();
        self.fetching = None;
    }

    // ---------- The room ----------

    /// An event from the link. One that arrives after the jam has ended (the link closed by us) is
    /// thrown away.
    pub(crate) fn event(&mut self, event: jam::Event) {
        if !self.view.is_active() {
            return;
        }
        match event {
            jam::Event::Joined { code, you, members, state } => {
                let view = self.view_mut();
                view.phase = JamPhase::Open;
                view.code = Some(code.clone());
                view.you = Some(you);
                view.members = Arc::from(members);
                self.raise(JamEvent::Joined(code));
                match state {
                    Some(remote) => self.arrived.push_back(Arrived::State { remote, first: true }),
                    // New room: the first state is one's own.
                    None => self.synced = true,
                }
            }
            jam::Event::Members(members) => {
                if *self.view.members != *members {
                    self.view_mut().members = Arc::from(members);
                }
            }
            jam::Event::State(remote) => self.arrived.push_back(Arrived::State { remote, first: false }),
            jam::Event::Rejected(reason) => self.raise(JamEvent::Rejected(reason)),
            jam::Event::Closed(reason) => {
                self.reset();
                self.to_playback.push(PlaybackCmd::Group(false));
                self.raise(JamEvent::Closed(reason));
            }
        }
    }

    pub(crate) fn done(&mut self, done: Done, _now: Instant, _effects: &mut Vec<Effect>) {
        match done {
            Done::Tracks { ticket, result } => {
                // A superseded reply (another state has arrived, or one has left) is thrown away.
                let Some(fetching) = self.fetching.take_if(|fetching| fetching.ticket == ticket) else {
                    return;
                };
                self.arrived.push_back(Arrived::Tracks { remote: fetching.remote, result });
            }
        }
    }

    /// The number of the last request of tracks, for the tests that hand over an outcome by hand.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn fetch_ticket(&self) -> Option<u64> {
        self.fetching.as_ref().map(|fetching| fetching.ticket)
    }

    // ---------- The passing between domains ----------

    /// Handles what has arrived from the room, reading the local queue: the missing tracks are
    /// asked of the catalog, then playback aligns to the group (`take_playback`).
    pub(crate) fn settle(&mut self, playback: &PlaybackView, now: Instant, effects: &mut Vec<Effect>) {
        while let Some(arrived) = self.arrived.pop_front() {
            if self.closing || !self.view.is_active() {
                self.arrived.clear();
                return;
            }
            match arrived {
                Arrived::State { remote, first } => self.remote_state(remote, first, playback, now, effects),
                Arrived::Tracks { remote, result } => match result {
                    Ok(fetched) => self.adopt(&remote, &fetched, playback),
                    Err(err) => self.raise(JamEvent::TracksFailed(err)),
                },
            }
        }
    }

    /// The group's state becomes the local one: the missing tracks are asked, then it aligns.
    fn remote_state(
        &mut self,
        remote: Remote,
        first: bool,
        playback: &PlaybackView,
        now: Instant,
        effects: &mut Vec<Effect>,
    ) {
        self.synced = true;
        // The state received is the reference: it must not be sent back as if it were local.
        self.last_sent = Some(Sent { snapshot: remote.snapshot.clone(), at: now });
        // A new state supersedes the tracks that were expected for the old one.
        self.fetching = None;
        let snapshot = &remote.snapshot;
        if snapshot.queue.is_empty() {
            self.to_playback.push(PlaybackCmd::Clear);
            return;
        }
        let wanted = snapshot.queue.get(snapshot.index).copied();
        if !first && playback.queue.current().map(|track| track.id) != wanted {
            self.raise(JamEvent::TrackChanged { from: remote.from.clone() });
        }
        let known: HashSet<Uuid> = playback.queue.entries().map(|(_, track)| track.id).collect();
        let mut asked = HashSet::new();
        let missing: Vec<Uuid> =
            snapshot.queue.iter().copied().filter(|id| !known.contains(id) && asked.insert(*id)).collect();
        if missing.is_empty() {
            self.adopt(&remote, &[], playback);
            return;
        }
        self.tickets += 1;
        let ticket = self.tickets;
        self.fetching = Some(Fetching { ticket, remote });
        effects.push(Effect::Run(effect::Job::Jam(Job::Tracks {
            api: self.api.clone(),
            ids: missing,
            ticket,
        })));
    }

    /// Aligns playback to the group: the queue in the group's order (with the tracks already queued and
    /// those arrived from the catalog; shuffle turns off, the order is decided by whoever plays first),
    /// then the same track at the same position, or the track changes. The rest, including the
    /// jump to the shared position when the track's audio is ready, is done by playback.
    fn adopt(&mut self, remote: &Remote, fetched: &[Track], playback: &PlaybackView) {
        let snapshot = &remote.snapshot;
        let same = playback.queue.entries().map(|(_, track)| track.id).eq(snapshot.queue.iter().copied());
        let tracks = if same {
            None
        } else {
            // The tracks already queued win over those arrived: it is the same copy the group has.
            let mut by_id: HashMap<Uuid, &Track> = HashMap::new();
            for track in playback.queue.entries().map(|(_, track)| track).chain(fetched) {
                by_id.entry(track.id).or_insert(track);
            }
            let ordered: Option<Vec<Track>> =
                snapshot.queue.iter().map(|id| by_id.get(id).map(|track| (*track).clone())).collect();
            let Some(ordered) = ordered else {
                self.raise(JamEvent::TracksMissing);
                return;
            };
            Some(ordered)
        };
        self.to_playback.push(PlaybackCmd::Align(Box::new(Align {
            tracks,
            index: snapshot.index,
            position: Duration::from_millis(snapshot.position_ms),
            playing: snapshot.playing,
        })));
    }

    /// What the domain asks of playback (the group, the alignment, the emptied queue): the passing between
    /// domains applies it, in the same round.
    pub(crate) fn take_playback(&mut self) -> Vec<PlaybackCmd> {
        std::mem::take(&mut self.to_playback)
    }

    /// Sends the room the local state, if it has changed or if it is time to send the position again. Nothing
    /// is sent while the group has not yet said its state, nor while a track arrived from the group is
    /// loading: otherwise the room would be overwritten with old data. Outside a jam it costs a comparison;
    /// inside, the queue is compared without copying it, and the state is built only when it leaves.
    pub(crate) fn publish(&mut self, playback: &PlaybackView, now: Instant, effects: &mut Vec<Effect>) {
        if self.closing || !self.view.is_active() || !self.synced || self.fetching.is_some() {
            return;
        }
        if matches!(playback.state, PlaybackState::Loading { .. }) {
            return;
        }
        let Some(index) = playback.queue.position() else { return };
        // A queue longer than the server accepts is not shared.
        if playback.queue.len() > jam::MAX_QUEUE {
            return;
        }
        let playing = playback.is_playing();
        let position_ms = jam::millis(playback.position(now));
        let ids = || playback.queue.entries().map(|(_, track)| track.id);
        if !jam::should_send_state(self.last_sent.as_ref(), ids(), index, position_ms, playing, now) {
            return;
        }
        let snapshot = jam::Snapshot { queue: ids().collect(), index, position_ms, playing };
        effects.push(Effect::Jam(JamOp::Send(snapshot.clone())));
        self.last_sent = Some(Sent { snapshot, at: now });
    }
}

impl Job {
    /// Runs the job: the catalog's tracks, a few at a time. The order does not matter: the domain
    /// realigns by id.
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Tracks { api, ids, ticket } => {
                let result = match api {
                    Ok(api) => {
                        futures_util::stream::iter(ids)
                            .map(|id| api.track(id))
                            .buffer_unordered(FETCH_PARALLEL)
                            .try_collect()
                            .await
                    }
                    Err(err) => Err(err),
                };
                Done::Tracks { ticket, result }
            }
        }
    }

    /// If the job is lost, the domain does not keep waiting: it fails as without network.
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Tracks { ticket, .. } => Done::Tracks { ticket: *ticket, result: Err(AppError::Network) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::api::models::fixtures::track;
    use crate::command::Command;
    use crate::effect::{Done as AnyDone, TimerKey};
    use crate::jam::Snapshot as Shared;
    use crate::player::stream::StreamBuffer;
    use crate::state::playback::{AudioFact, AudioOp, Done as PlaybackDone, Job as PlaybackJob, Timer};
    use crate::state::{CoreState, Input};

    const TICK: Duration = Duration::from_millis(250);

    fn id(n: u128) -> Uuid {
        track(n).id
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn shared(queue: &[u128], index: usize, position_ms: u64, playing: bool) -> Shared {
        Shared { queue: queue.iter().map(|n| id(*n)).collect(), index, position_ms, playing }
    }

    fn remote(from: &str, queue: &[u128], index: usize, position_ms: u64, playing: bool) -> Remote {
        Remote { snapshot: shared(queue, index, position_ms, playing), from: from.into() }
    }

    fn joined(code: &str, state: Option<Remote>) -> jam::Event {
        jam::Event::Joined {
            code: code.into(),
            you: Uuid::from_u128(77),
            members: vec![
                Member { id: Uuid::from_u128(77), name: "Prova".into() },
                Member { id: Uuid::from_u128(78), name: "Luca".into() },
            ],
            state,
        }
    }

    /// The whole core (the same passing between domains as the real one) with the time in the test's hands and
    /// the fake audio: each `Play` starts, and the tick is fired by hand.
    struct Rig {
        state: CoreState,
        now: Instant,
        /// What the jam has asked of the host, since the last reading.
        ops: Vec<JamOp>,
        /// The tracks asked of the catalog: (request number, id).
        fetches: Vec<(u64, Vec<Uuid>)>,
        /// The loads asked: (track, generation).
        loads: Vec<(Uuid, u64)>,
        /// The timers armed in all, apart from the tick and the settings' save.
        other_timers: usize,
        ticking: bool,
        audio: Vec<AudioOp>,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                state: CoreState::default(),
                now: Instant::now(),
                ops: Vec::new(),
                fetches: Vec::new(),
                loads: Vec::new(),
                other_timers: 0,
                ticking: false,
                audio: Vec::new(),
            }
        }

        /// Gives the input and puts back in motion what the audio would answer, as the host does.
        fn feed(&mut self, input: Input) {
            let mut pending = VecDeque::from([input]);
            while let Some(input) = pending.pop_front() {
                for effect in self.state.apply(input, self.now) {
                    match effect {
                        Effect::Jam(op) => self.ops.push(op),
                        Effect::Run(effect::Job::Jam(Job::Tracks { ids, ticket, .. })) => {
                            self.fetches.push((ticket, ids));
                        }
                        Effect::Run(effect::Job::Playback(PlaybackJob::Load {
                            track_id,
                            generation,
                            ..
                        })) => self.loads.push((track_id, generation)),
                        Effect::Audio(op) => {
                            if matches!(op, AudioOp::Play { .. }) {
                                pending.push_back(Input::Audio(AudioFact::Started(Ok(()))));
                            }
                            self.audio.push(op);
                        }
                        Effect::Timer { key: TimerKey::Playback(Timer::Tick), .. } => self.ticking = true,
                        Effect::CancelTimer(TimerKey::Playback(Timer::Tick)) => self.ticking = false,
                        // The delayed save of `config.json` is not the jam's.
                        Effect::Timer { key: TimerKey::Settings(_), .. } => {}
                        Effect::Timer { .. } => self.other_timers += 1,
                        _ => {}
                    }
                }
            }
        }

        fn jam(&mut self, command: JamCmd) {
            self.feed(Input::Command(Command::Jam(command)));
        }

        fn playback(&mut self, command: PlaybackCmd) {
            self.feed(Input::Command(Command::Playback(command)));
        }

        fn event(&mut self, event: jam::Event) {
            self.feed(Input::Jam(event));
        }

        fn elapse(&mut self, by: Duration) {
            self.now += by;
        }

        fn view(&self) -> Arc<JamView> {
            Arc::clone(&self.state.jam.view)
        }

        fn events(&self) -> Vec<JamEvent> {
            self.view().events.iter().map(|raised| raised.event.clone()).collect()
        }

        /// The states sent to the room since the last reading.
        fn sent(&mut self) -> Vec<Shared> {
            std::mem::take(&mut self.ops)
                .into_iter()
                .filter_map(|op| match op {
                    JamOp::Send(snapshot) => Some(snapshot),
                    _ => None,
                })
                .collect()
        }

        fn queue_ids(&self) -> Vec<Uuid> {
            self.state.playback.view.queue.entries().map(|(_, track)| track.id).collect()
        }

        fn position_ms(&self) -> u64 {
            jam::millis(self.state.playback.view.position(self.now))
        }

        /// The tracks play from the first: loaded, started.
        fn play(&mut self, queue: &[u128]) {
            let tracks: Vec<Track> = queue.iter().map(|n| track(*n)).collect();
            self.playback(PlaybackCmd::Play { tracks: tracks.into(), start: 0 });
            self.load_arrives();
            assert!(self.state.playback.view.is_playing(), "{:?}", self.state.playback.view.state);
        }

        /// The server sends the last track asked for.
        fn load_arrives(&mut self) {
            let (_, generation) = self.loads.pop().expect("un caricamento chiesto");
            self.loads.clear();
            self.feed(Input::Done(AnyDone::Playback(PlaybackDone::Loaded {
                generation,
                result: Ok(StreamBuffer::new()),
                lost: None,
            })));
        }

        /// A tick of playback: the audio has reached `heard` from the start of the track.
        fn beat(&mut self, heard: Duration) {
            self.elapse(TICK);
            assert!(self.ticking, "il battito deve essere armato");
            self.feed(Input::Timer(TimerKey::Playback(Timer::Tick)));
            self.feed(Input::Audio(AudioFact::Polled {
                position: heard,
                finished: false,
                seek_failed: false,
            }));
        }

        /// Ticks from `from` to `until`, one every 250 ms, as a track playing does.
        fn listen(&mut self, from: Duration, until: Duration) {
            let mut at = from;
            while at < until {
                at += TICK;
                self.beat(at);
            }
        }

        fn answer(&mut self, ticket: u64, result: Result<Vec<Track>, AppError>) {
            self.feed(Input::Done(AnyDone::Jam(Done::Tracks { ticket, result })));
        }

        /// An open jam in a new room (with no state), with `queue` playing.
        fn open_room(queue: &[u128]) -> Self {
            let mut rig = Self::new();
            rig.play(queue);
            rig.jam(JamCmd::Create { name: "Prova".into() });
            rig.event(joined("K7QXM3", None));
            rig
        }
    }

    // ---------- Commands ----------

    #[test]
    fn creating_a_jam_asks_the_host_for_a_room_and_puts_playback_in_the_group() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Create { name: "  Luca  ".into() });
        assert_eq!(
            std::mem::take(&mut rig.ops),
            [JamOp::Start { base: config::api_url(None), target: Target::Create, name: "Luca".into() }]
        );
        let view = rig.view();
        assert_eq!((view.phase, view.code.as_deref()), (JamPhase::Connecting, None));
        assert!(view.is_active() && view.events.is_empty());
        assert!(rig.state.playback.grouped(), "la coda è del gruppo: niente autoplay");
        assert_eq!(rig.other_timers, 0);
    }

    #[test]
    fn joining_sends_the_code_and_a_second_start_is_refused() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        assert!(matches!(
            rig.ops.as_slice(),
            [JamOp::Start { target: Target::Join(code), name, .. }] if code == "K7QXM3" && name.is_empty()
        ));
        rig.ops.clear();
        rig.jam(JamCmd::Create { name: "x".into() });
        rig.jam(JamCmd::Join { code: "AAAAAA".into(), name: "x".into() });
        assert!(rig.ops.is_empty(), "una jam c'è già: il collegamento non riparte");
        assert_eq!(rig.events(), [JamEvent::AlreadyIn, JamEvent::AlreadyIn]);
        assert_eq!(rig.view().phase, JamPhase::Connecting);
    }

    #[test]
    fn a_bad_code_never_reaches_the_host() {
        let mut rig = Rig::new();
        for code in ["", "K7QXM", "K7QXM3X", "K7-XM3", "K7QXM·"] {
            rig.jam(JamCmd::Join { code: code.into(), name: String::new() });
        }
        assert!(rig.ops.is_empty());
        assert!(!rig.view().is_active());
        assert_eq!(rig.events(), vec![JamEvent::BadCode; 5]);
        assert_eq!(JamEvent::BadCode.message(), "Il codice ha 6 lettere o cifre");
        assert!(JamEvent::BadCode.is_error());
        assert!(!rig.state.playback.grouped());
    }

    #[test]
    fn the_name_typed_wins_then_the_profile_and_both_are_cut_to_the_room_limit() {
        let name_of = |ops: &[JamOp]| match ops {
            [JamOp::Start { name, .. }] => name.clone(),
            other => panic!("{other:?}"),
        };
        let mut jam = Jam::default();
        let mut effects = Vec::new();
        jam.set_profile(Some("Sara Rossi"));
        jam.apply(JamCmd::Create { name: "   ".into() }, Instant::now(), &mut effects);
        let ops: Vec<JamOp> =
            effects.drain(..).filter_map(|e| if let Effect::Jam(op) = e { Some(op) } else { None }).collect();
        assert_eq!(name_of(&ops), "Sara Rossi", "senza un nome scritto, quello del profilo");

        jam.apply(JamCmd::Leave, Instant::now(), &mut effects);
        effects.clear();
        jam.apply(JamCmd::Create { name: "Luca".into() }, Instant::now(), &mut effects);
        let ops: Vec<JamOp> =
            effects.drain(..).filter_map(|e| if let Effect::Jam(op) = e { Some(op) } else { None }).collect();
        assert_eq!(name_of(&ops), "Luca", "il nome scritto vince sul profilo");

        jam.apply(JamCmd::Leave, Instant::now(), &mut effects);
        effects.clear();
        let long = "x".repeat(jam::MAX_NAME_CHARS + 20);
        jam.apply(JamCmd::Create { name: long }, Instant::now(), &mut effects);
        let ops: Vec<JamOp> =
            effects.drain(..).filter_map(|e| if let Effect::Jam(op) = e { Some(op) } else { None }).collect();
        assert_eq!(name_of(&ops).chars().count(), jam::MAX_NAME_CHARS);
    }

    // ---------- Entry ----------

    #[test]
    fn the_room_answers_with_its_code_and_members() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Create { name: String::new() });
        rig.event(joined("K7QXM3", None));
        let view = rig.view();
        assert_eq!(view.phase, JamPhase::Open);
        assert_eq!(view.code.as_deref(), Some("K7QXM3"));
        assert_eq!(view.you, Some(Uuid::from_u128(77)));
        assert_eq!(view.members.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Prova", "Luca"]);
        assert_eq!(rig.events(), [JamEvent::Joined("K7QXM3".into())]);
        assert_eq!(JamEvent::Joined("K7QXM3".into()).message(), "Sei nella jam K7QXM3");
        assert!(!JamEvent::Joined("K7QXM3".into()).is_error());
    }

    /// BACKLOG 10: whoever joins an already started jam sees "Sei nella jam CODE", and the state that the
    /// room sends at entry is not "someone else changed track": the notice does not replace it.
    #[test]
    fn a_late_joiner_keeps_the_you_are_in_the_jam_notice() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2], 1, 12_000, true))));
        assert_eq!(rig.events(), [JamEvent::Joined("K7QXM3".into())], "nessun «ha cambiato brano»");
        assert_eq!(
            rig.view().events.back().map(|raised| raised.event.message()),
            Some("Sei nella jam K7QXM3".to_owned()),
            "l'ultimo avviso è il codice"
        );
    }

    #[test]
    fn a_late_joiner_fetches_the_missing_tracks_then_aligns_to_the_shared_position() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2], 1, 12_000, true))));

        // The group's tracks are missing: they are asked of the catalog (once) and nothing is sent to the room.
        // meanwhile.
        assert_eq!(rig.fetches.len(), 1);
        assert_eq!(rig.fetches[0].1, [id(1), id(2)]);
        assert!(rig.sent().is_empty());
        assert!(rig.state.jam.fetch_ticket().is_some());

        // Arrived in any order, the queue takes the group's and the shared track starts. While it loads, the room
        // receives nothing.
        let ticket = rig.fetches[0].0;
        rig.answer(ticket, Ok(vec![track(2), track(1)]));
        assert_eq!(rig.queue_ids(), [id(1), id(2)]);
        assert_eq!(rig.state.playback.view.queue.position(), Some(1));
        assert_eq!(rig.state.playback.view.state, PlaybackState::Loading { track_id: id(2) });
        assert!(rig.sent().is_empty(), "un brano del gruppo in caricamento non si annuncia");
        assert!(rig.state.jam.fetch_ticket().is_none());

        // The audio ready starts from the group's position, not from the beginning, and nothing is sent back.
        rig.load_arrives();
        assert!(rig.state.playback.view.is_playing());
        assert_eq!(rig.position_ms(), 12_000);
        assert!(rig.audio.iter().any(|op| matches!(op, AudioOp::Seek(at) if *at == secs(12))));
        assert!(rig.sent().is_empty(), "lo stato è quello appena ricevuto");
        rig.listen(secs(12), Duration::from_millis(12_500));
        assert!(rig.sent().is_empty(), "il battito che segue il gruppo non manda niente");
    }

    #[test]
    fn a_late_joiner_of_a_paused_jam_waits_at_the_shared_position_and_the_first_local_change_is_sent() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2], 1, 12_000, false))));
        let ticket = rig.fetches[0].0;
        rig.answer(ticket, Ok(vec![track(1), track(2)]));
        rig.load_arrives();
        assert_eq!(rig.state.playback.view.state, PlaybackState::Paused, "il gruppo è in pausa");
        assert_eq!(rig.position_ms(), 12_000);
        assert!(rig.sent().is_empty());

        // The first local change, instead, is announced: the resume, at the shared position.
        rig.playback(PlaybackCmd::Toggle);
        let sent = rig.sent();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!((&sent[0].queue, sent[0].index, sent[0].playing), (&vec![id(1), id(2)], 1, true));
        assert!(sent[0].position_ms.abs_diff(12_000) <= 50, "{}", sent[0].position_ms);
    }

    #[test]
    fn a_state_whose_tracks_are_all_in_the_queue_aligns_without_asking_the_catalogue() {
        let mut rig = Rig::new();
        rig.play(&[1, 2, 3]);
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2, 3], 2, 40_000, true))));
        assert!(rig.fetches.is_empty());
        assert_eq!(rig.state.playback.view.queue.position(), Some(2));
        assert_eq!(rig.state.playback.view.state, PlaybackState::Loading { track_id: id(3) });
        assert!(rig.sent().is_empty());
    }

    #[test]
    fn a_remote_queue_with_other_tracks_and_a_known_one_fetches_only_what_is_missing() {
        let mut rig = Rig::new();
        rig.play(&[1, 2]);
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        // The group has 3 and 1 (2 is not there, and 3 appears twice): only 3 is asked, once.
        rig.event(joined("K7QXM3", Some(remote("Luca", &[3, 1, 3], 0, 0, true))));
        assert_eq!(rig.fetches.len(), 1);
        assert_eq!(rig.fetches[0].1, [id(3)]);
        rig.answer(rig.fetches[0].0, Ok(vec![track(3)]));
        assert_eq!(rig.queue_ids(), [id(3), id(1), id(3)]);
    }

    #[test]
    fn a_newer_state_supersedes_the_tracks_still_in_flight() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2], 0, 0, true))));
        let first = rig.fetches[0].0;
        rig.event(jam::Event::State(remote("Luca", &[3, 4], 1, 5_000, true)));
        assert_eq!(rig.fetches.len(), 2);
        let second = rig.fetches[1].0;
        assert_ne!(first, second);

        // The reply to the old state is superseded: the queue does not change and the rest waits.
        rig.answer(first, Ok(vec![track(1), track(2)]));
        assert!(rig.queue_ids().is_empty());
        assert!(rig.state.jam.fetch_ticket().is_some(), "si aspetta ancora il secondo");

        rig.answer(second, Ok(vec![track(3), track(4)]));
        assert_eq!(rig.queue_ids(), [id(3), id(4)]);
        assert_eq!(rig.state.playback.view.queue.position(), Some(1));
    }

    #[test]
    fn a_state_that_needs_nothing_cancels_the_wait_for_the_old_one() {
        let mut rig = Rig::new();
        rig.play(&[1, 2]);
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[8, 9], 0, 0, true))));
        let stale = rig.fetches[0].0;
        rig.event(jam::Event::State(remote("Luca", &[1, 2], 1, 3_000, true)));
        assert!(rig.state.jam.fetch_ticket().is_none());
        rig.answer(stale, Ok(vec![track(8), track(9)]));
        assert_eq!(rig.queue_ids(), [id(1), id(2)], "la risposta superata non tocca la coda");
    }

    #[test]
    fn a_failed_fetch_says_so_and_changes_nothing_and_missing_tracks_are_reported() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2], 0, 0, true))));
        rig.answer(rig.fetches[0].0, Err(AppError::Network));
        assert!(rig.queue_ids().is_empty());
        let last = rig.view().events.back().cloned().expect("an event");
        assert_eq!(last.event, JamEvent::TracksFailed(AppError::Network));
        assert!(last.event.is_error());
        assert_eq!(
            last.event.message(),
            "Brani della jam non caricati: Server non raggiungibile. Controlla la connessione."
        );

        // The catalog sends only one of the two: the queue stays as it is, and it is said.
        rig.event(jam::Event::State(remote("Luca", &[1, 2], 0, 0, true)));
        rig.answer(rig.fetches[1].0, Ok(vec![track(1)]));
        assert!(rig.queue_ids().is_empty());
        let last = rig.view().events.back().cloned().expect("an event");
        assert_eq!(last.event, JamEvent::TracksMissing);
        assert_eq!(last.event.message(), "Alcuni brani della jam non sono disponibili");
    }

    #[test]
    fn a_change_of_track_by_someone_else_is_announced_but_a_steady_heartbeat_is_not() {
        let mut rig = Rig::new();
        rig.play(&[1, 2]);
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[1, 2], 0, 0, true))));
        assert_eq!(rig.events(), [JamEvent::Joined("K7QXM3".into())]);
        rig.elapse(secs(5));
        rig.event(jam::Event::State(remote("Luca", &[1, 2], 0, 5_000, true)));
        assert_eq!(rig.events().len(), 1, "stesso brano: niente da dire");
        rig.event(jam::Event::State(remote("Luca", &[1, 2], 1, 0, true)));
        assert_eq!(
            rig.view().events.back().map(|raised| raised.event.clone()),
            Some(JamEvent::TrackChanged { from: "Luca".into() })
        );
        assert_eq!(JamEvent::TrackChanged { from: "Luca".into() }.message(), "Luca ha cambiato brano");
    }

    #[test]
    fn an_empty_shared_queue_clears_the_local_one() {
        let mut rig = Rig::new();
        rig.play(&[1, 2]);
        rig.jam(JamCmd::Join { code: "K7QXM3".into(), name: String::new() });
        rig.event(joined("K7QXM3", Some(remote("Luca", &[], 0, 0, false))));
        assert!(rig.queue_ids().is_empty());
        assert!(rig.fetches.is_empty());
    }

    #[test]
    fn members_change_the_view_only_when_they_change_and_a_rejection_keeps_the_jam_open() {
        let mut rig = Rig::open_room(&[1]);
        let before = rig.view();
        let same = vec![
            Member { id: Uuid::from_u128(77), name: "Prova".into() },
            Member { id: Uuid::from_u128(78), name: "Luca".into() },
        ];
        rig.event(jam::Event::Members(same));
        assert!(Arc::ptr_eq(&before, &rig.view()), "gli stessi membri non cambiano la vista");
        rig.event(jam::Event::Members(vec![Member { id: Uuid::from_u128(77), name: "Prova".into() }]));
        assert_eq!(rig.view().members.len(), 1);

        rig.event(jam::Event::Rejected("Troppi cambiamenti in poco tempo: riprova tra un attimo".into()));
        let last = rig.view().events.back().cloned().expect("an event");
        assert!(last.event.is_error());
        assert_eq!(last.event.message(), "Troppi cambiamenti in poco tempo: riprova tra un attimo");
        assert_eq!(rig.view().phase, JamPhase::Open, "il collegamento resta aperto");
    }

    // ---------- Exit ----------

    #[test]
    fn leaving_closes_the_link_leaves_the_group_and_a_late_event_is_ignored() {
        let mut rig = Rig::open_room(&[1]);
        rig.ops.clear();
        rig.jam(JamCmd::Leave);
        assert_eq!(std::mem::take(&mut rig.ops), [JamOp::Leave]);
        let view = rig.view();
        assert!(!view.is_active() && view.code.is_none() && view.you.is_none() && view.members.is_empty());
        assert_eq!(view.events.back().map(|raised| raised.event.clone()), Some(JamEvent::Left));
        assert_eq!(JamEvent::Left.message(), "Sei uscito dalla jam");
        assert!(!rig.state.playback.grouped(), "fuori dal gruppo l'autoplay torna");

        // Whatever the link still had in hand reopens nothing.
        let events = rig.events().len();
        rig.event(jam::Event::State(remote("Luca", &[1], 0, 0, true)));
        rig.event(jam::Event::Closed("Jam chiusa".into()));
        rig.event(joined("K7QXM3", None));
        assert!(!rig.view().is_active());
        assert_eq!(rig.events().len(), events);
        assert!(rig.ops.is_empty() && rig.fetches.is_empty());
        // Leaving again does nothing.
        rig.jam(JamCmd::Leave);
        assert!(rig.ops.is_empty());
        assert_eq!(rig.events().len(), events);
    }

    #[test]
    fn the_room_closing_ends_the_jam_and_tells_why() {
        let mut rig = Rig::open_room(&[1]);
        rig.ops.clear();
        rig.event(jam::Event::Closed("Jam chiusa dal server".into()));
        assert!(!rig.view().is_active());
        let last = rig.view().events.back().cloned().expect("an event");
        assert_eq!(last.event, JamEvent::Closed("Jam chiusa dal server".into()));
        assert!(last.event.is_error());
        assert_eq!(last.event.message(), "Jam chiusa dal server");
        assert!(!rig.state.playback.grouped());
        assert!(rig.ops.is_empty(), "il collegamento è già finito: non c'è altro da chiudere");

        // It can start again from scratch.
        rig.jam(JamCmd::Create { name: String::new() });
        assert!(matches!(rig.ops.as_slice(), [JamOp::Start { .. }]));
    }

    #[test]
    fn a_connection_that_never_opens_ends_the_jam_with_the_reason() {
        let mut rig = Rig::new();
        rig.jam(JamCmd::Create { name: String::new() });
        rig.event(jam::Event::Closed("Il server non risponde".into()));
        assert!(!rig.view().is_active());
        assert!(!rig.state.playback.grouped());
        assert_eq!(rig.events(), [JamEvent::Closed("Il server non risponde".into())]);
    }

    // ---------- When the state is sent ----------

    /// Idle, a jam sends nothing and arms no timer; each real change is a single send
    /// (and never while the track loads), and the tick every five seconds while playing.
    #[test]
    fn the_local_state_is_sent_on_real_transitions_and_on_a_five_second_heartbeat() {
        let mut rig = Rig::open_room(&[1, 2]);
        // Entered a new room with the queue playing: the first state is one's own.
        let first = rig.sent();
        assert_eq!(first, [shared(&[1, 2], 0, 0, true)]);
        assert_eq!(rig.other_timers, 0, "la jam non arma nessun timer");

        // Four seconds of listening: no send (the position follows the clock).
        rig.listen(Duration::ZERO, secs(4));
        assert!(rig.sent().is_empty());
        // At the fifth second, the tick.
        rig.listen(secs(4), secs(5));
        let beat = rig.sent();
        assert_eq!(beat.len(), 1);
        assert!(beat[0].playing && beat[0].position_ms.abs_diff(5_000) <= 300, "{beat:?}");
        rig.listen(secs(5), secs(9));
        assert!(rig.sent().is_empty());

        // Pause: a single send. While paused, nothing else: the core no longer has a tick.
        rig.playback(PlaybackCmd::Toggle);
        let paused = rig.sent();
        assert_eq!(paused.len(), 1);
        assert!(!paused[0].playing);
        assert!(!rig.ticking);
        rig.elapse(secs(60));
        rig.playback(PlaybackCmd::Volume(0.5));
        assert!(rig.sent().is_empty(), "un minuto di pausa non rimanda niente");

        // A jump while paused is sent; the resume too.
        rig.playback(PlaybackCmd::SeekTo(secs(100)));
        let jumped = rig.sent();
        assert_eq!(jumped.len(), 1);
        assert_eq!(jumped[0].position_ms, 100_000);
        rig.playback(PlaybackCmd::Toggle);
        assert_eq!(rig.sent().len(), 1);
    }

    #[test]
    fn a_new_track_is_sent_once_when_it_plays_and_never_while_it_loads() {
        let mut rig = Rig::open_room(&[1, 2]);
        rig.sent();
        rig.listen(Duration::ZERO, secs(2));
        rig.playback(PlaybackCmd::Next);
        assert!(matches!(rig.state.playback.view.state, PlaybackState::Loading { .. }));
        assert!(rig.sent().is_empty(), "in caricamento non si manda niente");
        rig.load_arrives();
        let sent = rig.sent();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!((sent[0].index, sent[0].playing), (1, true));
        assert_eq!(sent[0].queue, [id(1), id(2)]);
    }

    #[test]
    fn a_change_of_the_queue_is_sent_and_a_queue_too_long_for_the_server_is_not() {
        let mut rig = Rig::open_room(&[1]);
        rig.sent();
        rig.playback(PlaybackCmd::Enqueue(Box::new(track(2))));
        let sent = rig.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].queue, [id(1), id(2)]);

        let many: Vec<Track> = (1..=jam::MAX_QUEUE as u128 + 1).map(track).collect();
        // From the sixth: the first was the last one played and is listened to again from the buffer, with no loading.
        rig.playback(PlaybackCmd::Play { tracks: many.into(), start: 5 });
        rig.load_arrives();
        assert!(rig.sent().is_empty(), "più di {} brani non si condividono", jam::MAX_QUEUE);
    }

    #[test]
    fn nothing_is_sent_before_the_room_has_told_its_state_nor_with_nothing_queued() {
        let mut rig = Rig::new();
        rig.play(&[1]);
        rig.jam(JamCmd::Create { name: String::new() });
        assert!(rig.sent().is_empty(), "la stanza non ha ancora risposto");
        rig.playback(PlaybackCmd::Toggle);
        assert!(rig.sent().is_empty());
        rig.playback(PlaybackCmd::Toggle);

        let mut empty = Rig::new();
        empty.jam(JamCmd::Create { name: String::new() });
        empty.event(joined("K7QXM3", None));
        assert!(empty.sent().is_empty(), "niente in coda, niente da condividere");
        empty.play(&[1]);
        assert_eq!(empty.sent().len(), 1, "la prima coda parte appena c'è");
    }

    /// Closing the core stops playback, and at that point the room must not hear "in pausa": it would be a way
    /// of stopping the others' listening.
    #[test]
    fn closing_the_core_does_not_tell_the_room_that_playback_stopped() {
        let mut rig = Rig::open_room(&[1]);
        rig.sent();
        rig.listen(Duration::ZERO, secs(1));
        rig.feed(Input::Shutdown);
        assert!(!rig.state.playback.view.is_playing());
        assert!(rig.sent().is_empty());
    }

    #[test]
    fn a_remote_state_is_the_reference_and_a_local_jump_after_it_is_sent() {
        let mut rig = Rig::open_room(&[1, 2]);
        rig.sent();
        rig.listen(Duration::ZERO, secs(1));
        // The group is at 31 s on the same track: one moves and the same state is not sent back.
        rig.event(jam::Event::State(remote("Luca", &[1, 2], 0, 31_000, true)));
        assert!(rig.sent().is_empty());
        assert_eq!(rig.position_ms(), 31_000);
        // A far local jump is announced.
        rig.playback(PlaybackCmd::SeekTo(secs(90)));
        let sent = rig.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].position_ms, 90_000);
    }

    // ---------- View ----------

    #[test]
    fn events_keep_the_last_32_with_growing_numbers() {
        let mut rig = Rig::new();
        for _ in 0..40 {
            rig.jam(JamCmd::Join { code: "x".into(), name: String::new() });
        }
        let view = rig.view();
        assert_eq!(view.events.len(), EVENTS);
        assert_eq!(view.events.front().map(|raised| raised.seq), Some(9));
        assert_eq!(view.events.back().map(|raised| raised.seq), Some(40));
    }

    #[test]
    fn a_job_that_is_lost_fails_like_a_missing_network() {
        let job = Job::Tracks { api: Err(AppError::Network), ids: vec![id(1)], ticket: 5 };
        let Done::Tracks { ticket, result } = job.lost();
        assert_eq!((ticket, result), (5, Err(AppError::Network)));
    }

    #[tokio::test]
    async fn a_fetch_with_no_client_reports_the_client_error() {
        let job = Job::Tracks { api: Err(AppError::InvalidBaseUrl("x".into())), ids: vec![id(1)], ticket: 2 };
        let Done::Tracks { ticket, result } = job.run().await;
        assert_eq!((ticket, result), (2, Err(AppError::InvalidBaseUrl("x".into()))));
    }
}
