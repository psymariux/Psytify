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

/// Callback di un click (mouse o tastiera: Invio/Spazio sul controllo con il focus).
pub type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Stato mostrato da un controllo. `Live` segue il mouse e la tastiera; gli altri lo bloccano (la
/// galleria li usa per mostrare ogni stato affiancato senza dover tenere premuto il mouse).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preview {
    #[default]
    Live,
    Rest,
    Hover,
    Pressed,
    Focused,
}

/// Cosa disegnare in questo frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Visual {
    pub hovered: bool,
    pub pressed: bool,
    /// Solo per `Preview::Focused`: l'anello va mostrato anche senza tastiera.
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

/// Un elemento interattivo pronto: `element` ha id, focus, cursore e ascoltatori già collegati.
pub struct Wired {
    pub element: Stateful<Div>,
    pub visual: Visual,
    pub focus: FocusHandle,
}

/// Crea l'elemento interattivo di un controllo e legge il suo stato corrente.
///
/// Se `disabled` non registra né hover né focus e lascia il cursore normale: un controllo
/// disabilitato non reagisce a nulla (e mantiene la sua misura).
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

/// Registra un click sull'elemento, se c'è una callback.
pub fn with_click(element: Stateful<Div>, on_click: Option<OnClick>) -> Stateful<Div> {
    match on_click {
        Some(handler) => element.on_click(move |event, window, cx| handler(event, window, cx)),
        None => element,
    }
}

/// Centra i figli su entrambi gli assi.
pub fn centered<E: Styled + ParentElement + IntoElement>(element: E) -> E {
    element.flex().items_center().justify_center()
}
