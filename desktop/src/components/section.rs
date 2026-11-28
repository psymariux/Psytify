//! Page parts shared by the lists: the section header and the status row, as tall as a track row
//! (`TRACK_ROW_H`) so they sit in a `uniform_list` together with the rows, and the page title that
//! sits above the list.
//!
//! A screen's title (Libreria, Scaricati) is in pixel type (`TYPE_PAGE_TITLE`), with
//! under it a metadata line and the page's buttons. The name of an object (album, artist, playlist,
//! «Brani che ti piacciono») is not a screen title: it sits in `PageHeader` (`page_header.rs`).

use gpui::{AnyElement, App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use psytify_design::tokens::color::{TEXT_PRIMARY, TEXT_TERTIARY};
use psytify_design::tokens::size::TRACK_ROW_H;
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{TYPE_HEADING, TYPE_META, TYPE_PAGE_TITLE};

use crate::theme::{TypeStyle, col, dp};

/// Section header: as tall as a row, so the list stays uniform. `meta` is the count
/// («3 brani»), empty if not needed.
pub fn section_heading(title: SharedString, meta: SharedString) -> gpui::Div {
    section_heading_at(title, meta, SPACE_6)
}

/// [`section_heading`] with the page's canvas margin (`components::frame`).
pub fn section_heading_at(title: SharedString, meta: SharedString, margin: f32) -> gpui::Div {
    div()
        .h(dp(TRACK_ROW_H))
        .w_full()
        .flex_none()
        .flex()
        .items_end()
        .gap(dp(SPACE_3))
        .px(dp(margin))
        .pb(dp(SPACE_3))
        .child(div().type_role(TYPE_HEADING).text_color(col(TEXT_PRIMARY)).child(title))
        .child(div().type_role(TYPE_META).text_color(col(TEXT_TERTIARY)).child(meta))
}

/// A status row (loading, error, a phrase that explains), as tall as the others.
pub fn note(content: impl IntoElement) -> gpui::Div {
    note_at(content, SPACE_6)
}

/// [`note`] with the page's canvas margin.
pub fn note_at(content: impl IntoElement, margin: f32) -> gpui::Div {
    div().h(dp(TRACK_ROW_H)).w_full().flex_none().flex().items_center().px(dp(margin)).child(content)
}

/// The title at the top of a screen: pixel-type title, a metadata line and the buttons.
#[derive(IntoElement)]
pub struct PageTitle {
    title: SharedString,
    meta: SharedString,
    /// A second, fainter metadata line (the downloads folder).
    detail: SharedString,
    actions: Vec<AnyElement>,
    margin: f32,
}

impl PageTitle {
    /// A screen's title (Libreria, Scaricati).
    pub fn screen(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            meta: SharedString::default(),
            detail: SharedString::default(),
            actions: Vec::new(),
            margin: SPACE_6,
        }
    }

    /// The page's canvas margin (`components::frame`).
    pub fn margin(mut self, margin: f32) -> Self {
        self.margin = margin;
        self
    }

    /// The line under the title (counts, duration, space used): already ready.
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = meta.into();
        self
    }

    /// A second line under the metadata, already ready.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = detail.into();
        self
    }

    /// A page button, in reading order.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl RenderOnce for PageTitle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let title =
            div().text_color(col(TEXT_PRIMARY)).truncate().child(self.title).type_role(TYPE_PAGE_TITLE);
        let mut text = div().flex().flex_col().gap(dp(SPACE_2)).min_w_0().child(title);
        if !self.meta.is_empty() {
            text = text.child(div().type_role(TYPE_META).text_color(col(TEXT_TERTIARY)).child(self.meta));
        }
        if !self.detail.is_empty() {
            text = text.child(
                div().type_role(TYPE_META).text_color(col(TEXT_TERTIARY)).truncate().child(self.detail),
            );
        }
        let mut column = div()
            .flex_none()
            .w_full()
            .flex()
            .flex_col()
            .gap(dp(SPACE_5))
            .px(dp(self.margin))
            .pt(dp(SPACE_6))
            .pb(dp(SPACE_3))
            .child(text);
        if !self.actions.is_empty() {
            column = column.child(div().flex().items_center().gap(dp(SPACE_3)).children(self.actions));
        }
        column
    }
}
