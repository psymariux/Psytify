//! Motion (section 7 and desktop v2 §3). Two clocks: the sprites' (durations in `sprites.json`, never
//! interpolated) and the UI's (colours, opacity, offsets quantised to whole APs).

/// Curve of a UI transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    /// No interpolation: the value snaps (press, caret, sprite frames).
    Step,
    /// `cubic-bezier(0.2, 0.0, 0.0, 1.0)`; no springs, no bounce.
    EaseOut,
}

/// Control points of `EaseOut`: `(x1, y1, x2, y2)`.
pub const EASE_OUT_BEZIER: [f32; 4] = [0.2, 0.0, 0.0, 1.0];

/// A transition: duration in ms and curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub ms: u32,
    pub curve: Curve,
}

impl Motion {
    /// Duration in effect: with `reduce_motion` every UI duration is 0.
    pub const fn effective_ms(self, reduce_motion: bool) -> u32 {
        if reduce_motion { 0 } else { self.ms }
    }
}

/// Press and ledge collapse: instant, like a key.
pub const MOTION_PRESS: Motion = Motion { ms: 0, curve: Curve::Step };
/// Hover: fill, border, icon tint, the tile rise and the row actions.
pub const MOTION_HOVER: Motion = Motion { ms: 70, curve: Curve::EaseOut };
/// Tooltips and menus (opacity only), the toggle knob (2 steps of 50), the 3-frame steps.
pub const MOTION_FAST: Motion = Motion { ms: 100, curve: Curve::EaseOut };
/// Page content fade, toast, dialog (opacity + scrim), the player bar cover cross-fade.
pub const MOTION_BASE: Motion = Motion { ms: 160, curve: Curve::EaseOut };
/// Side panel and sidebar width, sheets; offsets quantised to `ap_device`.
pub const MOTION_PANEL: Motion = Motion { ms: 200, curve: Curve::EaseOut };
/// Page enter: each section rises `ENTER_RISE_AP` with opacity.
pub const MOTION_ENTER: Motion = Motion { ms: 220, curve: Curve::EaseOut };
/// Lyrics scroll, quantised to whole device pixels.
pub const MOTION_LYRICS: Motion = Motion { ms: 280, curve: Curve::EaseOut };

/// Delay between two sections of a page enter.
pub const STAGGER_MS: u32 = 40;
/// At most this many stagger steps: later sections start with the sixth.
pub const STAGGER_MAX_STEPS: u32 = 6;
/// How far a section rises when a page enters, in AP.
pub const ENTER_RISE_AP: u32 = 4;
/// Frames of a stepped move (the tile play button, the selection marker).
pub const STEP_FRAMES: u32 = 3;

/// Tooltip delay.
pub const TOOLTIP_DELAY_MS: u32 = 500;
/// How long a toast stays.
pub const TOAST_HOLD_MS: u32 = 4000;
/// Caret: 530 ms on, 530 ms off (stepped).
pub const CARET_BLINK_ON_MS: u32 = 530;
pub const CARET_BLINK_OFF_MS: u32 = 530;
/// Progress bar and clock refresh (never per frame). Stays on with `reduce_motion`.
pub const PROGRESS_TICK_MS: u32 = 250;
/// No sprite frame is shorter than this.
pub const SPRITE_FRAME_MIN_MS: u32 = 80;

/// Named UI transitions, for the tests.
pub const ALL: [(&str, Motion); 7] = [
    ("MOTION_PRESS", MOTION_PRESS),
    ("MOTION_HOVER", MOTION_HOVER),
    ("MOTION_FAST", MOTION_FAST),
    ("MOTION_BASE", MOTION_BASE),
    ("MOTION_PANEL", MOTION_PANEL),
    ("MOTION_ENTER", MOTION_ENTER),
    ("MOTION_LYRICS", MOTION_LYRICS),
];

/// `EaseOut` at `t` in 0..=1: solves the bezier's x for `t` (Newton, then bisection) and returns y.
pub fn ease_out(t: f32) -> f32 {
    if !t.is_finite() || t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let [x1, y1, x2, y2] = EASE_OUT_BEZIER;
    let bezier = |p1: f32, p2: f32, s: f32| {
        let u = 1.0 - s;
        3.0 * u * u * s * p1 + 3.0 * u * s * s * p2 + s * s * s
    };
    let slope = |p1: f32, p2: f32, s: f32| {
        let u = 1.0 - s;
        3.0 * u * u * p1 + 6.0 * u * s * (p2 - p1) + 3.0 * s * s * (1.0 - p2)
    };
    let mut s = t;
    for _ in 0..8 {
        let dx = bezier(x1, x2, s) - t;
        let d = slope(x1, x2, s);
        if dx.abs() < 1e-5 {
            return bezier(y1, y2, s);
        }
        if d.abs() < 1e-6 {
            break;
        }
        s = (s - dx / d).clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for _ in 0..24 {
        s = (lo + hi) / 2.0;
        if bezier(x1, x2, s) < t { lo = s } else { hi = s }
    }
    bezier(y1, y2, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_out_starts_fast_ends_flat_and_stays_in_range() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert_eq!(ease_out(-1.0), 0.0);
        assert_eq!(ease_out(f32::NAN), 0.0);
        let mut previous = 0.0;
        for step in 1..=20 {
            let y = ease_out(step as f32 / 20.0);
            assert!((0.0..=1.0).contains(&y));
            assert!(y >= previous, "monotonic");
            previous = y;
        }
        assert!(ease_out(0.25) > 0.5, "ease-out covers most of the way early");
    }
}
