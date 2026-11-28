//! Floating layers: Toast, Dialog (with Scrim) and EmptyState. The toast and the dialog are panels
//! of `SURFACE_RAISED` with a `LINE_STRONG` border and a 2 AP hard shadow; the placement and the duration
//! (4 s, sospesa in hover) spettano a chi li mostra.

use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString, Styled,
    Window, div, px,
};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    ERROR, LINE_STRONG, SCRIM, SHADOW_FLOAT, SURFACE_RAISED, TEXT_PRIMARY, TEXT_SECONDARY,
};
use psytify_design::tokens::shape::{MARKER_W, NOTCH_1, NOTCH_2, SHADOW_FLOAT_OFFSET};
use psytify_design::tokens::size::{DIALOG_W, TOAST_W};
use psytify_design::tokens::space::{SPACE_3, SPACE_4, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{TYPE_BODY, TYPE_HEADING};

use super::button::Button;
use crate::pixel::PixelSprite;
use crate::pixel::shape::{hard_shadow, notch_box};
use crate::theme::{Ap, TypeStyle, col, dp};

/// Toast: one line of text with an optional action («Annulla»). Errors have an `ERROR` bar
/// di `MARKER_W` a sinistra.
#[derive(IntoElement)]
pub struct Toast {
    text: SharedString,
    action: Option<AnyElement>,
    error: bool,
}

impl Toast {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into(), action: None, error: false }
    }

    pub fn action(mut self, button: Button) -> Self {
        self.action = Some(button.ghost().into_any_element());
        self
    }

    pub fn error(mut self, error: bool) -> Self {
        self.error = error;
        self
    }
}

impl RenderOnce for Toast {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let mut toast = div()
            .relative()
            .w(dp(TOAST_W))
            .min_h(dp(40.0))
            .px(dp(SPACE_5))
            .py(dp(SPACE_3))
            .flex()
            .items_center()
            .justify_between()
            .gap(dp(SPACE_4))
            .child(hard_shadow(NOTCH_1, SHADOW_FLOAT_OFFSET.0, col(SHADOW_FLOAT)))
            .child(notch_box(NOTCH_1, Some(col(SURFACE_RAISED)), Some(col(LINE_STRONG))))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_PRIMARY))
                    .child(self.text),
            )
            .children(self.action.map(|a| div().relative().child(a)));
        if self.error {
            toast = toast.child(
                div().absolute().left(ap.one()).top(ap.n(2)).bottom(ap.n(2)).w(ap.n(MARKER_W)).bg(col(ERROR)),
            );
        }
        toast
    }
}

/// Velo dietro un dialogo: `SCRIM` (unico colore traslucido), a tutta finestra.
pub fn scrim() -> gpui::Div {
    div().absolute().top(px(0.0)).left(px(0.0)).size_full().bg(col(SCRIM)).occlude()
}

/// Dialogo: titolo, testo, contenuto facoltativo e pulsanti allineati a destra (secondario poi
/// primario o distruttivo).
#[derive(IntoElement)]
pub struct Dialog {
    title: SharedString,
    body: SharedString,
    content: Option<AnyElement>,
    buttons: Vec<AnyElement>,
}

impl Dialog {
    pub fn new(title: impl Into<SharedString>, body: impl Into<SharedString>) -> Self {
        Self { title: title.into(), body: body.into(), content: None, buttons: Vec::new() }
    }

    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    pub fn button(mut self, button: Button) -> Self {
        self.buttons.push(button.into_any_element());
        self
    }
}

impl RenderOnce for Dialog {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .relative()
            .w(dp(DIALOG_W))
            .p(dp(SPACE_6))
            .flex()
            .flex_col()
            .gap(dp(SPACE_4))
            .child(hard_shadow(NOTCH_2, SHADOW_FLOAT_OFFSET.0, col(SHADOW_FLOAT)))
            .child(notch_box(NOTCH_2, Some(col(SURFACE_RAISED)), Some(col(LINE_STRONG))))
            .child(div().relative().type_role(TYPE_HEADING).text_color(col(TEXT_PRIMARY)).child(self.title))
            .child(div().relative().type_role(TYPE_BODY).text_color(col(TEXT_SECONDARY)).child(self.body))
            .children(self.content.map(|c| div().relative().child(c)))
            .child(
                div().relative().flex().justify_end().gap(dp(SPACE_3)).pt(dp(SPACE_3)).children(self.buttons),
            )
    }
}

/// Empty state: mascot or illustration, title, one phrase and a single action.
#[derive(IntoElement)]
pub struct EmptyState {
    sprite: &'static str,
    title: SharedString,
    body: SharedString,
    action: Option<AnyElement>,
}

impl EmptyState {
    pub fn new(sprite: &'static str, title: impl Into<SharedString>, body: impl Into<SharedString>) -> Self {
        Self { sprite, title: title.into(), body: body.into(), action: None }
    }

    /// Mascot that listens to the user: empty playlists and collections use `empty_crate`.
    pub fn crate_(title: impl Into<SharedString>, body: impl Into<SharedString>) -> Self {
        Self::new(sprite::EMPTY_CRATE, title, body)
    }

    pub fn action(mut self, button: Button) -> Self {
        self.action = Some(button.secondary().into_any_element());
        self
    }
}

impl RenderOnce for EmptyState {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .max_w(dp(360.0))
            .flex()
            .flex_col()
            .items_center()
            .text_center()
            .gap(dp(SPACE_6))
            .child(PixelSprite::new(self.sprite))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(dp(SPACE_3))
                    .child(div().type_role(TYPE_HEADING).text_color(col(TEXT_PRIMARY)).child(self.title))
                    .child(div().type_role(TYPE_BODY).text_color(col(TEXT_SECONDARY)).child(self.body)),
            )
            .children(self.action)
    }
}
