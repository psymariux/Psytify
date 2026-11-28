//! The playback clock: the point where the track was at a given instant and whether it is
//! running. The core re-anchors it at each transition (start, pause, seek, end) and when its
//! tick senses that the real position has drifted more than [`DRIFT`]; between one anchor and
//! the next the position is not published: each frontend computes it itself with [`Clock::at`], as the
//! progress bar and the synced lyrics do at each Tick.

use std::time::{Duration, Instant};

/// How far the real position may drift from the computed one before the core re-anchors
/// the clock and publishes it.
pub const DRIFT: Duration = Duration::from_millis(250);

/// Where the track was at `at`, and whether it has been running since then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    pub position: Duration,
    pub at: Instant,
    pub playing: bool,
}

impl Clock {
    /// A clock stopped at `position`.
    pub fn paused(position: Duration, at: Instant) -> Self {
        Self { position, at, playing: false }
    }

    /// A clock running from `position` starting at `at`.
    pub fn running(position: Duration, at: Instant) -> Self {
        Self { position, at, playing: true }
    }

    /// The position at `now`: the anchored one, plus the time passed if it is running.
    pub fn at(&self, now: Instant) -> Duration {
        if self.playing {
            self.position.saturating_add(now.saturating_duration_since(self.at))
        } else {
            self.position
        }
    }

    /// `true` if the real position `real` is beyond [`DRIFT`] from the one computed at `now`.
    pub fn drifted(&self, real: Duration, now: Instant) -> bool {
        real.abs_diff(self.at(now)) > DRIFT
    }
}

impl Default for Clock {
    /// Stopped at the start. The instant does not count until the clock runs.
    fn default() -> Self {
        Self::paused(Duration::ZERO, Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_clock_advances_and_a_paused_one_does_not() {
        let start = Instant::now();
        let running = Clock::running(Duration::from_secs(10), start);
        assert_eq!(running.at(start + Duration::from_secs(3)), Duration::from_secs(13));
        let paused = Clock::paused(Duration::from_secs(10), start);
        assert_eq!(paused.at(start + Duration::from_secs(3)), Duration::from_secs(10));
    }

    #[test]
    fn an_instant_before_the_anchor_never_runs_backwards() {
        let start = Instant::now();
        let clock = Clock::running(Duration::from_secs(10), start + Duration::from_secs(5));
        assert_eq!(clock.at(start), Duration::from_secs(10));
    }

    #[test]
    fn drift_is_measured_against_the_extrapolated_position() {
        let start = Instant::now();
        let clock = Clock::running(Duration::ZERO, start);
        let later = start + Duration::from_secs(2);
        assert!(!clock.drifted(Duration::from_millis(2_200), later), "220 ms: still fine");
        assert!(clock.drifted(Duration::from_millis(2_300), later), "300 ms: re-anchor");
        assert!(clock.drifted(Duration::from_millis(1_700), later), "late as well as early");
    }
}
