//! Calcolo della posizione di destinazione di un seek.

use std::time::Duration;

/// Sposta la posizione di `delta_secs` secondi, senza uscire da `[0, durata]`.
pub fn seek_by(position: Duration, delta_secs: i64, duration: Option<Duration>) -> Duration {
    let delta = Duration::from_secs(delta_secs.unsigned_abs());
    let target =
        if delta_secs >= 0 { position.saturating_add(delta) } else { position.saturating_sub(delta) };
    clamp(target, duration)
}

/// Posizione corrispondente a una frazione della durata (barra di avanzamento).
pub fn seek_fraction(fraction: f32, duration: Duration) -> Duration {
    if !fraction.is_finite() {
        return Duration::ZERO;
    }
    duration.mul_f32(fraction.clamp(0.0, 1.0))
}

/// Frazione già ascoltata, per disegnare la barra.
pub fn progress(position: Duration, duration: Duration) -> f32 {
    if duration.is_zero() {
        return 0.0;
    }
    (position.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
}

fn clamp(target: Duration, duration: Option<Duration>) -> Duration {
    match duration {
        Some(max) => target.min(max),
        None => target,
    }
}

/// `m:ss`, oppure `h:mm:ss` oltre l'ora.
pub fn format_time(time: Duration) -> String {
    let total = time.as_secs();
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN3: Duration = Duration::from_secs(180);

    #[test]
    fn forward_seek_adds_seconds() {
        assert_eq!(seek_by(Duration::from_secs(10), 5, Some(MIN3)), Duration::from_secs(15));
    }

    #[test]
    fn backward_seek_subtracts_seconds() {
        assert_eq!(seek_by(Duration::from_secs(10), -5, Some(MIN3)), Duration::from_secs(5));
    }

    #[test]
    fn backward_seek_stops_at_zero() {
        assert_eq!(seek_by(Duration::from_secs(3), -5, Some(MIN3)), Duration::ZERO);
    }

    #[test]
    fn forward_seek_stops_at_the_end() {
        assert_eq!(seek_by(Duration::from_secs(178), 5, Some(MIN3)), MIN3);
    }

    #[test]
    fn seek_without_known_duration_is_not_clamped() {
        assert_eq!(seek_by(Duration::from_secs(178), 5, None), Duration::from_secs(183));
    }

    #[test]
    fn fraction_maps_onto_duration() {
        assert_eq!(seek_fraction(0.5, MIN3), Duration::from_secs(90));
    }

    #[test]
    fn fraction_outside_range_is_clamped() {
        assert_eq!(seek_fraction(1.7, MIN3), MIN3);
    }

    #[test]
    fn nan_fraction_goes_to_start() {
        assert_eq!(seek_fraction(f32::NAN, MIN3), Duration::ZERO);
    }

    #[test]
    fn progress_of_zero_duration_is_zero() {
        assert_eq!(progress(Duration::from_secs(5), Duration::ZERO), 0.0);
    }

    #[test]
    fn time_under_an_hour_is_minutes_and_seconds() {
        assert_eq!(format_time(Duration::from_secs(215)), "3:35");
    }

    #[test]
    fn time_over_an_hour_shows_hours() {
        assert_eq!(format_time(Duration::from_secs(3725)), "1:02:05");
    }
}
