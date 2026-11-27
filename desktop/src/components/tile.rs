//! Cover (segnaposto o copertina) e AlbumTile/ArtistTile. La copertina vera arriva dal deposito
//! (`crate::covers`, che la chiede al core) e si disegna sulla griglia dei pixel fisici; finché non c'è,
//! o se non esiste, resta il segnaposto: campo `GRAPHITE` con `icon_library` (lato < 64 dp) o
//! `album_placeholder` (oltre), come da token. La galleria non ha il deposito e mostra il segnaposto.

use std::sync::Arc;

use gpui::{
    AnyElement, App, Bounds, ContentMask, Corners, ElementId, IntoElement, ParentElement, RenderImage,
    RenderOnce, SharedString, Styled, Window, canvas, div, point, px, size,
};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    CONTROL_BORDER_HOVER, GRAPHITE, PURE_WHITE, STONE, SURFACE_CANVAS, TEXT_DISABLED, TEXT_PRIMARY,
    TEXT_TERTIARY, WARM_WHITE,
};
use psytify_design::tokens::size::cover::PLACEHOLDER_ART_MIN;
use psytify_design::tokens::space::SPACE_3;
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_TRACK_TITLE};

use super::badge::{Badge, BadgeKind};
use super::control::{OnClick, Preview, wire, with_click};
use super::icon_button::IconButton;
use super::indicators::{Equalizer, PlayState};
use super::playing_spot::PlayingSpot;
use crate::covers::{CoverArt, Covers};
use crate::pixel::PixelSprite;
use crate::pixel::shape::{circle_masks, circle_outline, dither, paint_circle_fill, static_ring};
use crate::theme::{Ap, TypeStyle, col, dp};

/// Copertina quadrata di `side` dp. `unavailable` la copre con un dither al 50 % di `DEEP_BLACK`.
#[derive(IntoElement)]
pub struct Cover {
    side: f32,
    unavailable: bool,
    round: bool,
    large: bool,
    url: Option<SharedString>,
}

impl Cover {
    pub fn new(side: f32) -> Self {
        Self { side, unavailable: false, round: false, large: false, url: None }
    }

    /// La copertina intera (256 px) invece delle versioni ridotte: solo per la vista immersiva, dove la
    /// copertina arriva a 640 dp (vedi `covers::MAX_LARGE`). Finché non c'è si disegna la versione media.
    pub fn large(mut self) -> Self {
        self.large = true;
        self
    }

    /// L'indirizzo della copertina vera: il deposito la chiede al core la prima volta e la disegna
    /// quando arriva.
    pub fn url(mut self, url: Option<SharedString>) -> Self {
        self.url = url;
        self
    }

    pub fn unavailable(mut self, unavailable: bool) -> Self {
        self.unavailable = unavailable;
        self
    }

    /// Maschera a cerchio pixel (`MASK_PIXEL_CIRCLE`, foto d'artista e avatar): il campo e la foto si
    /// dipingono solo dentro il cerchio rasterizzato a risoluzione AP, bordi a gradini di 1 AP. Fuori dal
    /// cerchio resta la superficie sotto (la riga in hover, la tela).
    pub fn circle(mut self) -> Self {
        self.round = true;
        self
    }
}

/// L'immagine da disegnare: una copertina in due versioni ridotte, o quella intera.
enum Art {
    Sized(CoverArt),
    Whole(Arc<RenderImage>),
}

/// Disegna la copertina nel suo riquadro, con l'origine e il lato arrotondati ai pixel fisici (il
/// campionatore resta vicino al 1:1) e la versione più piccola che copre quei pixel.
fn paint_cover(window: &mut Window, cx: &mut App, art: &Art, slot: Bounds<gpui::Pixels>) {
    Covers::drain_retired(window, cx);
    let scale = window.scale_factor();
    let (x, y) = ((f32::from(slot.origin.x) * scale).round(), (f32::from(slot.origin.y) * scale).round());
    let side = (f32::from(slot.size.width) * scale).round();
    let image = match art {
        Art::Sized(art) => art.for_device_px(side),
        Art::Whole(image) => Arc::clone(image),
    };
    let bounds = Bounds::new(point(px(x / scale), px(y / scale)), size(px(side / scale), px(side / scale)));
    // Se l'immagine non entra nell'atlante resta il campo `GRAPHITE` dietro.
    let _ = window.paint_image(bounds, bounds, Corners::default(), image, 0, false);
}

/// La foto nel cerchio pixel: il campo `GRAPHITE` e la foto, fascia per fascia (ogni fascia è una
/// maschera rettangolare della stessa foto, così i gradini cadono sui pixel fisici e non c'è sfumatura).
fn paint_round(window: &mut Window, cx: &mut App, art: Option<&Art>, slot: Bounds<gpui::Pixels>) {
    let ap = Ap::of(window);
    paint_circle_fill(window, slot, ap, col(GRAPHITE));
    let Some(art) = art else { return };
    for mask in circle_masks(slot, ap) {
        window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
            paint_cover(window, cx, art, slot)
        });
    }
}

