//! Toggle (interruttore): binario 16 x 8 AP con bordo di 1 AP, pomello quadrato 6 x 6 AP rientrato
//! di 1 AP. Spento: bordo `CONTROL_BORDER`, pomello `STONE` a sinistra. Acceso: bordo e pomello
//! `LIME`, pomello a destra (la posizione dice lo stato anche senza colore).

use gpui::{App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use psytify_design::tokens::color::{
    CONTROL_BORDER, LIME, LINE_STRONG, PURE_WHITE, SOFT_GREY, STONE, SURFACE_WELL, TEXT_DISABLED,
    TEXT_PRIMARY, TEXT_TERTIARY,
};
use psytify_design::tokens::shape::NOTCH_1;
use psytify_design::tokens::space::SPACE_5;
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_TRACK_TITLE};

use super::control::{OnClick, Preview, wire, with_click};
use crate::pixel::shape::{focus_ring, notch_box, static_ring};
use crate::theme::{Ap, TypeStyle, col, dp};

#[derive(IntoElement)]
pub struct Toggle {
    id: ElementId,
    on: bool,
    label: Option<SharedString>,
    description: Option<SharedString>,
    disabled: bool,
    preview: Preview,
    on_click: Option<OnClick>,
}

impl Toggle {
    pub fn new(id: impl Into<ElementId>, on: bool) -> Self {
        Self {
            id: id.into(),
            on,
            label: None,
            description: None,
            disabled: false,
            preview: Preview::Live,
            on_click: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn preview(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }
}

impl RenderOnce for Toggle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let wired = wire(&self.id, self.disabled, self.preview, window, cx);
        let v = wired.visual;
        let (border, knob) = if self.disabled {
            (LINE_STRONG, LINE_STRONG)
        } else if self.on {
            (LIME, LIME)
        } else if v.hovered {
            (SOFT_GREY, SOFT_GREY)
        } else {
            (CONTROL_BORDER, STONE)
        };
        let (track_w, track_h) = (ap.n(16), ap.n(8));
        let knob_x = if self.on { ap.n(9) } else { ap.n(1) };
        let switch = div()
            .relative()
            .flex_none()
            .w(track_w)
            .h(track_h)
            .child(notch_box(NOTCH_1, Some(col(SURFACE_WELL)), Some(col(border))))
            .child(div().absolute().left(knob_x).top(ap.n(1)).size(ap.n(6)).bg(col(knob)))
            .child(if v.force_ring {
                static_ring(NOTCH_1, col(PURE_WHITE), ap)
            } else {
                focus_ring(NOTCH_1, col(PURE_WHITE), wired.focus.clone(), ap)
            });
        let bare = self.label.is_none() && self.description.is_none();
        let mut text = div().flex_1().min_w_0().flex().flex_col();
        if let Some(label) = self.label {
            let c = if self.disabled { TEXT_DISABLED } else { TEXT_PRIMARY };
            text = text.child(div().type_role(TYPE_TRACK_TITLE).text_color(col(c)).child(label));
        }
        if let Some(description) = self.description {
            text = text.child(div().type_role(TYPE_ARTIST).text_color(col(TEXT_TERTIARY)).child(description));
        }
        // Senza testo (il controllo di una `SettingRow`) resta solo l'interruttore, alto quanto l'area
        // sensibile.
        let row = if bare {
            wired.element.min_h(dp(32.0)).flex().items_center().child(switch)
        } else {
            wired.element.min_h(dp(32.0)).flex().items_center().gap(dp(SPACE_5)).child(text).child(switch)
        };
        with_click(row, if self.disabled { None } else { self.on_click })
    }
}
