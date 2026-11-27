//! Pixel motion (desktop v2 §3): small eased moves that land on whole APs on every frame and ask for frames
//! only while they run. `Reduce motion` makes every duration 0, so nothing runs and nothing is requested.
//!
//! - [`Tween`]: a value that eases from where it is to a target (`ease_out`, `MOTION_*` durations);
//! - [`follow`]: a tween kept per element id across frames (the selection marker, a seek bar's height);
//! - the hover of every control is a tween in `components::control::wire` (`Visual::hover_t`);
//! - [`Enter`]: a page enter, sections rising with opacity, staggered;
//! - [`steps`] and [`ap_round`]: quantise progress to frames and lengths to whole APs.
//!
//! A running animation calls `Window::request_animation_frame`, which redraws only the view being drawn
//! (and its ancestors, which reuse their cached children).

use std::time::{Duration, Instant};

use gpui::{AnyElement, App, Context, ElementId, IntoElement, ParentElement, Styled, Window, div};
use psytify_design::tokens::motion::{
    ENTER_RISE_AP, MOTION_ENTER, Motion, STAGGER_MAX_STEPS, STAGGER_MS, ease_out,
};

use crate::pixel::SpriteStore;
use crate::theme::Ap;

/// Motion is reduced (environment, preference or system): every duration is 0.
pub fn reduced(cx: &App) -> bool {
    cx.try_global::<SpriteStore>().is_some_and(SpriteStore::reduce_motion)
}

/// The duration of `motion` in effect now.
pub fn duration(motion: Motion, cx: &App) -> u32 {
    motion.effective_ms(reduced(cx))
}

/// A value easing from `from` to `to` over `ms` from `start`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    ms: u32,
}

impl Tween {
    /// Resting at `value`.
    pub fn at(value: f32) -> Self {
        Self { from: value, to: value, start: Instant::now(), ms: 0 }
    }

    /// Linear progress 0..=1.
    fn progress(&self, now: Instant) -> f32 {
        if self.ms == 0 {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32() * 1000.0;
        (elapsed / self.ms as f32).clamp(0.0, 1.0)
    }

    /// The eased value at `now`.
    pub fn value(&self, now: Instant) -> f32 {
        self.from + (self.to - self.from) * ease_out(self.progress(now))
    }

    pub fn running(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
    }

    /// Moves towards `to` from wherever the value is now; `false` if `to` is already the target.
    pub fn set(&mut self, to: f32, ms: u32, now: Instant) -> bool {
        if (to - self.to).abs() < f32::EPSILON {
            return false;
        }
        self.from = self.value(now);
        self.to = to;
        self.start = now;
        self.ms = ms;
        true
    }
}

/// Progress 0..=1 as `n` stepped frames: 0 at rest, `n` when done (a 3-frame step-in).
pub fn steps(progress: f32, n: u32) -> u32 {
    if !progress.is_finite() || progress <= 0.0 {
        return 0;
    }
    // A handful of frames: the cast is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = (progress.min(1.0) * n as f32).ceil() as u32;
    frames.min(n)
}

/// A length in AP rounded to whole APs (never blurred between two).
pub fn ap_round(value_ap: f32) -> u32 {
    if !value_ap.is_finite() || value_ap <= 0.0 {
        return 0;
    }
    // Small lengths in AP.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let whole = value_ap.round() as u32;
    whole
}

/// Asks for the next frame while something runs (only the view being drawn redraws).
pub fn keep_running(window: &Window, running: bool) {
    if running {
        window.request_animation_frame();
    }
}

/// A value that follows `target` for the element `id`, easing in `motion`: the first frame starts at the
/// target, later changes animate. Asks for frames while it runs.
pub fn follow(
    id: impl Into<ElementId>,
    target: f32,
    motion: Motion,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    let ms = duration(motion, cx);
    let state = window.use_keyed_state(id.into(), cx, |_, _: &mut Context<Tween>| Tween::at(target));
    let now = Instant::now();
    let tween = state.update(cx, |tween, _| {
        tween.set(target, ms, now);
        *tween
    });
    keep_running(window, tween.running(now));
    tween.value(now)
}

/// The last change of a value kept per element: what it was and when it changed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Change<T> {
    value: T,
    at: Option<Instant>,
}

/// Milliseconds since `value` last changed for the element `id`, while that is under `span_ms`; `None` at
/// rest, on the first frame and with reduced motion. Asks for frames while it returns `Some` (a glyph swap,
/// a like fill: a few stepped frames, then nothing).
pub fn since_change<T: PartialEq + Copy + 'static>(
    id: impl Into<ElementId>,
    value: T,
    span_ms: u32,
    window: &mut Window,
    cx: &mut App,
) -> Option<u32> {
    let reduced = reduced(cx);
    let state =
        window.use_keyed_state(id.into(), cx, |_, _: &mut Context<Change<T>>| Change { value, at: None });
    let now = Instant::now();
    let change = state.update(cx, |change, _| {
        if change.value != value {
            *change = Change { value, at: (!reduced).then_some(now) };
        }
        *change
    });
    let elapsed = change.at.map(|at| now.saturating_duration_since(at).as_millis())?;
    let elapsed = u32::try_from(elapsed).unwrap_or(u32::MAX);
    let running = elapsed < span_ms;
    keep_running(window, running);
    running.then_some(elapsed)
}

