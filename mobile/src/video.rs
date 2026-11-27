//! The clip of the current track for the full player's media slot. Which clip and when is the
//! core's (`VideoCmd`, `VideoView`: the address is asked only while the slot shows it, the 360p to
//! 720p upgrade, `PrepareAhead`); ExoPlayer plays it muted in the app, and the audio stays in the core.
//!
//! The alignment of ExoPlayer to the audio is computed here ([`VideoSync`]): the core's pure rules
//! (`video::Alignment`: a jump or a resume moves the clip, a pause stops it, past the 200 ms threshold
//! it seeks with up to 1.5 s of lead, at most every 2 s) and, under the threshold, a small change of
//! playback speed that closes the gap without a seek (ExoPlayer can do it smoothly, GStreamer on the
//! desktop cannot). The app measures its player and applies what comes back.
//!
//! The signed address never crosses in a record (whose Kotlin `toString` would print it): the app asks
//! [`clip_url`] for the one `seq` it opens.

use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use psytify_core::snapshot::{ClipState, PlaybackView, VideoView};
use psytify_core::video::{Adjust, Alignment, DRIFT, Measure};

use crate::Inner;
use crate::playback::millis;

/// Under the core's threshold, a gap wider than this starts a speed nudge…
const NUDGE_START: Duration = Duration::from_millis(60);
/// …that lasts until the gap is this small.
const NUDGE_STOP: Duration = Duration::from_millis(20);
/// Speed change per second of gap, and its cap: 100 ms of gap is a 4 % change, closed in about 2.5 s.
const NUDGE_GAIN: f32 = 0.4;
const NUDGE_MAX: f32 = 0.04;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ClipStatus {
    /// Nothing shows the clip, or the track has none.
    Idle,
    /// The server prepares it (the first time it downloads it: up to half a minute).
    Loading,
    /// Open the address of `seq` ([`clip_url`]); a new `seq` is a better version to swap in.
    Ready { seq: u64, height: Option<u32> },
    /// It did not open, twice (or the server refused it): «Riprova».
    Failed { message: String },
}

/// The clip as the media slot sees it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VideoSnapshot {
    /// The current track has a clip: the slot shows its Cover / Video switch.
    pub has_clip: bool,
    pub status: ClipStatus,
    /// Grows with every clip that stops for good: a new number is a notice.
    pub failures: u64,
}

impl VideoSnapshot {
    pub(crate) fn of(playback: &PlaybackView, video: &VideoView) -> Self {
        let current = playback.queue.current().filter(|track| track.has_video);
        let status = match &video.state {
            _ if current.is_none() || video.track != current.map(|track| track.id) => ClipStatus::Idle,
            ClipState::Idle => ClipStatus::Idle,
            ClipState::Loading => ClipStatus::Loading,
            ClipState::Ready(clip) => ClipStatus::Ready { seq: clip.seq, height: clip.height },
            ClipState::Failed(error) => ClipStatus::Failed { message: error.to_string() },
        };
        Self { has_clip: current.is_some(), status, failures: video.failures }
    }
}

/// The address of clip `seq`, if it is still the one published. Signed and short-lived: the app
/// hands it to ExoPlayer and never logs it.
pub(crate) fn clip_url(video: &VideoView, seq: u64) -> Option<String> {
    match &video.state {
        ClipState::Ready(clip) if clip.seq == seq => Some(clip.url().to_owned()),
        _ => None,
    }
}

/// What the app measures on its ExoPlayer at each beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct VideoMeasure {
    pub position_ms: u64,
    /// Zero while unknown.
    pub duration_ms: u64,
    /// `playWhenReady` is off.
    pub paused: bool,
    pub ended: bool,
    /// Not ready yet (buffering after a seek, or loading): no speed nudge then.
    pub buffering: bool,
}

/// What the app does to its player.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct VideoAdjust {
    /// `Some(true)`: pause; `Some(false)`: play.
    pub pause: Option<bool>,
    pub seek_ms: Option<u64>,
    /// The playback speed to set (1.0 but for a nudge).
    pub speed: f32,
    /// Clip position minus audio position at the measure, in ms (positive: the clip is ahead).
    pub drift_ms: i64,
    /// The lead the core aims a seek with now.
    pub lead_ms: u64,
}

/// The alignment of one player: the core's [`Alignment`], the last playback it followed and
/// whether a speed nudge runs.
#[derive(Debug)]
pub(crate) struct Sync {
    alignment: Alignment,
    previous: Option<Arc<PlaybackView>>,
    nudging: bool,
}

impl Sync {
    pub(crate) fn new(now: Instant) -> Self {
        Self { alignment: Alignment::new(now), previous: None, nudging: false }
    }

