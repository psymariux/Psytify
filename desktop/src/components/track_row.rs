//! TrackRow: a 48 dp row (`uniform_list`-ready: fixed height, no child measuring) with index | 40 dp cover
//! | title and artist | album | actions | duration. States: rest, hover (the tone eases in, the index turns
//! into the play glyph, the heart and «Altro» slide in 2 AP from the right), pressed, focus (inner ring),
//! disabled (unavailable), selected, playing.

use gpui::{
    App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div,
};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    ICON_ACTIVE, PURE_WHITE, SURFACE_RAISED, SURFACE_SELECTED, TEXT_DISABLED, TEXT_PRIMARY, TEXT_SECONDARY,
    TEXT_TERTIARY,
};
use psytify_design::tokens::size::{TRACK_INDEX_W, TRACK_ROW_H, cover::ART_ROW};
use psytify_design::tokens::space::{SPACE_3, SPACE_4};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_NUMERIC, TYPE_TRACK_TITLE};

use super::control::{OnClick, Preview, centered, wire, with_click};
use super::icon_button::{IconButton, OnStyle};
use super::indicators::{DownloadIndicator, DownloadState, Equalizer, PlayState};
use super::playing_spot::PlayingSpot;
use super::tile::Cover;
use crate::motion::ap_round;
use crate::pixel::PixelSprite;
use crate::pixel::shape::inset_ring;
use crate::theme::{Ap, TypeStyle, col, dp, fade};

/// Data of a row: strings already ready (the view does not format, see AGENTS.md).
#[derive(Clone)]
pub struct TrackData {
    pub index: u32,
    pub title: SharedString,
    pub artist: SharedString,
    pub album: SharedString,
    pub duration: SharedString,
    pub liked: bool,
    pub download: DownloadState,
    pub available: bool,
    /// Cover address: the store asks the core for it and draws it when it arrives.
    pub cover_url: Option<SharedString>,
}

#[derive(IntoElement)]
pub struct TrackRow {
    id: ElementId,
    data: TrackData,
    show_album: bool,
    selected: bool,
    playing: Option<PlayState>,
    /// Where `Shell` paints the equaliser in place of the row (see `playing_spot`).
    spot: Option<PlayingSpot>,
    preview: Preview,
    show_more: bool,
    on_click: Option<OnClick>,
    on_like: Option<OnClick>,
    on_download: Option<OnClick>,
    on_more: Option<OnClick>,
    on_artist: Option<OnClick>,
    on_album: Option<OnClick>,
}

/// A text that opens (the artist, the album): as wide as the text, underlined on mouse hover,
/// and the click does not reach the row.
fn link(id: SharedString, text: gpui::Div, handler: Option<OnClick>) -> gpui::AnyElement {
    match handler {
        Some(handler) => div()
            .flex()
            .min_w_0()
            .child(
                text.id(id)
                    .cursor_pointer()
                    .hover(|style| style.underline())
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |event, window, cx| {
                        cx.stop_propagation();
                        handler(event, window, cx);
                    }),
            )
            .into_any_element(),
        None => text.into_any_element(),
    }
}

impl TrackRow {
    pub fn new(id: impl Into<ElementId>, data: TrackData) -> Self {
        Self {
            id: id.into(),
            data,
            show_album: true,
            selected: false,
            playing: None,
            spot: None,
            preview: Preview::Live,
            show_more: true,
            on_click: None,
            on_like: None,
            on_download: None,
            on_more: None,
            on_artist: None,
            on_album: None,
        }
    }

    /// Il nome dell'artista apre la sua pagina.
    pub fn on_artist(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_artist = Some(std::rc::Rc::new(handler));
        self
    }

    /// The album title (in its column) opens its page.
    pub fn on_album(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_album = Some(std::rc::Rc::new(handler));
        self
    }

    /// «Altro»: opens the track's menu (whoever shows it anchors it to `ClickEvent::position`).
    pub fn on_more(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_more = Some(std::rc::Rc::new(handler));
        self
    }

