//! The core's state: a reducer with no IO. `apply` receives an input and the current instant and
//! returns the effects to run; it does not open the network, does not read the clock, does not touch files.
//! So the tests drive it with a fake time and check the effects without running them.
//!
//! Each domain sits in its own module and changes its view with `Arc::make_mut`: as long as a frontend
//! holds the previous snapshot the view is copied, and `publish` recognises from the changed pointer
//! which domains to publish.

pub mod catalog;
pub mod downloads;
pub mod external;
pub mod jam;
pub mod library;
pub mod playback;
pub mod settings;
pub mod social;
pub mod system;
#[cfg(feature = "video")]
pub mod video;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::command::{Command, SettingsCmd};
use crate::config::Config;
use crate::effect::{Done, Effect, TimerKey};
use crate::snapshot::{DomainVersions, Snapshot};

/// What makes the state move.
#[derive(Debug)]
pub(crate) enum Input {
    /// A command from a frontend.
    Command(Command),
    /// A timer armed with [`Effect::Timer`] has expired.
    Timer(TimerKey),
    /// A job asked for with [`Effect::Run`] has finished.
    Done(Done),
    /// L'uscita audio ha risposto a un'[`Effect::Audio`].
    Audio(playback::AudioFact),
    /// A command from the system (media keys, desktop widget, headphones) or the link
    /// multimediale pronto.
    Media(crate::media::Event),
    /// Something from the jam room (entry, members, the others' state, closing): the host
    /// waits for the link together with everything else, and an idle jam needs no tick.
    Jam(crate::jam::Event),
    /// What the system asks about motion, at each real change (after `Effect::WatchMotion`).
    SystemMotion(crate::system_motion::SystemMotion),
    /// The core has just started: the domains pass on what they read from `config.json` (Discord,
    /// if the user wants it, starts from here, before any command).
    Start,
    /// The frontend closes: the domains finish writing, then [`Effect::Flushed`].
    Shutdown,
}

/// The state of all the domains.
#[derive(Debug, Default)]
pub(crate) struct CoreState {
    pub(crate) settings: settings::Settings,
    pub(crate) library: library::Library,
    pub(crate) playback: playback::Playback,
    pub(crate) catalog: catalog::Catalog,
    pub(crate) external: external::External,
    pub(crate) downloads: downloads::Downloads,
    pub(crate) social: social::Social,
    pub(crate) jam: jam::Jam,
    #[cfg(feature = "video")]
    pub(crate) video: video::Video,
}

impl CoreState {
    /// The state at startup, with the configuration already read from `path` (the host reads it before
    /// starting: the reducer does not touch files). Without `path` nothing is written. `seed` keeps
    /// the Home's mixes fixed for the session (the host picks it: the reducer has no clock).
    pub(crate) fn new(config: Config, path: Option<PathBuf>, seed: u64) -> Self {
        let library = library::Library::new(config.core.library(), seed);
        let playback = playback::Playback::new(
            config.core.playback(),
            config.core.server.clone(),
            seed,
            crate::config::taste::now_secs(),
        );
        let catalog = catalog::Catalog::new(config.core.server.clone());
        let external = external::External::new(config.core.server.clone());
        let downloads = downloads::Downloads::new(
            &config.core.downloads(),
            config.core.server.clone(),
            downloads::Dirs::default(),
        );
        let social = social::Social::new(config.core.clerk_app(), config.core.server.clone());
        let jam = jam::Jam::new(config.core.server.clone());
        #[cfg(feature = "video")]
        let video = video::Video::new(config.core.server.clone());
        Self {
            settings: settings::Settings::new(config, path),
            library,
            playback,
            catalog,
            external,
            downloads,
            social,
            jam,
            #[cfg(feature = "video")]
            video,
        }
    }

    /// The folders where downloads are written: the tracks and the update packages. They are chosen by
    /// the host (the reducer does not know the system).
    #[must_use]
    pub(crate) fn with_dirs(mut self, downloads: PathBuf, updates: PathBuf) -> Self {
        self.downloads.set_dirs(downloads::Dirs { downloads, updates });
        self
    }

    /// The login: where `session.json` sits (in this core's data folder) and whether the host found it
    /// on disk. With a saved session the domain already starts on "sign-in in progress" and
    /// resumes at the first `Input::Start`.
    #[must_use]
    pub(crate) fn with_session(mut self, file: Option<PathBuf>, stored: bool) -> Self {
        self.social = std::mem::take(&mut self.social).with_session(file, stored);
        self
    }

    /// The audio output did not open: playback knows it from the start.
    pub(crate) fn with_device_error(mut self, error: Option<crate::error::AppError>) -> Self {
        self.playback.set_device_error(error);
        self
    }