impl RenderOnce for Cover {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let art = self.url.as_deref().and_then(|url| {
            let whole = if self.large { Covers::large(url, cx) } else { None };
            match whole {
                Some(image) => Some(Art::Whole(image)),
                None => Covers::art(url, cx).map(Art::Sized),
            }
        });
        if self.round {
            return self.render_round(art, cx);
        }
        let content: AnyElement = match art {
            Some(art) => {
                canvas(|_, _, _| (), move |bounds, (), window, cx| paint_cover(window, cx, &art, bounds))
                    .absolute()
                    .size_full()
                    .into_any_element()
            }
            None if self.side >= PLACEHOLDER_ART_MIN => {
                PixelSprite::new(sprite::ALBUM_PLACEHOLDER).into_any_element()
            }
            None => PixelSprite::new(sprite::ICON_LIBRARY).tint(STONE).into_any_element(),
        };
        let mut field = div()
            .relative()
            .flex_none()
            .size(dp(self.side))
            .bg(col(GRAPHITE))
            .flex()
            .items_center()
            .justify_center()
            .child(content);
        if self.unavailable {
            field = field.child(dither(col(SURFACE_CANVAS)));
        }
        field.into_any_element()
    }
}

impl Cover {
    /// Il cerchio: il campo e la foto li dipinge un canvas (la maschera), il segnaposto sta sopra al centro.
    fn render_round(self, art: Option<Art>, _cx: &mut App) -> AnyElement {
        let placeholder = art.is_none().then(|| {
            if self.side >= PLACEHOLDER_ART_MIN {
                PixelSprite::new(sprite::ALBUM_PLACEHOLDER).into_any_element()
            } else {
                PixelSprite::new(sprite::ICON_ACCOUNT).tint(STONE).into_any_element()
            }
        });
        let mut field = div()
            .relative()
            .flex_none()
            .size(dp(self.side))
            .flex()
            .items_center()
            .justify_center()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, cx| paint_round(window, cx, art.as_ref(), bounds),
                )
                .absolute()
                .size_full(),
            )
            .children(placeholder);
        if self.unavailable {
            field = field.child(dither(col(SURFACE_CANVAS)));
        }
        field.into_any_element()
    }
}

/// Il mosaico 2 x 2 di un mix: quattro copertine di mezzo lato, a filo (niente spazio fra le quattro).
fn mosaic(side: f32, urls: &[SharedString], disabled: bool) -> AnyElement {
    let half = side / 2.0;
    div()
        .flex_none()
        .size(dp(side))
        .flex()
        .flex_wrap()
        .children(
            urls.iter().take(4).map(|url| Cover::new(half).url(Some(url.clone())).unavailable(disabled)),
        )
        .into_any_element()
}

/// Tile di album o artista: copertina, titolo, sottotitolo. Hover: cornice di 1 AP e Play primario
/// in basso a destra (solo se c'è chi suona: [`AlbumTile::on_play`]); selezionato: cornice `WARM_WHITE`;
/// in riproduzione: `eq` prima del sottotitolo. ArtistTile ([`AlbumTile::round`]): foto nel cerchio pixel
/// e testo centrato.
#[derive(IntoElement)]
pub struct AlbumTile {
    id: ElementId,
    title: SharedString,
    subtitle: SharedString,
    cover_url: Option<SharedString>,
    round: bool,
    on_play: Option<OnClick>,
    side: f32,
    selected: bool,
    disabled: bool,
    playing: Option<PlayState>,
    /// Dove `Shell` dipinge l'equalizzatore al posto del tile (vedi `playing_spot`).
    spot: Option<PlayingSpot>,
    /// Un'etichetta sull'angolo della copertina («Nuovo»).
    badge: Option<SharedString>,
    /// Il mosaico 2 x 2 di un mix: fino a quattro copertine diverse al posto di una.
    mosaic: Vec<SharedString>,
    preview: Preview,
    on_click: Option<OnClick>,
}

