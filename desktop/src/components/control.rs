//! Shared control plumbing: interaction state (hover, press, keyboard focus) and the gallery override.
//!
//! Every interactive control keeps a small state per id (`use_keyed_state`): it lives while the control is
//! drawn frame after frame. A change redraws the view that draws it, only when it really changes. The hover
//! is also a tween (`MOTION_HOVER`): [`Visual::hover_t`] eases 0 to 1, and the view asks for frames only
//! while it runs (`motion`).

use std::rc::Rc;
use std::time::Instant;

use gpui::{
    App, ClickEvent, Context, CursorStyle, Div, ElementId, FocusHandle, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Stateful, StatefulInteractiveElement, Styled, Window, div,
};
use psytify_design::tokens::motion::MOTION_HOVER;

use crate::motion::{Tween, duration, keep_running};

/// Callback of a click (mouse or keyboard: Enter/Space on the control with focus).
pub type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// State shown by a control. `Live` follows the mouse and the keyboard; the others lock it (the
/// gallery uses them to show each state side by side without having to hold the mouse).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preview {
    #[default]
    Live,
    Rest,
    Hover,
    Pressed,
    Focused,
}

/// What to draw in this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Visual {
    pub hovered: bool,
    pub pressed: bool,
    /// Only for `Preview::Focused`: the ring must be shown even without the keyboard.
    pub force_ring: bool,
    /// The hover move, eased from 0 (rest) to 1 (hovered) in `MOTION_HOVER`.
    pub hover_t: f32,
}

struct ControlState {
    hovered: bool,
    pressed: bool,
    hover: Tween,
    focus: FocusHandle,
}

/// A ready interactive element: `element` has its id, focus, cursor and listeners already connected.
pub struct Wired {
    pub element: Stateful<Div>,
    pub visual: Visual,
    pub focus: FocusHandle,
}

/// Creates the interactive element of a control and reads its current state.
///
/// If `disabled`, it records neither hover nor focus and leaves the cursor normal: a control
/// that is disabled reacts to nothing (and keeps its size).
pub fn wire(id: &ElementId, disabled: bool, preview: Preview, window: &mut Window, cx: &mut App) -> Wired {
    let state = window.use_keyed_state(id.clone(), cx, |_, cx: &mut Context<ControlState>| ControlState {
        hovered: false,
        pressed: false,
        hover: Tween::at(0.0),
        focus: cx.focus_handle(),
    });
    let now = Instant::now();
    let (hovered, pressed, focus, hover) = {
        let s = state.read(cx);
        (s.hovered, s.pressed, s.focus.clone(), s.hover)
    };
    keep_running(window, hover.running(now));
    let mut element = div().id(id.clone());
    if !disabled {
        let hover_state = state.clone();
        let down_state = state.clone();
        let up_state = state.clone();
        let out_state = state;
        element = element
            .track_focus(&focus)
            .tab_stop(true)
            .cursor(CursorStyle::PointingHand)
            .on_hover(move |hovering, _, cx| {
                let ms = duration(MOTION_HOVER, cx);
                hover_state.update(cx, |s, cx| {
                    if s.hovered != *hovering {
                        s.hovered = *hovering;
                        s.hover.set(if *hovering { 1.0 } else { 0.0 }, ms, Instant::now());
                        cx.notify();
                    }
                });
            })
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                down_state.update(cx, |s, cx| {
                    if !s.pressed {
                        s.pressed = true;
                        cx.notify();
                    }
                });
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                up_state.update(cx, |s, cx| {
                    if s.pressed {
                        s.pressed = false;
                        cx.notify();
                    }
                });
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                out_state.update(cx, |s, cx| {
                    if s.pressed {
                        s.pressed = false;
                        cx.notify();
                    }
                });
            });
    }
    let visual = if disabled {
        Visual::default()
    } else {
        match preview {
            Preview::Live => Visual { hovered, pressed, force_ring: false, hover_t: hover.value(now) },
            Preview::Rest => Visual::default(),
            Preview::Hover => Visual { hovered: true, hover_t: 1.0, ..Visual::default() },
            Preview::Pressed => Visual { hovered: true, pressed: true, force_ring: false, hover_t: 1.0 },
            Preview::Focused => Visual { force_ring: true, ..Visual::default() },
        }
    };
    Wired { element, visual, focus }
}

/// Records a click on the element, if there is a callback.
pub fn with_click(element: Stateful<Div>, on_click: Option<OnClick>) -> Stateful<Div> {
    match on_click {
        Some(handler) => element.on_click(move |event, window, cx| handler(event, window, cx)),
        None => element,
    }
}

/// Centres the children on both axes.
pub fn centered<E: Styled + ParentElement + IntoElement>(element: E) -> E {
    element.flex().items_center().justify_center()
}
