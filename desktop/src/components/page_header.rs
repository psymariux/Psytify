//! PageHeader: la testa di una pagina che mostra un oggetto (album, artista, playlist, «Brani che ti
//! piacciono»), come da `PSYTIFY_ART_DIRECTION.md` (Album, Artist, Playlists). Sulla tela, senza tinta:
//! a sinistra l'arte (copertina `COVER_L` 224, la foto d'artista `AVATAR_L` 160 nel cerchio pixel, il
//! mosaico 2 x 2 di una playlist o uno sprite su campo `GRAPHITE`); a destra il kicker (`TYPE_LABEL`,
//! «ALBUM · 2024»), il nome in `TYPE_DISPLAY` (al più due righe), un collegamento facoltativo
//! (l'artista dell'album), la riga dei dati (`TYPE_META`, con un badge di qualità se c'è) e i pulsanti.
//!
//! I titoli delle schermate dell'app (Libreria, Scaricati) restano `PageTitle::screen`, in tipo pixel: il
//! nome di un oggetto del catalogo può essere in qualunque scrittura e lunghezza, quindi è Inter.
//! [`PageHeader::loading`] è lo scheletro alla stessa misura, finché la pagina non arriva.

use gpui::{
    AnyElement, App, ClickEvent, CursorStyle, ElementId, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div,
};
use psytify_design::tokens::color::{GRAPHITE, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY, WARM_WHITE};
use psytify_design::tokens::size::CONTROL_H_LARGE;
use psytify_design::tokens::size::cover::{AVATAR_L, AVATAR_M, COVER_L};
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{TYPE_DISPLAY, TYPE_LABEL, TYPE_META, TYPE_TRACK_TITLE};

use super::badge::{Badge, BadgeKind};
use super::control::OnClick;
use super::indicators::Skeleton;
use super::tile::Cover;
use crate::pixel::PixelSprite;
use crate::theme::{TypeStyle, cased, col, dp};

/// L'arte a sinistra.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeaderArt {
    /// Una copertina (il segnaposto finché non arriva o se non c'è).
    Cover(Option<SharedString>),
    /// La foto di un artista nel cerchio pixel.
    Round(Option<SharedString>),
    /// L'avatar di una persona nel cerchio pixel, `AVATAR_M` 96 (il profilo).
    Avatar(Option<SharedString>),
    /// Le copertine dei primi brani di una playlist: con quattro diverse un mosaico 2 x 2, altrimenti la
    /// prima (o il segnaposto).
    Mosaic(Vec<SharedString>),
    /// Uno sprite su campo `GRAPHITE` («Brani che ti piacciono»: il cuore pieno, `WARM_WHITE`).
    Sprite(&'static str),
}

impl HeaderArt {
    /// Il lato dell'arte: la foto d'artista è più piccola della copertina.
    fn side(&self) -> f32 {
        match self {
            Self::Round(_) => AVATAR_L,
            Self::Avatar(_) => AVATAR_M,
            _ => COVER_L,
        }
    }
}

/// Fino a quattro copertine diverse, nell'ordine dei brani: ciò che serve al mosaico.
pub fn mosaic_urls<'a>(urls: impl IntoIterator<Item = Option<&'a str>>) -> Vec<SharedString> {
    let mut found: Vec<SharedString> = Vec::with_capacity(4);
    for url in urls.into_iter().flatten() {
        if !found.iter().any(|known| known.as_ref() == url) {
            found.push(SharedString::from(url.to_owned()));
            if found.len() == 4 {
                break;
            }
        }
    }
    found
}

fn art(art: HeaderArt) -> AnyElement {
    let side = art.side();
    match art {
        HeaderArt::Cover(url) => Cover::new(side).url(url).into_any_element(),
        HeaderArt::Round(url) | HeaderArt::Avatar(url) => {
            Cover::new(side).url(url).circle().into_any_element()
        }
        HeaderArt::Mosaic(urls) if urls.len() >= 4 => {
            let half = side / 2.0;
            let tile = |url: &SharedString| Cover::new(half).url(Some(url.clone()));
            div()
                .flex_none()
                .size(dp(side))
                .flex()
                .flex_wrap()
                .children(urls.iter().take(4).map(tile))
                .into_any_element()
        }
        HeaderArt::Mosaic(urls) => Cover::new(side).url(urls.first().cloned()).into_any_element(),
        HeaderArt::Sprite(name) => div()
            .flex_none()
            .size(dp(side))
            .bg(col(GRAPHITE))
            .flex()
            .items_center()
            .justify_center()
            .child(PixelSprite::new(name).tint(WARM_WHITE))
            .into_any_element(),
    }
}

/// La testa di una pagina d'oggetto.
#[derive(IntoElement)]
pub struct PageHeader {
    art: HeaderArt,
    kicker: SharedString,
    title: SharedString,
    link: Option<(ElementId, SharedString, OnClick)>,
    meta: SharedString,
    badge: Option<SharedString>,
    actions: Vec<AnyElement>,
}

