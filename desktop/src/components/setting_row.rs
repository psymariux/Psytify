//! The row of a setting: on the left the name (`TYPE_TRACK_TITLE`) and a short explanation (`TYPE_ARTIST`,
//! within the running-text measure), on the right the control (a switch, a choice, a button or a value). At
//! least [`SETTING_ROW_MIN_H`] tall, with `SPACE_6` each side (pages add the rest of their frame margin), so
//! the rows of a group share one measure. The explanation may wrap: the control stays centred vertically.
//!
//! A group of rows sits under a section heading (`section::section_heading`); between two groups there is a
//! 1 AP separator (`setting_gap`), as the art direction asks.

use gpui::{AnyElement, App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use psytify_design::tokens::color::{LINE_STRONG, TEXT_ERROR, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY};
use psytify_design::tokens::size::{TRACK_ROW_DENSE_H, TRACK_ROW_H};
use psytify_design::tokens::space::{SPACE_1, SPACE_2, SPACE_4, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_BODY, TYPE_META, TYPE_TRACK_TITLE};

use super::frame::measure;
use super::separator::Separator;
use crate::pixel::shape::notch_box;
use crate::theme::{TypeStyle, col, dp};

/// The least height of a row: a track row plus `SPACE_5`, so name and explanation fit two lines with their
/// margin.
pub const SETTING_ROW_MIN_H: f32 = TRACK_ROW_H + SPACE_5;

#[derive(IntoElement)]
pub struct SettingRow {
    label: SharedString,
    description: SharedString,
    /// The explanation is an error (in `TEXT_ERROR`).
    error: bool,
    control: Option<AnyElement>,
}

impl SettingRow {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self { label: label.into(), description: SharedString::default(), error: false, control: None }
    }

    /// The explanation under the name, ready made.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = description.into();
        self
    }

    pub fn error(mut self, error: bool) -> Self {
        self.error = error;
        self
    }

    /// The control on the right.
    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.control = Some(control.into_any_element());
        self
    }
}

impl RenderOnce for SettingRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(dp(SPACE_1))
            .child(div().type_role(TYPE_TRACK_TITLE).text_color(col(TEXT_PRIMARY)).child(self.label));
        if !self.description.is_empty() {
            let tint = if self.error { TEXT_ERROR } else { TEXT_SECONDARY };
            text = text.child(
                div()
                    .max_w(measure(TYPE_ARTIST))
                    .type_role(TYPE_ARTIST)
                    .text_color(col(tint))
                    .child(self.description),
            );
        }
        div()
            .w_full()
            .min_h(dp(SETTING_ROW_MIN_H))
            .flex()
            .items_center()
            .gap(dp(SPACE_6))
            .px(dp(SPACE_6))
            .py(dp(SPACE_4))
            .child(text)
            .children(self.control.map(|control| div().flex_none().flex().items_center().child(control)))
    }
}

/// The space between two groups of rows: a 1 AP separator with `SPACE_5` above.
pub fn setting_gap() -> gpui::Div {
    div().w_full().pt(dp(SPACE_5)).px(dp(SPACE_6)).child(Separator::horizontal())
}

/// A drawn key: the name in `TYPE_META` in a 1 AP `LINE_STRONG` box (like the search field's «Ctrl K» hint).
pub fn key_cap(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .relative()
        .px(dp(SPACE_2))
        .type_role(TYPE_META)
        .text_color(col(TEXT_TERTIARY))
        .child(notch_box(0, None, Some(col(LINE_STRONG))))
        .child(div().relative().child(text.into()))
}

/// The dense row of a shortcut: what it does on the left, the keys on the right.
pub fn shortcut_row(what: impl Into<SharedString>, keys: &[&'static str]) -> gpui::Div {
    div()
        .w_full()
        .h(dp(TRACK_ROW_DENSE_H))
        .flex()
        .items_center()
        .gap(dp(SPACE_6))
        .px(dp(SPACE_6))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .type_role(TYPE_BODY)
                .text_color(col(TEXT_SECONDARY))
                .child(what.into()),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(dp(SPACE_2))
                .children(keys.iter().map(|key| key_cap(*key))),
        )
}
