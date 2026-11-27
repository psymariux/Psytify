//! The bridge with a real core on its own thread: no audio device, no system integrations, and a
//! backend address where nothing listens.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use psytify_core::{AudioChoice, CoreOptions, RuntimeChoice, SystemChoice};

use super::*;

const WAIT: Duration = Duration::from_secs(10);

#[derive(Debug)]
enum Heard {
    Playback(PlaybackSnapshot),
    Home(HomeSnapshot),
    Failed(Option<String>),
    Account(AccountSnapshot),
    Settings(Box<SettingsSnapshot>),
    Lyrics(LyricsSnapshot),
    Video(VideoSnapshot),
}

struct Recorder(Mutex<Sender<Heard>>);

impl CoreListener for Recorder {
    fn playback_changed(&self, playback: PlaybackSnapshot) {
        if let Ok(tx) = self.0.lock() {
            let _ = tx.send(Heard::Playback(playback));
        }
    }

    fn home_changed(&self, home: HomeSnapshot) {
        if let Ok(tx) = self.0.lock() {
            let _ = tx.send(Heard::Home(home));
        }
    }

    fn now_playing(&self, _now: Option<NowPlaying>) {}

    fn notice(&self, _text: String, _error: bool) {}

    fn open(&self, _route: Route) {}

    fn core_failed(&self, reason: Option<String>) {
        if let Ok(tx) = self.0.lock() {
            let _ = tx.send(Heard::Failed(reason));
        }
    }

    fn account_changed(&self, account: AccountSnapshot) {
        if let Ok(tx) = self.0.lock() {
            let _ = tx.send(Heard::Account(account));
        }
    }

    fn open_browser(&self, _url: String) -> bool {
        false
    }

    fn downloads_changed(&self, _downloads: DownloadsSnapshot) {}

    fn marks_changed(&self, _marks: SocialMarks) {}

    fn settings_changed(&self, settings: SettingsSnapshot) {
        if let Ok(tx) = self.0.lock() {
            let _ = tx.send(Heard::Settings(Box::new(settings)));
        }
    }

    fn lyrics_changed(&self, lyrics: LyricsSnapshot) {
        if let Ok(tx) = self.0.lock() {
            let _ = tx.send(Heard::Lyrics(lyrics));
        }
    }

    fn video_changed(&self, video: VideoSnapshot) {
        if let Ok(tx) = self.0.lock() {
            let _ = tx.send(Heard::Video(video));
        }
    }

    fn install_update(&self, _path: String) {}

    fn update_offered(&self, _tag: String) {}
}

/// A data directory whose `config.json` points at a port where nothing listens.
fn data_dir() -> PathBuf {
    data_dir_with("")
}

