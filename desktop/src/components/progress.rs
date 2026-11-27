//! ProgressBar (seek) and Slider (volume): the same anatomy, a 24 dp hit area around a track that grows
//! from 1 to 2 AP on hover and shows a 2 x 4 AP thumb. Segmenti da sinistra: trascorso `WARM_WHITE`, bufferizzato `IRON`, resto a
//! dither 50 % `CHARCOAL`/`GRAPHITE`. Trascinabile con il mouse (anche fuori dal binario), ←/→ e
//! rotella per i passi.
//!
//! Il valore è sempre 0..=1 e arriva dal chiamante; durante il trascinamento si mostra il valore
//! locale. `on_change` scatta a ogni movimento (slider) o solo al rilascio (seek, che non deve
//! chiedere un seek al player a ogni pixel).

use std::rc::Rc;

use gpui::{
    App, Bounds, Context, CursorStyle, ElementId, Entity, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    RenderOnce, StatefulInteractiveElement, Styled, Window, canvas, div, fill, point, px, size,
};
use psytify_design::tokens::color::{
    CHARCOAL, GRAPHITE, IRON, PURE_WHITE, STONE, SURFACE_RAISED, WARM_WHITE,
};
use psytify_design::tokens::motion::MOTION_HOVER;
use psytify_design::tokens::size::{SEEK_HIT_H, VOLUME_W};

use super::control::Preview;
use super::tooltip::TooltipBox;
use crate::motion::{ap_round, follow};
use crate::pixel::shape::{focus_ring, static_ring};
use crate::theme::{Ap, col, dp, to_dp};

/// Callback con il nuovo valore (0..=1).
pub type OnValue = Rc<dyn Fn(f32, &mut Window, &mut App)>;
/// Testo dell'anteprima (tempo di destinazione, percentuale) per un valore 0..=1.
pub type Label = Rc<dyn Fn(f32) -> String>;

struct TrackState {
    bounds: Bounds<Pixels>,
    drag: Option<f32>,
    hover: Option<f32>,
    focus: FocusHandle,
}

fn frac_at(bounds: Bounds<Pixels>, x: Pixels) -> f32 {
    let w = f32::from(bounds.size.width);
    if w <= 0.0 { 0.0 } else { ((f32::from(x - bounds.origin.x)) / w).clamp(0.0, 1.0) }
}

#[derive(IntoElement)]
pub struct ProgressBar {
    id: ElementId,
    value: f32,
    buffered: Option<f32>,
    width: Option<f32>,
    step: f32,
    disabled: bool,
    commit_on_release: bool,
    label: Option<Label>,
    preview: Preview,
    on_change: Option<OnValue>,
    on_commit: Option<OnValue>,
}

impl ProgressBar {
    /// Barra di avanzamento: larga quanto il contenitore, commit al rilascio, passo 2 %.
    pub fn new(id: impl Into<ElementId>, value: f32) -> Self {
        Self {
            id: id.into(),
            value: value.clamp(0.0, 1.0),
            buffered: None,
            width: None,
            step: 0.02,
            disabled: false,
            commit_on_release: true,
            label: None,
            preview: Preview::Live,
            on_change: None,
            on_commit: None,
        }
    }

    /// Slider (volume): `VOLUME_W` di larghezza, commit a ogni movimento, passo 5 %.
    pub fn slider(id: impl Into<ElementId>, value: f32) -> Self {
        let mut s = Self::new(id, value);
        s.width = Some(VOLUME_W);
        s.step = 0.05;
        s.commit_on_release = false;
        s
    }

    pub fn buffered(mut self, buffered: f32) -> Self {
        self.buffered = Some(buffered.clamp(0.0, 1.0));
        self
    }

    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn label(mut self, label: impl Fn(f32) -> String + 'static) -> Self {
        self.label = Some(Rc::new(label));
        self
    }

