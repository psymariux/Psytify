//! The system's media controls (notification, lock screen, headset buttons) for the Kotlin app. The
//! core decides what the system shows and when (`state/system.rs`, only at real transitions); on
//! Android it hands it to the hook `media::set_phone_media`, which this module forwards to the
//! app's `CoreListener::now_playing`. The buttons come back as [`MediaCommand`] and enter the core
//! as the same media events the NativeActivity shell sends (`media::phone_command`).

#[cfg(target_os = "android")]
use std::sync::{Mutex, OnceLock, PoisonError, Weak};

#[cfg(target_os = "android")]
use crate::Inner;

/// A cover already decoded by the core: a square of `side` pixels, RGBA, not premultiplied.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Cover {
    pub side: u32,
    pub rgba: Vec<u8>,
}

/// What the system shows of the current track.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: u64,
    /// The position when the core decided to show it; while `playing` the system advances it.
    pub position_ms: u64,
    pub playing: bool,
    /// Present once the core has the decoded cover of this track.
    pub artwork: Option<Cover>,
}

/// A button of the system's media controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MediaCommand {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    SeekTo { position_ms: u64 },
}

impl MediaCommand {
    /// The code and value of `media::phone_command`.
    pub fn code(self) -> (i32, i64) {
        match self {
            Self::Play => (0, 0),
            Self::Pause => (1, 0),
            Self::Next => (2, 0),
            Self::Previous => (3, 0),
            Self::SeekTo { position_ms } => (4, i64::try_from(position_ms).unwrap_or(i64::MAX)),
            Self::Toggle => (5, 0),
        }
    }
}

/// Sends a button to the core. On the desktop the system's controls are MPRIS, inside the core:
/// nothing to forward.
pub(crate) fn send(command: MediaCommand) {
    #[cfg(target_os = "android")]
    {
        let (code, value) = command.code();
        psytify_core::media::phone_command(code, value);
    }
    #[cfg(not(target_os = "android"))]
    let _ = command;
}

/// What the system's media controls show, between the core and the app. The core shows nothing
/// once it stops or while its queue entry is stopped (after a restart), but a queue with an entry
/// keeps the last track on the lock screen, paused, so «play» there can start the core again.
#[derive(Debug, Default)]
pub(crate) struct Shown {
    /// What the core last showed, while it shows something or the track is parked.
    last: Option<NowPlaying>,
    /// The last track is shown paused although the core shows nothing.
    parked: bool,
    /// The core's queue has a current entry.
    has_entry: bool,
}

impl Shown {
    /// The core shows `now` (`None`: nothing, or it stopped). What the app should show, if that
    /// changes what it shows.
    pub(crate) fn core_shows(&mut self, now: Option<NowPlaying>) -> Option<Option<NowPlaying>> {
        match now {
            Some(now) => {
                self.parked = false;
                self.last = Some(now.clone());
                Some(Some(now))
            }
            None if self.has_entry => {
                let last = self.last.as_mut()?;
                if self.parked {
                    return None;
                }
                self.parked = true;
                last.playing = false;
                Some(Some(last.clone()))
            }
            None => {
                self.parked = false;
                self.last.take().map(|_| None)
            }
        }
    }

    /// The core published its queue: without a current entry a parked track goes away.
    pub(crate) fn queue(&mut self, has_entry: bool) -> Option<Option<NowPlaying>> {
        self.has_entry = has_entry;
        if has_entry || !self.parked {
            return None;
        }
        self.parked = false;
        self.last.take().map(|_| None)
    }
}

/// The bridge that receives what the system shows. The core's hook is registered once per
/// process, so it reads this slot, which each start fills and each stop empties.
#[cfg(target_os = "android")]
static SHOWN_TO: Mutex<Option<Weak<Inner>>> = Mutex::new(None);

/// Registers the core's media hook (once) and points it at `inner`. Must run before
/// `Core::start`: without a hook the core's media stream ends at once and nothing is ever shown.
#[cfg(target_os = "android")]
pub(crate) fn attach(inner: Weak<Inner>) {
    static HOOKED: OnceLock<()> = OnceLock::new();
    *SHOWN_TO.lock().unwrap_or_else(PoisonError::into_inner) = Some(inner);
    HOOKED.get_or_init(|| {
        psytify_core::media::set_phone_media(|now| {
            let inner = SHOWN_TO.lock().unwrap_or_else(PoisonError::into_inner).clone();
            if let Some(inner) = inner.and_then(|inner| inner.upgrade()) {
                inner.show_media(now.map(record));
            }
        });
    });
}

#[cfg(target_os = "android")]
pub(crate) fn detach() {
    *SHOWN_TO.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

#[cfg(target_os = "android")]
fn record(now: &psytify_core::media::NowPlaying<'_>) -> NowPlaying {
    use crate::playback::millis;
    NowPlaying {
        title: now.title.to_owned(),
        artist: now.artist.to_owned(),
        album: now.album.map(str::to_owned),
        duration_ms: millis(now.duration),
        position_ms: millis(now.position),
        playing: now.playing,
        artwork: now.artwork.map(|(side, rgba)| Cover { side, rgba: rgba.to_vec() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_use_the_codes_of_the_phone_media_stream() {
        assert_eq!(MediaCommand::Play.code(), (0, 0));
        assert_eq!(MediaCommand::Pause.code(), (1, 0));
        assert_eq!(MediaCommand::Next.code(), (2, 0));
        assert_eq!(MediaCommand::Previous.code(), (3, 0));
        assert_eq!(MediaCommand::SeekTo { position_ms: 42_000 }.code(), (4, 42_000));
        assert_eq!(MediaCommand::Toggle.code(), (5, 0));
        assert_eq!(MediaCommand::SeekTo { position_ms: u64::MAX }.code(), (4, i64::MAX));
    }

    fn now(playing: bool) -> NowPlaying {
        NowPlaying {
            title: "Track".into(),
            artist: "Artist".into(),
            album: None,
            duration_ms: 180_000,
            position_ms: 42_000,
            playing,
            artwork: None,
        }
    }

    #[test]
    fn a_stopped_core_with_a_queue_keeps_the_last_track_paused() {
        let mut shown = Shown::default();
        assert_eq!(shown.queue(true), None);
        assert_eq!(shown.core_shows(Some(now(true))), Some(Some(now(true))));
        // The core stops (idle stop): the track stays, paused at its position.
        assert_eq!(shown.core_shows(None), Some(Some(now(false))));
        // The core that starts again shows nothing until it plays: no change.
        assert_eq!(shown.core_shows(None), None);
        assert_eq!(shown.queue(true), None);
        assert_eq!(shown.core_shows(Some(now(true))), Some(Some(now(true))), "playing again");
    }

    #[test]
    fn without_a_queue_entry_nothing_is_shown() {
        let mut shown = Shown::default();
        shown.queue(true);
        shown.core_shows(Some(now(false)));
        assert_eq!(shown.core_shows(None), Some(Some(now(false))), "parked");
        assert_eq!(shown.queue(false), Some(None), "the queue was cleared: the parked track goes");
        assert_eq!(shown.queue(false), None);
        shown.core_shows(Some(now(true)));
        assert_eq!(shown.core_shows(None), Some(None), "no entry: nothing, at once");
        assert_eq!(shown.core_shows(None), None, "and only once");
    }
}