impl PageHeader {
    pub fn new(art: HeaderArt, kicker: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            art,
            kicker: kicker.into(),
            title: title.into(),
            link: None,
            meta: SharedString::default(),
            badge: None,
            actions: Vec::new(),
        }
    }

    /// Un collegamento sotto il titolo (l'artista di un album): sottolineato al passaggio del mouse.
    pub fn link(
        mut self,
        id: impl Into<ElementId>,
        text: impl Into<SharedString>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.link = Some((id.into(), text.into(), std::rc::Rc::new(handler)));
        self
    }

    /// La riga dei dati (brani, durata, ascoltatori), già pronta.
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = meta.into();
        self
    }

    /// Un badge di qualità dopo i dati (`LOSSLESS`): non è lime, la qualità non è uno stato.
    pub fn quality(mut self, text: impl Into<SharedString>) -> Self {
        self.badge = Some(text.into());
        self
    }

    /// Un pulsante della riga delle azioni, nell'ordine di lettura.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    /// Lo scheletro della testa, alla stessa misura: arte, kicker, titolo, dati e una riga di pulsanti.
    pub fn loading(round: bool) -> impl IntoElement {
        Self::skeleton(if round { AVATAR_L } else { COVER_L }, round)
    }

    /// Lo scheletro della testa di un profilo (avatar `AVATAR_M` nel cerchio).
    pub fn loading_person() -> impl IntoElement {
        Self::skeleton(AVATAR_M, true)
    }

    fn skeleton(side: f32, round: bool) -> impl IntoElement {
        frame(
            if round {
                Cover::new(side).circle().into_any_element()
            } else {
                Skeleton::new(side, side).into_any_element()
            },
            div()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .flex_col()
                .gap(dp(SPACE_3))
                .child(Skeleton::new(64.0, 12.0))
                .child(Skeleton::new(320.0, 32.0))
                .child(Skeleton::new(200.0, 12.0))
                .child(div().pt(dp(SPACE_3)).child(Skeleton::new(240.0, CONTROL_H_LARGE))),
        )
    }
}

/// L'arte a sinistra e la colonna di testo a destra, allineata in basso.
fn frame(art: AnyElement, column: impl IntoElement) -> gpui::Div {
    div()
        .flex_none()
        .w_full()
        .flex()
        .items_end()
        .gap(dp(SPACE_6))
        .px(dp(SPACE_6))
        .pt(dp(SPACE_6))
        .pb(dp(SPACE_5))
        .child(art)
        // La colonna prende la larghezza che resta e può stringersi sotto il suo contenuto (`min_w_0`):
        // il titolo va a capo lì dentro e le righe corte si troncano, non escono dalla testa.
        .child(div().flex_1().min_w_0().flex().flex_col().child(column))
}

impl RenderOnce for PageHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut text = div().flex().flex_col().gap(dp(SPACE_2)).min_w_0();
        if !self.kicker.is_empty() {
            text = text.child(
                div()
                    .type_role(TYPE_LABEL)
                    .text_color(col(TEXT_TERTIARY))
                    .truncate()
                    .child(cased(&self.kicker, TYPE_LABEL)),
            );
        }
        // Al più due righe; se non basta, l'ultima finisce con «…».
        text = text.child(
            div()
                .w_full()
                .type_role(TYPE_DISPLAY)
                .text_color(col(TEXT_PRIMARY))
                .line_clamp(2)
                .text_ellipsis()
                .child(self.title),
        );
        if let Some((id, label, handler)) = self.link {
            text = text.child(
                div().flex().min_w_0().child(
                    div()
                        .id(id)
                        .min_w_0()
                        .type_role(TYPE_TRACK_TITLE)
                        .text_color(col(TEXT_PRIMARY))
                        .truncate()
                        .cursor(CursorStyle::PointingHand)
                        .hover(|style| style.underline())
                        .on_click(move |event, window, cx| handler(event, window, cx))
                        .child(label),
                ),
            );
        }
        if !self.meta.is_empty() || self.badge.is_some() {
            let mut meta = div().flex().items_center().gap(dp(SPACE_3)).min_w_0();
            if !self.meta.is_empty() {
                meta = meta.child(
                    div()
                        .min_w_0()
                        .type_role(TYPE_META)
                        .text_color(col(TEXT_SECONDARY))
                        .truncate()
                        .child(self.meta),
                );
            }
            if let Some(badge) = self.badge {
                meta = meta.child(div().flex_none().child(Badge::new(badge, BadgeKind::Quality)));
            }
            text = text.child(meta);
        }
        let mut column = div().w_full().min_w_0().flex().flex_col().gap(dp(SPACE_5)).child(text);
        if !self.actions.is_empty() {
            // In una colonna stretta i pulsanti vanno a capo invece di uscire dalla testa.
            column =
                column.child(div().flex().flex_wrap().items_center().gap(dp(SPACE_3)).children(self.actions));
        }
        frame(art(self.art), column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mosaic_takes_four_different_covers_in_order() {
        let urls = [Some("a"), None, Some("a"), Some("b"), Some("c"), Some("d"), Some("e")];
        let found: Vec<String> = mosaic_urls(urls).iter().map(ToString::to_string).collect();
        assert_eq!(found, ["a", "b", "c", "d"]);
        assert_eq!(mosaic_urls([Some("a"), Some("a")]).len(), 1);
        assert!(mosaic_urls([None, None]).is_empty());
    }

    #[test]
    fn an_artist_photo_is_smaller_than_a_cover() {
        assert_eq!(HeaderArt::Round(None).side(), AVATAR_L);
        assert_eq!(HeaderArt::Cover(None).side(), COVER_L);
        assert_eq!(HeaderArt::Sprite("x").side(), COVER_L);
        assert_eq!(HeaderArt::Avatar(None).side(), AVATAR_M, "un profilo: avatar da 96");
    }
}
