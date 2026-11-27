//! Il posto dell'equalizzatore del brano in corso in una lista (BACKLOG 23).
//!
//! Un equalizzatore animato dentro una vista `.cached()` la invalida a ogni scatto (`pixel::anim`
//! notifica la vista che l'ha dipinto e GPUI segna sporchi anche i suoi antenati, `Window::mark_view_dirty`):
//! la pagina rifaceva la lista intera dieci volte al secondo solo per cambiare dodici pixel. Qui la riga
//! non dipinge l'equalizzatore: mette al suo posto un segnaposto della stessa misura ([`PlayingSpot::marker`])
//! che, quando la vista dipinge, annota dove sta e sotto quale maschera (la lista che scorre lo taglia).
//! Lo dipinge `Shell` sopra la pagina ([`overlay`]): il timer dello sprite invalida così solo `Shell`,
//! che non è in cache, e la pagina si riusa com'era (`AnyView::cached` riusa prepaint e paint finché la
//! vista non è sporca e la sua misura non cambia).
//!
//! Il posto vale finché la vista non si ridisegna: la vista lo cancella all'inizio di `render`
//! ([`PlayingSpot::clear`]) e la riga lo segna di nuovo nel paint dello stesso fotogramma, prima di
//! `Shell` (che dipinge il suo strato dopo la pagina). Se la vista è riusata dalla cache, la posizione è
//! quella del fotogramma prima, che non è cambiata.
//!
//! Una vista può mostrare lo stesso brano in più posti (la Home: una scorciatoia, una copertina e una riga
//! della classifica): il posto ne tiene fino a [`MAX_SPOTS`], quelli in più non si dipingono.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, Bounds, ContentMask, IntoElement, ParentElement, Pixels, RenderOnce, Styled, Window, canvas, div, px,
};
use psytify_design::sprite;
use psytify_design::tokens::Rgba;
use psytify_design::tokens::color::{STATE_ON, STONE};

use super::indicators::PlayState;
use crate::pixel::sprite::{nominal_size, paint_sprite};

/// Dove dipingere l'equalizzatore e come.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spot {
    slot: Bounds<Pixels>,
    mask: ContentMask<Pixels>,
    state: PlayState,
}

/// Gli equalizzatori di una vista al massimo: il limite tiene fisso lo spazio (nessuna allocazione dopo il
/// primo fotogramma) e nessuna pagina ne mostra tanti.
pub const MAX_SPOTS: usize = 8;

/// Il posto dell'equalizzatore di una vista: lo segna la riga che suona, lo dipinge `Shell`.
#[derive(Clone, Debug, Default)]
pub struct PlayingSpot(Rc<RefCell<Vec<Spot>>>);

impl PlayingSpot {
    pub fn new() -> Self {
        Self::default()
    }

    /// La vista si ridisegna: nessun posto finché una riga visibile non lo segna di nuovo.
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    /// Una riga ha dipinto il suo segnaposto: il posto si aggiunge (oltre [`MAX_SPOTS`] si scarta).
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

    /// Il segnaposto che una riga mette al posto dell'equalizzatore, della sua misura.
    pub fn marker(&self, state: PlayState) -> Marker {
        Marker { spot: self.clone(), state }
    }

    /// Dipinge l'equalizzatore annotato, se c'è, sotto la maschera della lista da cui viene.
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

/// Lo sprite e la tinta di ogni stato, come `Equalizer`: lime animato, `eq_rest` in pausa, `loader`.
fn look(state: PlayState) -> (&'static str, Option<Rgba>) {
    match state {
        PlayState::Playing => (sprite::EQ, Some(STATE_ON)),
        PlayState::Paused => (sprite::EQ_REST, Some(STONE)),
        PlayState::Buffering => (sprite::LOADER, None),
    }
}

/// Il segnaposto: vuoto, grande quanto lo sprite, e nel paint annota dove sta.
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
        .w(px(w))
        .h(px(h))
        .flex_none()
    }
}

/// Lo strato di `Shell` sopra una vista: dipinge l'equalizzatore annotato dalle sue righe. Va messo dopo
/// la vista (nello stesso contenitore, `relative`), così si dipinge dopo di lei e sotto ciò che viene dopo
/// (pannello che galleggia, menu, dialoghi). Non ha hitbox: il mouse arriva alla pagina.
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
            slot: Bounds { origin: gpui::point(px(x), px(0.0)), size: Default::default() },
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
