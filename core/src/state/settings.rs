//! Settings and saving of `config.json` (step 4.1). The core is the only one that writes the file: the
//! frontends send the changes, the domain keeps them in its view and writes them at the latest after
//! [`SAVE_DELAY`], one write at a time and always with the most recent state. At closing it waits for
//! the write in progress, writes at once what is missing, and only then lets the core stop.
//!
//! The fields of the domains still in the frontend all arrive together with [`SettingsCmd::Store`]; a
//! domain that moves into the core hands them over itself (the library, with [`Settings::store_library`],
//! playback with [`Settings::store_playback`], the catalog with [`Settings::store_query`], the
//! downloads with [`Settings::store_downloads`]) and the frontend stops sending them: `Store` no longer touches
//! the fields that a core domain owns.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::config::{self, Config, CoreConfig, DownloadsFields, LibraryFields, PlaybackFields};
use crate::effect::{self, Effect, TimerKey};
use crate::error::AppError;
use crate::system_motion::SystemMotion;

/// How long a change waits before reaching the disk: those that arrive meanwhile go into the
/// same write. A new change does not postpone the write already waiting.
pub(crate) const SAVE_DELAY: Duration = Duration::from_millis(500);

/// The domain's commands.
#[derive(Debug, Clone)]
pub enum SettingsCmd {
    /// The core's fields as the frontend keeps them, until their domains move into the core
    /// (steps 4.3-4.9). They are written only if they have changed. The library's fields (step 4.2) are
    /// kept by its domain: those of this value are ignored.
    Store(Box<CoreConfig>),
    /// A frontend's preferences: each key replaces the saved one, `null` removes it, the
    /// altre restano ([`config::merge_ui`]).
    SaveUiPrefs(Map<String, Value>),
    /// Listens to (or stops listening to) what the system asks about motion: the frontend
    /// turns it on when its preference follows the system. The reading arrives in
    /// [`SettingsView::system_motion`]; off, the host closes the link and the view goes back to
    /// «nessuna preferenza».
    WatchSystemMotion(bool),
    /// Writes at once what waits for [`SAVE_DELAY`], without stopping the core: the frontend sends it
    /// when the app goes to the background (Android can close it at any moment, BACKLOG 12). Without
    /// changes it does nothing; with a write in progress, the next starts as soon as it ends.
    SaveNow,
}

/// What the frontends see of the domain. It is published only when a value really changes.
#[derive(Debug, Clone, Default)]
pub struct SettingsView {
    /// I campi del core in `config.json`: quelli letti all'avvio, poi quelli dell'ultimo
    /// [`SettingsCmd::Store`]. The domains still in the frontend start from here.
    pub config: Arc<CoreConfig>,
    /// The other keys of the file: the frontends' preferences and those this version does not know.
    pub ui: Arc<Map<String, Value>>,
    /// Why the last write did not succeed; `None` after a successful one. Meanwhile the file stays
    /// the previous one, whole, and it is retried at the next change and at closing.
    pub save_error: Option<AppError>,
    /// What the system asks about motion (accessibility, power saving), for as long as a
    /// frontend listens with [`SettingsCmd::WatchSystemMotion`]. It is not saved: it belongs to the system.
    pub system_motion: SystemMotion,
}

/// The domain's timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {
    /// Time to write the collected changes.
    Save,
}

/// The domain's async jobs.
#[derive(Debug)]
pub(crate) enum Job {
    /// Scrive `config.json` in `path` ([`config::write`]), direttamente dalla vista: niente copie.
    Write { path: PathBuf, core: Arc<CoreConfig>, ui: Arc<Map<String, Value>> },
}

/// The outcome of a [`Job`].
#[derive(Debug)]
pub(crate) enum Done {
    Written(Result<(), AppError>),
}

/// Where the closing is, for the save.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Closing {
    /// The core is running.
    #[default]
    No,
    /// The core is stopping: it waits for the outcome of the write started before.
    Requested,
    /// The write in progress is the last: if it fails, the core stops all the same and says so.
    LastWrite,
}

