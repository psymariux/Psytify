//! Gallery states of the now-playing places (desktop v2 §5): the side panel on *In riproduzione* with the cover
//! and the lyrics excerpt, the same with the clip, *Testo*, the stage with lyrics, with the clip and without
//! lyrics, the dissolve frames and the like fill. Still compositions of the real components: no core, no clip
//! (the clip's well is drawn as an empty slot). The lines are invented placeholders.

use gpui::{AnyElement, IntoElement, ParentElement, SharedString, Styled, div};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    SURFACE_CANVAS, SURFACE_CHROME, SURFACE_WELL, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
};
use psytify_design::tokens::size::SIDE_PANEL_W;
use psytify_design::tokens::size::cover::{ART_PANEL, ART_SHORTCUT};
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{
    TITLE, TYPE_ARTIST, TYPE_DISPLAY, TYPE_HEADING, TYPE_LABEL, TYPE_LYRIC_FOCUS, TYPE_LYRIC_PANEL,
    TYPE_TRACK_TITLE,
};

use crate::components::badge::{Badge, BadgeKind};
use crate::components::indicators::LoadingIndicator;
use crate::components::like_button::LikeButton;
use crate::components::navigation::{Tab, Tabs};
use crate::components::now_playing::{LyricLines, MediaSwitch, UpNext, UpNextRow, veil};
use crate::components::overlay::EmptyState;
use crate::components::player_control::{PlayerControl, Transport};
use crate::components::progress::ProgressBar;
use crate::components::separator::Separator;
use crate::components::tile::Cover;
use crate::theme::{TypeStyle, cased, col, dp};

/// Invented placeholder lines (never real lyrics).
const LINES: [&str; 8] = [
    "Line one of the verse",
    "Second line, a little longer than the first",
    "A third line",
    "The fourth line comes in quietly",
    "Line five",
    "Six, and the chorus starts",
    "Seventh line of the chorus",
    "The eighth line closes it",
];

fn lines(range: std::ops::Range<usize>) -> Vec<SharedString> {
    LINES[range].iter().map(|line| SharedString::new_static(line)).collect()
}

fn next(n: usize) -> Vec<UpNext> {
    (0..n)
        .map(|ix| UpNext {
            position: ix + 1,
            title: format!("Next track {}", ix + 1).into(),
            artist: "Some Artist".into(),
            cover_url: None,
        })
        .collect()
}

/// The 16:9 well with the switch on «VIDEO» and the waiting line.
fn clip_slot(width: f32) -> AnyElement {
    div()
        .relative()
        .w(dp(width))
        .h(dp((width * 9.0 / 16.0).round()))
        .bg(col(SURFACE_WELL))
        .flex()
        .items_center()
        .justify_center()
        .child(LoadingIndicator::new().caption("Preparo il videoclip…"))
        .child(
            div().absolute().top(dp(SPACE_2)).right(dp(SPACE_2)).child(MediaSwitch::new("g-switch-v", true)),
        )
        .into_any_element()
}

fn cover_slot(side: f32, id: &'static str) -> AnyElement {
    div()
        .relative()
        .child(Cover::new(side))
        .child(div().absolute().top(dp(SPACE_2)).right(dp(SPACE_2)).child(MediaSwitch::new(id, false)))
        .into_any_element()
}

fn tabs(selected: usize, id: &'static str) -> AnyElement {
    div()
        .px(dp(SPACE_3))
        .pt(dp(SPACE_3))
        .child(Tabs::new(
            vec![
                Tab::new((id, 0usize), "In riproduzione"),
                Tab::new((id, 1usize), "Testo"),
                Tab::new((id, 2usize), "Coda"),
            ],
            selected,
        ))
        .into_any_element()
}

/// *In riproduzione*: the slot, title, artist, badge, excerpt and «Prossimo».
fn now_panel(slot: AnyElement, id: &'static str) -> AnyElement {
    div()
        .w(dp(SIDE_PANEL_W))
        .h(dp(900.0))
        .bg(col(SURFACE_CHROME))
        .flex()
        .flex_col()
        .child(tabs(0, id))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(dp(SPACE_5))
                .px(dp(SPACE_5 + 4.0))
                .pt(dp(SPACE_5 + 4.0))
                .child(slot)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(dp(SPACE_2))
                        .child(
                            div()
                                .type_role(TYPE_HEADING)
                                .text_color(col(TEXT_PRIMARY))
                                .child("A Track Title"),
                        )
                        .child(
                            div().type_role(TYPE_ARTIST).text_color(col(TEXT_SECONDARY)).child("Some Artist"),
                        )
                        .child(div().flex().child(Badge::new("Lossless", BadgeKind::Quality))),
                )
                .child(LyricLines::new(lines(1..6), Some(0), TYPE_TRACK_TITLE).gap(SPACE_2))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(dp(SPACE_2))
                        .child(
                            div()
                                .type_role(TYPE_LABEL)
                                .text_color(col(TEXT_TERTIARY))
                                .child(cased("Prossimo", TYPE_LABEL)),
                        )
                        .child(
                            div().flex().flex_col().mx(dp(-SPACE_2)).children(
                                next(3).into_iter().enumerate().map(|(ix, item)| {
                                    UpNextRow::new((id, ix + 10), item).on_click(|_, _, _| {})
                                }),
                            ),
                        ),
                ),
        )
        .into_any_element()
}

