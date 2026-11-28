//! Tooltip: name and shortcut of a control. `SURFACE_RAISED`, 1 AP `LINE_STRONG` border, notch 1,
//! `TYPE_META` text. Appears after `TOOLTIP_DELAY_MS` without moving.

use std::time::Duration;

use gpui::{
    AnyView, App, AppContext, Context, IntoElement, ParentElement, Render, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div,
};
use psytify_design::tokens::color::{LINE_STRONG, SURFACE_RAISED, TEXT_PRIMARY, TEXT_TERTIARY};
use psytify_design::tokens::motion::TOOLTIP_DELAY_MS;
use psytify_design::tokens::shape::NOTCH_1;
use psytify_design::tokens::size::TOOLTIP_MAX_W;
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_4};
use psytify_design::tokens::type_::TYPE_META;

use crate::pixel::shape::notch_box;
use crate::theme::{Ap, TypeStyle, col, dp};

/// The tooltip box (without placement): used by the tooltip view, the gallery and
/// the time preview on the seek bar.
#[derive(IntoElement)]
pub struct TooltipBox {
    text: SharedString,
    shortcut: Option<SharedString>,
}

impl TooltipBox {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into(), shortcut: None }
    }

    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }
}

impl RenderOnce for TooltipBox {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let _ = Ap::of(window);
        div()
            .relative()
            .flex()
            .items_center()
            .gap(dp(SPACE_4))
            .max_w(dp(TOOLTIP_MAX_W))
            .px(dp(SPACE_3))
            .py(dp(SPACE_2))
            .type_role(TYPE_META)
            .child(notch_box(NOTCH_1, Some(col(SURFACE_RAISED)), Some(col(LINE_STRONG))))
            .child(div().relative().text_color(col(TEXT_PRIMARY)).child(self.text))
            .children(self.shortcut.map(|keys| div().relative().text_color(col(TEXT_TERTIARY)).child(keys)))
    }
}

/// Tooltip view that GPUI shows above the rest, near the pointer.
pub struct TooltipView {
    text: SharedString,
    shortcut: Option<SharedString>,
}

impl Render for TooltipView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // A margin keeps the tooltip 4 dp below the pointer, as the guidelines say.
        let mut tip = TooltipBox::new(self.text.clone());
        if let Some(keys) = &self.shortcut {
            tip = tip.shortcut(keys.clone());
        }
        div().pt(dp(SPACE_2)).child(tip)
    }
}

/// Tooltip builder for `.tooltip(...)`.
pub fn tooltip(
    text: impl Into<SharedString>,
    shortcut: Option<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| TooltipView { text: text.clone(), shortcut: shortcut.clone() }).into()
}

/// Ritardo del tooltip.
pub fn delay() -> Duration {
    Duration::from_millis(u64::from(TOOLTIP_DELAY_MS))
}

/// Aggiunge un tooltip a un elemento con id.
pub fn with_tooltip<E: StatefulInteractiveElement>(
    element: E,
    text: impl Into<SharedString>,
    shortcut: Option<SharedString>,
) -> E {
    element.tooltip(tooltip(text, shortcut)).tooltip_show_delay(delay())
}