/// The same, with more fields of `config.json` (`extra` starts with a comma).
fn data_dir_with(extra: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("psytify-mobile-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    std::fs::write(dir.join("config.json"), format!(r#"{{"server":"http://127.0.0.1:{port}"{extra}}}"#))
        .unwrap();
    dir
}

fn start(dir: &std::path::Path) -> (Arc<PsytifyCore>, Receiver<Heard>) {
    start_with(dir, Duration::from_secs(3600))
}

fn start_with(dir: &std::path::Path, idle_stop: Duration) -> (Arc<PsytifyCore>, Receiver<Heard>) {
    launch_with(dir, idle_stop, Core::start)
}

fn launch_with(
    dir: &std::path::Path,
    idle_stop: Duration,
    start: fn(CoreOptions) -> Result<Core, psytify_core::CoreError>,
) -> (Arc<PsytifyCore>, Receiver<Heard>) {
    let (tx, rx) = channel();
    let launch = Launch {
        start,
        options: CoreOptions {
            runtime: RuntimeChoice::Own { workers: 1 },
            audio: AudioChoice::None,
            system: SystemChoice::None,
            data_dir: Some(dir.to_path_buf()),
            downloads_dir: Some(dir.join("Download")),
        },
        system_media: false,
        idle_stop,
    };
    let core = PsytifyCore::launch(launch, Arc::new(Recorder(Mutex::new(tx)))).unwrap();
    (core, rx)
}

/// The next home snapshot that `accept` takes, skipping the others.
fn home_until(rx: &Receiver<Heard>, accept: impl Fn(&HomeSnapshot) -> bool) -> HomeSnapshot {
    loop {
        match rx.recv_timeout(WAIT) {
            Ok(Heard::Home(home)) if accept(&home) => return home,
            Ok(_) => {}
            Err(err) => panic!("no matching home snapshot: {err}"),
        }
    }
}

#[test]
fn the_first_state_of_each_domain_arrives_at_start() {
    let dir = data_dir();
    let (core, rx) = start(&dir);
    let mut playback = None;
    let mut home = None;
    let mut settings_seen = None;
    let (mut lyrics, mut video) = (None, None);
    while playback.is_none() || home.is_none() {
        match rx.recv_timeout(WAIT).unwrap() {
            Heard::Playback(p) => playback = Some(p),
            Heard::Home(h) => home = Some(h),
            Heard::Failed(reason) => panic!("unexpected failure {reason:?}"),
            Heard::Account(_) => {}
            Heard::Settings(settings) => settings_seen = Some(settings),
            Heard::Lyrics(l) => lyrics = Some(l),
            Heard::Video(v) => video = Some(v),
        }
    }
    // Lyrics and the clip come with the playback, before the Home.
    assert_eq!(lyrics.map(|l| l.status), Some(LyricsStatus::NoTrack));
    assert_eq!(video, Some(VideoSnapshot { has_clip: false, status: ClipStatus::Idle, failures: 0 }));
    // The settings come right after, in the same publication.
    while settings_seen.is_none() {
        if let Heard::Settings(settings) = rx.recv_timeout(WAIT).unwrap() {
            settings_seen = Some(settings);
        }
    }
    assert_eq!(settings_seen.map(|s| s.quality.options.len()), Some(2));
    assert_eq!(playback.map(|p| p.state), Some(PlayerState::Stopped));
    assert_eq!(home.map(|h| h.version), Some(1));
    drop(core);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_unreachable_backend_turns_the_home_into_an_error_and_retry_loads_again() {
    let dir = data_dir();
    let (core, rx) = start(&dir);
    core.home(HomeAction::Ensure);
    let failed = home_until(&rx, |home| matches!(home.status, HomeStatus::Failed { .. }));
    assert!(failed.version > 1, "the first Home was not a failure yet");
    core.home(HomeAction::Retry);
    // A refused connection answers at once: the core may publish the reload and its failure as one
    // snapshot, so only a newer failure is certain.
    home_until(&rx, |home| matches!(home.status, HomeStatus::Failed { .. }) && home.version > failed.version);
    drop(core);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn commands_reach_the_core_and_only_changed_domains_are_told() {
    let dir = data_dir();
    let (core, rx) = start(&dir);
    // The first state of the playback, lyrics, clip, Home, account and settings.
    for _ in 0..6 {
        rx.recv_timeout(WAIT).unwrap();
    }
    // Shuffle changes the playback domain only: no Home rebuild comes with it.
    core.playback(PlaybackAction::ToggleShuffle);
    match rx.recv_timeout(WAIT).unwrap() {
        Heard::Playback(playback) => assert!(playback.shuffle),
        other => panic!("expected a playback change, got {other:?}"),
    }
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "nothing else changed");
    drop(core);
    let _ = std::fs::remove_dir_all(dir);
}

fn running(core: &PsytifyCore) -> bool {
    core.inner.running.lock().unwrap().is_some()
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(future)
}

/// The next playback snapshot that `accept` takes, skipping the others.
fn playback_until(rx: &Receiver<Heard>, accept: impl Fn(&PlaybackSnapshot) -> bool) -> PlaybackSnapshot {
    loop {
        match rx.recv_timeout(WAIT) {
            Ok(Heard::Playback(playback)) if accept(&playback) => return playback,
            Ok(_) => {}
            Err(err) => panic!("no matching playback snapshot: {err}"),
        }
    }
}

#[test]
fn covers_from_a_dead_backend_are_none_and_a_stopped_core_does_not_restart_for_them() {
    let dir = data_dir();
    let (core, _rx) = start(&dir);
    block_on(async {
        assert_eq!(core.artwork("https://archive.org/services/img/x".into()).await, None);
        assert_eq!(core.stop().await, Ok(()));
        assert!(!running(&core));
        assert_eq!(core.artwork("https://archive.org/services/img/x".into()).await, None);
        assert!(!running(&core), "a cover does not start the core");
        assert_eq!(core.stop().await, Ok(()), "a second stop is fine");
    });
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_stopped_core_starts_again_on_the_next_command_with_its_saved_state() {
    let dir = data_dir();
    let (core, rx) = start(&dir);
    playback_until(&rx, |playback| !playback.shuffle);
    core.playback(PlaybackAction::ToggleShuffle);
    playback_until(&rx, |playback| playback.shuffle);
    assert_eq!(block_on(core.stop()), Ok(()));
    while rx.try_recv().is_ok() {}
    // The next command starts a new core, whose first state comes from the config.json just written.
    core.home(HomeAction::Ensure);
    assert!(running(&core));
    let restored = playback_until(&rx, |_| true);
    assert!(restored.shuffle, "shuffle was saved at the stop and read at the start");
    // A refused connection can fail the Home within the core's first publication: either is its first state.
    home_until(&rx, |home| matches!(home.status, HomeStatus::Loading | HomeStatus::Failed { .. }));
    drop(core);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn hidden_with_nothing_playing_the_core_stops_after_the_delay_and_visible_it_runs() {
    let dir = data_dir();
    let (core, _rx) = start_with(&dir, Duration::from_millis(200));
    core.set_visible(true);
    std::thread::sleep(Duration::from_millis(600));
    assert!(running(&core), "on screen the core keeps running");
    core.set_visible(false);
    let give_up = std::time::Instant::now() + WAIT;
    while running(&core) && std::time::Instant::now() < give_up {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!running(&core), "hidden and idle, the core stopped");
    assert!(dir.join("config.json").is_file());
    core.set_visible(true);
    assert!(running(&core), "back on screen, it starts again");
    drop(core);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_queue_is_read_a_page_at_a_time_at_the_published_version() {
    let dir = data_dir();
    let (core, rx) = start(&dir);
    let tracks: Arc<[psytify_core::api::Track]> = (1..=5).map(crate::test_support::track).collect();
    core.inner.send(Command::Playback(psytify_core::command::PlaybackCmd::Play { tracks, start: 1 }));
    let playback = playback_until(&rx, |playback| playback.queue_length == 5);
    let page = core.queue_page(3, 10);
    assert_eq!((page.version, page.total, page.current), (playback.queue_version, 5, Some(1)));
    let titles: Vec<&str> = page.entries.iter().map(|entry| entry.track.title.as_str()).collect();
    assert_eq!(titles, ["Track 4", "Track 5"]);
    core.playback(PlaybackAction::Remove { position: 4 });
    let after = playback_until(&rx, |playback| playback.queue_length == 4);
    assert!(after.queue_version > playback.queue_version);
    assert_eq!(core.queue_page(0, 10).entries.len(), 4);
    drop(core);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_art_pixel_follows_the_design_rule() {
    assert_eq!(art_px(1.0), 2);
    assert_eq!(art_px(2.625), 5);
    assert_eq!(art_px(3.0), 6);
    assert_eq!(art_px(0.0), 1);
}

/// Starts 1 and 4 work, 2 and 3 fail: a restart that fails twice, then works.
fn flaky(options: CoreOptions) -> Result<Core, psytify_core::CoreError> {
    static STARTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    match STARTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) {
        1 | 2 => Err(psytify_core::CoreError::Start(std::io::Error::other("no thread"))),
        _ => Core::start(options),
    }
}

/// The next failure report, skipping the snapshots.
fn failure(rx: &Receiver<Heard>) -> Option<String> {
    loop {
        match rx.recv_timeout(WAIT) {
            Ok(Heard::Failed(reason)) => return reason,
            Ok(_) => {}
            Err(err) => panic!("no failure report: {err}"),
        }
    }
}

#[test]
fn a_restart_that_fails_is_reported_and_retry_reports_the_recovery() {
    let dir = data_dir();
    let (core, rx) = launch_with(&dir, Duration::from_secs(3600), flaky);
    assert_eq!(block_on(core.stop()), Ok(()));
    core.playback(PlaybackAction::Toggle);
    assert!(!running(&core));
    assert!(
        failure(&rx).is_some_and(|reason| reason.contains("no thread")),
        "the command is not dropped silently"
    );
    core.retry_start();
    assert!(failure(&rx).is_some(), "the retry failed too, and says so");
    core.retry_start();
    assert!(running(&core));
    assert_eq!(failure(&rx), None, "the core runs again: the error goes away");
    core.retry_start();
    assert!(rx.try_iter().all(|heard| !matches!(heard, Heard::Failed(_))), "a running core reports nothing");
    drop(core);
    let _ = std::fs::remove_dir_all(dir);
}

struct PageRecorder(Mutex<Sender<PageView>>);

impl PageListener for PageRecorder {
    fn page_changed(&self, page: PageView) {
        if let Ok(tx) = self.0.lock() {
            let _ = tx.send(page);
        }
    }
}

#[test]
fn an_open_page_hears_its_changes_and_a_tap_opens_or_plays() {
    let dir = data_dir();
    let (core, _rx) = start(&dir);
    let (tx, pages) = channel();
    let library = core.open_page(Route::Library, Arc::new(PageRecorder(Mutex::new(tx))));
    let first = pages.recv_timeout(WAIT).unwrap();
    assert_eq!((first.title.as_str(), first.status.clone()), ("Libreria", PageStatus::Ready));
    assert!(matches!(&first.rows[1], Row::Item { title, .. } if title == "Brani che ti piacciono"));
    assert_eq!(library.tap(first.version, 1), Tap::Open { route: Route::Liked });
    assert_eq!(library.tap(first.version + 7, 1), Tap::Nothing, "a stale version does nothing");

    core.inner
        .send(Command::library(psytify_core::command::LibraryCmd::ToggleLike(crate::test_support::track(1))));
    let liked = loop {
        let page = pages.recv_timeout(WAIT).unwrap();
        if matches!(&page.rows[1], Row::Item { detail, .. } if detail.ends_with("1 brano")) {
            break page;
        }
    };
    assert!(liked.version > first.version);
    library.chip(liked.version, 0, 3);
    let artists = pages.recv_timeout(WAIT).unwrap();
    assert!(matches!(&artists.rows[0], Row::Chips { chips, .. } if chips[3].on));
    assert_eq!(library.rows(0, 10).rows.len(), artists.total as usize);

    drop(library);
    core.inner
        .send(Command::library(psytify_core::command::LibraryCmd::ToggleLike(crate::test_support::track(2))));
    assert!(pages.recv_timeout(Duration::from_millis(500)).is_err(), "a closed page hears nothing");
    assert!(core.inner.pages().live().is_empty());
    drop(core);
    let _ = std::fs::remove_dir_all(dir);
}

fn account_until(rx: &Receiver<Heard>, accept: impl Fn(&AccountSnapshot) -> bool) -> AccountSnapshot {
    loop {
        match rx.recv_timeout(WAIT) {
            Ok(Heard::Account(account)) if accept(&account) => return account,
            Ok(_) => {}
            Err(err) => panic!("no matching account: {err}"),
        }
    }
}

#[test]
fn a_sign_in_against_a_clerk_that_does_not_answer_ends_in_an_error_under_the_form() {
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let dir = data_dir_with(&format!(r#","clerk_issuer":"https://127.0.0.1:{port}""#));
    let (core, rx) = start(&dir);
    let first = account_until(&rx, |_| true);
    assert_eq!((first.state, first.step), (AccountState::SignedOut, SignInStep::Form));
    core.sign_in("ada".into(), "not-a-real-password".into());
    let failed = account_until(&rx, |account| account.error.is_some());
    assert!(!failed.busy && failed.state == AccountState::SignedOut);
    assert!(!format!("{failed:?}").contains("not-a-real-password"));
    core.dismiss_sign_in_error();
    account_until(&rx, |account| account.error.is_none());
    drop(core);
    let _ = std::fs::remove_dir_all(dir);
}