    /// One beat (or, with `force`, a player that just became ready): first what a change of the
    /// playback asks (`Alignment::follow`), else the drift check (`Alignment::align`), and the speed.
    pub(crate) fn step(
        &mut self,
        view: &Arc<PlaybackView>,
        measure: VideoMeasure,
        force: bool,
        now: Instant,
    ) -> VideoAdjust {
        let video = Measure {
            position: Duration::from_millis(measure.position_ms),
            duration: Duration::from_millis(measure.duration_ms),
            paused: measure.paused,
            ended: measure.ended,
        };
        let followed = match &self.previous {
            Some(previous) if !Arc::ptr_eq(previous, view) => self.alignment.follow(previous, view, now),
            _ => None,
        };
        self.previous = Some(Arc::clone(view));
        let adjust = match followed {
            Some(adjust) if !force => adjust,
            _ => self.alignment.align(view, video, force, now),
        };
        let audio = view.position(now);
        let drift_ms = signed_ms(video.position, audio);
        let speed = self.speed(view, measure, &adjust, drift_ms);
        VideoAdjust {
            pause: adjust.pause,
            seek_ms: adjust.seek.map(millis),
            speed,
            drift_ms,
            lead_ms: millis(self.alignment.lead()),
        }
    }

    /// A nudge only while both play, the clip is ready, nothing else moves it and the gap is under
    /// the core's threshold (past it, the core's seek is due once its pause between two ends).
    fn speed(&mut self, view: &PlaybackView, measure: VideoMeasure, adjust: &Adjust, drift_ms: i64) -> f32 {
        let steady = view.is_playing()
            && !measure.paused
            && !measure.ended
            && !measure.buffering
            && adjust.is_none()
            && drift_ms.unsigned_abs() <= millis(DRIFT)
            && (measure.duration_ms == 0 || measure.position_ms < measure.duration_ms);
        let gap = drift_ms.unsigned_abs();
        self.nudging =
            steady && if self.nudging { gap > millis(NUDGE_STOP) } else { gap > millis(NUDGE_START) };
        if !self.nudging {
            return 1.0;
        }
        // Ahead: slower; behind: faster.
        let change = (drift_ms as f32 / 1000.0 * NUDGE_GAIN).clamp(-NUDGE_MAX, NUDGE_MAX);
        1.0 - change
    }
}

fn signed_ms(video: Duration, audio: Duration) -> i64 {
    let gap = i64::try_from(millis(video.abs_diff(audio))).unwrap_or(i64::MAX);
    if video >= audio { gap } else { -gap }
}

/// The alignment of one ExoPlayer to the core's audio: one per player the app opens.
#[derive(uniffi::Object)]
pub struct VideoSync {
    inner: Weak<Inner>,
    sync: Mutex<Sync>,
}

impl VideoSync {
    pub(crate) fn new(inner: &Arc<Inner>) -> Arc<Self> {
        Arc::new(Self { inner: Arc::downgrade(inner), sync: Mutex::new(Sync::new(Instant::now())) })
    }
}

#[uniffi::export]
impl VideoSync {
    /// Where the audio is now: a player that just opened goes there first and settles, before the
    /// forced alignment (as the desktop's pipeline seeks on opening), so the time it took to open
    /// does not count as the cost of a seek in the core's lead.
    pub fn audio_ms(&self) -> u64 {
        self.inner
            .upgrade()
            .and_then(|inner| inner.current(|core| millis(core.snapshot().playback.position(Instant::now()))))
            .unwrap_or(0)
    }

