//! A core's pump, a task on the bridge's thread: it waits on the core's `watch` (no polling: it
//! sleeps until the core publishes) and tells the listener which domains changed, comparing the
//! domain versions as `CoreModel::apply` does on the desktop. It ends when its core stops.
//!
//! It also gives the core the decoded cover of the current track, which the Android media session
//! wants in pixels (`PlaybackCmd::Artwork`); the desktop's MPRIS reads the address instead. And it
//! stops its core once the app has been hidden for `idle_stop` with nothing playing, no sign-in on
//! its way and no download running.

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use psytify_core::Command;
use psytify_core::Snapshot;
use psytify_core::command::{Artwork, PlaybackCmd};
use psytify_core::player::PlaybackState;
use psytify_core::snapshot::{CatalogEvent, DomainVersions, DownloadsEvent};
use tokio::sync::watch;

use crate::Inner;
use crate::account::AccountSnapshot;
use crate::downloads::{self, DownloadsSnapshot, Shape};
use crate::home::HomeModel;
use crate::pages::{Changes, Route};
use crate::playback::PlaybackSnapshot;
use crate::settings::SettingsSnapshot;
use crate::social::SocialMarks;
use crate::video::VideoSnapshot;

pub(crate) async fn run(
    inner: Weak<Inner>,
    generation: u64,
    mut changes: watch::Receiver<Arc<Snapshot>>,
    mut visible: watch::Receiver<bool>,
    idle_stop: Duration,
) {
    let mut pump = Pump {
        inner,
        seen: None,
        cover: None,
        events: Events::default(),
        shape: Shape::default(),
        notification: None,
        marks: None,
        settings: None,
        free: None,
        video: None,
    };
    let mut deadline: Option<tokio::time::Instant> = None;
    loop {
        let snapshot = Arc::clone(&changes.borrow_and_update());
        if !pump.publish(&snapshot) {
            return;
        }
        let idle = !*visible.borrow_and_update() && !keeps_running(&snapshot);
        deadline = match (idle, deadline) {
            (false, _) => None,
            (true, None) => Some(tokio::time::Instant::now() + idle_stop),
            (true, armed) => armed,
        };
        tokio::select! {
            changed = changes.changed() => if changed.is_err() { return },
            changed = visible.changed() => if changed.is_err() { return },
            () = sleep_until(deadline) => {
                if let Some(inner) = pump.inner.upgrade() {
                    // A failed last write leaves the previous config.json whole; nobody to tell here.
                    let _ = inner.stop(Some(generation)).await;
                }
                return;
            }
        }
    }
}

/// Playing, or about to; a sign-in on its way, the browser's return included (the app is hidden
/// while the provider's page is open); or a download waiting or running: the core must not stop
/// under it.
fn keeps_running(snapshot: &Snapshot) -> bool {
    let auth = &snapshot.social.auth;
    matches!(snapshot.playback.state, PlaybackState::Playing | PlaybackState::Loading { .. })
        || auth.busy
        || auth.waiting.is_some()
        || snapshot.downloads.busy_count() > 0
}

async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

struct Pump {
    inner: Weak<Inner>,
    /// The versions last told to the listener; `None` before the first state of this core.
    seen: Option<DomainVersions>,
    /// The cover address last asked for the system, while a track is active.
    cover: Option<String>,
    /// The last events told to the app.
    events: Events,
    /// The downloads as the pages last saw them.
    shape: Shape,
    /// The notification of the downloads last told to the app.
    notification: Option<DownloadsSnapshot>,
    /// The jam marker and counts last told.
    marks: Option<SocialMarks>,
    /// The settings page last told.
    settings: Option<SettingsSnapshot>,
    /// The free space of the downloads folder, read when the downloads change.
    free: Option<u64>,
    /// The clip last told to the media slot.
    video: Option<VideoSnapshot>,
}

/// The `seq` of the last event of each ring told to the app.
#[derive(Debug, Clone, Copy, Default)]
struct Events {
    catalog: u64,
    external: u64,
    social: u64,
    downloads: u64,
    jam: u64,
}

