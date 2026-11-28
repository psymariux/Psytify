//! Shortcut: an item of *Riprendi* at the top of the Home: a compact 56 dp row with the 56 dp cover flush
//! and the title, in a 2 to 4 column grid like a quick access panel. At rest the fill is `SURFACE_RAISED`
//! (it reads as a target without a pointer); on hover the tone eases to `SURFACE_SELECTED` and, with a play
//! action, the play button steps in from the right in three frames. Pressed takes the same tone, focus the
//! inner ring. If it is what plays, the equalizer sits at the end (painted by `Shell`, see `playing_spot`).

use gpui::{App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use psytify_design::sprite;
use psytify_design::tokens::color::{PURE_WHITE, SURFACE_RAISED, SURFACE_SELECTED, TEXT_PRIMARY};
use psytify_design::tokens::motion::STEP_FRAMES;
use psytify_design::tokens::size::cover::ART_SHORTCUT;
use psytify_design::tokens::space::{SPACE_3, SPACE_4};
use psytify_design::tokens::type_::TYPE_TRACK_TITLE;

use super::control::{OnClick, Preview, wire, with_click};
use super::icon_button::IconButton;
use super::indicators::{Equalizer, PlayState};
use super::list_row::{RowArt, row_art};
use super::playing_spot::PlayingSpot;
use crate::motion::steps;
use crate::pixel::shape::inset_ring;
use crate::theme::{Ap, TypeStyle, col, dp, mix};

#[derive(IntoElement)]
pub struct Shortcut {
    id: ElementId,
    art: RowArt,
    title: SharedString,
    playing: Option<(PlayState, Option<PlayingSpot>)>,
    preview: Preview,
    on_click: Option<OnClick>,
    on_play: Option<OnClick>,
}

impl Shortcut {
    pub fn new(id: impl Into<ElementId>, art: RowArt, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            art,
            title: title.into(),
            playing: None,
            preview: Preview::Live,
            on_click: None,
            on_play: None,
        }
    }

    /// The play button that steps in on hover (it does not also open the item).
    pub fn on_play(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_play = Some(std::rc::Rc::new(handler));
        self
    }

    /// It is what plays: the equaliser at the bottom, painted by `Shell` over the cached view.
    pub fn playing_at(mut self, state: PlayState, spot: &PlayingSpot) -> Self {
        self.playing = Some((state, Some(spot.clone())));
        self
    }

    /// Come [`Shortcut::playing_at`], con l'equalizzatore dentro la riga (la galleria).
    pub fn playing(mut self, state: PlayState) -> Self {
        self.playing = Some((state, None));
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

impl RenderOnce for Shortcut {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let wired = wire(&self.id, false, self.preview, window, cx);
        let v = wired.visual;
        let fill =
            if v.pressed { col(SURFACE_SELECTED) } else { mix(SURFACE_RAISED, SURFACE_SELECTED, v.hover_t) };
        let mut row = wired
            .element
            .relative()
            .h(dp(ART_SHORTCUT))
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(dp(SPACE_4))
            .pr(dp(SPACE_3))
            .bg(fill)
            .child(row_art(self.art, ART_SHORTCUT))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .type_role(TYPE_TRACK_TITLE)
                    .text_color(col(TEXT_PRIMARY))
                    .truncate()
                    .child(self.title),
            );
        let frames = steps(v.hover_t, STEP_FRAMES);
        if let Some(play) = self.on_play.filter(|_| frames > 0) {
            // Three frames from 4 AP to the right to its place.
            let button =
                IconButton::new(SharedString::from(format!("{:?}-play", self.id)), sprite::ICON_PLAY)
                    .primary()
                    .tooltip("Riproduci", None)
                    .on_click(move |event, window, cx| {
                        cx.stop_propagation();
                        play(event, window, cx);
                    });
            row = row
                .child(div().relative().left(ap.n(4 * (STEP_FRAMES - frames) / STEP_FRAMES)).child(button));
        }
        if let Some((state, spot)) = self.playing {
            row = row.child(match spot {
                Some(spot) => spot.marker(state).into_any_element(),
                None => Equalizer::new(state).into_any_element(),
            });
        }
        let row = row.child(inset_ring(col(PURE_WHITE), wired.focus.clone(), v.force_ring));
        with_click(row, self.on_click)
    }
}
