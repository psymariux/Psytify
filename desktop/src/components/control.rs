//! Infrastruttura comune dei controlli: stato di interazione (hover, premuto, focus da tastiera) e
//! sovrascrittura per la galleria.
//!
//! Ogni controllo interattivo tiene un piccolo stato per id (`use_keyed_state`): vive finché il
//! controllo viene disegnato frame dopo frame. Cambiarlo ridisegna la finestra (come l'hover di
//! GPUI), ma solo quando cambia davvero.

use std::rc::Rc;

use gpui::{
    App, ClickEvent, Context, CursorStyle, Div, ElementId, FocusHandle, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Stateful, StatefulInteractiveElement, Styled, Window, div,
};

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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Visual {
    pub hovered: bool,
    pub pressed: bool,
    /// Solo per `Preview::Focused`: l'anello va mostrato anche senza tastiera.
    pub force_ring: bool,
}

struct ControlState {
    hovered: bool,
    pressed: bool,
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
        focus: cx.focus_handle(),
    });
    let (hovered, pressed, focus) = {
        let s = state.read(cx);
        (s.hovered, s.pressed, s.focus.clone())
    };
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
                hover_state.update(cx, |s, cx| {
                    if s.hovered != *hovering {
                        s.hovered = *hovering;
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
            Preview::Live => Visual { hovered, pressed, force_ring: false },
            Preview::Rest => Visual::default(),
            Preview::Hover => Visual { hovered: true, ..Visual::default() },
            Preview::Pressed => Visual { hovered: true, pressed: true, force_ring: false },
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