    /// The «Altro» button (track menu): it is removed where the menu is not there yet.
    pub fn show_more(mut self, show: bool) -> Self {
        self.show_more = show;
        self
    }

    /// Cuore: mette o toglie «mi piace».
    pub fn on_like(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_like = Some(std::rc::Rc::new(handler));
        self
    }

    /// Download: downloads the track (only if its state allows it: available or failed).
    pub fn on_download(
        mut self,
        handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_download = Some(std::rc::Rc::new(handler));
        self
    }

    /// Colonna album (visibile da 1100 dp in su).
    pub fn show_album(mut self, show: bool) -> Self {
        self.show_album = show;
        self
    }

    /// Multiple selection: the index becomes a tick.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// The row's track is the one playing/paused/loading.
    pub fn playing(mut self, state: PlayState) -> Self {
        self.playing = Some(state);
        self
    }

    /// Come [`TrackRow::playing`], ma l'equalizzatore lo dipinge `Shell` sopra la vista: la riga segna
    /// only the place. For rows inside a `.cached()` view, so that it does not redo at every frame.
    pub fn playing_at(mut self, state: PlayState, spot: &PlayingSpot) -> Self {
        self.playing = Some(state);
        self.spot = Some(spot.clone());
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

impl RenderOnce for TrackRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let available = self.data.available;
        let ap = Ap::of(window);
        let wired = wire(&self.id, !available, self.preview, window, cx);
        let v = wired.visual;
        // The hover tone eases in; press and selection are solid at once.
        let fill = if v.pressed || self.selected {
            Some(col(SURFACE_SELECTED))
        } else if v.hover_t > 0.0 && available {
            Some(fade(SURFACE_RAISED, v.hover_t))
        } else {
            None
        };
        let hot = v.hovered && available;
        // The heart and «Altro» slide in 2 AP from the right with the hover, and fade.
        let shown = v.hover_t > 0.0 && available;
        let slide = |element: gpui::AnyElement| {
            div()
                .relative()
                .left(ap.n(ap_round((1.0 - v.hover_t) * 2.0)))
                .opacity(v.hover_t)
                .child(element)
                .into_any_element()
        };
        let text_primary = if available { TEXT_PRIMARY } else { TEXT_DISABLED };
        let text_secondary = if available { TEXT_SECONDARY } else { TEXT_DISABLED };

        // Colonna indice: numero, play (hover), spunta (selezione), equalizzatore (riproduzione).
        let index: gpui::AnyElement = if self.selected {
            PixelSprite::new(sprite::ICON_CHECK).tint(ICON_ACTIVE).into_any_element()
        } else if let Some(state) = self.playing {
            match &self.spot {
                Some(spot) => spot.marker(state).into_any_element(),
                None => Equalizer::new(state).into_any_element(),
            }
        } else if hot {
            PixelSprite::new(sprite::ICON_PLAY).tint(ICON_ACTIVE).into_any_element()
        } else {
            div()
                .type_role(TYPE_NUMERIC)
                .text_color(col(TEXT_TERTIARY))
                .child(SharedString::from(self.data.index.to_string()))
                .into_any_element()
        };

        let id = format!("{:?}", self.id);
        let heart = if self.data.liked || shown {
            let mut heart = IconButton::new(SharedString::from(format!("{id}-heart")), sprite::ICON_HEART)
                .icon_on(sprite::ICON_HEART_FILL)
                .on_style(OnStyle::Active)
                .selected(self.data.liked)
                .size(32.0)
                .tooltip(
                    if self.data.liked { "Rimuovi dai preferiti" } else { "Aggiungi ai preferiti" },
                    None,
                );
            if let Some(handler) = self.on_like.clone() {
                // The heart must not also start the row's click.
                heart = heart.on_click(move |event, window, cx| {
                    cx.stop_propagation();
                    handler(event, window, cx);
                });
            }
            if self.data.liked { heart.into_any_element() } else { slide(heart.into_any_element()) }
        } else {
            div().size(dp(32.0)).into_any_element()
        };
        let more = if shown && self.show_more {
            let mut more = IconButton::new(SharedString::from(format!("{id}-more")), sprite::ICON_MORE)
                .tooltip("Altro", None);
            if let Some(handler) = self.on_more.clone() {
                more = more.on_click(move |event, window, cx| {
                    cx.stop_propagation();
                    handler(event, window, cx);
                });
            }
            slide(more.into_any_element())
        } else {
            div().size(dp(32.0)).into_any_element()
        };
        // The download shows on hover; a state (queued, running, done, failed, lost file) is seen
        // always. If it cannot be downloaded, the space stays.
        let download_visible = hot || !matches!(self.data.download, DownloadState::Available);
        let download = if download_visible {
            let clickable = matches!(
                self.data.download,
                DownloadState::Available | DownloadState::Error | DownloadState::Lost
            );
            let indicator =
                DownloadIndicator::new(SharedString::from(format!("{id}-dl")), self.data.download);
            let slot = div().w(dp(32.0)).flex().justify_center();
            match self.on_download.clone().filter(|_| clickable) {
                Some(handler) => slot
                    .child(
                        div()
                            .id(SharedString::from(format!("{id}-dl-hit")))
                            .cursor_pointer()
                            .on_click(move |event, window, cx| {
                                cx.stop_propagation();
                                handler(event, window, cx);
                            })
                            .child(indicator),
                    )
                    .into_any_element(),
                None => slot.child(indicator).into_any_element(),
            }
        } else {
            div().w(dp(32.0)).into_any_element()
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
            .child(Cover::new(ART_ROW).url(self.data.cover_url.clone()).unavailable(!available))
            // Title and album share the row 3 to 2, like table columns.
            .child(
                div()
                    .flex_1()
                    .flex_grow(3.0)
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .type_role(TYPE_TRACK_TITLE)
                            .text_color(col(text_primary))
                            .truncate()
                            .child(self.data.title),
                    )
                    .child(link(
                        SharedString::from(format!("{id}-artist")),
                        div().type_role(TYPE_ARTIST).text_color(col(text_secondary)).truncate().child(
                            if available { self.data.artist } else { SharedString::from("Non disponibile") },
                        ),
                        self.on_artist.filter(|_| available),
                    )),
            );
        if self.show_album {
            row = row.child(
                div().flex_1().flex_grow(2.0).min_w_0().child(link(
                    SharedString::from(format!("{id}-album")),
                    div()
                        .type_role(TYPE_ARTIST)
                        .text_color(col(text_secondary))
                        .truncate()
                        .child(self.data.album),
                    self.on_album.filter(|_| available),
                )),
            );
        }
        row = row
            .child(div().flex().items_center().flex_none().child(heart).child(download).child(more))
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
        with_click(row, if available { self.on_click } else { None })
    }
}

