//! PersonRow: la riga di 48 dp (`TRACK_ROW_H`, pronta per `uniform_list`) di una persona, come da
//! `PSYTIFY_ART_DIRECTION.md` («Friends»): l'avatar `AVATAR_S` 32 nel cerchio pixel (`Cover::circle`), il
//! nome e sotto lo stato: «Sta ascoltando …» con il punto lime del badge dal vivo (2 x 2 AP, uno spazio di
//! 1 AP, il testo; niente bordo) mentre l'amico ascolta, «In pausa: …» senza punto, altrimenti `@nome`.
//! A destra le azioni (Accetta secondario, Rifiuta ghost, Aggiungi…), che non fanno partire il gesto della
//! riga.
//!
//! Stati come `ListRow`: rest, hover (fill `SURFACE_RAISED`), premuto (`SURFACE_SELECTED`), focus (anello
//! interno). Il lime c'è solo nel punto dal vivo (la direzione artistica: «Social: live badge dot»).

use gpui::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window, div,
};
use psytify_design::tokens::color::{
    LIME, PURE_WHITE, SURFACE_RAISED, SURFACE_SELECTED, TEXT_PRIMARY, TEXT_SECONDARY,
};
use psytify_design::tokens::size::{TRACK_ROW_H, cover::AVATAR_S};
use psytify_design::tokens::space::{SPACE_3, SPACE_6};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_TRACK_TITLE};

use super::control::{OnClick, Preview, wire, with_click};
use super::tile::Cover;
use crate::pixel::shape::inset_ring;
use crate::theme::{Ap, TypeStyle, col, dp};

/// La seconda riga: lo stato della persona.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// `@nome` (o un'altra frase tenue: «Richiesta inviata»).
    Plain(SharedString),
    /// Sta ascoltando adesso: il punto lime e il testo («Sta ascoltando Titolo · Artista»).
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

    /// L'indirizzo dell'avatar (il segnaposto nel cerchio finché non arriva o se non c'è).
    pub fn avatar(mut self, url: Option<SharedString>) -> Self {
        self.avatar = url;
        self
    }

    /// Un'azione a destra (un pulsante del kit), nell'ordine di lettura.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn preview(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    /// Il gesto della riga (aprire il profilo).
    pub fn on_click(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }
}

/// Un'azione che non deve far partire anche il gesto della riga.
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
            // L'anatomia del badge dal vivo (linee guida §5) con il testo nel tipo della riga: il punto lime
            // 2 x 2 AP, 1 AP di spazio, il testo.
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
            // L'avatar si allinea alle intestazioni di sezione, come l'arte di `ListRow` senza indice.
            .pl(dp(SPACE_6))
            .pr(dp(SPACE_3))
            .child(Cover::new(AVATAR_S).url(self.avatar).circle())
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
        } else if v.hovered {
            row = row.bg(col(SURFACE_RAISED));
        }
        with_click(row, self.on_click)
    }
}
