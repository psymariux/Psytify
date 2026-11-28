//! Slider verticale e banco di slider (le bande dell'equalizzatore nelle impostazioni): la stessa
//! the anatomy of the volume slider (`progress.rs`) set up. A track 2 AP wide and high
//! [`BAND_TRACK_AP`] AP inside a hit area of `ICON_BUTTON`; from the bottom the value in `WARM_WHITE`
//! (`PURE_WHITE` on mouse hover and while dragging), above the `CHARCOAL`/`GRAPHITE` dither; the
//! 6 x 2 AP knob on the head only on mouse hover and while dragging. Under each slider the value
//! (`TYPE_NUMERIC`) and the name (`TYPE_META`); a 1 AP `LINE_STRONG` line crosses the whole bank
//! at the height of `tick` (0 dB).
//!
//! The value is 0..=1 and comes from the caller; while dragging the local one is shown.
//! `on_change` fires at every movement, `on_commit` on release (and at every wheel or arrow step, which
//! have no release): the equaliser is heard at once and saved only once.

use std::rc::Rc;

use gpui::{
    App, Bounds, Context, CursorStyle, ElementId, Entity, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, canvas, div, fill, point, px, size,
};
use psytify_design::tokens::color::{
    CHARCOAL, GRAPHITE, LINE_STRONG, PURE_WHITE, TEXT_PRIMARY, TEXT_TERTIARY, WARM_WHITE,
};
use psytify_design::tokens::size::ICON_BUTTON;
use psytify_design::tokens::space::{SPACE_2, SPACE_3};
use psytify_design::tokens::type_::{TYPE_META, TYPE_NUMERIC};

use super::control::Preview;
use crate::pixel::shape::{focus_ring, static_ring};
use crate::theme::{Ap, TypeStyle, col, dp};

/// Track height of a band, in AP (the guidelines: 64 AP).
pub const BAND_TRACK_AP: u32 = 64;

/// Callback with the band and the new value (0..=1).
pub type OnBand = Rc<dyn Fn(usize, f32, &mut Window, &mut App)>;

struct SliderState {
    bounds: Bounds<Pixels>,
    drag: Option<f32>,
    hover: bool,
    focus: FocusHandle,
}

/// The value under the pointer: 1 at the top, 0 at the bottom.
fn frac_at(bounds: Bounds<Pixels>, y: Pixels) -> f32 {
    let h = f32::from(bounds.size.height);
    if h <= 0.0 { 0.0 } else { (1.0 - f32::from(y - bounds.origin.y) / h).clamp(0.0, 1.0) }
}

/// The bank: a value, a value label and a name per slider.
#[derive(IntoElement)]
pub struct SliderBank {
    id: ElementId,
    values: Vec<f32>,
    values_text: Vec<SharedString>,
    names: Vec<SharedString>,
    tick: Option<f32>,
    step: f32,
    previews: Vec<(usize, Preview)>,
    on_change: Option<OnBand>,
    on_commit: Option<OnBand>,
}

impl SliderBank {
    pub fn new(id: impl Into<ElementId>, values: Vec<f32>) -> Self {
        Self {
            id: id.into(),
            values: values.into_iter().map(|v| v.clamp(0.0, 1.0)).collect(),
            values_text: Vec::new(),
            names: Vec::new(),
            tick: None,
            step: 0.05,
            previews: Vec::new(),
            on_change: None,
            on_commit: None,
        }
    }

    /// The value text under each slider («+3», «0»), already ready.
    pub fn values_text(mut self, texts: Vec<SharedString>) -> Self {
        self.values_text = texts;
        self
    }

    /// The name under the value («31», «1k»).
    pub fn names(mut self, names: Vec<SharedString>) -> Self {
        self.names = names;
        self
    }

    /// The reference line (0 dB) across the whole bank.
    pub fn tick(mut self, at: f32) -> Self {
        self.tick = Some(at.clamp(0.0, 1.0));
        self
    }

    /// Il passo di frecce e rotella.
    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// Gallery only: the state of a slider.
    pub fn preview(mut self, band: usize, preview: Preview) -> Self {
        self.previews.push((band, preview));
        self
    }

