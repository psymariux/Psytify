//! The equaliser spot of the playing track in a list (BACKLOG 23).
//!
//! An animated equaliser inside a `.cached()` view invalidates it at every frame (`pixel::anim`
//! notifies the view that painted it, and GPUI marks its ancestors dirty too, `Window::mark_view_dirty`):
//! the page redid the whole list ten times a second just to change twelve pixels. Here the row
//! does not paint the equaliser: it puts in its place a placeholder of the same size ([`PlayingSpot::marker`])
//! that, when the view paints, notes where it is and under which mask (the scrolling list clips it).
//! `Shell` paints it over the page ([`overlay`]): the sprite's timer invalidates only `Shell`,
//! which is not cached, and the page is reused as it was (`AnyView::cached` reuses prepaint and paint while the
//! view is not dirty and its size does not change).
//!
//! The spot holds until the view redraws: the view clears it at the start of `render`
//! ([`PlayingSpot::clear`]) and the row marks it again in the paint of the same frame, before
//! `Shell` (which paints its layer after the page). If the view is reused from the cache, the position is
//! that of the previous frame, which has not changed.
//!
//! A view can show the same track in several places (the Home: a shortcut, a cover and a row of the
//! chart): the spot keeps up to [`MAX_SPOTS`] of them, the extra ones are not painted.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, Bounds, ContentMask, IntoElement, ParentElement, Pixels, RenderOnce, Styled, Window, canvas, div,
};
use psytify_design::sprite;
use psytify_design::tokens::Rgba;
use psytify_design::tokens::color::{STATE_ON, STONE};

use super::indicators::PlayState;
use crate::pixel::sprite::{nominal_size, paint_sprite};
use crate::theme::dp;

/// Where to paint the equaliser and how.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spot {
    slot: Bounds<Pixels>,
    mask: ContentMask<Pixels>,
    state: PlayState,
}

/// The equalisers of a view at most: the limit keeps the space fixed (no allocation after the
/// primo fotogramma) e nessuna pagina ne mostra tanti.
pub const MAX_SPOTS: usize = 8;

/// The equaliser spot of a view: the playing row marks it, `Shell` paints it.
#[derive(Clone, Debug, Default)]
pub struct PlayingSpot(Rc<RefCell<Vec<Spot>>>);

impl PlayingSpot {
    pub fn new() -> Self {
        Self::default()
    }

    /// The view redraws: no spot until a visible row marks it again.
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    /// A row has painted its placeholder: the spot is added (beyond [`MAX_SPOTS`] it is dropped).
    fn mark(&self, spot: Spot) {
        let mut spots = self.0.borrow_mut();
        if spots.len() < MAX_SPOTS {
            spots.push(spot);
        }
    }

    /// I posti segnati, nell'ordine del paint.
    #[cfg(test)]
    fn marked(&self) -> Vec<Spot> {
        self.0.borrow().clone()
    }

    /// The placeholder a row puts in place of the equaliser, of its size.
    pub fn marker(&self, state: PlayState) -> Marker {
        Marker { spot: self.clone(), state }
    }

    /// Paints the annotated equaliser, if there is one, under the mask of the list it comes from.
    fn paint(&self, window: &mut Window, cx: &mut App) {
        let spots = self.0.borrow();
        for spot in spots.iter() {
            let (name, tint) = look(spot.state);
            window.with_content_mask(Some(spot.mask), |window| {
                paint_sprite(window, cx, name, tint, true, 0, spot.slot);
            });
        }
    }
}

/// The sprite and the tint of each state, as `Equalizer`: lime animated, `eq_rest` when paused, `loader`.
fn look(state: PlayState) -> (&'static str, Option<Rgba>) {
    match state {
        PlayState::Playing => (sprite::EQ, Some(STATE_ON)),
        PlayState::Paused => (sprite::EQ_REST, Some(STONE)),
        PlayState::Buffering => (sprite::LOADER, None),
    }
}

/// The placeholder: empty, as big as the sprite, and in the paint it notes where it is.
#[derive(IntoElement)]
pub struct Marker {
    spot: PlayingSpot,
    state: PlayState,
}

impl RenderOnce for Marker {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (name, _) = look(self.state);
        let (w, h) = nominal_size(cx, name).unwrap_or((0.0, 0.0));
        let Self { spot, state } = self;
        canvas(
            |_, _, _| (),
            move |slot, (), window, _| spot.mark(Spot { slot, mask: window.content_mask(), state }),
        )
        .w(dp(w))
        .h(dp(h))
        .flex_none()
    }
}

/// `Shell`'s layer over a view: it paints the equaliser annotated by its rows. It must be put after
/// the view (in the same container, `relative`), so it is painted after it and under what comes after
/// (floating panel, menus, dialogs). It has no hitbox: the mouse reaches the page.
pub fn overlay(spot: PlayingSpot) -> gpui::Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(canvas(|_, _, _| (), move |_, (), window, cx| spot.paint(window, cx)).size_full())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cleared_spot_paints_nothing_until_a_row_marks_it_again() {
        let spot = PlayingSpot::new();
        let other = spot.clone();
        let mark = Spot {
            slot: Bounds::default(),
            mask: ContentMask { bounds: Bounds::default() },
            state: PlayState::Playing,
        };
        other.mark(mark);
        assert_eq!(spot.marked(), vec![mark], "la riga e Shell vedono lo stesso posto");
        spot.clear();
        assert!(other.marked().is_empty());
    }

    #[test]
    fn a_view_keeps_a_few_spots_and_drops_the_rest() {
        let spot = PlayingSpot::new();
        let mark = |x: f32| Spot {
            slot: Bounds { origin: gpui::point(gpui::px(x), gpui::px(0.0)), size: Default::default() },
            mask: ContentMask { bounds: Bounds::default() },
            state: PlayState::Paused,
        };
        for n in 0..MAX_SPOTS + 3 {
            #[allow(clippy::cast_precision_loss)]
            spot.mark(mark(n as f32));
        }
        assert_eq!(spot.marked().len(), MAX_SPOTS);
        assert_eq!(spot.marked()[1], mark(1.0), "in the order they were painted");
    }

    #[test]
    fn each_state_has_the_equalizer_look() {
        assert_eq!(look(PlayState::Playing), (sprite::EQ, Some(STATE_ON)));
        assert_eq!(look(PlayState::Paused), (sprite::EQ_REST, Some(STONE)));
        assert_eq!(look(PlayState::Buffering).0, sprite::LOADER);
    }
}