/// Deterministic sample row for the gallery and the load tests (no real data).
pub fn sample(ix: usize) -> TrackData {
    const TITLES: [&str; 8] = [
        "Neon Harbour",
        "Paper Lanterns",
        "Static Bloom",
        "Low Tide Radio",
        "Glass Orchard",
        "Night Ferry",
        "Copper Skies",
        "Slow Machines",
    ];
    const ARTISTS: [&str; 5] = ["Ada Vale", "The Quiet Engines", "Mio Kanda", "Orbit Choir", "Sol Ferro"];
    let secs = 120 + (ix * 37) % 240;
    TrackData {
        index: (ix + 1) as u32,
        title: SharedString::from(format!("{} {}", TITLES[ix % TITLES.len()], ix / TITLES.len() + 1)),
        artist: SharedString::from(ARTISTS[ix % ARTISTS.len()]),
        album: SharedString::from(format!("Album {}", ix / 12 + 1)),
        duration: SharedString::from(format!("{}:{:02}", secs / 60, secs % 60)),
        liked: ix.is_multiple_of(7),
        download: match ix % 9 {
            0 => DownloadState::Done,
            1 => DownloadState::NotDownloadable,
            _ => DownloadState::Available,
        },
        available: ix % 13 != 5,
        cover_url: None,
    }
}