    /// Passes the input to the domain that concerns it and collects the effects it asks for.
    pub(crate) fn apply(&mut self, input: Input, now: Instant) -> Vec<Effect> {
        let mut effects = Vec::new();
        match input {
            Input::Command(command) => match command {
                Command::Settings(command) => {
                    // «Save now» also saves where the current entry is.
                    if matches!(command, SettingsCmd::SaveNow) {
                        self.settings.store_playback(self.playback.fields(now), &mut effects);
                    }
                    self.settings.apply(command, now, &mut effects);
                }
                Command::Library(command) => self.library.apply(*command, now, &mut effects),
                Command::Playback(command) => self.playback.apply(command, now, &mut effects),
                Command::Catalog(command) => self.catalog.apply(command, now, &mut effects),
                Command::External(command) => self.external.apply(command, now, &mut effects),
                Command::Downloads(command) => self.downloads.apply(command, now, &mut effects),
                Command::Social(command) => self.social.apply(command, now, &mut effects),
                Command::Jam(command) => self.jam.apply(command, now, &mut effects),
                #[cfg(feature = "video")]
                Command::Video(command) => self.video.apply(command, now, &mut effects),
            },
            Input::Timer(key) => match key {
                TimerKey::Settings(timer) => self.settings.fire(timer, now, &mut effects),
                TimerKey::Library(timer) => self.library.fire(timer, now, &mut effects),
                TimerKey::Playback(timer) => self.playback.fire(timer, now, &mut effects),
                TimerKey::Catalog(timer) => self.catalog.fire(timer, now, &mut effects),
                TimerKey::External(timer) => self.external.fire(timer, now, &mut effects),
                TimerKey::Downloads(timer) => self.downloads.fire(timer, now, &mut effects),
                TimerKey::Social(timer) => self.social.fire(timer, now, &mut effects),
                #[cfg(feature = "video")]
                TimerKey::Video(timer) => self.video.fire(timer, now, &mut effects),
            },
            Input::Audio(fact) => self.playback.audio(fact, now, &mut effects),
            Input::Media(event) => self.playback.media(event, now, &mut effects),
            Input::Jam(event) => self.jam.event(event),
            Input::SystemMotion(motion) => self.settings.system_motion(motion),
            Input::Done(done) => match done {
                Done::Settings(done) => self.settings.done(done, now, &mut effects),
                Done::Library(done) => self.library.done(done, now, &mut effects),
                Done::Playback(done) => self.playback.done(done, now, &mut effects),
                Done::Catalog(done) => self.catalog.done(done, now, &mut effects),
                Done::External(done) => self.external.done(done, now, &mut effects),
                Done::Downloads(done) => self.downloads.done(done, now, &mut effects),
                Done::Social(done) => self.social.done(done, now, &mut effects),
                Done::Jam(done) => self.jam.done(done, now, &mut effects),
                #[cfg(feature = "video")]
                Done::Video(done) => self.video.done(done, now, &mut effects),
            },
            // The passing between domains happens after each input, below: the catalog rereads
            // the saved tracks without a cover.
            Input::Start => {
                self.catalog.start();
                self.external.start();
                self.downloads.start(&mut effects);
                self.social.start(&mut effects);
            }
            // Each domain saves through the settings: that is where the closing waits for the disk.
            Input::Shutdown => {
                // Before playback: stopping at close is not to be told to the room.
                self.jam.close();
                self.playback.close(now, &mut effects);
                self.handoff(now, &mut effects);
                self.settings.close(&mut effects);
            }
        }
        self.handoff(now, &mut effects);
        effects
    }

    /// What a domain passes to the others after an input: the store of recent tracks and the
    /// backend for whoever needs it, what playback asks of the library (plays, listening
    /// time), and the fields a domain owns for the settings, which write the file.
    fn handoff(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        // The external sources first: they read the library, the catalog and playback, and what
        // they ask (an imported track, a `Play`, "the catalog has changed") is applied by the other
        // domains in the same round, here and in the passes that follow. The next import starts
        // only afterwards, with the library already updated.
        self.external.set_server(&self.settings.view.config.server);
        self.external.settle(&self.library.view, &self.catalog.view, &self.playback.view, effects);
        for command in self.external.take_library() {
            self.library.apply(command, now, effects);
        }
        for command in self.external.take_playback() {
            self.playback.apply(command, now, effects);
        }
        for command in self.external.take_catalog() {
            self.catalog.apply(command, now, effects);
        }
        self.external.pace(&self.library.view, effects);
        // The catalog: finds again the tracks that playback says have vanished, rereads the one just
        // started, and hands to the library and playback what it has asked to change (the recent
        // tracks the feed comes from, the correct covers, the tracks found again).
        for track in self.playback.take_heal() {
            self.catalog.heal(track, effects);
        }
        self.catalog.set_server(&self.settings.view.config.server, effects);
        let current = self.playback.view.queue.current().map(|track| track.id);
        self.catalog.track_started(self.playback.view.started, current, effects);
        for command in self.catalog.take_library() {
            self.library.apply(command, now, effects);
        }
        for command in self.catalog.take_playback() {
            self.playback.apply(command, now, effects);
        }
        self.catalog.after_library(&self.library.view, effects);
        if let Some(query) = self.catalog.take_query() {
            self.settings.store_query(query, effects);
        }
        self.downloads.set_server(&self.settings.view.config.server);
        self.playback.set_pool(self.library.view.feed_source.as_ref());
        self.playback.set_server(&self.settings.view.config.server);
        self.playback.set_feed(&self.library.view.feed);
        let config = &self.settings.view.config;
        self.playback.set_discord(config.discord_presence, &config.discord_app_id, now, effects);
        // The jam: reads playback and the profile. What it asks of playback (the group,
        // the alignment) applies at once, and only afterwards the room receives the local state,
        // already aligned. The frontend does not act as go-between.
        self.jam.set_server(&self.settings.view.config.server);
        self.jam.set_profile(self.social.view.account.profile().map(|profile| profile.display_name.as_str()));
        self.jam.settle(&self.playback.view, now, effects);
        for command in self.jam.take_playback() {
            self.playback.apply(command, now, effects);
        }
        self.jam.publish(&self.playback.view, now, effects);
        for command in self.playback.take_library() {
            self.library.apply(command, now, effects);
        }
        // The profile: the plays just counted, and then what changed in the track being listened to and
        // in the public playlists, once things have settled. A play without access is thrown away.
        self.social.set_server(&self.settings.view.config.server);
        self.social.set_jam(self.jam.code());
        for track in self.playback.take_listens() {
            self.social.listen(track, effects);
        }
        self.social.sync(&self.playback.view, &self.library.view, now, effects);
        if self.library.take_persist() {
            self.settings.store_library(self.library.fields(), effects);
        }
        if self.playback.take_persist() {
            self.settings.store_playback(self.playback.fields(now), effects);
        }
        // The downloaded files: playback says which ones it could not open (the downloads give the
        // mark them as lost), and receives the map of the ones to play from.
        let lost = self.playback.take_lost_files();
        if !lost.is_empty() {
            self.downloads.files_lost(lost);
        }
        self.playback.set_local(self.downloads.local());
        if self.downloads.take_persist() {
            self.settings.store_downloads(self.downloads.fields(), effects);
        }
        // The music video, last: it reads playback already settled (the track playing, the queue,
        // the track that has just started) and asks nothing of any other domain.
        #[cfg(feature = "video")]
        {
            self.video.set_server(&self.settings.view.config.server);
            self.video.settle(&self.playback.view, effects);
        }
    }