impl Pump {
    /// Tells the listener what changed in `snapshot`. `false` once the bridge is gone.
    fn publish(&mut self, snapshot: &Arc<Snapshot>) -> bool {
        let Some(inner) = self.inner.upgrade() else { return false };
        // With a fixture on (debug builds), its social and jam views.
        let snapshot = &inner.dressed(Arc::clone(snapshot));
        let new = snapshot.versions;
        let first = self.seen.is_none();
        let old = self.seen.unwrap_or_default();
        self.seen = Some(new);

        if first || old.playback != new.playback {
            let queue_version = inner.queue().observe(&snapshot.playback.queue);
            let playback =
                PlaybackSnapshot::of(new.playback, queue_version, &snapshot.playback, Instant::now());
            inner.listener().playback_changed(playback);
            let lyrics = inner.lyrics().observe(&snapshot.playback, first);
            if let Some(lyrics) = lyrics {
                inner.listener().lyrics_changed(lyrics);
            }
            if inner.system_media() {
                inner.media_queue(snapshot.playback.queue.current().is_some());
                self.ask_cover(&inner, snapshot);
            }
        }
        if first || old.playback != new.playback || old.video != new.video {
            let video = VideoSnapshot::of(&snapshot.playback, &snapshot.video);
            if self.video.as_ref() != Some(&video) {
                self.video = Some(video.clone());
                inner.listener().video_changed(video);
            }
        }
        if first || old.catalog != new.catalog || old.library != new.library {
            let version = inner.next_home_version();
            let (home, model) = HomeModel::build(version, &snapshot.catalog, &snapshot.library);
            *inner.home() = model;
            inner.listener().home_changed(home);
        }
        let progress = old.downloads != new.downloads;
        let shape = if first || progress {
            let shape = Shape::of(&snapshot.downloads);
            let changed = !shape.same(&self.shape);
            self.shape = shape;
            changed
        } else {
            false
        };
        if first || progress {
            let notification = DownloadsSnapshot::of(&snapshot.downloads);
            if self.notification.as_ref() != Some(&notification) {
                self.notification = Some(notification.clone());
                inner.listener().downloads_changed(notification);
            }
        }
        let pages = old.catalog != new.catalog
            || old.library != new.library
            || old.external != new.external
            || old.playback != new.playback
            || old.social != new.social
            || old.jam != new.jam
            || shape;
        if first || old.social != new.social {
            inner.listener().account_changed(AccountSnapshot::of(&snapshot.social));
        }
        if first || old.social != new.social || old.jam != new.jam {
            let marks = SocialMarks::of(snapshot);
            if self.marks != Some(marks) {
                self.marks = Some(marks);
                inner.listener().marks_changed(marks);
            }
        }
        if first || progress {
            self.free = downloads::free_bytes(&snapshot.downloads.folder);
        }
        let settings_read = old.settings != new.settings
            || old.playback != new.playback
            || old.social != new.social
            || progress;
        if first || settings_read {
            let settings = SettingsSnapshot::of(snapshot, self.free);
            if self.settings.as_ref() != Some(&settings) {
                self.settings = Some(settings.clone());
                inner.listener().settings_changed(settings);
            }
        }
        if first || pages || progress {
            let open = inner.pages().live();
            let changes = Changes { pages, progress };
            for page in open {
                page.refresh(&inner, snapshot, first, changes);
            }
        }
        self.tell(&inner, snapshot, first);
        true
    }

    /// The events since the last ones told: notices, and the artist page to open once a catalogue
    /// artist is on the server. A new core starts from its latest events: nothing is told twice.
    fn tell(&mut self, inner: &Arc<Inner>, snapshot: &Snapshot, first: bool) {
        let latest = Events {
            catalog: snapshot.catalog.events.back().map_or(0, |raised| raised.seq),
            external: snapshot.external.events.back().map_or(0, |raised| raised.seq),
            social: snapshot.social.events.back().map_or(0, |raised| raised.seq),
            downloads: snapshot.downloads.events.back().map_or(0, |raised| raised.seq),
            jam: snapshot.jam.events.back().map_or(0, |raised| raised.seq),
        };
        if first {
            self.events = latest;
            return;
        }
        let seen = self.events;
        let listener = inner.listener();
        for raised in snapshot.catalog.events.iter().filter(|raised| raised.seq > seen.catalog) {
            match &raised.event {
                CatalogEvent::MetaArtistOpened(id) => listener.open(Route::artist(*id)),
                CatalogEvent::MetaArtistFailed(error) | CatalogEvent::LibraryPageFailed(error) => {
                    listener.notice(error.to_string(), true);
                }
                CatalogEvent::Replaced(_) | CatalogEvent::Prefetched(_) => {}
            }
        }
        for raised in snapshot.external.events.iter().filter(|raised| raised.seq > seen.external) {
            if let Some(message) = raised.event.message() {
                listener.notice(message, raised.event.is_error());
            }
        }
        // Jam invites are notices too; the Jam page lists them with Entra and Ignora.
        for raised in snapshot.social.events.iter().filter(|raised| raised.seq > seen.social) {
            listener.notice(raised.event.message(), raised.event.is_error());
        }
        for raised in snapshot.jam.events.iter().filter(|raised| raised.seq > seen.jam) {
            listener.notice(raised.event.message(), raised.event.is_error());
        }
        for raised in snapshot.downloads.events.iter().filter(|raised| raised.seq > seen.downloads) {
            match &raised.event {
                DownloadsEvent::UpdateSaved(path) => listener.install_update(path.display().to_string()),
                DownloadsEvent::UpdateOffered { tag, notify: true } => listener.update_offered(tag.clone()),
                _ => {}
            }
            if let Some((text, error)) = downloads::notice(&raised.event) {
                listener.notice(text, error);
            }
        }
        self.events = latest;
    }

    /// When the active track changes, asks the core for its cover and hands the pixels back to it.
    fn ask_cover(&mut self, inner: &Arc<Inner>, snapshot: &Snapshot) {
        let playback = &snapshot.playback;
        let url = playback.queue.current().filter(|_| playback.is_active()).and_then(|t| t.cover_url.clone());
        if url == self.cover {
            return;
        }
        self.cover.clone_from(&url);
        let Some(url) = url else { return };
        let Some(answer) = inner.artwork(url.clone()) else { return };
        let inner = Arc::downgrade(inner);
        tokio::spawn(async move {
            let Ok(Some(pixels)) = answer.await else { return };
            if let Some(inner) = inner.upgrade() {
                let art = Artwork { url, side: pixels.side, rgba: pixels.rgba };
                // Only to the core that runs: a cover that arrives after a stop does not restart it.
                inner.current(|core| core.send(Command::Playback(PlaybackCmd::Artwork(Box::new(art)))));
            }
        });
    }
}
