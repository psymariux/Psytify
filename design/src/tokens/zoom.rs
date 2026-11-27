//! UI zoom (desktop v2 §1): the whole desktop UI is laid out in dp times a zoom `z`. `Auto` follows the
//! window's logical width; the user can pin one of [`PRESETS`].

/// Window width at which `Auto` is 100 %.
pub const AUTO_BASE_W: f32 = 1600.0;
/// `Auto` stays between these.
pub const AUTO_MIN: f32 = 1.0;
pub const AUTO_MAX: f32 = 1.5;
/// `Auto` moves in these steps.
pub const STEP: f32 = 0.125;
/// A step is left only this many dp below where it was entered, so a resize does not flicker.
pub const HYSTERESIS: f32 = 48.0;
/// The values the user can pin (Impostazioni → Aspetto, Ctrl+= / Ctrl+-), smallest first.
pub const PRESETS: [f32; 5] = [0.9, 1.0, 1.125, 1.25, 1.5];

/// The window width (logical px) at which `Auto` enters `zoom`.
fn threshold(zoom: f32) -> f32 {
    AUTO_BASE_W * zoom
}

/// `Auto` for a window `width` logical px wide, given the zoom in effect (`current`, for the hysteresis):
/// `clamp(width / 1600, 1, 1.5)` floored to a step; a lower step only once the width is `HYSTERESIS` below
/// the current step's threshold.
pub fn auto_zoom(width: f32, current: f32) -> f32 {
    if !width.is_finite() || width <= 0.0 {
        return AUTO_MIN;
    }
    let raw = (width / AUTO_BASE_W).clamp(AUTO_MIN, AUTO_MAX);
    let stepped = ((raw / STEP + 1e-4).floor() * STEP).clamp(AUTO_MIN, AUTO_MAX);
    if current.is_finite() && current > stepped && current <= AUTO_MAX {
        // Still inside the band below the current step: keep it.
        if width >= threshold(current) - HYSTERESIS {
            return current;
        }
    }
    stepped
}

/// The next preset up from `zoom` (Ctrl+=), or the largest.
pub fn zoom_in(zoom: f32) -> f32 {
    PRESETS.into_iter().find(|preset| *preset > zoom + 1e-3).unwrap_or(PRESETS[PRESETS.len() - 1])
}

/// The next preset down from `zoom` (Ctrl+-), or the smallest.
pub fn zoom_out(zoom: f32) -> f32 {
    PRESETS.into_iter().rev().find(|preset| *preset < zoom - 1e-3).unwrap_or(PRESETS[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_follows_the_width_in_steps_between_one_and_one_and_a_half() {
        assert_eq!(auto_zoom(1280.0, 1.0), 1.0);
        assert_eq!(auto_zoom(1600.0, 1.0), 1.0);
        assert_eq!(auto_zoom(1799.0, 1.0), 1.0);
        assert_eq!(auto_zoom(1800.0, 1.0), 1.125);
        assert_eq!(auto_zoom(2000.0, 1.0), 1.25);
        assert_eq!(auto_zoom(2560.0, 1.0), 1.5);
        assert_eq!(auto_zoom(5000.0, 1.0), 1.5);
        assert_eq!(auto_zoom(f32::NAN, 1.25), 1.0);
    }

    #[test]
    fn a_lower_step_needs_the_hysteresis_band() {
        // 1.25 is entered at 2000: 1990 keeps it, 1951 too, 1952 - 48 = below the band drops to 1.125.
        assert_eq!(auto_zoom(1990.0, 1.25), 1.25);
        assert_eq!(auto_zoom(1952.0, 1.25), 1.25);
        assert_eq!(auto_zoom(1951.0, 1.25), 1.125);
        // Growing never waits.
        assert_eq!(auto_zoom(2250.0, 1.25), 1.375);
        // A pinned value outside the auto range does not hold.
        assert_eq!(auto_zoom(1280.0, 0.9), 1.0);
    }

    #[test]
    fn the_keys_walk_the_presets() {
        assert_eq!(zoom_in(1.0), 1.125);
        assert_eq!(zoom_in(1.2), 1.25);
        assert_eq!(zoom_in(1.5), 1.5);
        assert_eq!(zoom_out(1.0), 0.9);
        assert_eq!(zoom_out(1.375), 1.25);
        assert_eq!(zoom_out(0.9), 0.9);
    }
}
