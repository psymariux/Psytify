//! ListRow: a 48 dp row (`TRACK_ROW_H`, `uniform_list`-ready) for what is not a catalogue track row: the
//! library items (playlists, liked songs, saved albums, followed artists), running downloads and the tracks
//! on the device. Grid: optional index | 40 dp art (`ART_ROW`) | title over detail | optional ten-step bar |
//! actions.
//!
//! States as TrackRow: rest, hover (the `SURFACE_RAISED` tone eases in; the index becomes `icon_play`),
//! pressed (`SURFACE_SELECTED`), focus (inner ring), playing (the equalizer instead of the index, lime only
//! while it plays). The detail can be an error (`TEXT_ERROR`). Actions are always visible (cancel a download)
//! or come in on hover, sliding 2 AP from the right with the space kept (rename, remove).

use gpui::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window, div,
};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    GRAPHITE, ICON_ACTIVE, PURE_WHITE, SURFACE_RAISED, SURFACE_SELECTED, TEXT_ERROR, TEXT_PRIMARY,
    TEXT_SECONDARY, TEXT_TERTIARY,
};
use psytify_design::tokens::size::{ICON_BUTTON, TRACK_INDEX_W, TRACK_ROW_H, cover::ART_ROW};
use psytify_design::tokens::space::{SPACE_3, SPACE_6};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_NUMERIC, TYPE_TRACK_TITLE};

use super::badge::{Badge, BadgeKind};
use super::control::{OnClick, Preview, centered, wire, with_click};
use super::icon_button::IconButton;
use super::indicators::{DownloadSteps, Equalizer, PlayState};
use super::playing_spot::PlayingSpot;
use super::tile::Cover;
use crate::motion::ap_round;
use crate::pixel::PixelSprite;
use crate::pixel::shape::inset_ring;
use crate::theme::{Ap, TypeStyle, col, dp, fade};

/// L'arte a sinistra del testo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowArt {
    /// A cover (the placeholder until it arrives or if there is none).
    Cover(Option<SharedString>),
    /// An artist's photo, in the circle.
    Round(Option<SharedString>),
    /// A sprite on a `GRAPHITE` field (the heart of «Brani che ti piacciono»), tinted `ICON_ACTIVE`.
    Sprite(&'static str),
    /// No art (a ranking of names, where the index is already there).
    None,
}

#[derive(IntoElement)]
pub struct ListRow {
    id: ElementId,
    index: Option<u32>,
    art: RowArt,
    title: SharedString,
    detail: SharedString,
    detail_error: bool,
    /// A label after the title («Nuovo», «Album»).
    badge: Option<(SharedString, BadgeKind)>,
    steps: Option<f32>,
    playing: Option<PlayState>,
    /// Where `Shell` paints the equaliser in place of the row (see `playing_spot`).
    spot: Option<PlayingSpot>,
    selected: bool,
    actions: Vec<IconButton>,
    hover_actions: Vec<IconButton>,
    inset: Option<f32>,
    preview: Preview,
    on_click: Option<OnClick>,
}

impl ListRow {
    pub fn new(id: impl Into<ElementId>, art: RowArt, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            index: None,
            art,
            title: title.into(),
            detail: SharedString::default(),
            detail_error: false,
            badge: None,
            steps: None,
            playing: None,
            spot: None,
            selected: false,
            actions: Vec::new(),
            hover_actions: Vec::new(),
            inset: None,
            preview: Preview::Live,
            on_click: None,
        }
    }

    /// The number column (from 1), as in track lists.
    pub fn index(mut self, index: u32) -> Self {
        self.index = Some(index);
        self
    }

    /// The second line, already ready.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = detail.into();
        self
    }

    /// La seconda riga dice un problema: `TEXT_ERROR`.
    pub fn detail_error(mut self, error: bool) -> Self {
        self.detail_error = error;
        self
    }

    /// A label right after the title: a count («Nuovo» in «Novità») or neutral (the kind of
    /// a release in the Home's log).
    pub fn badge(mut self, text: impl Into<SharedString>, kind: BadgeKind) -> Self {
        self.badge = Some((text.into(), kind));
        self
    }

    /// La barra a dieci passi di un download in corso (0..=1).
    pub fn steps(mut self, progress: f32) -> Self {
        self.steps = Some(progress);
        self
    }

    /// The row is the playing track: the equaliser in place of the index.
    pub fn playing(mut self, state: PlayState) -> Self {
        self.playing = Some(state);
        self
    }

    /// Come [`ListRow::playing`], ma l'equalizzatore lo dipinge `Shell` sopra la vista in cache.
    pub fn playing_at(mut self, state: PlayState, spot: &PlayingSpot) -> Self {
        self.playing = Some(state);
        self.spot = Some(spot.clone());
        self
    }

    /// The entry is the open page: `SURFACE_SELECTED` fill.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Un'azione sempre visibile (annulla, riprova).
    pub fn action(mut self, button: IconButton) -> Self {
        self.actions.push(button);
        self
    }

    /// An action that appears only on mouse hover; the space stays.
    pub fn hover_action(mut self, button: IconButton) -> Self {
        self.hover_actions.push(button);
        self
    }

    /// The left padding in dp, for rows inside a flowing column (`components::frame::Flow`).
    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = Some(inset);
        self
    }

    pub fn preview(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    /// The row's gesture: whoever uses it decides whether a click (open) is enough or a double one (play) is needed.
    pub fn on_click(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }
}

