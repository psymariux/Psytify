//! The playback domain as the Kotlin app sees it: plain records built from `PlaybackView`, and the
//! commands the app may send. Identifiers cross the boundary as strings, durations as milliseconds.

use std::time::{Duration, Instant};

use psytify_core::api::Track;
use psytify_core::command::PlaybackCmd;
use psytify_core::player::{PlaybackState, Repeat};
use psytify_core::snapshot::PlaybackView;

/// A track, with what a row or the player bar shows.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TrackItem {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub cover_url: Option<String>,
    pub duration_ms: u64,
}

impl TrackItem {
    pub fn of(track: &Track) -> Self {
        Self {
            id: track.id.to_string(),
            title: track.title.clone(),
            artist: track.artist.name.clone(),
            album: track.album.as_ref().map(|album| album.title.clone()),
            cover_url: track.cover_url.clone(),
            duration_ms: track.duration_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PlayerState {
    Stopped,
    Loading,
    Playing,
    Paused,
    Failed { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RepeatMode {
    Off,
    All,
    One,
}

/// The playback domain at one publication. The position is not published while it runs: the app
/// anchors `position_ms` to its own clock on receipt and, while `running`, adds the time elapsed
/// since (the same rule as `Clock::at`).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PlaybackSnapshot {
    /// The core's playback version: it grows with every change of this domain.
    pub version: u64,
    pub state: PlayerState,
    /// The current queue entry, also while it loads or after an error.
    pub track: Option<TrackItem>,
    pub position_ms: u64,
    pub running: bool,
    pub queue_length: u32,
    /// Position of the current entry in listening order.
    pub queue_position: Option<u32>,
    /// Grows when the queue changes (tracks, order or current entry): pages read with
    /// `PsytifyCore::queue_page` before it are stale.
    pub queue_version: u64,
    pub has_next: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub volume: f32,
    pub muted: bool,
    /// Why the audio output did not open at startup.
    pub device_error: Option<String>,
}

impl PlaybackSnapshot {
    pub fn of(version: u64, queue_version: u64, view: &PlaybackView, now: Instant) -> Self {
        let queue = &view.queue;
        Self {
            version,
            state: match &view.state {
                PlaybackState::Stopped => PlayerState::Stopped,
                PlaybackState::Loading { .. } => PlayerState::Loading,
                PlaybackState::Playing => PlayerState::Playing,
                PlaybackState::Paused => PlayerState::Paused,
                PlaybackState::Error(message) => PlayerState::Failed { message: message.clone() },
            },
            track: queue.current().map(TrackItem::of),
            position_ms: millis(view.clock.at(now)),
            running: view.clock.playing,
            queue_length: count(queue.len()),
            queue_position: queue.position().map(count),
            queue_version,
            has_next: queue.peek_next().is_some(),
            shuffle: queue.shuffle(),
            repeat: match queue.repeat() {
                Repeat::Off => RepeatMode::Off,
                Repeat::All => RepeatMode::All,
                Repeat::One => RepeatMode::One,
            },
            volume: view.volume,
            muted: view.muted,
            device_error: view.device_error.as_ref().map(ToString::to_string),
        }
    }
}

/// What the app's own controls ask of the player. Playing something from a page goes through that
/// page's command (for the Home, `PsytifyCore::activate_home`), which picks the right core command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PlaybackAction {
    /// Play or pause; stopped or after an error, restart the current track.
    Toggle,
    Next,
    Previous,
    SeekTo {
        position_ms: u64,
    },
    /// Jump to the queue entry at this position of the listening order.
    PlayAt {
        index: u32,
    },
    /// Take the entry at this position of the listening order out of the queue.
    Remove {
        position: u32,
    },
    /// Move an entry of the listening order; the current track stays current.
    Move {
        from: u32,
        to: u32,
    },
    ClearQueue,
    ToggleShuffle,
    CycleRepeat,
}

impl PlaybackAction {
    pub fn command(self) -> PlaybackCmd {
        match self {
            Self::Toggle => PlaybackCmd::Toggle,
            Self::Next => PlaybackCmd::Next,
            Self::Previous => PlaybackCmd::Previous,
            Self::SeekTo { position_ms } => PlaybackCmd::SeekTo(Duration::from_millis(position_ms)),
            Self::PlayAt { index } => PlaybackCmd::PlayAt(index as usize),
            Self::Remove { position } => PlaybackCmd::Remove(position as usize),
            Self::Move { from, to } => PlaybackCmd::Move { from: from as usize, to: to as usize },
            Self::ClearQueue => PlaybackCmd::Clear,
            Self::ToggleShuffle => PlaybackCmd::ToggleShuffle,
            Self::CycleRepeat => PlaybackCmd::CycleRepeat,
        }
    }
}

pub(crate) fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

pub(crate) fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use psytify_core::player::{Clock, Queue};
    use uuid::Uuid;

    use super::*;
    use crate::test_support::track;

    fn queue(tracks: Vec<Track>, start: usize) -> Arc<Queue> {
        let mut queue = Queue::default();
        queue.replace(tracks, start, 7);
        Arc::new(queue)
    }

    #[test]
    fn a_stopped_player_has_no_track_and_no_position() {
        let snapshot = PlaybackSnapshot::of(0, 0, &PlaybackView::default(), Instant::now());
        assert_eq!(snapshot.state, PlayerState::Stopped);
        assert_eq!((snapshot.track, snapshot.position_ms, snapshot.running), (None, 0, false));
        assert_eq!((snapshot.queue_length, snapshot.queue_position, snapshot.has_next), (0, None, false));
    }

    #[test]
    fn a_running_clock_gives_the_position_at_the_time_of_mapping() {
        let at = Instant::now();
        let view = PlaybackView {
            state: PlaybackState::Playing,
            clock: Clock::running(Duration::from_secs(10), at),
            queue: queue(vec![track(1), track(2)], 0),
            ..PlaybackView::default()
        };
        let snapshot = PlaybackSnapshot::of(3, 1, &view, at + Duration::from_millis(1500));
        assert_eq!(snapshot.version, 3);
        assert_eq!(snapshot.state, PlayerState::Playing);
        assert_eq!((snapshot.position_ms, snapshot.running), (11_500, true));
        assert_eq!(snapshot.track.as_ref().map(|t| t.title.as_str()), Some("Track 1"));
        assert_eq!((snapshot.queue_length, snapshot.queue_position, snapshot.has_next), (2, Some(0), true));
    }

    #[test]
    fn a_paused_clock_does_not_move_and_errors_keep_their_text() {
        let at = Instant::now();
        let view = PlaybackView {
            state: PlaybackState::Error("Il file audio non è riproducibile.".into()),
            clock: Clock::paused(Duration::from_secs(4), at),
            queue: queue(vec![track(1)], 0),
            ..PlaybackView::default()
        };
        let snapshot = PlaybackSnapshot::of(1, 1, &view, at + Duration::from_secs(60));
        assert_eq!(snapshot.position_ms, 4_000);
        assert!(!snapshot.running && !snapshot.has_next);
        assert_eq!(
            snapshot.state,
            PlayerState::Failed { message: "Il file audio non è riproducibile.".into() }
        );
    }

    #[test]
    fn a_track_crosses_with_string_ids_and_its_album() {
        let mut with_album = track(5);
        with_album.album =
            Some(psytify_core::api::models::AlbumRef { id: Uuid::from_u128(9), title: "Album".into() });
        let item = TrackItem::of(&with_album);
        assert_eq!(item.id, Uuid::from_u128(5).to_string());
        assert_eq!((item.artist.as_str(), item.album.as_deref()), ("Artist 5", Some("Album")));
        assert_eq!(item.duration_ms, 180_000);
    }

    #[test]
    fn actions_become_the_core_commands() {
        assert!(matches!(PlaybackAction::Toggle.command(), PlaybackCmd::Toggle));
        assert!(matches!(PlaybackAction::Next.command(), PlaybackCmd::Next));
        assert!(matches!(PlaybackAction::Previous.command(), PlaybackCmd::Previous));
        assert!(matches!(PlaybackAction::PlayAt { index: 2 }.command(), PlaybackCmd::PlayAt(2)));
        assert!(matches!((PlaybackAction::Remove { position: 3 }).command(), PlaybackCmd::Remove(3)));
        assert!(matches!(
            (PlaybackAction::Move { from: 4, to: 1 }).command(),
            PlaybackCmd::Move { from: 4, to: 1 }
        ));
        assert!(matches!(PlaybackAction::ClearQueue.command(), PlaybackCmd::Clear));
        match (PlaybackAction::SeekTo { position_ms: 61_250 }).command() {
            PlaybackCmd::SeekTo(at) => assert_eq!(at, Duration::from_millis(61_250)),
            other => panic!("unexpected {other:?}"),
        }
    }
}