    pub fn on_change(mut self, handler: impl Fn(usize, f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    pub fn on_commit(mut self, handler: impl Fn(usize, f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_commit = Some(Rc::new(handler));
        self
    }
}

/// Paints the vertical track on the grid of physical pixels.
fn paint_track(window: &mut Window, bounds: Bounds<Pixels>, ap: Ap, value: f32, lit: bool) {
    let scale = ap.scale;
    let d = ap.device as f32;
    let track_w = 2.0 * d;
    let y0 = (f32::from(bounds.origin.y) * scale).round();
    let h = (f32::from(bounds.size.height) * scale).round();
    let x0 =
        ((f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0) * scale - track_w / 2.0).round();
    let rect = |window: &mut Window, x: f32, y: f32, rw: f32, rh: f32, color| {
        if rw > 0.0 && rh > 0.0 {
            let b = Bounds::new(point(px(x / scale), px(y / scale)), size(px(rw / scale), px(rh / scale)));
            window.paint_quad(fill(b, col(color)));
        }
    };
    // The value is rounded to whole APs: the track never fades.
    let filled = ((h * value) / d).round() * d;
    let head = if lit { PURE_WHITE } else { WARM_WHITE };
    rect(window, x0, y0 + h - filled, track_w, filled, head);
    let rows = ((h - filled) / d).ceil() as u32;
    for r in 0..rows {
        for c in 0..2u32 {
            let color = if (c + r) % 2 == 0 { CHARCOAL } else { GRAPHITE };
            let cy = y0 + r as f32 * d;
            let ch = d.min(y0 + h - filled - cy);
            rect(window, x0 + c as f32 * d, cy, d, ch, color);
        }
    }
    if lit {
        // Pomello 6 x 2 AP sulla testa.
        let ty = (y0 + h - filled - d).clamp(y0, (y0 + h - 2.0 * d).max(y0));
        rect(window, x0 - 2.0 * d, ty, 6.0 * d, 2.0 * d, PURE_WHITE);
    }
}

/// Uno slider verticale del banco.
#[allow(clippy::too_many_arguments)]
fn band(
    id: ElementId,
    index: usize,
    value: f32,
    step: f32,
    preview: Preview,
    on_change: Option<OnBand>,
    on_commit: Option<OnBand>,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Stateful<gpui::Div> {
    let ap = Ap::of(window);
    let state: Entity<SliderState> =
        window.use_keyed_state(id.clone(), cx, |_, cx: &mut Context<SliderState>| SliderState {
            bounds: Bounds::default(),
            drag: None,
            hover: false,
            focus: cx.focus_handle(),
        });
    let (drag, hover, focus) = {
        let s = state.read(cx);
        (s.drag, s.hover, s.focus.clone())
    };
    let (hovered, dragging) = match preview {
        Preview::Live => (hover, drag.is_some()),
        Preview::Rest | Preview::Focused => (false, false),
        Preview::Hover => (true, false),
        Preview::Pressed => (true, true),
    };
    let shown = drag.unwrap_or(value);
    let lit = hovered || dragging;
    let track_h = ap.n(BAND_TRACK_AP);
    let paint_state = state.clone();
    let track = canvas(
        |_, _, _| (),
        move |bounds, (), window, cx| {
            paint_state.update(cx, |s, _| s.bounds = bounds);
            paint_track(window, bounds, Ap::of(window), shown, lit);
        },
    )
    .absolute()
    .top(px(0.0))
    .left(px(0.0))
    .size_full();
    // The global drag listeners exist only while dragging.
    let drag_listeners = {
        let state = state.clone();
        let on_change = on_change.clone();
        let on_commit = on_commit.clone();
        canvas(
            |_, _, _| (),
            move |_, (), window, _| {
                if drag.is_none() {
                    return;
                }
                let (move_state, move_change) = (state.clone(), on_change.clone());
                window.on_mouse_event(move |event: &MouseMoveEvent, _, window, cx| {
                    let frac = frac_at(move_state.read(cx).bounds, event.position.y);
                    move_state.update(cx, |s, cx| {
                        s.drag = Some(frac);
                        cx.notify();
                    });
                    if let Some(f) = &move_change {
                        f(index, frac, window, cx);
                    }
                });
                let (up_state, up_commit) = (state.clone(), on_commit.clone());
                window.on_mouse_event(move |event: &MouseUpEvent, _, window, cx| {
                    let frac = frac_at(up_state.read(cx).bounds, event.position.y);
                    let was_dragging = up_state.update(cx, |s, cx| {
                        let was = s.drag.take().is_some();
                        cx.notify();
                        was
                    });
                    if was_dragging && let Some(f) = &up_commit {
                        f(index, frac, window, cx);
                    }
                });
            },
        )
        .absolute()
    };
    let down = state.clone();
    let hover_state = state.clone();
    let wheel = state.clone();
    let (down_change, wheel_change, key_change) = (on_change.clone(), on_change.clone(), on_change);
    let (wheel_commit, key_commit) = (on_commit.clone(), on_commit);
    let ring = if preview == Preview::Focused {
        static_ring(1, col(PURE_WHITE), ap)
    } else {
        focus_ring(1, col(PURE_WHITE), focus.clone(), ap)
    };
    div()
        .id(id)
        .relative()
        .w(dp(ICON_BUTTON))
        .h(track_h)
        .flex_none()
        .track_focus(&focus)
        .tab_stop(true)
        .cursor(CursorStyle::PointingHand)
        .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
            let frac = frac_at(down.read(cx).bounds, event.position.y);
            down.update(cx, |s, cx| {
                s.drag = Some(frac);
                cx.notify();
            });
            if let Some(f) = &down_change {
                f(index, frac, window, cx);
            }
        })
        .on_hover(move |hovering, _, cx| {
            hover_state.update(cx, |s, cx| {
                if s.hover != *hovering {
                    s.hover = *hovering;
                    cx.notify();
                }
            });
        })
        .on_scroll(move |event, window, cx| {
            let dy = f32::from(event.delta.pixel_delta(px(16.0)).y);
            if dy == 0.0 {
                return;
            }
            let current = wheel.read(cx).drag.unwrap_or(value);
            let next = (current + dy.signum() * step).clamp(0.0, 1.0);
            if let Some(f) = &wheel_change {
                f(index, next, window, cx);
            }
            if let Some(f) = &wheel_commit {
                f(index, next, window, cx);
            }
        })
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            let dir = match event.keystroke.key.as_str() {
                "down" => -1.0,
                "up" => 1.0,
                _ => return,
            };
            let next = (value + dir * step).clamp(0.0, 1.0);
            if let Some(f) = &key_change {
                f(index, next, window, cx);
            }
            if let Some(f) = &key_commit {
                f(index, next, window, cx);
            }
            cx.stop_propagation();
        })
        .child(track)
        .child(drag_listeners)
        .child(
            div()
                .absolute()
                .top(px(0.0))
                .bottom(px(0.0))
                .left(dp(ICON_BUTTON / 2.0) - ap.one())
                .w(ap.n(2))
                .child(ring),
        )
}

