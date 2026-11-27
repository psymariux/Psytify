//! L'orologio della riproduzione: il punto in cui si trovava il brano a un certo istante e se sta
//! scorrendo. Il core lo riancora a ogni transizione (avvio, pausa, seek, fine) e quando il suo
//! battito sente che la posizione vera si è allontanata di più di [`DRIFT`]; fra un'ancora e
//! l'altra la posizione non si pubblica: ogni frontend la calcola da sé con [`Clock::at`], come la
//! barra di avanzamento e il testo sincronizzato fanno a ogni Tick.

use std::time::{Duration, Instant};

/// Di quanto può scostarsi la posizione vera da quella calcolata prima che il core riancori
/// l'orologio e lo pubblichi.
pub const DRIFT: Duration = Duration::from_millis(250);

/// Dove si trovava il brano a `at`, e se da allora scorre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    pub position: Duration,
    pub at: Instant,
    pub playing: bool,
}

impl Clock {
    /// Un orologio fermo a `position`.
    pub fn paused(position: Duration, at: Instant) -> Self {
        Self { position, at, playing: false }
    }

    /// Un orologio che scorre da `position` a partire da `at`.
    pub fn running(position: Duration, at: Instant) -> Self {
        Self { position, at, playing: true }
    }

    /// La posizione a `now`: quella ancorata, più il tempo passato se scorre.
    pub fn at(&self, now: Instant) -> Duration {
        if self.playing {
            self.position.saturating_add(now.saturating_duration_since(self.at))
        } else {
            self.position
        }
    }

    /// `true` se la posizione vera `real` è oltre [`DRIFT`] da quella calcolata a `now`.
    pub fn drifted(&self, real: Duration, now: Instant) -> bool {
        real.abs_diff(self.at(now)) > DRIFT
    }
}

impl Default for Clock {
    /// Fermo all'inizio. L'istante non conta finché l'orologio non scorre.
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