    /// At each beat while the clip shows and plays, at each playback change, and with `force` once
    /// the player is ready: what to do to it. Nothing (speed 1) while the core is stopped.
    pub fn align(&self, measure: VideoMeasure, force: bool) -> VideoAdjust {
        let view = self
            .inner
            .upgrade()
            .and_then(|inner| inner.current(|core| Arc::clone(&core.snapshot().playback)));
        let Some(view) = view else {
            return VideoAdjust { pause: None, seek_ms: None, speed: 1.0, drift_ms: 0, lead_ms: 0 };
        };
        self.sync.lock().unwrap_or_else(PoisonError::into_inner).step(&view, measure, force, Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use psytify_core::api::Track;
    use psytify_core::player::{Clock, PlaybackState, Queue};

    use super::*;
    use crate::test_support::track;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn clip_track(n: u128) -> Track {
        Track { has_video: true, ..track(n) }
    }

    fn playing(position: Duration, at: Instant) -> Arc<PlaybackView> {
        let mut queue = Queue::default();
        queue.replace(vec![clip_track(1)], 0, 1);
        Arc::new(PlaybackView {
            state: PlaybackState::Playing,
            clock: Clock::running(position, at),
            queue: Arc::new(queue),
            ..PlaybackView::default()
        })
    }

    fn measure(position_ms: u64) -> VideoMeasure {
        VideoMeasure { position_ms, duration_ms: 200_000, paused: false, ended: false, buffering: false }
    }

    #[test]
    fn a_ready_player_takes_the_audio_position_and_play_at_once() {
        let start = Instant::now();
        let mut sync = Sync::new(start);
        let view = playing(ms(30_000), start);
        let paused = VideoMeasure { paused: true, ..measure(0) };
        let adjust = sync.step(&view, paused, true, start);
        // The core aims a first jump ahead by the gap, at most 1.5 s.
        assert_eq!((adjust.pause, adjust.seek_ms, adjust.speed), (Some(false), Some(31_500), 1.0));
        assert_eq!(adjust.drift_ms, -30_000);
    }

    #[test]
    fn a_small_gap_is_closed_with_speed_and_a_big_one_with_a_seek() {
        let start = Instant::now();
        let mut sync = Sync::new(start - Duration::from_secs(10));
        let view = playing(ms(10_000), start);
        let at = start + ms(3_000);
        // On the audio (13 s): nothing.
        let adjust = sync.step(&view, measure(13_030), false, at);
        assert_eq!((adjust.seek_ms, adjust.speed, adjust.drift_ms), (None, 1.0, 30));
        // 100 ms ahead: 4 % slower, no seek.
        let adjust = sync.step(&view, measure(13_100), false, at);
        assert_eq!(adjust.seek_ms, None);
        assert!((adjust.speed - 0.96).abs() < 1e-4, "{}", adjust.speed);
        // The nudge holds down to 20 ms, then stops.
        assert!(sync.step(&view, measure(13_040), false, at).speed < 1.0);
        assert_eq!(sync.step(&view, measure(13_010), false, at).speed, 1.0);
        // 50 ms behind is not enough to start again; 80 ms behind is faster.
        assert_eq!(sync.step(&view, measure(12_950), false, at).speed, 1.0);
        assert!(sync.step(&view, measure(12_920), false, at).speed > 1.0);
        // A second behind: the core's seek with lead, and no nudge with it.
        let adjust = sync.step(&view, measure(12_000), false, at);
        assert_eq!((adjust.seek_ms, adjust.speed, adjust.lead_ms), (Some(14_000), 1.0, 1_000));
    }

    #[test]
    fn a_gap_past_the_threshold_waits_for_the_core_and_is_never_nudged() {
        let start = Instant::now();
        let mut sync = Sync::new(start);
        let view = playing(ms(10_000), start);
        // One second after a jump the core waits; the gap is past 200 ms: no nudge either.
        let adjust = sync.step(&view, measure(11_900), false, start + ms(1_000));
        assert_eq!((adjust.seek_ms, adjust.speed, adjust.drift_ms), (None, 1.0, 900));
    }

    #[test]
    fn no_nudge_while_buffering_or_ended() {
        let start = Instant::now();
        let mut sync = Sync::new(start - Duration::from_secs(10));
        let view = playing(ms(10_000), start);
        let buffering = VideoMeasure { buffering: true, ..measure(10_150) };
        assert_eq!(sync.step(&view, buffering, false, start).speed, 1.0);
        let ended = VideoMeasure { ended: true, ..measure(10_150) };
        assert_eq!(sync.step(&view, ended, false, start).speed, 1.0);
    }

    #[test]
    fn a_pause_of_the_audio_pauses_the_clip_and_a_resume_moves_it() {
        let start = Instant::now();
        let mut sync = Sync::new(start - Duration::from_secs(10));
        let running = playing(ms(10_000), start);
        assert_eq!(sync.step(&running, measure(10_000), false, start).pause, None);
        let paused = Arc::new(PlaybackView {
            state: PlaybackState::Paused,
            clock: Clock::paused(ms(11_000), start + ms(1_000)),
            ..(*running).clone()
        });
        let adjust = sync.step(&paused, measure(11_000), false, start + ms(1_000));
        assert_eq!((adjust.pause, adjust.seek_ms), (Some(true), None));
        let resumed = playing(ms(11_000), start + ms(5_000));
        let adjust =
            sync.step(&resumed, VideoMeasure { paused: true, ..measure(11_000) }, false, start + ms(5_000));
        assert_eq!((adjust.pause, adjust.seek_ms), (Some(false), Some(11_000)));
    }

    #[test]
    fn the_snapshot_shows_the_clip_of_the_current_track_only() {
        let start = Instant::now();
        let view = playing(ms(0), start);
        let id = view.queue.current().map(|t| t.id);
        assert_eq!(
            VideoSnapshot::of(&view, &VideoView::default()),
            VideoSnapshot { has_clip: true, status: ClipStatus::Idle, failures: 0 }
        );
        let loading = VideoView { track: id, state: ClipState::Loading, failures: 2 };
        assert_eq!(VideoSnapshot::of(&view, &loading).status, ClipStatus::Loading);
        let elsewhere = VideoView { track: Some(track(9).id), ..loading.clone() };
        assert_eq!(VideoSnapshot::of(&view, &elsewhere).status, ClipStatus::Idle);
        let without = PlaybackView::default();
        assert!(!VideoSnapshot::of(&without, &loading).has_clip);
        assert_eq!(clip_url(&loading, 1), None);
    }
}