impl RenderOnce for SliderBank {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let track_h = f32::from(ap.n(BAND_TRACK_AP));
        let mut columns = div().relative().flex().items_start().gap(dp(SPACE_3));
        if let Some(tick) = self.tick {
            // The 0 line sits behind the tracks, on the art-pixel grid.
            let top = ((1.0 - tick) * track_h / f32::from(ap.one())).round() * f32::from(ap.one());
            columns = columns.child(
                div().absolute().left(px(0.0)).right(px(0.0)).top(px(top)).h(ap.one()).bg(col(LINE_STRONG)),
            );
        }
        for (ix, value) in self.values.iter().copied().enumerate() {
            let preview =
                self.previews.iter().find(|(band_ix, _)| *band_ix == ix).map_or(Preview::Live, |(_, p)| *p);
            let slider = band(
                (self.id.clone(), SharedString::from(format!("band-{ix}"))).into(),
                ix,
                value,
                self.step,
                preview,
                self.on_change.clone(),
                self.on_commit.clone(),
                window,
                cx,
            );
            let value_text = self.values_text.get(ix).cloned().unwrap_or_default();
            let name = self.names.get(ix).cloned().unwrap_or_default();
            columns = columns.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(dp(SPACE_2))
                    .child(slider)
                    .child(div().type_role(TYPE_NUMERIC).text_color(col(TEXT_PRIMARY)).child(value_text))
                    .child(div().type_role(TYPE_META).text_color(col(TEXT_TERTIARY)).child(name)),
            );
        }
        columns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_top_of_the_track_is_one_and_the_bottom_is_zero() {
        let bounds = Bounds::new(point(px(0.0), px(100.0)), size(px(32.0), px(128.0)));
        assert_eq!(frac_at(bounds, px(100.0)), 1.0);
        assert_eq!(frac_at(bounds, px(228.0)), 0.0);
        assert_eq!(frac_at(bounds, px(164.0)), 0.5);
        assert_eq!(frac_at(bounds, px(0.0)), 1.0, "sopra il binario resta in cima");
        assert_eq!(frac_at(bounds, px(900.0)), 0.0);
        assert_eq!(frac_at(Bounds::default(), px(10.0)), 0.0);
    }
}
