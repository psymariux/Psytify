//! Pieces of the now-playing places (desktop v2 §5), shared by the side panel, the stage and the gallery:
//!
//! - [`UpNextRow`]: a «Prossimo» row, 40 dp cover, title and artist (48 dp tall in the panel, 40 on the stage
//!   strip); the hover tone eases in, a click plays it;
//! - [`MediaSwitch`]: the small Cover / Video switch that sits on a media slot;
//! - [`LyricLines`]: a still run of lyric lines with the current one marked, for the gallery (the live ones
//!   come from `LyricsView`);
//! - [`veil`]: the ordered-dither dissolve over a cover (the clip's veil is painted by `VideoStage`).

use gpui::{
    AnyElement, App, Canvas, ElementId, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, canvas, div,
};
use psytify_design::tokens::color::{
    STATE_ON, STONE, SURFACE_CANVAS, SURFACE_RAISED, SURFACE_SELECTED, TEXT_PRIMARY, TEXT_SECONDARY,
    TEXT_TERTIARY,
};
use psytify_design::tokens::size::cover::ART_ROW;
use psytify_design::tokens::space::{SPACE_1, SPACE_2, SPACE_3, SPACE_4};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_LABEL, TYPE_TRACK_TITLE, TypeRole};

use super::control::{OnClick, Preview, wire, with_click};
use super::tile::Cover;
use crate::pixel::shape::paint_dither_veil;
use crate::theme::{Ap, TypeStyle, col, dp, fade, mix, to_dp};

/// Texts of a «Prossimo» row, ready made.
#[derive(Clone, Debug, PartialEq)]
pub struct UpNext {
    /// Position in the play order (for `PlayAt`).
    pub position: usize,
    pub title: SharedString,
    pub artist: SharedString,
    pub cover_url: Option<SharedString>,
}

#[derive(IntoElement)]
pub struct UpNextRow {
    id: ElementId,
    data: UpNext,
    height: f32,
    preview: Preview,
    on_click: Option<OnClick>,
}

impl UpNextRow {
    pub fn new(id: impl Into<ElementId>, data: UpNext) -> Self {
        Self { id: id.into(), data, height: 48.0, preview: Preview::Live, on_click: None }
    }

    /// The row height in dp (48 in the panel, 40 on the stage strip).
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
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

impl RenderOnce for UpNextRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let wired = wire(&self.id, self.on_click.is_none(), self.preview, window, cx);
        let v = wired.visual;
        let fill = if v.pressed { col(SURFACE_SELECTED) } else { fade(SURFACE_RAISED, v.hover_t) };
        let row = wired
            .element
            .h(dp(self.height))
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(dp(SPACE_3))
            .px(dp(SPACE_2))
            .bg(fill)
            .child(Cover::new(ART_ROW).url(self.data.cover_url))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .type_role(TYPE_TRACK_TITLE)
                            .text_color(col(if v.hovered { TEXT_PRIMARY } else { TEXT_SECONDARY }))
                            .truncate()
                            .child(self.data.title),
                    )
                    .child(
                        div()
                            .type_role(TYPE_ARTIST)
                            .text_color(col(TEXT_TERTIARY))
                            .truncate()
                            .child(self.data.artist),
                    ),
            );
        with_click(row, self.on_click)
    }
}

/// Called with the face asked for (`true`: the clip).
pub type OnToggle = std::rc::Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// The Cover / Video switch on a media slot: two caption segments, the chosen one raised.
#[derive(IntoElement)]
pub struct MediaSwitch {
    id: SharedString,
    video: bool,
    on_toggle: Option<OnToggle>,
}

impl MediaSwitch {
    pub fn new(id: impl Into<SharedString>, video: bool) -> Self {
        Self { id: id.into(), video, on_toggle: None }
    }

    /// Called with the face asked for (`true`: the clip).
    pub fn on_toggle(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(std::rc::Rc::new(handler));
        self
    }
}

impl RenderOnce for MediaSwitch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let segment = |label: &'static str, video: bool, window: &mut Window, cx: &mut App| {
            let id = ElementId::Name(format!("{}-{label}", self.id).into());
            let chosen = self.video == video;
            let wired = wire(&id, chosen, Preview::Live, window, cx);
            let fill = if chosen {
                col(SURFACE_SELECTED)
            } else {
                mix(SURFACE_CANVAS, SURFACE_RAISED, wired.visual.hover_t)
            };
            let handler = self.on_toggle.clone();
            let element = wired
                .element
                .px(dp(SPACE_2))
                .py(dp(SPACE_1))
                .bg(fill)
                .type_role(TYPE_LABEL)
                .text_color(col(if chosen { TEXT_PRIMARY } else { TEXT_TERTIARY }))
                .child(label);
            match handler {
                Some(handler) if !chosen => {
                    element.on_click(move |_, window, cx| handler(video, window, cx)).into_any_element()
                }
                _ => element.into_any_element(),
            }
        };
        let cover = segment("COVER", false, window, cx);
        let video = segment("VIDEO", true, window, cx);
        div()
            .flex()
            .bg(col(SURFACE_CANVAS))
            .border(ap.one())
            .border_color(col(SURFACE_SELECTED))
            .child(cover)
            .child(video)
    }
}

/// A still run of lyric lines (the gallery): the current one in `TEXT_PRIMARY` with the cursor, the others
/// `TEXT_SECONDARY`.
#[derive(IntoElement)]
pub struct LyricLines {
    lines: Vec<SharedString>,
    current: Option<usize>,
    playing: bool,
    role: TypeRole,
    gap: f32,
}

impl LyricLines {
    pub fn new(lines: Vec<SharedString>, current: Option<usize>, role: TypeRole) -> Self {
        Self { lines, current, playing: true, role, gap: SPACE_4 }
    }

    pub fn playing(mut self, playing: bool) -> Self {
        self.playing = playing;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }
}

impl RenderOnce for LyricLines {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let role = self.role;
        div().flex().flex_col().gap(dp(self.gap)).children(self.lines.into_iter().enumerate().map(
            |(ix, line)| {
                let current = self.current == Some(ix);
                let past = self.current.is_some_and(|now| ix < now);
                let color = if current {
                    TEXT_PRIMARY
                } else if past {
                    TEXT_TERTIARY
                } else {
                    TEXT_SECONDARY
                };
                let cursor: Option<AnyElement> = current.then(|| {
                    div()
                        .absolute()
                        .left_0()
                        .top(dp((role.line_height - to_dp(ap.n(3))) / 2.0))
                        .w(ap.n(2))
                        .h(ap.n(3))
                        .bg(col(if self.playing { STATE_ON } else { STONE }))
                        .into_any_element()
                });
                div()
                    .relative()
                    .pl(dp(SPACE_4))
                    .children(cursor)
                    .child(div().type_role(role).text_color(col(color)).child(line))
            },
        ))
    }
}

/// The ordered-dither veil over its parent: `covered` cells of every 4 in `SURFACE_CANVAS` (a dissolve step).
pub fn veil(covered: u8) -> Canvas<()> {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            paint_dither_veil(window, bounds, Ap::of(window), covered, col(SURFACE_CANVAS))
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