    pub fn preview(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    /// Scatta al rilascio del mouse, dopo l'ultimo `on_change`: lo slider del volume manda il volume a
    /// ogni movimento e lo salva solo qui.
    pub fn on_commit(mut self, handler: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_commit = Some(Rc::new(handler));
        self
    }
}

/// Dipinge i segmenti del binario sulla griglia dei pixel fisici.
#[allow(clippy::too_many_arguments)]
fn paint_track(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    ap: Ap,
    value: f32,
    buffered: Option<f32>,
    head: psytify_design::tokens::color::Rgba,
    track_ap: u32,
    show_thumb: bool,
    disabled: bool,
) {
    let scale = ap.scale;
    let d = ap.device as f32;
    let track_h = track_ap as f32 * d;
    let x0 = (f32::from(bounds.origin.x) * scale).round();
    let w = (f32::from(bounds.size.width) * scale).round();
    let y0 =
        ((f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0) * scale - track_h / 2.0).round();
    let rect = |window: &mut Window, x: f32, y: f32, rw: f32, rh: f32, color| {
        if rw > 0.0 && rh > 0.0 {
            let b = Bounds::new(point(px(x / scale), px(y / scale)), size(px(rw / scale), px(rh / scale)));
            window.paint_quad(fill(b, col(color)));
        }
    };
    let snap = |frac: f32| ((w * frac) / d).round() * d;
    let elapsed = snap(value);
    let buffer = snap(buffered.unwrap_or(0.0)).max(elapsed);
    let elapsed_color = if disabled { STONE } else { head };
    rect(window, x0, y0, elapsed, track_h, elapsed_color);
    rect(window, x0 + elapsed, y0, buffer - elapsed, track_h, IRON);
    // Il resto è un dither a scacchiera di quadrati di 1 AP: CHARCOAL e GRAPHITE.
    let cols = ((w - buffer) / d).ceil() as u32;
    for c in 0..cols {
        for r in 0..track_ap {
            let color = if (c + r) % 2 == 0 { CHARCOAL } else { GRAPHITE };
            let cx = x0 + buffer + c as f32 * d;
            let cw = d.min(x0 + w - cx);
            rect(window, cx, y0 + r as f32 * d, cw, d, color);
        }
    }
    if show_thumb && !disabled {
        // A 2 x 4 AP thumb on the head, centred on the track.
        let tx = (x0 + elapsed - d).clamp(x0, (x0 + w - 2.0 * d).max(x0));
        rect(window, tx, y0 + track_h / 2.0 - 2.0 * d, 2.0 * d, 4.0 * d, PURE_WHITE);
    }
}

impl RenderOnce for ProgressBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let state: Entity<TrackState> =
            window.use_keyed_state(self.id.clone(), cx, |_, cx: &mut Context<TrackState>| TrackState {
                bounds: Bounds::default(),
                drag: None,
                hover: None,
                focus: cx.focus_handle(),
            });
        let (drag, hover, focus) = {
            let s = state.read(cx);
            (s.drag, s.hover, s.focus.clone())
        };
        let (hovered, dragging) = match self.preview {
            Preview::Live => (hover.is_some(), drag.is_some()),
            Preview::Rest | Preview::Focused => (false, false),
            Preview::Hover => (true, false),
            Preview::Pressed => (true, true),
        };
        let shown = drag.unwrap_or(self.value);
        // The track grows from 1 to 2 AP on hover (and while dragging), with the thumb once it is 2 AP.
        let grow_id = ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "grow".into());
        let grow = follow(grow_id, if hovered || dragging { 1.0 } else { 0.0 }, MOTION_HOVER, window, cx);
        let track_ap = 1 + ap_round(grow);
        let head = if hovered || dragging { PURE_WHITE } else { WARM_WHITE };
        let disabled = self.disabled;
        let buffered = self.buffered;
        let paint_state = state.clone();
        let track = canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| {
                paint_state.update(cx, |s, _| s.bounds = bounds);
                let ap = Ap::of(window);
                paint_track(window, bounds, ap, shown, buffered, head, track_ap, track_ap > 1, disabled);
            },
        )
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .size_full();

        let mut root = div().id(self.id.clone()).relative().h(dp(SEEK_HIT_H)).flex_none();
        root = match self.width {
            Some(w) => root.w(dp(w)),
            None => root.w_full(),
        };
        if self.disabled {
            return root.child(track).into_any_element();
        }
        let on_change = self.on_change.clone();
        let on_commit = self.on_commit.clone();
        let commit_on_release = self.commit_on_release;
        let step = self.step;
        let (down, wheel, key, mv) = (state.clone(), state.clone(), state.clone(), state.clone());
        // Ascoltatori globali del trascinamento: esistono solo mentre si trascina.
        let drag_listeners = {
            let state = state.clone();
            let on_change = on_change.clone();
            let on_commit = on_commit.clone();
            canvas(
                |_, _, _| (),
                move |_, (), window, _| {
                    if !dragging || drag.is_none() {
                        return;
                    }
                    let (move_state, move_change) = (state.clone(), on_change.clone());
                    window.on_mouse_event(move |event: &MouseMoveEvent, _, window, cx| {
                        let frac = frac_at(move_state.read(cx).bounds, event.position.x);
                        move_state.update(cx, |s, cx| {
                            s.drag = Some(frac);
                            cx.notify();
                        });
                        if !commit_on_release && let Some(f) = &move_change {
                            f(frac, window, cx);
                        }
                    });
                    let (up_state, up_change, up_commit) =
                        (state.clone(), on_change.clone(), on_commit.clone());
                    window.on_mouse_event(move |event: &MouseUpEvent, _, window, cx| {
                        let frac = frac_at(up_state.read(cx).bounds, event.position.x);
                        up_state.update(cx, |s, cx| {
                            s.drag = None;
                            cx.notify();
                        });
                        if let Some(f) = &up_change {
                            f(frac, window, cx);
                        }
                        if let Some(f) = &up_commit {
                            f(frac, window, cx);
                        }
                    });
                },
            )
            .absolute()
        };
        let key_change = on_change.clone();
        let wheel_change = on_change.clone();
        let down_change = on_change.clone();
        // Rotella e frecce non hanno un rilascio: ogni passo è già un valore definitivo.
        let key_commit = on_commit.clone();
        let wheel_commit = on_commit.clone();
        let value = self.value;
        let mut root = root
            .track_focus(&focus)
            .tab_stop(true)
            .cursor(CursorStyle::PointingHand)
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                let frac = frac_at(down.read(cx).bounds, event.position.x);
                down.update(cx, |s, cx| {
                    s.drag = Some(frac);
                    cx.notify();
                });
                if !commit_on_release && let Some(f) = &down_change {
                    f(frac, window, cx);
                }
            })
            .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                let frac = frac_at(mv.read(cx).bounds, event.position.x);
                mv.update(cx, |s, cx| {
                    s.hover = Some(frac);
                    cx.notify();
                });
            })
            .on_hover({
                let state = state.clone();
                move |hovering, _, cx| {
                    if !*hovering {
                        state.update(cx, |s, cx| {
                            s.hover = None;
                            cx.notify();
                        });
                    }
                }
            })
            .on_scroll(move |event, window, cx| {
                let dy = f32::from(event.delta.pixel_delta(px(16.0)).y);
                if dy == 0.0 {
                    return;
                }
                let current = wheel.read(cx).drag.unwrap_or(value);
                let next = (current + dy.signum() * 0.05).clamp(0.0, 1.0);
                if let Some(f) = &wheel_change {
                    f(next, window, cx);
                }
                if let Some(f) = &wheel_commit {
                    f(next, window, cx);
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let dir = match event.keystroke.key.as_str() {
                    "left" => -1.0,
                    "right" => 1.0,
                    _ => return,
                };
                let _ = &key;
                let next = (value + dir * step).clamp(0.0, 1.0);
                if let Some(f) = &key_change {
                    f(next, window, cx);
                }
                if let Some(f) = &key_commit {
                    f(next, window, cx);
                }
            })
            .child(track)
            .child(drag_listeners);
        // Anteprima (tempo o percentuale) sopra il puntatore, mentre si sta sopra o si trascina.
        if let (Some(label), true) = (&self.label, hovered || dragging) {
            let frac = drag.or(hover).unwrap_or(self.value);
            let width = to_dp(state.read(cx).bounds.size.width);
            root = root.child(
                div()
                    .absolute()
                    .bottom(dp(SEEK_HIT_H))
                    .left(dp((width * frac - 24.0).max(0.0)))
                    .child(TooltipBox::new(label(frac))),
            );
        }
        // Anello di focus sul binario (1 AP di margine), solo da tastiera.
        let ring_host = div().absolute().left(px(0.0)).right(px(0.0)).top(dp(10.0)).h(ap.n(2));
        let ring_host = if self.preview == Preview::Focused {
            ring_host.child(static_ring(1, col(PURE_WHITE), ap))
        } else {
            ring_host.child(focus_ring(1, col(PURE_WHITE), focus, ap))
        };
        let _ = SURFACE_RAISED;
        root.child(ring_host).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_position_maps_to_a_clamped_fraction() {
        let bounds = Bounds::new(point(px(100.0), px(0.0)), size(px(200.0), px(24.0)));
        assert_eq!(frac_at(bounds, px(100.0)), 0.0);
        assert_eq!(frac_at(bounds, px(200.0)), 0.5);
        assert_eq!(frac_at(bounds, px(300.0)), 1.0);
        assert_eq!(frac_at(bounds, px(-50.0)), 0.0);
        assert_eq!(frac_at(bounds, px(900.0)), 1.0);
        assert_eq!(frac_at(Bounds::default(), px(10.0)), 0.0);
    }
}
