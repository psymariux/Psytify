//! QueueRow: riga di 48 dp del pannello della coda. Posizione (o equalizzatore se è il brano in
//! corso) | copertina | titolo e artista | durata; in hover la durata lascia il posto a «Rimuovi».
//! Stati come TrackRow: rest, hover, premuto, focus (anello interno). Altezza fissa, quindi va bene
//! per `uniform_list`: una coda lunga costruisce solo le righe visibili.

use gpui::{App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    PURE_WHITE, SURFACE_RAISED, SURFACE_SELECTED, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
};
use psytify_design::tokens::size::{TRACK_ROW_H, cover::COVER_XS};
use psytify_design::tokens::space::{SPACE_2, SPACE_3};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_NUMERIC, TYPE_TRACK_TITLE};

use super::control::{OnClick, Preview, centered, wire, with_click};
use super::icon_button::IconButton;
use super::indicators::{Equalizer, PlayState};
use super::playing_spot::PlayingSpot;
use super::tile::Cover;
use crate::pixel::shape::inset_ring;
use crate::theme::{TypeStyle, col, dp};

/// Larghezza della colonna della posizione.
const INDEX_W: f32 = 24.0;

/// Stringhe già pronte: la view non formatta.
#[derive(Clone)]
pub struct QueueRowData {
    /// Posizione nell'ordine di ascolto, da 1.
    pub index: u32,
    pub title: SharedString,
    pub artist: SharedString,
    pub duration: SharedString,
    pub cover_url: Option<SharedString>,
}

#[derive(IntoElement)]
pub struct QueueRow {
    id: ElementId,
    data: QueueRowData,
    current: Option<PlayState>,
    /// Dove `Shell` dipinge l'equalizzatore al posto della riga (vedi `playing_spot`).
    spot: Option<PlayingSpot>,
    preview: Preview,
    on_play: Option<OnClick>,
    on_remove: Option<OnClick>,
}

impl QueueRow {
    pub fn new(id: impl Into<ElementId>, data: QueueRowData) -> Self {
        Self {
            id: id.into(),
            data,
            current: None,
            spot: None,
            preview: Preview::Live,
            on_play: None,
            on_remove: None,
        }
    }

    /// Il brano della riga è quello in corso (suona, è in pausa o si sta caricando).
    pub fn current(mut self, state: PlayState) -> Self {
        self.current = Some(state);
        self
    }

    /// Come [`QueueRow::current`], ma l'equalizzatore lo dipinge `Shell` sopra il pannello (in cache).
    pub fn current_at(mut self, state: PlayState, spot: &PlayingSpot) -> Self {
        self.current = Some(state);
        self.spot = Some(spot.clone());
        self
    }

    pub fn preview(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    pub fn on_play(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_play = Some(std::rc::Rc::new(handler));
        self
    }

    pub fn on_remove(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_remove = Some(std::rc::Rc::new(handler));
        self
    }
}

impl RenderOnce for QueueRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let wired = wire(&self.id, false, self.preview, window, cx);
        let v = wired.visual;
        let fill = if v.pressed {
            Some(SURFACE_SELECTED)
        } else if v.hovered {
            Some(SURFACE_RAISED)
        } else {
            None
        };
        let slot: gpui::AnyElement = match self.current {
            Some(state) => match &self.spot {
                Some(spot) => spot.marker(state).into_any_element(),
                None => Equalizer::new(state).into_any_element(),
            },
            None => div()
                .type_role(TYPE_NUMERIC)
                .text_color(col(TEXT_TERTIARY))
                .child(SharedString::from(self.data.index.to_string()))
                .into_any_element(),
        };
        let id = format!("{:?}", self.id);
        let tail: gpui::AnyElement = match (v.hovered, self.on_remove) {
            (true, Some(handler)) => {
                IconButton::new(SharedString::from(format!("{id}-remove")), sprite::ICON_CLOSE)
                    .tooltip("Rimuovi dalla coda", None)
                    .on_click(move |event, window, cx| {
                        // Togliere una voce non deve farla partire.
                        cx.stop_propagation();
                        handler(event, window, cx);
                    })
                    .into_any_element()
            }
            _ => div()
                .type_role(TYPE_NUMERIC)
                .text_color(col(TEXT_TERTIARY))
                .child(self.data.duration)
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
            .px(dp(SPACE_3))
            .child(centered(div().w(dp(INDEX_W)).flex_none()).child(slot))
            .child(Cover::new(COVER_XS).url(self.data.cover_url))
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
                            .child(self.data.title),
                    )
                    .child(
                        div()
                            .type_role(TYPE_ARTIST)
                            .text_color(col(TEXT_SECONDARY))
                            .truncate()
                            .child(self.data.artist),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .min_w(dp(32.0))
                    .h(dp(32.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .pr(dp(SPACE_2))
                    .child(tail),
            )
            .child(inset_ring(col(PURE_WHITE), wired.focus.clone(), v.force_ring));
        if let Some(color) = fill {
            row = row.bg(col(color));
        }
        with_click(row, self.on_play)
    }
}