/// A page enter: each section rises `ENTER_RISE_AP` with opacity in `MOTION_ENTER`, staggered by `STAGGER_MS`
/// (at most `STAGGER_MAX_STEPS`). Back and forward use the same move without the rise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Enter {
    start: Instant,
    rise: bool,
}

impl Enter {
    /// A new enter, or `None` with reduced motion (the page simply appears).
    pub fn begin(rise: bool, cx: &App) -> Option<Self> {
        (!reduced(cx)).then(|| Self { start: Instant::now(), rise })
    }

    /// The rise (in AP) and opacity of section `ix` at `now`.
    pub fn section(&self, ix: usize, now: Instant) -> (u32, f32) {
        let step = u32::try_from(ix).unwrap_or(u32::MAX).min(STAGGER_MAX_STEPS);
        let begin = self.start + Duration::from_millis(u64::from(step * STAGGER_MS));
        let elapsed = now.saturating_duration_since(begin).as_secs_f32() * 1000.0;
        let t = ease_out((elapsed / MOTION_ENTER.ms as f32).clamp(0.0, 1.0));
        let rise = if self.rise { ap_round((1.0 - t) * ENTER_RISE_AP as f32) } else { 0 };
        (rise, t)
    }

    /// Still moving: the last section has not landed.
    pub fn running(&self, now: Instant) -> bool {
        let total = MOTION_ENTER.ms + STAGGER_MAX_STEPS * STAGGER_MS;
        now.saturating_duration_since(self.start) < Duration::from_millis(u64::from(total))
    }

    /// Section `ix` wrapped in its move (nothing changes once it has landed).
    pub fn wrap(&self, element: AnyElement, ix: usize, ap: Ap, now: Instant) -> AnyElement {
        let (rise, opacity) = self.section(ix, now);
        if rise == 0 && opacity >= 1.0 {
            return element;
        }
        div().relative().top(ap.n(rise)).opacity(opacity).child(element).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn a_tween_eases_to_its_target_and_retargets_from_where_it_is() {
        let t0 = Instant::now();
        let mut tween = Tween::at(0.0);
        assert!(!tween.running(t0));
        assert!(tween.set(1.0, 100, t0));
        assert!(!tween.set(1.0, 100, t0), "same target: nothing to do");
        assert!(tween.running(t0 + MS(50)));
        let half = tween.value(t0 + MS(50));
        assert!(half > 0.5 && half < 1.0, "ease-out is past half way at half time: {half}");
        assert_eq!(tween.value(t0 + MS(100)), 1.0);
        // Back to 0 from the middle: starts where it was.
        tween.set(0.0, 100, t0 + MS(50));
        assert!((tween.value(t0 + MS(50)) - half).abs() < 1e-4);
        assert_eq!(tween.value(t0 + MS(200)), 0.0);
        // Zero duration (reduced motion): the target at once, never running.
        let mut cut = Tween::at(0.0);
        cut.set(1.0, 0, t0);
        assert_eq!(cut.value(t0), 1.0);
        assert!(!cut.running(t0));
    }

    #[test]
    fn progress_steps_in_whole_frames_and_lengths_in_whole_aps() {
        assert_eq!(steps(0.0, 3), 0);
        assert_eq!(steps(0.01, 3), 1);
        assert_eq!(steps(0.5, 3), 2);
        assert_eq!(steps(1.0, 3), 3);
        assert_eq!(steps(f32::NAN, 3), 0);
        assert_eq!(ap_round(1.49), 1);
        assert_eq!(ap_round(1.5), 2);
        assert_eq!(ap_round(-2.0), 0);
    }

    #[test]
    fn sections_enter_staggered_and_land_together_after_the_last_step() {
        let start = Instant::now();
        let enter = Enter { start, rise: true };
        assert_eq!(enter.section(0, start), (ENTER_RISE_AP, 0.0));
        let (rise, opacity) = enter.section(1, start + MS(40));
        assert_eq!((rise, opacity), (ENTER_RISE_AP, 0.0), "the second section starts 40 ms later");
        assert_eq!(enter.section(0, start + MS(220)), (0, 1.0));
        // The seventh and later sections share the sixth step.
        assert_eq!(enter.section(9, start + MS(240)), enter.section(6, start + MS(240)));
        assert!(enter.running(start + MS(400)));
        assert!(!enter.running(start + MS(461)));
        let back = Enter { start, rise: false };
        assert_eq!(back.section(0, start).0, 0, "back and forward do not rise");
    }
}