/// The domain's state: the published view and what stays inside the core.
#[derive(Debug, Default)]
pub(crate) struct Settings {
    pub(crate) view: Arc<SettingsView>,
    /// Where `config.json` sits. Without it, the configuration stays in memory and is never written (the
    /// tests, a frontend without a data folder).
    path: Option<PathBuf>,
    /// The view has changes the file does not have.
    dirty: bool,
    /// The timer [`Timer::Save`] is armed.
    scheduled: bool,
    /// A write is in progress. The next starts only after its outcome: two writes do not
    /// overlap, and two never use `config.json.tmp` at the same time.
    writing: bool,
    closing: Closing,
    /// A frontend listens to the motion requested by the system: the host holds the link.
    watching_motion: bool,
    /// [`SettingsCmd::SaveNow`] arrived during a write: what remains is written at its
    /// outcome, without waiting for the timer.
    hurry: bool,
}

impl Settings {
    /// The domain with the configuration read at startup, to be rewritten in `path`.
    pub(crate) fn new(config: Config, path: Option<PathBuf>) -> Self {
        let view = SettingsView {
            config: Arc::new(config.core),
            ui: Arc::new(config.ui),
            save_error: None,
            system_motion: SystemMotion::default(),
        };
        Self { view: Arc::new(view), path, ..Self::default() }
    }

    pub(crate) fn apply(&mut self, command: SettingsCmd, _now: Instant, effects: &mut Vec<Effect>) {
        match command {
            SettingsCmd::Store(core) => {
                let mut core = *core;
                core.set_library(self.view.config.library());
                core.set_playback(self.view.config.playback());
                core.set_downloads(self.view.config.downloads());
                core.last_query.clone_from(&self.view.config.last_query);
                self.store(core, effects);
            }
            SettingsCmd::SaveUiPrefs(prefs) => {
                if let Some(ui) = config::merge_ui(&self.view.ui, prefs) {
                    Arc::make_mut(&mut self.view).ui = Arc::new(ui);
                    self.changed(effects);
                }
            }
            SettingsCmd::SaveNow => self.save_now(effects),
            SettingsCmd::WatchSystemMotion(on) => {
                if on == self.watching_motion {
                    return;
                }
                self.watching_motion = on;
                effects.push(Effect::WatchMotion(on));
                if !on && self.view.system_motion != SystemMotion::default() {
                    Arc::make_mut(&mut self.view).system_motion = SystemMotion::default();
                }
            }
        }
    }

    /// A reading from the system. It is published only if it changes, and only while someone listens: a
    /// reading that arrives after switching off is thrown away.
    pub(crate) fn system_motion(&mut self, motion: SystemMotion) {
        if self.watching_motion && self.view.system_motion != motion {
            Arc::make_mut(&mut self.view).system_motion = motion;
        }
    }

    /// Replaces the core's fields, if they have changed: equal, nothing is published or written.
    pub(crate) fn store(&mut self, core: CoreConfig, effects: &mut Vec<Effect>) {
        if *self.view.config != core {
            Arc::make_mut(&mut self.view).config = Arc::new(core);
            self.changed(effects);
        }
    }

    /// Replaces the library's fields, if they have changed. The comparison looks at the pointers of the
    /// lists (`Shared`), not the elements, and the configuration's `Arc` is copied only then.
    pub(crate) fn store_library(&mut self, fields: LibraryFields, effects: &mut Vec<Effect>) {
        if self.view.config.library() != fields {
            Arc::make_mut(&mut Arc::make_mut(&mut self.view).config).set_library(fields);
            self.changed(effects);
        }
    }

    /// Replaces the download fields, if they have changed.
    pub(crate) fn store_downloads(&mut self, fields: DownloadsFields, effects: &mut Vec<Effect>) {
        if self.view.config.downloads() != fields {
            Arc::make_mut(&mut Arc::make_mut(&mut self.view).config).set_downloads(fields);
            self.changed(effects);
        }
    }