impl AlbumTile {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            subtitle: subtitle.into(),
            cover_url: None,
            round: false,
            on_play: None,
            side: 160.0,
            selected: false,
            disabled: false,
            playing: None,
            spot: None,
            badge: None,
            mosaic: Vec::new(),
            preview: Preview::Live,
            on_click: None,
        }
    }

    pub fn side(mut self, side: f32) -> Self {
        self.side = side;
        self
    }

    /// La copertina vera (o la foto, per un artista).
    pub fn cover_url(mut self, url: Option<SharedString>) -> Self {
        self.cover_url = url;
        self
    }

    /// ArtistTile: foto nel cerchio pixel, testo centrato.
    pub fn round(mut self) -> Self {
        self.round = true;
        self
    }

    /// Il Play sulla copertina (in hover): senza, il tile si apre soltanto.
    pub fn on_play(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_play = Some(std::rc::Rc::new(handler));
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn playing(mut self, state: PlayState) -> Self {
        self.playing = Some(state);
        self
    }

    /// Come [`AlbumTile::playing`], ma l'equalizzatore lo dipinge `Shell` sopra la vista in cache.
    pub fn playing_at(mut self, state: PlayState, spot: &PlayingSpot) -> Self {
        self.playing = Some(state);
        self.spot = Some(spot.clone());
        self
    }

    /// Un'etichetta sull'angolo in alto a sinistra della copertina (`Badge` di conteggio: «Nuovo»).
    pub fn badge(mut self, text: impl Into<SharedString>) -> Self {
        self.badge = Some(text.into());
        self
    }

    /// Il mosaico di un mix: con quattro copertine diverse un 2 x 2, altrimenti la prima.
    pub fn mosaic(mut self, urls: Vec<SharedString>) -> Self {
        self.mosaic = urls;
        self
    }

    pub fn preview(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }
}

impl RenderOnce for AlbumTile {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let wired = wire(&self.id, self.disabled, self.preview, window, cx);
        let v = wired.visual;
        let art = if self.mosaic.len() >= 4 {
            mosaic(self.side, &self.mosaic, self.disabled)
        } else {
            let url = self.mosaic.first().cloned().or_else(|| self.cover_url.clone());
            let mut art = Cover::new(self.side).url(url).unavailable(self.disabled);
            if self.round {
                art = art.circle();
            }
            art.into_any_element()
        };
        let mut cover = div().relative().flex_none().size(dp(self.side)).child(art);
        if let Some(text) = self.badge.clone() {
            cover = cover.child(
                div().absolute().left(dp(SPACE_3)).top(dp(SPACE_3)).child(Badge::new(text, BadgeKind::Count)),
            );
        }
        // Cornice di 1 AP dentro il bordo della copertina (sul cerchio, il suo contorno).
        let frame = if self.selected {
            Some(WARM_WHITE)
        } else if v.hovered && !self.disabled {
            Some(CONTROL_BORDER_HOVER)
        } else {
            None
        };
        if let Some(color) = frame {
            cover = cover.child(if self.round {
                circle_outline(col(color)).into_any_element()
            } else {
                crate::pixel::shape::inset_ring(col(color), wired.focus.clone(), true).into_any_element()
            });
        }
        if v.force_ring {
            cover = cover.child(static_ring(0, col(PURE_WHITE), ap));
        } else {
            cover = cover.child(crate::pixel::shape::focus_ring(0, col(PURE_WHITE), wired.focus.clone(), ap));
        }
        if let Some(play) =
            self.on_play.clone().filter(|_| (v.hovered || self.playing.is_some()) && !self.disabled)
        {
            let icon =
                if self.playing == Some(PlayState::Playing) { sprite::ICON_PAUSE } else { sprite::ICON_PLAY };
            cover = cover.child(div().absolute().right(dp(SPACE_3)).bottom(dp(SPACE_3)).child(
                IconButton::new(SharedString::from(format!("{:?}-play", self.id)), icon).primary().on_click(
                    move |event, window, cx| {
                        // Il Play suona e basta: non apre anche il tile.
                        cx.stop_propagation();
                        play(event, window, cx);
                    },
                ),
            ));
        }
        let subtitle_color = if self.disabled { TEXT_DISABLED } else { TEXT_TERTIARY };
        let mut subtitle =
            div().flex().items_center().gap(dp(4.0)).type_role(TYPE_ARTIST).text_color(col(subtitle_color));
        if let Some(state) = self.playing {
            subtitle = subtitle.child(match &self.spot {
                Some(spot) => spot.marker(state).into_any_element(),
                None => Equalizer::new(state).into_any_element(),
            });
        }
        let subtitle = subtitle.child(div().truncate().child(self.subtitle));
        let title_color = if self.disabled { TEXT_DISABLED } else { TEXT_PRIMARY };
        let mut text = div().flex().flex_col().gap(px(2.0));
        if self.round {
            text = text.items_center();
        }
        let element =
            wired.element.flex().flex_col().flex_none().w(dp(self.side)).gap(dp(SPACE_3)).child(cover).child(
                text.child(
                    div()
                        .type_role(TYPE_TRACK_TITLE)
                        .text_color(col(title_color))
                        .truncate()
                        .child(self.title),
                )
                .child(subtitle),
            );
        with_click(element, if self.disabled { None } else { self.on_click })
    }
}
