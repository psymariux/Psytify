//! PersonRow: the 48 dp row (`TRACK_ROW_H`, `uniform_list`-ready) of a person (`PSYTIFY_ART_DIRECTION.md`,
//! «Friends»): the avatar on the row art size (`ART_ROW` 40) in the pixel circle, the name and the status
//! under it: «Sta ascoltando …» with the lime dot of the live badge (2 x 2 AP, 1 AP gap, the text) while the
//! friend listens, «In pausa: …» without the dot, else `@name`. Actions on the right (Accetta secondary,
//! Rifiuta ghost, Aggiungi…) do not start the row's gesture.
//!
//! States as `ListRow`: rest, hover (the `SURFACE_RAISED` tone eases in), pressed (`SURFACE_SELECTED`),
//! focus (inner ring). The row has a `SPACE_3` inset: pages place it at the frame margin. Lime only in the
//! live dot.

use gpui::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window, div,
};
use psytify_design::tokens::color::{
    LIME, PURE_WHITE, SURFACE_RAISED, SURFACE_SELECTED, TEXT_PRIMARY, TEXT_SECONDARY,
};
use psytify_design::tokens::size::{TRACK_ROW_H, cover::ART_ROW};
use psytify_design::tokens::space::SPACE_3;
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_TRACK_TITLE};

use super::control::{OnClick, Preview, wire, with_click};
use super::tile::Cover;
use crate::pixel::shape::inset_ring;
use crate::theme::{Ap, TypeStyle, col, dp, fade};

/// The second line: the person's status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// `@name` (or another quiet line: «Richiesta inviata»).
    Plain(SharedString),
    /// Listening now: the lime dot and the text («Sta ascoltando Title · Artist»).
    Live(SharedString),
}

#[derive(IntoElement)]
pub struct PersonRow {
    id: ElementId,
    avatar: Option<SharedString>,
    name: SharedString,
    status: Status,
    actions: Vec<AnyElement>,
    preview: Preview,
    on_click: Option<OnClick>,
}

impl PersonRow {
    pub fn new(id: impl Into<ElementId>, name: impl Into<SharedString>, status: Status) -> Self {
        Self {
            id: id.into(),
            avatar: None,
            name: name.into(),
            status,
            actions: Vec::new(),
            preview: Preview::Live,
            on_click: None,
        }
    }

    /// The avatar's address (the placeholder in the circle until it arrives, or if there is none).
    pub fn avatar(mut self, url: Option<SharedString>) -> Self {
        self.avatar = url;
        self
    }

    /// An action on the right (a kit button), in reading order.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn preview(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    /// The row's gesture (open the profile).
    pub fn on_click(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }
}

/// An action that must not start the row's gesture too.
fn isolated(action: AnyElement) -> AnyElement {
    div()
        .flex_none()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(action)
        .into_any_element()
}

impl RenderOnce for PersonRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let wired = wire(&self.id, false, self.preview, window, cx);
        let v = wired.visual;
        let status = match self.status {
            Status::Plain(text) => div()
                .type_role(TYPE_ARTIST)
                .text_color(col(TEXT_SECONDARY))
                .truncate()
                .child(text)
                .into_any_element(),
            // The live badge's anatomy (guidelines §5) in the row's type: the 2 x 2 AP lime dot, 1 AP, the
            // text.
            Status::Live(text) => div()
                .flex()
                .items_center()
                .gap(ap.one())
                .min_w_0()
                .child(div().flex_none().size(ap.n(2)).bg(col(LIME)))
                .child(
                    div()
                        .min_w_0()
                        .type_role(TYPE_ARTIST)
                        .text_color(col(TEXT_PRIMARY))
                        .truncate()
                        .child(text),
                )
                .into_any_element(),
        };
        let mut row = wired
            .element
            .relative()
            .h(dp(TRACK_ROW_H))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .gap(dp(SPACE_3))
            .px(dp(SPACE_3))
            .child(Cover::new(ART_ROW).url(self.avatar).circle())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .type_role(TYPE_TRACK_TITLE)
                            .text_color(col(TEXT_PRIMARY))
                            .truncate()
                            .child(self.name),
                    )
                    .child(status),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(dp(SPACE_3))
                    .children(self.actions.into_iter().map(isolated)),
            )
            .child(inset_ring(col(PURE_WHITE), wired.focus.clone(), v.force_ring));
        if v.pressed {
            row = row.bg(col(SURFACE_SELECTED));
        } else if v.hover_t > 0.0 {
            row = row.bg(fade(SURFACE_RAISED, v.hover_t));
        }
        with_click(row, self.on_click)
    }
}
