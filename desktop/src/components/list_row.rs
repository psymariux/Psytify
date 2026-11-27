//! ListRow: riga di 48 dp (`TRACK_ROW_H`, pronta per `uniform_list`) per ciò che non è una riga di brano
//! del catalogo: le voci della Libreria (playlist, brani piaciuti, album salvati, artisti seguiti), i
//! download in corso e i brani sul dispositivo. Griglia: indice facoltativo | arte di `COVER_XS` |
//! titolo sopra dettaglio | barra a dieci passi facoltativa | azioni.
//!
//! Stati come TrackRow: rest, hover (fill `SURFACE_RAISED`; l'indice diventa `icon_play` se la riga
//! suona), premuto (`SURFACE_SELECTED`), focus (anello interno), in riproduzione (l'equalizzatore al posto
//! dell'indice, lime solo mentre suona). Il dettaglio può essere un errore (`TEXT_ERROR`, file mancante o
//! download non riuscito). Le azioni sono di due tipi: quelle che si vedono sempre (annulla un download) e
//! quelle che compaiono al passaggio del mouse, con lo spazio tenuto (rinomina, rimuovi).

use gpui::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window, div,
};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    GRAPHITE, ICON_ACTIVE, PURE_WHITE, SURFACE_RAISED, SURFACE_SELECTED, TEXT_ERROR, TEXT_PRIMARY,
    TEXT_SECONDARY, TEXT_TERTIARY,
};
use psytify_design::tokens::size::{ICON_BUTTON, TRACK_INDEX_W, TRACK_ROW_H, cover::COVER_XS};
use psytify_design::tokens::space::{SPACE_3, SPACE_6};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_NUMERIC, TYPE_TRACK_TITLE};

use super::badge::{Badge, BadgeKind};
use super::control::{OnClick, Preview, centered, wire, with_click};
use super::icon_button::IconButton;
use super::indicators::{DownloadSteps, Equalizer, PlayState};
use super::playing_spot::PlayingSpot;
use super::tile::Cover;
use crate::pixel::PixelSprite;
use crate::pixel::shape::inset_ring;
use crate::theme::{TypeStyle, col, dp};

/// L'arte a sinistra del testo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowArt {
    /// Una copertina (il segnaposto finché non arriva o se non c'è).
    Cover(Option<SharedString>),
    /// Una foto d'artista, nel cerchio.
    Round(Option<SharedString>),
    /// Uno sprite su campo `GRAPHITE` (il cuore di «Brani che ti piacciono»), tinto `ICON_ACTIVE`.
    Sprite(&'static str),
    /// Nessuna arte (una classifica di nomi, dove c'è già l'indice).
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
    /// Un'etichetta dopo il titolo («Nuovo», «Album»).
    badge: Option<(SharedString, BadgeKind)>,
    steps: Option<f32>,
    playing: Option<PlayState>,
    /// Dove `Shell` dipinge l'equalizzatore al posto della riga (vedi `playing_spot`).
    spot: Option<PlayingSpot>,
    selected: bool,
    actions: Vec<IconButton>,
    hover_actions: Vec<IconButton>,
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
            preview: Preview::Live,
            on_click: None,
        }
    }

    /// La colonna del numero (da 1), come nelle liste di brani.
    pub fn index(mut self, index: u32) -> Self {
        self.index = Some(index);
        self
    }

    /// La seconda riga, già pronta.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = detail.into();
        self
    }

    /// La seconda riga dice un problema: `TEXT_ERROR`.
    pub fn detail_error(mut self, error: bool) -> Self {
        self.detail_error = error;
        self
    }

    /// Un'etichetta subito dopo il titolo: di conteggio («Nuovo» in «Novità») o neutra (il tipo di
    /// un'uscita nel registro della Home).
    pub fn badge(mut self, text: impl Into<SharedString>, kind: BadgeKind) -> Self {
        self.badge = Some((text.into(), kind));
        self
    }

    /// La barra a dieci passi di un download in corso (0..=1).
    pub fn steps(mut self, progress: f32) -> Self {
        self.steps = Some(progress);
        self
    }

    /// La riga è il brano in corso: l'equalizzatore al posto dell'indice.
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

    /// La voce è la pagina aperta: fill `SURFACE_SELECTED`.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Un'azione sempre visibile (annulla, riprova).
    pub fn action(mut self, button: IconButton) -> Self {
        self.actions.push(button);
        self
    }

    /// Un'azione che compare solo al passaggio del mouse; lo spazio resta.
    pub fn hover_action(mut self, button: IconButton) -> Self {
        self.hover_actions.push(button);
        self
    }

    pub fn preview(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    /// Il gesto della riga: chi la usa decide se basta un clic (aprire) o serve il doppio (suonare).
    pub fn on_click(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }
}

/// L'arte di una riga, di lato `side` (`COVER_XS` nelle righe, `COVER_S` nelle scorciatoie della Home).
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

/// Un'azione che non deve far partire anche il gesto della riga.
fn stop_propagation(button: IconButton) -> AnyElement {
    div()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(button)
        .into_any_element()
}

impl RenderOnce for ListRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let wired = wire(&self.id, false, self.preview, window, cx);
        let v = wired.visual;
        let fill = if v.pressed || self.selected {
            Some(SURFACE_SELECTED)
        } else if v.hovered {
            Some(SURFACE_RAISED)
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
            .pl(dp(if numbered { SPACE_3 } else { SPACE_6 }))
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
            row = row.child(row_art(self.art, COVER_XS));
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
        let hovered = v.hovered;
        let hover_actions = self.hover_actions.into_iter().map(|button| {
            if hovered { stop_propagation(button) } else { div().size(dp(ICON_BUTTON)).into_any_element() }
        });
        let actions = self.actions.into_iter().map(stop_propagation);
        row = row
            .child(div().flex().flex_none().items_center().children(actions).children(hover_actions))
            .child(inset_ring(col(PURE_WHITE), wired.focus.clone(), v.force_ring));
        if let Some(color) = fill {
            row = row.bg(col(color));
        }
        with_click(row, self.on_click)
    }
}