/// *Testo*: the 64 dp header and the lyrics with the current line at the upper third.
fn lyrics_panel() -> AnyElement {
    div()
        .w(dp(SIDE_PANEL_W))
        .h(dp(900.0))
        .bg(col(SURFACE_CHROME))
        .flex()
        .flex_col()
        .child(tabs(1, "g-tabs-l"))
        .child(
            div()
                .h(dp(64.0))
                .px(dp(SPACE_5))
                .flex()
                .items_center()
                .gap(dp(SPACE_4))
                .child(Cover::new(ART_SHORTCUT))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .type_role(TYPE_TRACK_TITLE)
                                .text_color(col(TEXT_PRIMARY))
                                .child("A Track Title"),
                        )
                        .child(
                            div().type_role(TYPE_ARTIST).text_color(col(TEXT_SECONDARY)).child("Some Artist"),
                        ),
                ),
        )
        .child(Separator::horizontal())
        .child(div().pt(dp(180.0)).px(dp(SPACE_4)).child(LyricLines::new(
            lines(0..8),
            Some(2),
            TYPE_LYRIC_PANEL,
        )))
        .into_any_element()
}

/// A stage at 1200 x 720 dp: left column, right column, strip.
fn stage(media: AnyElement, right: AnyElement) -> AnyElement {
    let info = div()
        .w(dp(400.0))
        .flex()
        .flex_col()
        .gap(dp(SPACE_4))
        .child(div().type_role(TYPE_DISPLAY).text_color(col(TEXT_PRIMARY)).child("A Track Title"))
        .child(div().type_role(TITLE.weight(500)).text_color(col(TEXT_SECONDARY)).child("Some Artist"))
        .child(ProgressBar::new("g-stage-seek", 0.34))
        .child(
            div()
                .flex()
                .justify_between()
                .pixel_type(true)
                .text_color(col(TEXT_TERTIARY))
                .child("1:12")
                .child("-2:26"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(LikeButton::new("g-stage-like", true))
                .child(PlayerControl::new("g-stage-pc").transport(Transport::Playing).play_size(56.0))
                .child(MediaSwitch::new("g-stage-switch", false)),
        );
    div()
        .w(dp(1200.0))
        .h(dp(720.0))
        .bg(col(SURFACE_CANVAS))
        .flex()
        .flex_col()
        .child(
            div()
                .flex_1()
                .flex()
                .child(
                    div()
                        .w(dp(540.0))
                        .pt(dp(SPACE_6))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(dp(SPACE_5))
                        .child(media)
                        .child(info),
                )
                .child(div().flex_1().pt(dp(120.0)).pr(dp(SPACE_6)).child(right)),
        )
        .child(Separator::horizontal())
        .child(
            div()
                .h(dp(72.0))
                .px(dp(SPACE_6))
                .flex()
                .items_center()
                .gap(dp(SPACE_6))
                .child(
                    div()
                        .type_role(TYPE_LABEL)
                        .text_color(col(TEXT_TERTIARY))
                        .child(cased("Prossimo", TYPE_LABEL)),
                )
                .children(next(3).into_iter().enumerate().map(|(ix, item)| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(UpNextRow::new(("g-strip", ix), item).height(40.0).on_click(|_, _, _| {}))
                })),
        )
        .into_any_element()
}

pub fn now_playing() -> AnyElement {
    let label = |text: &'static str| {
        div().type_role(TYPE_LABEL).text_color(col(TEXT_TERTIARY)).child(cased(text, TYPE_LABEL))
    };
    let dissolve = div().flex().gap(dp(SPACE_5)).children((1..=4u8).rev().map(|covered| {
        div()
            .flex()
            .flex_col()
            .gap(dp(SPACE_2))
            .child(div().relative().child(Cover::new(120.0)).child(veil(covered)))
            .child(label(["frame 1", "frame 2", "frame 3", "frame 4"][usize::from(4 - covered)]))
    }));
    div()
        .flex()
        .flex_col()
        .gap(dp(SPACE_6))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(dp(SPACE_6))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(dp(SPACE_3))
                        .child(now_panel(cover_slot(ART_PANEL, "g-switch-c"), "g-tabs-c"))
                        .child(label("panel · cover + lyrics")),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(dp(SPACE_3))
                        .child(now_panel(clip_slot(ART_PANEL), "g-tabs-v"))
                        .child(label("panel · clip in the slot")),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(dp(SPACE_3))
                        .child(lyrics_panel())
                        .child(label("panel · testo")),
                ),
        )
        .child(stage(
            Cover::new(420.0).into_any_element(),
            LyricLines::new(lines(0..8), Some(2), TYPE_LYRIC_FOCUS).gap(SPACE_5).into_any_element(),
        ))
        .child(label("stage · lyrics"))
        .child(stage(
            clip_slot(500.0),
            LyricLines::new(lines(0..8), Some(5), TYPE_LYRIC_FOCUS).gap(SPACE_5).into_any_element(),
        ))
        .child(label("stage · clip"))
        .child(stage(
            Cover::new(420.0).into_any_element(),
            div()
                .pt(dp(80.0))
                .child(EmptyState::new(
                    sprite::NASTRO_LISTEN,
                    "Nessun testo per questo brano",
                    "LRCLIB non ha il testo di questo brano.",
                ))
                .into_any_element(),
        ))
        .child(label("stage · no lyrics"))
        .child(dissolve)
        .child(label("cover ↔ clip dissolve, 160 ms"))
        .child(
            div()
                .flex()
                .gap(dp(SPACE_5))
                .child(LikeButton::new("g-like-off", false))
                .child(LikeButton::new("g-like-on", true)),
        )
        .child(label("like · off, on (liking fills in two frames with a 1 AP pop)"))
        .into_any_element()
}
