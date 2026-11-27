//! Separatore: 1 AP di `LINE_SUBTLE` (strong: `LINE_STRONG`, solo ai bordi dei livelli flottanti).
//! Mai più sottile, mai sfumato, mai rientrato.

use gpui::{App, IntoElement, RenderOnce, Styled, Window, div};
use psytify_design::tokens::color::{LINE_STRONG, LINE_SUBTLE};

use crate::theme::{Ap, col};

#[derive(IntoElement)]
pub struct Separator {
    vertical: bool,
    strong: bool,
}

impl Separator {
    pub fn horizontal() -> Self {
        Self { vertical: false, strong: false }
    }

    pub fn vertical() -> Self {
        Self { vertical: true, strong: false }
    }

    pub fn strong(mut self) -> Self {
        self.strong = true;
        self
    }
}

impl RenderOnce for Separator {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let color = col(if self.strong { LINE_STRONG } else { LINE_SUBTLE });
        let line = div().flex_none().bg(color);
        if self.vertical { line.w(ap.one()).h_full() } else { line.h(ap.one()).w_full() }
    }
}
