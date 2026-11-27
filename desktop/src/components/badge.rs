//! Badge: etichetta di 16 dp di altezza (`TYPE_LABEL`, tacca 1). Cinque varianti, nessun hover.

use gpui::{App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use psytify_design::tokens::color::{
    CONTROL_BORDER, CONTROL_BORDER_HOVER, ERROR, LIME, TEXT_DISABLED, TEXT_ERROR, TEXT_ON_LIGHT,
    TEXT_PRIMARY, TEXT_SECONDARY, WARM_WHITE,
};
use psytify_design::tokens::shape::NOTCH_1;
use psytify_design::tokens::space::{SPACE_1, SPACE_2};
use psytify_design::tokens::type_::TYPE_LABEL;

use crate::pixel::shape::notch_box;
use crate::theme::{Ap, TypeStyle, cased, col, dp};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeKind {
    /// `MP3 320`, licenza.
    Neutral,
    /// `LOSSLESS`, `FLAC 24/96`: non è lime, la qualità non è uno stato.
    Quality,
    /// Conteggio non letto: fill chiaro.
    Count,
    /// Quadratino lime di 2 x 2 AP più testo.
    Live,
    Error,
}

#[derive(IntoElement)]
pub struct Badge {
    text: SharedString,
    kind: BadgeKind,
    disabled: bool,
}

impl Badge {
    pub fn new(text: impl Into<SharedString>, kind: BadgeKind) -> Self {
        Self { text: text.into(), kind, disabled: false }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for Badge {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let (fill, border, text) = match self.kind {
            BadgeKind::Neutral => (None, Some(CONTROL_BORDER), TEXT_SECONDARY),
            BadgeKind::Quality => (None, Some(CONTROL_BORDER_HOVER), TEXT_PRIMARY),
            BadgeKind::Count => (Some(WARM_WHITE), None, TEXT_ON_LIGHT),
            BadgeKind::Live => (None, None, TEXT_PRIMARY),
            BadgeKind::Error => (None, Some(ERROR), TEXT_ERROR),
        };
        let text = if self.disabled { TEXT_DISABLED } else { text };
        let mut row = div()
            .relative()
            .h(dp(16.0))
            .min_w(dp(16.0))
            .px(dp(SPACE_2))
            .flex()
            .items_center()
            .justify_center()
            .gap(ap.one())
            .flex_none()
            .text_color(col(text))
            .type_role(TYPE_LABEL)
            .whitespace_nowrap();
        if fill.is_some() || border.is_some() {
            row = row.child(notch_box(NOTCH_1, fill.map(col), border.map(col)));
        }
        if self.kind == BadgeKind::Live {
            row = row.child(div().size(ap.n(2)).bg(col(LIME)).flex_none());
        }
        let _ = SPACE_1;
        row.child(div().relative().child(cased(&self.text, TYPE_LABEL)))
    }
}
