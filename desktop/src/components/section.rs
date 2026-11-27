//! Le parti di pagina comuni alle liste: l'intestazione di sezione e la riga di stato, alte come una riga
//! di brano (`TRACK_ROW_H`) così stanno in una `uniform_list` insieme alle righe, e il titolo della
//! pagina che sta sopra la lista.
//!
//! Il titolo di una schermata dell'app (Libreria, Scaricati) è in tipo pixel (`TYPE_PAGE_TITLE`), con
//! sotto una riga di metadati e i pulsanti della pagina. Il nome di un oggetto (album, artista, playlist,
//! «Brani che ti piacciono») non è un titolo di schermata: sta in `PageHeader` (`page_header.rs`).

use gpui::{AnyElement, App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use psytify_design::tokens::color::{TEXT_PRIMARY, TEXT_TERTIARY};
use psytify_design::tokens::size::TRACK_ROW_H;
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{TYPE_HEADING, TYPE_META, TYPE_PAGE_TITLE};

use crate::theme::{TypeStyle, col, dp};

/// Intestazione di sezione: alta come una riga, così la lista resta uniforme. `meta` è il conteggio
/// («3 brani»), vuoto se non serve.
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

/// Una riga di stato (caricamento, errore, una frase che spiega), alta come le altre.
pub fn note(content: impl IntoElement) -> gpui::Div {
    note_at(content, SPACE_6)
}

/// [`note`] with the page's canvas margin.
pub fn note_at(content: impl IntoElement, margin: f32) -> gpui::Div {
    div().h(dp(TRACK_ROW_H)).w_full().flex_none().flex().items_center().px(dp(margin)).child(content)
}

/// Il titolo in cima a una schermata: titolo in tipo pixel, una riga di metadati e i pulsanti.
#[derive(IntoElement)]
pub struct PageTitle {
    title: SharedString,
    meta: SharedString,
    /// Una seconda riga di metadati, più tenue (la cartella dei download).
    detail: SharedString,
    actions: Vec<AnyElement>,
    margin: f32,
}

impl PageTitle {
    /// Il titolo di una schermata (Libreria, Scaricati).
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

    /// La riga sotto il titolo (conteggi, durata, spazio occupato): già pronta.
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = meta.into();
        self
    }

    /// Una seconda riga sotto i metadati, già pronta.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = detail.into();
        self
    }

    /// Un pulsante della pagina, nell'ordine di lettura.
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