    /// The first snapshot, before any input: version zero for everyone.
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            version: 0,
            versions: DomainVersions::default(),
            settings: Arc::clone(&self.settings.view),
            library: Arc::clone(&self.library.view),
            playback: Arc::clone(&self.playback.view),
            catalog: Arc::clone(&self.catalog.view),
            external: Arc::clone(&self.external.view),
            downloads: Arc::clone(&self.downloads.view),
            social: Arc::clone(&self.social.view),
            jam: Arc::clone(&self.jam.view),
            #[cfg(feature = "video")]
            video: Arc::clone(&self.video.view),
        }
    }

    /// The snapshot that follows `previous`, if at least one domain has changed; `None` otherwise, so
    /// an idle core wakes no frontend. A domain's version grows when its view
    /// is no longer the one published.
    pub(crate) fn publish(&self, previous: &Snapshot) -> Option<Snapshot> {
        let mut versions = previous.versions;
        let mut changed = false;
        changed |= bump(&previous.settings, &self.settings.view, &mut versions.settings);
        changed |= bump(&previous.library, &self.library.view, &mut versions.library);
        changed |= bump(&previous.playback, &self.playback.view, &mut versions.playback);
        changed |= bump(&previous.catalog, &self.catalog.view, &mut versions.catalog);
        changed |= bump(&previous.external, &self.external.view, &mut versions.external);
        changed |= bump(&previous.downloads, &self.downloads.view, &mut versions.downloads);
        changed |= bump(&previous.social, &self.social.view, &mut versions.social);
        changed |= bump(&previous.jam, &self.jam.view, &mut versions.jam);
        #[cfg(feature = "video")]
        {
            changed |= bump(&previous.video, &self.video.view, &mut versions.video);
        }
        changed.then(|| Snapshot { version: previous.version + 1, versions, ..self.snapshot() })
    }
}

