//! SourceRow: riga di 48 dp di un risultato che non è ancora nel catalogo del server (il catalogo
//! ufficiale, Internet Archive, le fonti esterne). Stessa griglia di `TrackRow` (indice | copertina |
//! titolo e dettaglio | azioni | durata), così le due righe stanno nella stessa lista con le durate
//! allineate. Un risultato non è un brano finché il server non lo importa: niente cuore né download, e
//! mentre il core lo importa (`ExternalView::importing`) l'indice diventa il loader e la coda della
//! riga dice «Apro…»; la riga non risponde finché dura (il core ignora comunque un secondo play).
//! Stati come TrackRow: rest, hover, premuto, focus (anello interno).

use gpui::{App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    ICON_ACTIVE, PURE_WHITE, SURFACE_RAISED, SURFACE_SELECTED, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
};
use psytify_design::tokens::size::{TRACK_INDEX_W, TRACK_ROW_H, cover::COVER_XS};
use psytify_design::tokens::space::{SPACE_3, SPACE_4};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_META, TYPE_NUMERIC, TYPE_TRACK_TITLE};

use super::control::{OnClick, Preview, centered, wire, with_click};
use super::indicators::{Equalizer, PlayState};
use super::tile::Cover;
use crate::pixel::PixelSprite;
use crate::pixel::shape::inset_ring;
use crate::theme::{TypeStyle, col, dp, fade};

/// Larghezza della coda della riga: quanto occupano cuore, download e «Altro» in `TrackRow` (3 x 32 dp).
const TAIL_W: f32 = 96.0;

/// Stringhe già pronte: la view non formatta.
#[derive(Clone)]
pub struct SourceData {
    /// Posizione nella sezione, da 1.
    pub index: u32,
    pub title: SharedString,
    /// Artista (e fonte, se non è il catalogo ufficiale), creatore e licenza per Archive.
    pub detail: SharedString,
    /// Vuota se la fonte non dichiara la durata.
    pub duration: SharedString,
    pub cover_url: Option<SharedString>,
}

#[derive(IntoElement)]
pub struct SourceRow {
    id: ElementId,
    data: SourceData,
    busy: bool,
    preview: Preview,
    on_click: Option<OnClick>,
}

impl SourceRow {
    pub fn new(id: impl Into<ElementId>, data: SourceData) -> Self {
        Self { id: id.into(), data, busy: false, preview: Preview::Live, on_click: None }
    }

    /// Il server sta importando questo risultato: loader al posto dell'indice, «Apro…», nessun click.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub fn preview(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    /// Doppio clic o Invio (come `TrackRow`): importa e suona.
    pub fn on_click(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }
}

impl RenderOnce for SourceRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let wired = wire(&self.id, false, self.preview, window, cx);
        let v = wired.visual;
        // The hover tone eases in, as on `TrackRow`; the press is solid at once.
        let fill = if v.pressed && !self.busy {
            Some(col(SURFACE_SELECTED))
        } else if v.hover_t > 0.0 {
            Some(fade(SURFACE_RAISED, v.hover_t))
        } else {
            None
        };
        // Colonna indice: numero, play (hover), loader (importazione in corso).
        let index: gpui::AnyElement = if self.busy {
            Equalizer::new(PlayState::Buffering).into_any_element()
        } else if v.hovered {
            PixelSprite::new(sprite::ICON_PLAY).tint(ICON_ACTIVE).into_any_element()
        } else {
            div()
                .type_role(TYPE_NUMERIC)
                .text_color(col(TEXT_TERTIARY))
                .child(SharedString::from(self.data.index.to_string()))
                .into_any_element()
        };
        let tail = div().w(dp(TAIL_W)).flex_none().flex().items_center().justify_end();
        let tail = if self.busy {
            tail.child(div().type_role(TYPE_META).text_color(col(TEXT_TERTIARY)).child("Apro…"))
        } else {
            tail
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
            .child(centered(div().w(dp(TRACK_INDEX_W)).flex_none()).child(index))
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
                            .child(self.data.detail),
                    ),
            )
            .child(tail)
            .child(
                div()
                    .w(dp(48.0))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .pr(dp(SPACE_4 - 8.0))
                    .type_role(TYPE_NUMERIC)
                    .text_color(col(TEXT_TERTIARY))
                    .child(self.data.duration),
            )
            .child(inset_ring(col(PURE_WHITE), wired.focus.clone(), v.force_ring));
        if let Some(color) = fill {
            row = row.bg(color);
        }
        with_click(row, if self.busy { None } else { self.on_click })
    }
}
