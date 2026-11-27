//! PageHeader: the head of a page that shows an object (album, artist, playlist, liked songs, a profile).
//! On the canvas, no tint: on the left the art (cover `ART_HEADER` 232, the artist photo `ART_ARTIST` 200 in
//! the pixel circle, a person `AVATAR_L` 160, a playlist's 2 x 2 mosaic or a sprite on `GRAPHITE`); on the
//! right the kicker (`TYPE_LABEL`), the name in `TYPE_DISPLAY` (two lines at most), an optional link (the
//! album's artist), the data line (`TYPE_META`, with a quality badge) and the action row, led by the 56 dp
//! primary play button (desktop v2 §6). Everything is in dp, so the head grows with the zoom.
//!
//! App screens (Libreria, Scaricati) keep `PageTitle::screen`. [`PageHeader::loading`] is the skeleton at the
//! same size.

use gpui::{
    AnyElement, App, ClickEvent, CursorStyle, ElementId, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div,
};
use psytify_design::sprite;
use psytify_design::tokens::color::{GRAPHITE, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY, WARM_WHITE};
use psytify_design::tokens::size::cover::{ART_ARTIST, ART_HEADER, AVATAR_L};
use psytify_design::tokens::size::{FRAME_MARGIN, PLAY_HERO};
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{TYPE_DISPLAY, TYPE_LABEL, TYPE_META, TYPE_TRACK_TITLE};

use super::badge::{Badge, BadgeKind};
use super::control::OnClick;
use super::icon_button::IconButton;
use super::indicators::Skeleton;
use super::tile::Cover;
use crate::pixel::PixelSprite;
use crate::theme::{TypeStyle, cased, col, dp};

/// The art on the left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeaderArt {
    /// A cover (the placeholder until it arrives, or if there is none).
    Cover(Option<SharedString>),
    /// An artist photo in the pixel circle.
    Round(Option<SharedString>),
    /// A person's avatar in the pixel circle (the profile).
    Avatar(Option<SharedString>),
    /// The covers of a playlist's first tracks: a 2 x 2 mosaic with four different ones, else the first
    /// (or the placeholder).
    Mosaic(Vec<SharedString>),
    /// A sprite on a `GRAPHITE` field (liked songs: the filled heart, `WARM_WHITE`).
    Sprite(&'static str),
}

impl HeaderArt {
    /// The side of the art: photos are smaller than covers.
    fn side(&self) -> f32 {
        match self {
            Self::Round(_) => ART_ARTIST,
            Self::Avatar(_) => AVATAR_L,
            _ => ART_HEADER,
        }
    }
}

/// Up to four different covers, in track order: what the mosaic needs.
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

/// The head of an object page.
#[derive(IntoElement)]
pub struct PageHeader {
    art: HeaderArt,
    kicker: SharedString,
    title: SharedString,
    link: Option<(ElementId, SharedString, OnClick)>,
    meta: SharedString,
    badge: Option<SharedString>,
    play: Option<(ElementId, OnClick)>,
    actions: Vec<AnyElement>,
    margin: f32,
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
            play: None,
            actions: Vec::new(),
            margin: FRAME_MARGIN,
        }
    }

    /// The page's canvas margin (`components::frame`).
    pub fn margin(mut self, margin: f32) -> Self {
        self.margin = margin;
        self
    }

    /// The 56 dp primary play button that leads the action row.
    pub fn play(
        mut self,
        id: impl Into<ElementId>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.play = Some((id.into(), std::rc::Rc::new(handler)));
        self
    }

    /// A link under the title (an album's artist): underlined on hover.
    pub fn link(
        mut self,
        id: impl Into<ElementId>,
        text: impl Into<SharedString>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.link = Some((id.into(), text.into(), std::rc::Rc::new(handler)));
        self
    }

    /// The data line (tracks, duration, listeners), ready made.
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = meta.into();
        self
    }

    /// A quality badge after the data (`LOSSLESS`): not lime, quality is not a state.
    pub fn quality(mut self, text: impl Into<SharedString>) -> Self {
        self.badge = Some(text.into());
        self
    }

    /// A button of the action row, in reading order (after the play button).
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    /// The skeleton of the head at the same size: art, kicker, title, data and the action row.
    pub fn loading(round: bool, margin: f32) -> impl IntoElement {
        Self::skeleton(if round { ART_ARTIST } else { ART_HEADER }, round, margin)
    }

    /// The skeleton of a profile head (the avatar in the circle).
    pub fn loading_person(margin: f32) -> impl IntoElement {
        Self::skeleton(AVATAR_L, true, margin)
    }

    fn skeleton(side: f32, round: bool, margin: f32) -> impl IntoElement {
        frame(
            margin,
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
                .child(div().pt(dp(SPACE_3)).child(Skeleton::new(240.0, PLAY_HERO))),
        )
    }
}

/// The art on the left and the text column on the right, aligned to the bottom.
fn frame(margin: f32, art: AnyElement, column: impl IntoElement) -> gpui::Div {
    div()
        .flex_none()
        .w_full()
        .flex()
        .items_end()
        .gap(dp(SPACE_6))
        .px(dp(margin))
        .pt(dp(SPACE_6))
        .pb(dp(SPACE_6))
        .child(art)
        // The column takes the width left and may shrink below its content (`min_w_0`): the title wraps
        // inside it and short lines truncate instead of leaving the head.
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
        // Two lines at most; the last ends with an ellipsis.
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
        let play = self.play.map(|(id, handler)| {
            IconButton::new(id, sprite::ICON_PLAY)
                .primary()
                .size(PLAY_HERO)
                .tooltip("Riproduci", None)
                .on_click(move |event, window, cx| handler(event, window, cx))
        });
        if play.is_some() || !self.actions.is_empty() {
            // In a narrow column the buttons wrap instead of leaving the head.
            column = column.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(dp(SPACE_3))
                    .children(play)
                    .children(self.actions),
            );
        }
        frame(self.margin, art(self.art), column)
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
        assert_eq!(HeaderArt::Round(None).side(), ART_ARTIST);
        assert_eq!(HeaderArt::Cover(None).side(), ART_HEADER);
        assert_eq!(HeaderArt::Sprite("x").side(), ART_HEADER);
        assert_eq!(HeaderArt::Avatar(None).side(), AVATAR_L, "a profile: a 160 avatar");
    }
}
