//! La riga di un'impostazione: a sinistra il nome (`TYPE_TRACK_TITLE`) e una spiegazione breve
//! (`TYPE_ARTIST`), a destra il controllo (un interruttore, una scelta, un pulsante o un valore). Alta
//! almeno [`SETTING_ROW_MIN_H`], allineata alle intestazioni di sezione (`SPACE_6` ai lati), così le
//! righe di un gruppo hanno tutte la stessa misura. La spiegazione può andare a capo: il controllo resta
//! centrato in verticale.
//!
//! Il gruppo di righe sta sotto un'intestazione di sezione (`section::section_heading`); fra un gruppo e
//! il prossimo c'è un separatore di 1 AP (`setting_gap`), come chiede la direzione artistica.

use gpui::{AnyElement, App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use psytify_design::tokens::color::{LINE_STRONG, TEXT_ERROR, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY};
use psytify_design::tokens::size::{TRACK_ROW_DENSE_H, TRACK_ROW_H};
use psytify_design::tokens::space::{SPACE_1, SPACE_2, SPACE_4, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_BODY, TYPE_META, TYPE_TRACK_TITLE};

use super::separator::Separator;
use crate::pixel::shape::notch_box;
use crate::theme::{TypeStyle, col, dp};

/// Altezza minima di una riga: una riga di brano più `SPACE_5`, così nome e spiegazione stanno su due
/// righe di testo con il loro margine.
pub const SETTING_ROW_MIN_H: f32 = TRACK_ROW_H + SPACE_5;

#[derive(IntoElement)]
pub struct SettingRow {
    label: SharedString,
    description: SharedString,
    /// La spiegazione è un errore (in `TEXT_ERROR`).
    error: bool,
    control: Option<AnyElement>,
}

impl SettingRow {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self { label: label.into(), description: SharedString::default(), error: false, control: None }
    }

    /// La spiegazione sotto il nome, già pronta.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = description.into();
        self
    }

    pub fn error(mut self, error: bool) -> Self {
        self.error = error;
        self
    }

    /// Il controllo a destra.
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
            text = text.child(div().type_role(TYPE_ARTIST).text_color(col(tint)).child(self.description));
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

/// Lo spazio fra due gruppi di righe: un separatore di 1 AP con `SPACE_5` sopra.
pub fn setting_gap() -> gpui::Div {
    div().w_full().pt(dp(SPACE_5)).px(dp(SPACE_6)).child(Separator::horizontal())
}

/// Un tasto disegnato: il nome in `TYPE_META` in una scatola di 1 AP `LINE_STRONG` (come il suggerimento
/// «Ctrl K» del campo di ricerca).
pub fn key_cap(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .relative()
        .px(dp(SPACE_2))
        .type_role(TYPE_META)
        .text_color(col(TEXT_TERTIARY))
        .child(notch_box(0, None, Some(col(LINE_STRONG))))
        .child(div().relative().child(text.into()))
}

/// La riga fitta di una scorciatoia: che cosa fa a sinistra, i tasti a destra.
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