    /// Replaces the text searched last, if it has changed.
    pub(crate) fn store_query(&mut self, query: String, effects: &mut Vec<Effect>) {
        if self.view.config.last_query != query {
            Arc::make_mut(&mut Arc::make_mut(&mut self.view).config).last_query = query;
            self.changed(effects);
        }
    }

    /// Replaces the playback fields, if they have changed (the queue is compared by pointer).
    pub(crate) fn store_playback(&mut self, fields: PlaybackFields, effects: &mut Vec<Effect>) {
        if self.view.config.playback() != fields {
            Arc::make_mut(&mut Arc::make_mut(&mut self.view).config).set_playback(fields);
            self.changed(effects);
        }
    }

    pub(crate) fn fire(&mut self, timer: Timer, _now: Instant, effects: &mut Vec<Effect>) {
        match timer {
            Timer::Save => {
                self.scheduled = false;
                self.write(effects);
            }
        }
    }

    pub(crate) fn done(&mut self, done: Done, _now: Instant, effects: &mut Vec<Effect>) {
        match done {
            Done::Written(result) => {
                self.writing = false;
                if result.is_err() {
                    // The file does not have these changes: it is retried at the next one or at closing, never
                    // on its own to nothing while the app is idle.
                    self.dirty = true;
                }
                let error = result.as_ref().err();
                if self.view.save_error.as_ref() != error {
                    Arc::make_mut(&mut self.view).save_error = error.cloned();
                }
                match self.closing {
                    // Asked for right during the write: what has changed meanwhile is written now.
                    Closing::No if std::mem::take(&mut self.hurry) && self.dirty && result.is_ok() => {
                        self.write(effects);
                    }
                    Closing::No if self.dirty && result.is_ok() => self.schedule(effects),
                    Closing::No => {}
                    Closing::LastWrite if result.is_err() => effects.push(Effect::Flushed(result)),
                    Closing::Requested | Closing::LastWrite => self.flush(effects),
                }
            }
        }
    }

    /// The core stops: no more waits. What is missing is written at once, after the outcome of the
    /// write in progress if there is one; then [`Effect::Flushed`] lets the core stop.
    pub(crate) fn close(&mut self, effects: &mut Vec<Effect>) {
        if std::mem::take(&mut self.scheduled) {
            effects.push(Effect::CancelTimer(TimerKey::Settings(Timer::Save)));
        }
        self.closing = Closing::Requested;
        if !self.writing {
            self.flush(effects);
        }
    }

    /// Writes now what the timer awaits: the timer is no longer needed. With a write in progress, it does
    /// [`Settings::done`] at its outcome. During closing [`Settings::close`] already takes care of it.
    fn save_now(&mut self, effects: &mut Vec<Effect>) {
        if self.closing != Closing::No || !self.dirty {
            return;
        }
        if std::mem::take(&mut self.scheduled) {
            effects.push(Effect::CancelTimer(TimerKey::Settings(Timer::Save)));
        }
        if self.writing {
            self.hurry = true;
        } else {
            self.write(effects);
        }
    }

    /// At closing, with no writes in progress: the last write, if something is missing, or the end.
    fn flush(&mut self, effects: &mut Vec<Effect>) {
        if self.write(effects) {
            self.closing = Closing::LastWrite;
        } else {
            effects.push(Effect::Flushed(Ok(())));
        }
    }

    /// The view has changed: the file must be rewritten.
    fn changed(&mut self, effects: &mut Vec<Effect>) {
        if self.path.is_some() {
            self.dirty = true;
            self.schedule(effects);
        }
    }

    /// Arms the timer, if it is not already armed and no write is in progress: in that case it re-arms
    /// [`Settings::done`], if something has changed meanwhile. During closing it is not needed.
    fn schedule(&mut self, effects: &mut Vec<Effect>) {
        if self.scheduled || self.writing || self.closing != Closing::No {
            return;
        }
        self.scheduled = true;
        effects.push(Effect::Timer { key: TimerKey::Settings(Timer::Save), after: SAVE_DELAY });
    }