/// Grows `version` if the current view is not the published one. Says whether it has changed.
fn bump<T>(published: &Arc<T>, current: &Arc<T>, version: &mut u64) -> bool {
    if Arc::ptr_eq(published, current) {
        return false;
    }
    *version += 1;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures::track;
    use crate::command::{LibraryCmd, PlaybackCmd, SettingsCmd};
    use crate::config::CoreConfig;
    use crate::effect;
    use crate::snapshot::PlaybackView;

    fn command(state: &mut CoreState, command: Command) -> Vec<Effect> {
        state.apply(Input::Command(command), Instant::now())
    }

    fn like(n: u128) -> Command {
        Command::library(LibraryCmd::ToggleLike(track(n)))
    }

    /// A core with a file to write, so the library's changes arm the save.
    fn state_with_a_file() -> CoreState {
        CoreState::new(Config::default(), Some(PathBuf::from("/dati/psytify/config.json")), 1)
    }

    #[test]
    fn an_untouched_state_publishes_nothing() {
        let state = CoreState::default();
        assert!(state.publish(&state.snapshot()).is_none());
    }

    #[test]
    fn a_changed_domain_bumps_only_its_own_version() {
        let mut state = CoreState::default();
        let first = state.snapshot();
        state.playback.view = Arc::new(PlaybackView::default());
        let next = state.publish(&first).unwrap();
        assert_eq!(next.version, 1);
        assert_eq!(next.versions, DomainVersions { playback: 1, ..DomainVersions::default() });
        assert!(Arc::ptr_eq(&next.settings, &first.settings), "gli altri domini restano gli stessi");
        assert!(!Arc::ptr_eq(&next.playback, &first.playback));
        assert!(state.publish(&next).is_none(), "pubblicata una volta, non si ripubblica");
    }

    /// Performance (step 4.2): idle, or with commands that change nothing, the core does not publish;
    /// a real change is a single publish, with only the version of the domains touched.
    #[test]
    fn idle_and_unrelated_commands_publish_only_real_changes() {
        let mut state = CoreState::default();
        let start = state.snapshot();
        assert!(state.publish(&start).is_none(), "idle");

        // Commands of other domains and library commands with no effect: no publish.
        let nothing = [
            Command::Settings(SettingsCmd::Store(Box::default())),
            Command::library(LibraryCmd::Listen(0)),
            Command::library(LibraryCmd::ForgetSearch(None)),
            Command::library(LibraryCmd::RefreshFeed),
            Command::library(LibraryCmd::MarkNewsSeen),
            Command::library(LibraryCmd::DeletePlaylist(uuid::Uuid::from_u128(1))),
            Command::library(LibraryCmd::HomeTracks(None)),
        ];
        for each in nothing {
            command(&mut state, each);
        }
        assert!(state.publish(&start).is_none(), "commands that change nothing");

        // A real change: one publish, with the library and the settings (the file to write).
        command(&mut state, like(1));
        let next = state.publish(&start).expect("a like is a change");
        assert_eq!(next.version, 1);
        assert_eq!(next.versions.library, 1);
        assert_eq!(next.versions.settings, 1, "the file has to follow");
        assert_eq!(next.versions.playback, 0);
        assert!(Arc::ptr_eq(&next.playback, &start.playback));
        assert!(state.publish(&next).is_none(), "published once");

        // The same track at the top of the plays twice: the second changes nothing.
        command(&mut state, Command::library(LibraryCmd::RememberListen(track(1))));
        let heard = state.publish(&next).expect("first listen");
        command(&mut state, Command::library(LibraryCmd::RememberListen(track(1))));
        assert!(state.publish(&heard).is_none());
    }

    /// A burst of library changes is a single round: the fields end up together in `config.json`.
    #[test]
    fn library_changes_reach_the_settings_and_arm_one_save() {
        let mut state = state_with_a_file();
        let start = state.apply(Input::Start, Instant::now());
        assert!(
            start.iter().all(|e| matches!(e, Effect::Discord(_))),
            "il core appena partito accende solo Discord, se c'è: {start:?}"
        );
        let effects = command(&mut state, like(1));
        assert!(matches!(effects.as_slice(), [Effect::Timer { .. }]), "one save armed: {effects:?}");
        assert!(command(&mut state, like(2)).is_empty(), "the second change joins the same save");
        let config = &state.settings.view.config;
        assert_eq!(config.liked.iter().map(|t| t.id).collect::<Vec<_>>(), [track(2).id, track(1).id]);
        assert!(state.library.view.liked.same(&config.liked), "the view and the file share the list");
    }

    /// The frontend's `SettingsCmd::Store` does not delete what the library has: its fields are
    /// ignored, the others pass.
    #[test]
    fn the_frontends_stored_config_never_overwrites_the_library() {
        let mut state = state_with_a_file();
        command(&mut state, like(1));
        let stale = CoreConfig { clerk_client_id: "rock".into(), ..CoreConfig::default() };
        command(&mut state, Command::Settings(SettingsCmd::Store(Box::new(stale))));
        let config = &state.settings.view.config;
        assert_eq!(config.clerk_client_id, "rock", "the other fields pass");
        assert_eq!(config.liked.len(), 1, "the library fields stay");
    }

    #[test]
    fn the_library_starts_from_the_saved_file() {
        let mut config = Config::default();
        config.core.liked = vec![track(3)].into();
        config.core.news_seen = "2026-01-01".into();
        let state = CoreState::new(config, None, 1);
        let snapshot = state.snapshot();
        assert_eq!(snapshot.library.liked.len(), 1);
        assert_eq!(snapshot.library.news_seen, "2026-01-01");
        assert!(snapshot.library.liked.same(&snapshot.settings.config.liked), "the same list, not a copy");
    }

    /// Step 4.3b: the Discord switch and the app (settings) reach playback with the passing between
    /// domains: turning it on starts the link, changing the app restarts it,
    /// turning it off stops it; an identical `Store` does nothing.
    #[test]
    fn the_discord_setting_reaches_playback_through_the_handoff() {
        use crate::state::system::DiscordOp;

        let mut state = CoreState::new(
            Config {
                core: CoreConfig { discord_presence: false, ..CoreConfig::default() },
                ..Config::default()
            },
            None,
            1,
        );
        let discord = |effects: Vec<Effect>| -> Vec<String> {
            effects
                .into_iter()
                .filter_map(|effect| match effect {
                    Effect::Discord(DiscordOp::Start(id)) => Some(format!("start {id}")),
                    Effect::Discord(DiscordOp::Stop) => Some("stop".into()),
                    Effect::Discord(DiscordOp::Set(_)) => Some("set".into()),
                    _ => None,
                })
                .collect()
        };
        let store = |presence: bool, app: &str| {
            Command::Settings(SettingsCmd::Store(Box::new(CoreConfig {
                discord_presence: presence,
                discord_app_id: app.into(),
                ..CoreConfig::default()
            })))
        };
        assert!(discord(command(&mut state, store(false, "99"))).is_empty(), "spento");
        assert_eq!(discord(command(&mut state, store(true, "99"))), vec!["start 99", "set"]);
        assert!(discord(command(&mut state, store(true, "99"))).is_empty(), "uguale: niente");
        assert_eq!(discord(command(&mut state, store(true, "100"))), vec!["stop", "start 100", "set"]);
        assert_eq!(discord(command(&mut state, store(false, "100"))), vec!["stop"]);
    }

    /// Performance (step 4.3): idle, or paused, the core does not publish and does not arm the tick; a
    /// real transition is a single publish, and the position (which advances) is not in the snapshot.
    #[test]
    fn idle_and_paused_playback_publish_nothing_and_a_transition_publishes_once() {
        use std::time::Duration;

        use crate::api::Track;
        use crate::player::stream::StreamBuffer;
        use crate::state::playback::{AudioFact, AudioOp, Done as PlaybackDone};

        let mut state = CoreState::default();
        let mut snapshot = state.snapshot();
        let run = |state: &mut CoreState, input: Input| -> Vec<Effect> { state.apply(input, Instant::now()) };
        let ticks = |effects: &[Effect]| effects.iter().filter(|e| matches!(e, Effect::Timer { .. })).count();

        // Stopped: no commands, no snapshots, no tick.
        assert!(state.publish(&snapshot).is_none());
        let effects = run(&mut state, Input::Command(Command::Playback(PlaybackCmd::Toggle)));
        assert!(
            ticks(&effects) == 0 && state.publish(&snapshot).is_none(),
            "play con la coda vuota non fa niente"
        );

        // Play: Loading, a single publish and only of the playback domain.
        let tracks: Arc<[Track]> = vec![track(1), track(2)].into();
        let effects =
            run(&mut state, Input::Command(Command::Playback(PlaybackCmd::Play { tracks, start: 0 })));
        let generation = state.playback.view.generation;
        assert_eq!(ticks(&effects), 0, "in caricamento il battito non gira");
        let next = state.publish(&snapshot).expect("play is a transition");
        assert_eq!((next.versions.playback, next.versions.library), (1, 0));
        snapshot = next;

        // The track arrives and starts: the tick arms, one publish.
        run(
            &mut state,
            Input::Done(Done::Playback(PlaybackDone::Loaded {
                generation,
                result: Ok(StreamBuffer::new()),
                lost: None,
            })),
        );
        let effects = run(&mut state, Input::Audio(AudioFact::Started(Ok(()))));
        assert_eq!(ticks(&effects), 1);
        assert!(effects.iter().all(|e| !matches!(e, Effect::Audio(AudioOp::Play { .. }))));
        let next = state.publish(&snapshot).expect("playing");
        assert!(next.playback.is_playing());
        snapshot = next;

        // A tick that finds the audio where it is expected does not change the view: no publish.
        let effects = run(&mut state, Input::Timer(TimerKey::Playback(playback::Timer::Tick)));
        assert_eq!(ticks(&effects), 1, "il battito si riarma finché suona");
        run(
            &mut state,
            Input::Audio(AudioFact::Polled { position: Duration::ZERO, finished: false, seek_failed: false }),
        );
        assert!(state.publish(&snapshot).is_none(), "la posizione non è nell'istantanea");

        // Pause: one publish, the tick stops; then the core is fully idle again.
        let effects = run(&mut state, Input::Command(Command::Playback(PlaybackCmd::Toggle)));
        assert!(effects.iter().any(|e| matches!(e, Effect::CancelTimer(TimerKey::Playback(_)))));
        snapshot = state.publish(&snapshot).expect("paused");
        assert_eq!(snapshot.playback.state, crate::player::PlaybackState::Paused);
        let same_quality = PlaybackCmd::Quality(snapshot.playback.quality);
        let effects = run(&mut state, Input::Command(Command::Playback(same_quality)));
        assert!(effects.is_empty(), "in pausa nessun timer, e un comando senza effetto non fa lavoro");
        assert!(state.publish(&snapshot).is_none(), "in pausa il core non pubblica");
    }

    /// Performance (step 4.4): idle, the catalog publishes nothing; writing the same text again
    /// changes nothing; the real search is one publish, each reply one; a reply
    /// superata nessuna.
    #[test]
    fn catalogue_idle_and_repeated_identical_queries_publish_nothing_and_an_answer_publishes_once() {
        use crate::api::SearchResults;
        use crate::command::CatalogCmd;
        use crate::state::catalog::{self, Done as CatalogDone, Job as CatalogJob};

        let mut state = CoreState::default();
        let mut snapshot = state.snapshot();
        let run = |state: &mut CoreState, input: Input| state.apply(input, Instant::now());
        let typed = |text: &str| Input::Command(Command::Catalog(CatalogCmd::SearchInput(text.into())));
        assert!(state.publish(&snapshot).is_none(), "idle");

        // Writing arms the wait, but does not change the view; the same text again, not even the wait.
        let effects = run(&mut state, typed("low"));
        assert!(effects.iter().any(|e| matches!(e, Effect::Timer { key: TimerKey::Catalog(_), .. })));
        assert!(state.publish(&snapshot).is_none(), "typing alone publishes nothing");
        assert!(run(&mut state, typed("low")).is_empty() && state.publish(&snapshot).is_none(), "same text");

        // The wait expires: one publish, of the catalog and of what follows from it (the history).
        let effects = run(&mut state, Input::Timer(TimerKey::Catalog(catalog::Timer::Search)));
        let ticket = effects
            .iter()
            .find_map(|e| match e {
                Effect::Run(effect::Job::Catalog(CatalogJob::Search { ticket, .. })) => Some(*ticket),
                _ => None,
            })
            .expect("the search starts");
        let next = state.publish(&snapshot).expect("the search starts");
        assert_eq!((next.versions.catalog, next.versions.library), (1, 1));
        assert_eq!(next.library.search_history.first().map(String::as_str), Some("low"));
        assert_eq!(next.settings.config.last_query, "low", "the saved query follows through the settings");
        assert!(Arc::ptr_eq(&next.playback, &snapshot.playback));
        snapshot = next;

        // A real reply: one publish, and only of the catalog. A superseded one: none.
        let answer = |ticket| {
            Input::Done(Done::Catalog(CatalogDone::Search {
                ticket,
                result: Ok(Box::<SearchResults>::default()),
            }))
        };
        run(&mut state, answer(ticket - 1));
        assert!(state.publish(&snapshot).is_none(), "a late answer is dropped");
        run(&mut state, answer(ticket));
        let next = state.publish(&snapshot).expect("an answer is a change");
        assert_eq!((next.versions.catalog, next.versions.library), (2, 1));
        assert!(matches!(next.catalog.search.results, crate::catalog::Load::Ready(_)));
        snapshot = next;

        // The same text again and the core stays fully idle.
        assert!(run(&mut state, typed("low")).is_empty());
        assert!(state.publish(&snapshot).is_none());
    }

    /// The recent tracks of the Home reach the library with the passing, and the Home shows them
    /// with their feed in the same snapshot; the artists in view are loaded in advance.
    #[test]
    fn the_home_and_its_feed_arrive_in_one_snapshot() {
        use crate::api::{Page, Track};
        use crate::command::CatalogCmd;
        use crate::state::catalog::{Done as CatalogDone, Job as CatalogJob, Resource, Section};

        let mut state = CoreState::default();
        let snapshot = state.snapshot();
        let effects = command(&mut state, Command::Catalog(CatalogCmd::Ensure(Resource::Home)));
        let epoch = effects
            .iter()
            .find_map(|e| match e {
                Effect::Run(effect::Job::Catalog(CatalogJob::Home { epoch, .. })) => Some(*epoch),
                _ => None,
            })
            .expect("the home loads");
        let loading = state.publish(&snapshot).expect("loading");
        assert!(loading.catalog.home.recent.is_loading());

        let recent: Vec<Track> = (1..=6).map(track).collect();
        let page = Page { items: recent, limit: 100, offset: 0, next_offset: None };
        let effects = state.apply(
            Input::Done(Done::Catalog(CatalogDone::Home {
                epoch,
                section: Section::Recent,
                result: Ok(page),
            })),
            Instant::now(),
        );
        let next = state.publish(&loading).expect("the tracks arrived");
        let crate::catalog::Load::Ready(shown) = &next.catalog.home.recent else { panic!("not ready") };
        assert!(
            next.library.feed_source.as_ref().is_some_and(|source| Arc::ptr_eq(source, shown)),
            "the feed is built from the very tracks the home shows"
        );
        assert!(!next.library.feed.releases.is_empty());
        let prefetched = effects
            .iter()
            .filter(|e| matches!(e, Effect::Run(effect::Job::Catalog(CatalogJob::Artist { .. }))))
            .count();
        assert!(prefetched > 0, "the artists on the home are loaded ahead: {effects:?}");

        // The catalog changes: the Home has to be loaded again and the library drops the feed.
        command(&mut state, Command::Catalog(CatalogCmd::Changed));
        let after = state.publish(&next).expect("changed");
        assert!(after.catalog.home.recent.needs_load());
        assert!(after.library.feed_source.is_none() && after.library.feed.releases.is_empty());
    }

    /// A track the server no longer has: playback says so, the catalog finds it again, and the new one
    /// takes the old one's place in the queue and among the saved, without the frontend doing anything.
    #[test]
    fn a_missing_track_is_healed_inside_the_core() {
        use std::sync::Arc;

        use crate::api::Track;
        use crate::command::PlaybackCmd;
        use crate::error::AppError;
        use crate::state::catalog::{Done as CatalogDone, Job as CatalogJob};
        use crate::state::playback::Done as PlaybackDone;

        let mut state = CoreState::default();
        let missing = track(1);
        command(&mut state, like(1));
        let tracks: Arc<[Track]> = vec![missing.clone(), track(2)].into();
        command(&mut state, Command::Playback(PlaybackCmd::Play { tracks, start: 0 }));
        let generation = state.playback.view.generation;
        let not_found =
            AppError::Api { status: 404, code: "not_found".into(), message: "Brano non trovato".into() };
        let effects = state.apply(
            Input::Done(Done::Playback(PlaybackDone::Loaded {
                generation,
                result: Err(not_found),
                lost: None,
            })),
            Instant::now(),
        );
        assert!(
            effects.iter().any(|e| matches!(e, Effect::Run(effect::Job::Catalog(CatalogJob::Heal { track, .. })) if track.id == missing.id)),
            "the catalogue looks for it: {effects:?}"
        );

        let found = track(9);
        let effects = state.apply(
            Input::Done(Done::Catalog(CatalogDone::Healed {
                old: missing.id,
                result: Ok(Box::new(found.clone())),
            })),
            Instant::now(),
        );
        assert_eq!(state.playback.view.queue.current().map(|t| t.id), Some(found.id));
        assert_eq!(state.library.view.liked.first().map(|t| t.id), Some(found.id));
        assert!(
            effects.iter().any(|e| matches!(e, Effect::Run(effect::Job::Playback(playback::Job::Load { track_id, .. })) if *track_id == found.id)),
            "and it starts again: {effects:?}"
        );

        // If it does not find it again, playback stays in error.
        let mut state = CoreState::default();
        let tracks: Arc<[Track]> = vec![missing.clone()].into();
        command(&mut state, Command::Playback(PlaybackCmd::Play { tracks, start: 0 }));
        let generation = state.playback.view.generation;
        let not_found =
            AppError::Api { status: 404, code: "not_found".into(), message: "Brano non trovato".into() };
        state.apply(
            Input::Done(Done::Playback(PlaybackDone::Loaded {
                generation,
                result: Err(not_found),
                lost: None,
            })),
            Instant::now(),
        );
        state.apply(
            Input::Done(Done::Catalog(CatalogDone::Healed {
                old: missing.id,
                result: Err(AppError::InvalidResponse),
            })),
            Instant::now(),
        );
        assert!(matches!(state.playback.view.state, crate::player::PlaybackState::Error(_)));
    }

    /// The social jobs the effects ask for, in order.
    fn social_work(effects: &[Effect]) -> Vec<crate::testing::SocialWork> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Run(effect::Job::Social(job)) => Some(crate::testing::SocialWork::of(job)),
                _ => None,
            })
            .collect()
    }

    fn adopt(state: &mut CoreState) -> Vec<Effect> {
        let clerk = state.social.clerk_app().expect("la configurazione di default ha un Clerk");
        let me = crate::api::Me {
            user_id: "user_prova".into(),
            admin: false,
            profile: Some(crate::api::social::MyProfile {
                handle: "prova".into(),
                display_name: "Prova".into(),
                avatar_url: None,
                share_activity: true,
            }),
        };
        command(
            state,
            Command::Social(crate::command::SocialCmd::Adopt {
                session: crate::auth::Session::for_tests(clerk),
                me,
            }),
        )
    }

    /// Step 4.7, snapshot versions: a core without access, even with playback in
    /// motion, never publishes the social domain; a real social change (the access starting)
    /// only raises its version, and playback does not raise its own.
    #[test]
    fn the_social_domain_bumps_its_own_version_only_on_real_changes() {
        let mut state = CoreState::new(Config::default(), None, 1);
        let start = state.snapshot();
        let effects = state.apply(Input::Start, Instant::now());
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::Run(_) | Effect::Timer { .. } | Effect::OpenUrl(_))),
            "un core senza accesso non lancia lavori né arma timer: {effects:?}"
        );
        assert!(state.publish(&start).is_none());

        // Playback changes, social does not: without access `sync` does nothing.
        command(&mut state, Command::Playback(PlaybackCmd::Play { tracks: vec![track(1)].into(), start: 0 }));
        let playing = state.publish(&start).expect("la riproduzione è partita");
        assert_eq!(playing.versions.social, 0);
        assert!(Arc::ptr_eq(&playing.social, &start.social));

        // A social command that changes nothing does not publish.
        command(&mut state, Command::Social(crate::command::SocialCmd::DismissError));
        command(&mut state, Command::Social(crate::command::SocialCmd::RefreshFriends));
        assert!(state.publish(&playing).is_none());

        // The access starting is a real change: "sign-in in progress", and that is all.
        let effects = command(
            &mut state,
            Command::Social(crate::command::SocialCmd::SignIn {
                identifier: "psy".into(),
                password: crate::auth::Secret::new("una-password"),
            }),
        );
        assert_eq!(social_work(&effects), [crate::testing::SocialWork::SignIn]);
        let working = state.publish(&playing).expect("accesso in corso");
        assert_eq!(working.versions.social, 1);
        assert_eq!(working.versions.playback, playing.versions.playback);
        assert_eq!(working.social.account, social::Account::Working);
        assert!(state.publish(&working).is_none(), "pubblicato una volta");
    }

    /// The passing between domains brings playback and the library to the profile: while a track
    /// loads no presence starts (BACKLOG 9), and a playlist made public starts by itself.
    #[test]
    fn the_handoff_feeds_the_profile_from_playback_and_library() {
        let mut state = CoreState::new(Config::default(), None, 1);
        adopt(&mut state);

        let effects = command(
            &mut state,
            Command::Playback(PlaybackCmd::Play { tracks: vec![track(1)].into(), start: 0 }),
        );
        assert!(matches!(state.playback.view.state, crate::player::PlaybackState::Loading { .. }));
        assert!(
            !social_work(&effects).iter().any(|w| matches!(w, crate::testing::SocialWork::Presence { .. })),
            "mentre si carica non si dice niente: {effects:?}"
        );

        let playlist = crate::config::Playlist::new("Sera", vec![track(2)]);
        let id = playlist.id;
        command(
            &mut state,
            Command::library(LibraryCmd::CreatePlaylist {
                id,
                name: playlist.name,
                tracks: playlist.tracks,
                pending: Vec::new(),
            }),
        );
        let effects =
            command(&mut state, Command::library(LibraryCmd::SetPlaylistPublic { id, public: true }));
        assert_eq!(
            social_work(&effects),
            [crate::testing::SocialWork::PublishPlaylist {
                id,
                name: "Sera".into(),
                tracks: vec![track(2).id]
            }]
        );
        // A command that does not concern it returns nothing.
        let effects = command(&mut state, Command::library(LibraryCmd::RefreshFeed));
        assert!(social_work(&effects).is_empty());
    }

    /// Step 4.8, snapshot versions: an idle jam publishes nothing and arms no timer
    /// (the core has no timer for it: it is the room that wakes it); each real change is a
    /// single publish, with the version of the domains touched only; repeating what is already there does not
    /// cambia nulla.
    #[test]
    fn an_idle_jam_arms_nothing_and_publishes_nothing_and_a_remote_change_publishes_once() {
        use crate::command::JamCmd;
        use crate::jam::{Event as RoomEvent, Member, Remote, Snapshot as Shared};

        let mut state = CoreState::new(Config::default(), None, 1);
        let start = state.snapshot();
        let effects = state.apply(Input::Start, Instant::now());
        assert!(effects.iter().all(|e| !matches!(e, Effect::Jam(_))));
        assert!(state.publish(&start).is_none(), "idle");
        let timers =
            |effects: &[Effect]| effects.iter().filter(|e| matches!(e, Effect::Timer { .. })).count();
        let run = |state: &mut CoreState, input: Input| state.apply(input, Instant::now());

        // Without a jam: leaving, or an event from a room that does not exist, change nothing.
        let effects = run(&mut state, Input::Command(Command::Jam(JamCmd::Leave)));
        assert!(effects.is_empty());
        let effects = run(&mut state, Input::Jam(RoomEvent::Closed("Jam chiusa".into())));
        assert!(effects.is_empty() && state.publish(&start).is_none());

        // Creating the room: one publish, of the jam only; the link starts, no timer.
        let effects = run(&mut state, Input::Command(Command::Jam(JamCmd::Create { name: "Luca".into() })));
        assert!(matches!(effects.as_slice(), [Effect::Jam(crate::state::jam::JamOp::Start { .. })]));
        let connecting = state.publish(&start).expect("la jam parte");
        assert_eq!(connecting.versions, DomainVersions { jam: 1, ..DomainVersions::default() });
        assert!(Arc::ptr_eq(&connecting.playback, &start.playback), "il gruppo non è nella vista");

        // The room answers without state: one publish, of the jam only; no send, no timer.
        let members = vec![Member { id: uuid::Uuid::from_u128(77), name: "Luca".into() }];
        let joined = RoomEvent::Joined {
            code: "K7QXM3".into(),
            you: uuid::Uuid::from_u128(77),
            members: members.clone(),
            state: None,
        };
        let effects = run(&mut state, Input::Jam(joined));
        assert!(effects.is_empty(), "una jam ferma non ha niente da fare: {effects:?}");
        let open = state.publish(&connecting).expect("in stanza");
        assert_eq!((open.versions.jam, open.versions.playback), (2, 0));
        assert_eq!(open.jam.code.as_deref(), Some("K7QXM3"));

        // The same members again, a state equal to nothing: no publish and no effect.
        let effects = run(&mut state, Input::Jam(RoomEvent::Members(members)));
        assert!(effects.is_empty() && state.publish(&open).is_none(), "la stessa lista non è un cambio");
        assert_eq!(timers(&effects), 0);

        // The others' state with an empty queue: the queue is already empty, nothing to change. With tracks
        // already queued, instead, it is a single round: playback aligns (one publish, two
        // domains), and receiving it again identical returns nothing to the room.
        let empty = Remote {
            snapshot: Shared { queue: vec![], index: 0, position_ms: 0, playing: false },
            from: "Luca".into(),
        };
        run(&mut state, Input::Jam(RoomEvent::State(empty)));
        assert!(state.publish(&open).is_none(), "una coda vuota su una coda vuota");

        let known = [track(1), track(2)];
        command(&mut state, Command::Playback(PlaybackCmd::Enqueue(Box::new(known[0].clone()))));
        command(&mut state, Command::Playback(PlaybackCmd::Enqueue(Box::new(known[1].clone()))));
        let queued = state.publish(&open).expect("la coda è cambiata");
        let remote = Remote {
            snapshot: Shared {
                queue: vec![known[0].id, known[1].id],
                index: 1,
                position_ms: 9_000,
                playing: true,
            },
            from: "Luca".into(),
        };
        let effects = run(&mut state, Input::Jam(RoomEvent::State(remote.clone())));
        assert!(!effects.iter().any(|e| matches!(e, Effect::Jam(_))), "lo stato ricevuto non si rimanda");
        let aligned = state.publish(&queued).expect("la riproduzione si allinea in un giro solo");
        assert_eq!(aligned.versions.playback, queued.versions.playback + 1);
        assert_eq!(aligned.versions.jam, queued.versions.jam + 1, "l'avviso «ha cambiato brano»");
        assert_eq!(aligned.versions.library, queued.versions.library);
        assert_eq!(aligned.versions.social, queued.versions.social);
        assert!(state.publish(&aligned).is_none(), "pubblicato una volta");

        // The same state again (the tick of whoever plays): playback already aligned changes
        // nothing and the room receives nothing; no timer.
        let effects = run(&mut state, Input::Jam(RoomEvent::State(remote)));
        assert!(!effects.iter().any(|e| matches!(e, Effect::Jam(_) | Effect::Timer { .. })), "{effects:?}");
        assert!(state.publish(&aligned).is_none());

        // Leaving: one publish, the link closes.
        let effects = run(&mut state, Input::Command(Command::Jam(JamCmd::Leave)));
        assert!(matches!(effects.as_slice(), [Effect::Jam(crate::state::jam::JamOp::Leave)]));
        let left = state.publish(&aligned).expect("uscito");
        assert!(!left.jam.is_active());
        assert!(state.publish(&left).is_none());
    }

    /// The passing between domains brings the profile to the jam (the in-room name of whoever does not write
    /// one) and the room to the social domain (the invitations go out with its code, and the one to
    /// one's own room is not announced): the frontend does not act as go-between.
    #[test]
    fn the_handoff_gives_the_jam_the_profile_name_and_the_social_domain_the_room() {
        use crate::command::{JamCmd, SocialCmd};
        use crate::jam::{Event as RoomEvent, Member, Target};
        use crate::state::jam::JamOp;

        let mut state = CoreState::new(Config::default(), None, 1);
        adopt(&mut state);

        // Invitations without a room: "wait until the jam is ready".
        let effects = command(&mut state, Command::Social(SocialCmd::InviteToJam(vec!["luca".into()])));
        assert!(social_work(&effects).is_empty());
        assert!(matches!(
            state.social.view.events.back().map(|raised| &raised.event),
            Some(social::SocialEvent::JamNotReady)
        ));

        // Without a name written, in the room one introduces oneself with the profile's name.
        let effects = command(&mut state, Command::Jam(JamCmd::Create { name: String::new() }));
        assert!(matches!(
            effects.as_slice(),
            [Effect::Jam(JamOp::Start { target: Target::Create, name, .. })] if name == "Prova"
        ));
        // The code is not there yet: the invitations wait.
        let before = state.social.view.events.len();
        command(&mut state, Command::Social(SocialCmd::InviteToJam(vec!["luca".into()])));
        assert_eq!(state.social.view.events.len(), before + 1);

        // The room says its code: the invitations go out with that one.
        let members = vec![Member { id: uuid::Uuid::from_u128(77), name: "Prova".into() }];
        let joined =
            RoomEvent::Joined { code: "K7QXM3".into(), you: uuid::Uuid::from_u128(77), members, state: None };
        state.apply(Input::Jam(joined), Instant::now());
        let effects = command(&mut state, Command::Social(SocialCmd::InviteToJam(vec!["luca".into()])));
        assert_eq!(
            social_work(&effects),
            [crate::testing::SocialWork::InviteToJam { code: "K7QXM3".into(), handles: vec!["luca".into()] }]
        );
        // The invitation to one's own room is not announced; one to another is.
        let invite = |code: &str| crate::api::social::JamInvite {
            code: code.into(),
            from_handle: "luca".into(),
            from_name: "LUCA".into(),
        };
        let announced = state.social.view.events.len();
        state.apply(
            Input::Done(Done::Social(social::Done::Invites(Ok(vec![invite("K7QXM3")])))),
            Instant::now(),
        );
        assert_eq!(state.social.view.events.len(), announced, "è la mia stanza");
        state.apply(
            Input::Done(Done::Social(social::Done::Invites(Ok(vec![invite("K7QXM3"), invite("ZZZ999")])))),
            Instant::now(),
        );
        assert_eq!(state.social.view.events.len(), announced + 1);

        // Leaving: the social domain forgets the room (the invitations say "not ready" again).
        command(&mut state, Command::Jam(JamCmd::Leave));
        let before = state.social.view.events.len();
        command(&mut state, Command::Social(SocialCmd::InviteToJam(vec!["luca".into()])));
        assert_eq!(state.social.view.events.len(), before + 1);
        assert!(matches!(
            state.social.view.events.back().map(|raised| &raised.event),
            Some(social::SocialEvent::JamNotReady)
        ));
    }

    /// The backend chosen in the settings reaches the social domain too.
    #[test]
    fn the_server_choice_reaches_the_social_domain() {
        let mut state = CoreState::new(Config::default(), None, 1);
        let before = state.social.api_base();
        let mut config = state.settings.view.config.as_ref().clone();
        config.server = Some("https://altro.example".into());
        command(&mut state, Command::Settings(SettingsCmd::Store(Box::new(config))));
        assert_ne!(state.social.api_base(), before);
    }
}
