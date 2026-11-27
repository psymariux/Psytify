//! Shortcut: la voce di *Riprendi* in cima alla Home (`PSYTIFY_ART_DIRECTION.md`, Home): una riga compatta
//! con la copertina di `COVER_S` a filo e il titolo, in una griglia di due colonne come un pannello di
//! accesso rapido. A riposo ha il fondo `SURFACE_RAISED` (si legge come un bersaglio anche senza mouse),
//! in hover `SURFACE_SELECTED`; premuta la stessa tinta, focus con l'anello interno. Se è ciò che suona,
//! l'equalizzatore sta in fondo alla riga (lo dipinge `Shell`, vedi `playing_spot`).

use gpui::{App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use psytify_design::tokens::color::{PURE_WHITE, SURFACE_RAISED, SURFACE_SELECTED, TEXT_PRIMARY};
use psytify_design::tokens::size::cover::COVER_S;
use psytify_design::tokens::space::{SPACE_3, SPACE_4};
use psytify_design::tokens::type_::TYPE_TRACK_TITLE;

use super::control::{OnClick, Preview, wire, with_click};
use super::indicators::{Equalizer, PlayState};
use super::list_row::{RowArt, row_art};
use super::playing_spot::PlayingSpot;
use crate::pixel::shape::inset_ring;
use crate::theme::{TypeStyle, col, dp};

#[derive(IntoElement)]
pub struct Shortcut {
    id: ElementId,
    art: RowArt,
    title: SharedString,
    playing: Option<(PlayState, Option<PlayingSpot>)>,
    preview: Preview,
    on_click: Option<OnClick>,
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
        }
    }

    /// È ciò che suona: l'equalizzatore in fondo, dipinto da `Shell` sopra la vista in cache.
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
        let wired = wire(&self.id, false, self.preview, window, cx);
        let v = wired.visual;
        let fill = if v.pressed || v.hovered { SURFACE_SELECTED } else { SURFACE_RAISED };
        let mut row = wired
            .element
            .relative()
            .h(dp(COVER_S))
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(dp(SPACE_4))
            .pr(dp(SPACE_3))
            .bg(col(fill))
            .child(row_art(self.art, COVER_S))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .type_role(TYPE_TRACK_TITLE)
                    .text_color(col(TEXT_PRIMARY))
                    .truncate()
                    .child(self.title),
            );
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
