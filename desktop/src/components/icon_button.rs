//! IconButton: 32 x 32 dp, icona di 12 AP centrata, con tooltip (nome e scorciatoia). Ha anche
//! l'aspetto «primario» (fill chiaro con ledge) del play di trasporto e del play dei tile.

use gpui::{App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    ICON_ACTIVE, ICON_DISABLED, ICON_IDLE, LEDGE_PRIMARY, PURE_WHITE, Rgba, STATE_ON, SURFACE_RAISED,
    SURFACE_SELECTED, TEXT_ON_LIGHT, WARM_WHITE,
};
use psytify_design::tokens::shape::NOTCH_1;
use psytify_design::tokens::size::ICON_BUTTON;

use super::button::{Look, pressable};
use super::control::{OnClick, Preview, Visual, centered, wire, with_click};
use super::tooltip::with_tooltip;
use crate::pixel::PixelSprite;
use crate::theme::{Ap, col, dp};

/// Come si segna uno stato acceso.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnStyle {
    /// Icona `STATE_ON` (lime) più un punto lime sotto: shuffle, repeat.
    Lime,
    /// Icona `ICON_ACTIVE`, senza lime: il cuore del «mi piace» (non è uno stato di riproduzione).
    Active,
}

#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: &'static str,
    /// Icona piena usata quando il bottone è acceso (`icon_heart_fill`).
    icon_on: Option<&'static str>,
    selected: bool,
    on_style: OnStyle,
    disabled: bool,
    loading: bool,
    primary: bool,
    size: f32,
    tooltip: Option<(SharedString, Option<SharedString>)>,
    preview: Preview,
    on_click: Option<OnClick>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: &'static str) -> Self {
        Self {
            id: id.into(),
            icon,
            icon_on: None,
            selected: false,
            on_style: OnStyle::Lime,
            disabled: false,
            loading: false,
            primary: false,
            size: ICON_BUTTON,
            tooltip: None,
            preview: Preview::Live,
            on_click: None,
        }
    }

    /// Acceso/spento (toggle).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn on_style(mut self, style: OnStyle) -> Self {
        self.on_style = style;
        self
    }

    /// Icona da mostrare quando è acceso.
    pub fn icon_on(mut self, icon: &'static str) -> Self {
        self.icon_on = Some(icon);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Attesa: fill `SURFACE_SELECTED` con il `loader` al posto dell'icona.
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// Aspetto primario (fill `WARM_WHITE`, ledge, icona scura).
    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    /// Lato in dp (32 di base, 40 per il play di trasporto).
    pub fn size(mut self, dp: f32) -> Self {
        self.size = dp;
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>, shortcut: Option<&str>) -> Self {
        self.tooltip = Some((text.into(), shortcut.map(|s| SharedString::from(s.to_owned()))));
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

struct Style {
    look: Look,
    tint: Rgba,
}

fn style(button: &IconButton, v: Visual) -> Style {
    let inert = button.loading;
    if button.primary {
        if button.disabled || inert {
            let tint = if inert { TEXT_ON_LIGHT } else { ICON_DISABLED };
            let tint = if inert { ICON_ACTIVE } else { tint };
            return Style {
                look: Look { fill: Some(SURFACE_SELECTED), text: Some(tint), ..Look::default() },
                tint,
            };
        }
        let pressed = v.pressed;
        return Style {
            look: Look {
                fill: Some(if v.hovered { PURE_WHITE } else { WARM_WHITE }),
                text: Some(TEXT_ON_LIGHT),
                ledge: if pressed { None } else { Some(LEDGE_PRIMARY) },
                shifted: pressed,
                border: None,
            },
            tint: TEXT_ON_LIGHT,
        };
    }
    if button.disabled {
        return Style { look: Look::default(), tint: ICON_DISABLED };
    }
    let fill = if v.pressed {
        Some(SURFACE_SELECTED)
    } else if v.hovered {
        Some(SURFACE_RAISED)
    } else {
        None
    };
    let tint = if button.selected {
        match button.on_style {
            OnStyle::Lime => STATE_ON,
            OnStyle::Active => ICON_ACTIVE,
        }
    } else if v.hovered || v.pressed {
        ICON_ACTIVE
    } else {
        ICON_IDLE
    };
    Style { look: Look { fill, ..Look::default() }, tint }
}

impl RenderOnce for IconButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let inert = self.disabled || self.loading;
        let wired = wire(&self.id, inert, self.preview, window, cx);
        let visual = if self.loading { Visual::default() } else { wired.visual };
        let Style { look, tint } = style(&self, visual);
        let icon_name = match (self.selected, self.icon_on) {
            (true, Some(on)) => on,
            _ => self.icon,
        };
        let mut content = centered(div().relative().size_full());
        content = if self.loading {
            content.child(PixelSprite::new(sprite::LOADER))
        } else {
            content.child(PixelSprite::new(icon_name).tint(tint))
        };
        if self.selected && self.on_style == OnStyle::Lime && !self.disabled && !self.primary {
            // Punto lime di 2 x 1 AP, 1 AP sotto l'icona: lo stato non dipende solo dal colore.
            content = content.child(
                div()
                    .absolute()
                    .bottom(dp(2.0))
                    .left(px(0.0))
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(div().w(ap.n(2)).h(ap.one()).bg(col(STATE_ON))),
            );
        }
        let side = dp(self.size);
        let on_click = if inert { None } else { self.on_click };
        let element = pressable(wired, (Some(side), side), NOTCH_1, look, ap, content);
        let element = with_click(element, on_click);
        match self.tooltip {
            Some((text, shortcut)) => with_tooltip(element, text, shortcut),
            None => element,
        }
    }
}