/// The art of a row, `side` dp (`ART_ROW` in rows, `ART_SHORTCUT` in the Home shortcuts).
pub fn row_art(art: RowArt, side: f32) -> AnyElement {
    match art {
        RowArt::Cover(url) => Cover::new(side).url(url).into_any_element(),
        RowArt::Round(url) => Cover::new(side).url(url).circle().into_any_element(),
        RowArt::Sprite(name) => centered(div().flex_none().size(dp(side)).bg(col(GRAPHITE)))
            .child(PixelSprite::new(name).tint(ICON_ACTIVE))
            .into_any_element(),
        RowArt::None => div().into_any_element(),
    }
}

/// An action that must not also trigger the row's gesture.
fn stop_propagation(button: IconButton) -> AnyElement {
    div()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(button)
        .into_any_element()
}

impl RenderOnce for ListRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let wired = wire(&self.id, false, self.preview, window, cx);
        let v = wired.visual;
        let fill = if v.pressed || self.selected {
            Some(col(SURFACE_SELECTED))
        } else if v.hover_t > 0.0 {
            Some(fade(SURFACE_RAISED, v.hover_t))
        } else {
            None
        };
        let numbered = self.index.is_some() || self.playing.is_some();
        let mut row = wired
            .element
            .relative()
            .h(dp(TRACK_ROW_H))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .gap(dp(SPACE_3))
            // Senza la colonna del numero l'arte si allinea alle intestazioni di sezione (`SPACE_6`).
            .pl(dp(self.inset.unwrap_or(if numbered { SPACE_3 } else { SPACE_6 })))
            .pr(dp(SPACE_3));
        if numbered {
            let index: AnyElement = match (self.playing, v.hovered) {
                (Some(state), false) => match &self.spot {
                    Some(spot) => spot.marker(state).into_any_element(),
                    None => Equalizer::new(state).into_any_element(),
                },
                (_, true) => PixelSprite::new(sprite::ICON_PLAY).tint(ICON_ACTIVE).into_any_element(),
                (None, false) => div()
                    .type_role(TYPE_NUMERIC)
                    .text_color(col(TEXT_TERTIARY))
                    .child(SharedString::from(self.index.unwrap_or_default().to_string()))
                    .into_any_element(),
            };
            row = row.child(centered(div().w(dp(TRACK_INDEX_W)).flex_none()).child(index));
        }
        let detail_color = if self.detail_error { TEXT_ERROR } else { TEXT_SECONDARY };
        if self.art != RowArt::None {
            row = row.child(row_art(self.art, ART_ROW));
        }
        row = row.child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(dp(SPACE_3))
                        .min_w_0()
                        .child(
                            div()
                                .min_w_0()
                                .type_role(TYPE_TRACK_TITLE)
                                .text_color(col(TEXT_PRIMARY))
                                .truncate()
                                .child(self.title),
                        )
                        .children(self.badge.map(|(text, kind)| Badge::new(text, kind))),
                )
                .child(
                    div().type_role(TYPE_ARTIST).text_color(col(detail_color)).truncate().child(self.detail),
                ),
        );
        if let Some(progress) = self.steps {
            row = row.child(div().flex_none().child(DownloadSteps::new(progress)));
        }
        // Hover actions slide in 2 AP from the right and fade with the hover; the space stays.
        let t = v.hover_t;
        let hover_actions = self.hover_actions.into_iter().map(|button| {
            if t > 0.0 {
                div()
                    .relative()
                    .left(ap.n(ap_round((1.0 - t) * 2.0)))
                    .opacity(t)
                    .child(stop_propagation(button))
                    .into_any_element()
            } else {
                div().size(dp(ICON_BUTTON)).into_any_element()
            }
        });
        let actions = self.actions.into_iter().map(stop_propagation);
        row = row
            .child(div().flex().flex_none().items_center().children(actions).children(hover_actions))
            .child(inset_ring(col(PURE_WHITE), wired.focus.clone(), v.force_ring));
        if let Some(color) = fill {
            row = row.bg(color);
        }
        with_click(row, self.on_click)
    }
}