    /// Writes now, if there is something to write and no write is in progress. Says whether one started.
    fn write(&mut self, effects: &mut Vec<Effect>) -> bool {
        let Some(path) = &self.path else { return false };
        if self.writing || !self.dirty {
            return false;
        }
        self.dirty = false;
        self.writing = true;
        let job = Job::Write {
            path: path.clone(),
            core: Arc::clone(&self.view.config),
            ui: Arc::clone(&self.view.ui),
        };
        effects.push(Effect::Run(effect::Job::Settings(job)));
        true
    }
}

impl Job {
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Write { path, core, ui } => Done::Written(config::write(&path, &core, &ui).await),
        }
    }

    /// If the job is lost, the file was not written.
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Write { path, .. } => Done::Written(Err(AppError::Io(path.display().to_string()))),
        }
    }
}

/// This domain's step 0 (`saves_do_not_overlap_and_the_last_state_wins`,
/// `close_flushes_before_exit`), brought to the reducer: the time is fake, the timers expire when
/// the test says and the writes finish when the test hands over their outcome.
#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings::new(Config::default(), Some(PathBuf::from("/dati/psytify/config.json")))
    }

    fn query(text: &str) -> SettingsCmd {
        SettingsCmd::Store(Box::new(CoreConfig { clerk_client_id: text.to_owned(), ..CoreConfig::default() }))
    }

    fn prefs(value: Value) -> SettingsCmd {
        match value {
            Value::Object(map) => SettingsCmd::SaveUiPrefs(map),
            other => panic!("not an object: {other}"),
        }
    }

    fn send(settings: &mut Settings, commands: impl IntoIterator<Item = SettingsCmd>) -> Vec<Effect> {
        let mut effects = Vec::new();
        for command in commands {
            settings.apply(command, Instant::now(), &mut effects);
        }
        effects
    }

    /// The system's motion: listening is turned on and off once, a reading changes
    /// the view only if it is different and only while listening, and none of this is saved.
    #[test]
    fn the_system_motion_is_published_only_on_a_real_change_while_watched() {
        let reduced = SystemMotion { reduced_motion: Some(false), power_saver: Some(true) };
        let mut settings = settings();
        let start = Arc::clone(&settings.view);

        settings.system_motion(reduced);
        assert!(Arc::ptr_eq(&start, &settings.view), "nessuno ascolta: la lettura si butta");

        let effects =
            send(&mut settings, [SettingsCmd::WatchSystemMotion(true), SettingsCmd::WatchSystemMotion(true)]);
        assert!(matches!(effects[..], [Effect::WatchMotion(true)]), "{effects:?}");
        assert!(Arc::ptr_eq(&start, &settings.view), "accendere non cambia la vista");

        settings.system_motion(reduced);
        assert_eq!(settings.view.system_motion, reduced);
        let published = Arc::clone(&settings.view);
        settings.system_motion(reduced);
        assert!(Arc::ptr_eq(&published, &settings.view), "la stessa lettura non si ripubblica");
        assert!(!settings.dirty && !settings.scheduled, "il movimento del sistema non si salva");

        let effects = send(&mut settings, [SettingsCmd::WatchSystemMotion(false)]);
        assert!(matches!(effects[..], [Effect::WatchMotion(false)]), "{effects:?}");
        assert_eq!(settings.view.system_motion, SystemMotion::default(), "spento: nessuna preferenza");
        settings.system_motion(reduced);
        assert_eq!(settings.view.system_motion, SystemMotion::default(), "una lettura in ritardo");
    }

    fn fire(settings: &mut Settings) -> Vec<Effect> {
        let mut effects = Vec::new();
        settings.fire(Timer::Save, Instant::now(), &mut effects);
        effects
    }

    fn written(settings: &mut Settings, result: Result<(), AppError>) -> Vec<Effect> {
        let mut effects = Vec::new();
        settings.done(Done::Written(result), Instant::now(), &mut effects);
        effects
    }

    fn close(settings: &mut Settings) -> Vec<Effect> {
        let mut effects = Vec::new();
        settings.close(&mut effects);
        effects
    }

    fn io_error() -> AppError {
        AppError::Io("/dati/psytify/config.json".into())
    }

    /// The last searches that the effects write to disk, in order.
    fn writes(effects: &[Effect]) -> Vec<String> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Run(effect::Job::Settings(Job::Write { core, .. })) => {
                    Some(core.clerk_client_id.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn arms_the_timer(effects: &[Effect]) -> bool {
        matches!(effects, [Effect::Timer { key: TimerKey::Settings(Timer::Save), after }] if *after == SAVE_DELAY)
    }

    #[test]
    fn changes_are_written_once_after_the_delay_with_the_last_state() {
        let mut settings = settings();
        assert!(arms_the_timer(&send(&mut settings, [query("a")])));
        // Other changes before the deadline: no new timer, the write is not postponed.
        assert!(send(&mut settings, [query("b"), query("c")]).is_empty());
        assert_eq!(writes(&fire(&mut settings)), ["c"]);
        assert!(written(&mut settings, Ok(())).is_empty(), "nothing left to write");
        assert_eq!(settings.view.config.clerk_client_id, "c");
    }

    #[test]
    fn an_unchanged_value_publishes_and_writes_nothing() {
        let mut settings = settings();
        let published = Arc::clone(&settings.view);
        let same = [SettingsCmd::Store(Box::default()), prefs(serde_json::json!({}))];
        assert!(send(&mut settings, same).is_empty());
        assert!(Arc::ptr_eq(&published, &settings.view), "same pointer: nothing to publish");

        assert!(arms_the_timer(&send(&mut settings, [prefs(serde_json::json!({"panel_open": true}))])));
        let published = Arc::clone(&settings.view);
        assert!(
            send(&mut settings, [prefs(serde_json::json!({"panel_open": true, "queue_open": null}))])
                .is_empty()
        );
        assert!(Arc::ptr_eq(&published, &settings.view));
    }

    #[test]
    fn a_change_during_a_write_waits_for_it_and_the_last_state_wins() {
        let mut settings = settings();
        send(&mut settings, [query("a")]);
        assert_eq!(writes(&fire(&mut settings)), ["a"]);
        // The write is in flight: no timer and no second write next to the first.
        assert!(send(&mut settings, [query("b"), query("c")]).is_empty());
        assert!(fire(&mut settings).is_empty(), "a stray timer starts no second write");
        // Once its outcome has arrived, one starts again with the state of now.
        assert!(arms_the_timer(&written(&mut settings, Ok(()))));
        assert_eq!(writes(&fire(&mut settings)), ["c"]);
        assert!(written(&mut settings, Ok(())).is_empty());
    }

    /// BACKLOG 12: the app goes to the background and Android can close it. `SaveNow` writes at once what
    /// waits for the timer (and turns it off), does nothing without changes, and during a write starts
    /// the next one at its outcome, without the timer.
    #[test]
    fn save_now_writes_what_waits_without_the_delay() {
        let mut settings = settings();
        assert!(send(&mut settings, [SettingsCmd::SaveNow]).is_empty(), "niente da scrivere");

        send(&mut settings, [query("a")]);
        let effects = send(&mut settings, [SettingsCmd::SaveNow]);
        assert!(
            matches!(effects.first(), Some(Effect::CancelTimer(TimerKey::Settings(Timer::Save)))),
            "{effects:?}"
        );
        assert_eq!(writes(&effects), ["a"]);
        assert!(fire(&mut settings).is_empty(), "il timer spento non scrive di nuovo");

        // A change and `SaveNow` while the write is in flight: the next starts at its outcome.
        assert!(send(&mut settings, [query("b"), SettingsCmd::SaveNow]).is_empty());
        assert_eq!(writes(&written(&mut settings, Ok(()))), ["b"], "subito, non dopo il timer");
        assert!(written(&mut settings, Ok(())).is_empty());
        assert!(send(&mut settings, [SettingsCmd::SaveNow]).is_empty(), "tutto scritto");

        // Afterwards, the changes go back to waiting for the timer.
        assert!(arms_the_timer(&send(&mut settings, [query("c")])));
    }

    #[test]
    fn a_failed_write_is_shown_and_retried_with_the_next_change() {
        let mut settings = settings();
        send(&mut settings, [query("a")]);
        fire(&mut settings);
        assert!(written(&mut settings, Err(io_error())).is_empty(), "no retry loop while idle");
        assert_eq!(settings.view.save_error, Some(io_error()));

        assert!(arms_the_timer(&send(&mut settings, [query("b")])));
        assert_eq!(writes(&fire(&mut settings)), ["b"]);
        written(&mut settings, Ok(()));
        assert_eq!(settings.view.save_error, None);
    }

    #[test]
    fn ui_prefs_merge_into_the_view_and_go_with_the_core_fields() {
        let mut config = Config::default();
        config.ui.insert("gpui_theme".into(), "dark".into());
        let mut settings = Settings::new(config, Some(PathBuf::from("/dati/psytify/config.json")));
        send(
            &mut settings,
            [prefs(serde_json::json!({"panel_open": true, "clerk_client_id": "z"})), query("d")],
        );
        let effects = fire(&mut settings);
        let [Effect::Run(effect::Job::Settings(Job::Write { core, ui, .. }))] = effects.as_slice() else {
            panic!("one write: {effects:?}");
        };
        assert_eq!(
            Value::Object((**ui).clone()),
            serde_json::json!({"gpui_theme": "dark", "panel_open": true})
        );
        assert_eq!(core.clerk_client_id, "d", "a ui key never overrides a core field");
    }

    #[test]
    fn close_cancels_the_wait_and_writes_at_once() {
        let mut settings = settings();
        send(&mut settings, [query("a")]);
        let effects = close(&mut settings);
        assert!(matches!(effects.first(), Some(Effect::CancelTimer(TimerKey::Settings(Timer::Save)))));
        assert_eq!(writes(&effects), ["a"]);
        assert!(matches!(written(&mut settings, Ok(())).as_slice(), [Effect::Flushed(Ok(()))]));
    }

    /// `docs/transformation/BACKLOG.md`, 7: closing does not write next to a write in progress.
    #[test]
    fn close_waits_for_the_write_in_flight_then_writes_the_rest() {
        let mut settings = settings();
        send(&mut settings, [query("a")]);
        fire(&mut settings);
        send(&mut settings, [query("b")]);
        assert!(close(&mut settings).is_empty(), "the first write is still in flight");
        assert_eq!(writes(&written(&mut settings, Ok(()))), ["b"]);
        assert!(matches!(written(&mut settings, Ok(())).as_slice(), [Effect::Flushed(Ok(()))]));
    }

    #[test]
    fn close_with_nothing_to_write_stops_at_once() {
        let mut settings = settings();
        assert!(matches!(close(&mut settings).as_slice(), [Effect::Flushed(Ok(()))]));
    }

    #[test]
    fn a_failed_last_write_is_reported_and_the_core_stops_anyway() {
        let mut settings = settings();
        send(&mut settings, [query("a")]);
        fire(&mut settings);
        close(&mut settings);
        // The write started before closing fails: one last attempt is made.
        assert_eq!(writes(&written(&mut settings, Err(io_error()))), ["a"]);
        // The last one fails too: the core stops and says why.
        let effects = written(&mut settings, Err(io_error()));
        assert!(
            matches!(effects.as_slice(), [Effect::Flushed(Err(err))] if *err == io_error()),
            "{effects:?}"
        );
    }

    #[test]
    fn without_a_path_nothing_is_ever_written() {
        let mut settings = Settings::new(Config::default(), None);
        assert!(send(&mut settings, [query("a")]).is_empty());
        assert_eq!(settings.view.config.clerk_client_id, "a", "the view still changes");
        assert!(matches!(close(&mut settings).as_slice(), [Effect::Flushed(Ok(()))]));
    }
}
