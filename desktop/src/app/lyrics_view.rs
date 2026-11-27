//! The playing track's lyrics: one engine for three places. `Look::Panel` is the side panel's *Testo* tab (a
//! 64 dp header with the 56 dp cover, title and artist, the lyrics at `LYRIC_PANEL`), `Look::Stage` the right
//! column of the stage (`LYRIC_STAGE`), `Look::Excerpt` the live excerpt of *In riproduzione* (the current line
//! and the next four; a click asks for *Testo*, `LyricsEvent::Open`).
//!
//! Look: the current line is `TEXT_PRIMARY` with a 2 x 3 AP cursor on its left (lime while playing, `STONE`
//! paused), the coming lines `TEXT_SECONDARY`, the past ones `TEXT_TERTIARY`; no growing line, no word
//! highlight. An instrumental pause is three 2 x 2 AP dots that light one at a time. A click on a verse seeks
//! there. Plain (untimed) lyrics scroll by hand.
//!
//! Synced lyrics are not a scrolling list: each verse sits where the current line puts it (a `Tween` measured in
//! verses), with the current line at the upper third; before the first verse the text starts there too (§5). The text glides to a new line in `MOTION_LYRICS` (280 ms, ease-out) at 24 fps
//! at most, on physical pixels. Verse heights are measured once per text and width; only the verses inside the
//! frame are built.
//!
//! Timers: one **while visible and playing**, which sleeps until the next change (the next verse or the next
//! pause dot, from `clock.at(now)`) and redraws only if the line or the dots changed. Paused, hidden or without
//! synced lyrics there is no timer. The glide has its own 42 ms timer that lives only for the 280 ms of the
//! move (never in the excerpt, which cuts).

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, TextRun,
    Window, canvas, div,
};
use psytify_core::command::PlaybackCmd;
use psytify_core::player::PlaybackState;
use psytify_core::player::lyrics::{DOTS, Text, current, dots_lit, is_pause, next_change};
use psytify_core::snapshot::{LyricsState, PlaybackView};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    STATE_ON, STONE, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY, WARM_WHITE,
};
use psytify_design::tokens::motion::{EASE_OUT_BEZIER, MOTION_LYRICS};
use psytify_design::tokens::size::cover::ART_SHORTCUT;
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{
    TYPE_ARTIST, TYPE_BODY, TYPE_LYRIC_FOCUS, TYPE_LYRIC_PANEL, TYPE_META, TYPE_TRACK_TITLE, TypeRole,
};

use crate::components::button::Button;
use crate::components::indicators::Skeleton;
use crate::components::overlay::EmptyState;
use crate::components::separator::Separator;
use crate::components::tile::Cover;
use crate::core_model::{Changed, CoreModel};
use crate::pixel::SpriteStore;
use crate::theme::{Ap, TypeStyle, col, dp, to_dp};

/// Where the lyrics are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// The side panel's *Testo* tab.
    Panel,
    /// The live excerpt in *In riproduzione*.
    Excerpt,
    /// The stage's right column.
    Stage,
}

impl Look {
    /// The lyric pair: 20 / 28 in the panel, 32 / 40 on the stage; the excerpt reads at the Body size.
    fn role(self) -> TypeRole {
        match self {
            Look::Panel => TYPE_LYRIC_PANEL,
            Look::Excerpt => TYPE_TRACK_TITLE,
            Look::Stage => TYPE_LYRIC_FOCUS,
        }
    }

    /// Space between two verses.
    fn gap(self) -> f32 {
        match self {
            Look::Panel => SPACE_4,
            Look::Excerpt => SPACE_2,
            Look::Stage => SPACE_5,
        }
    }

    /// Where the current line sits, from the top, as a fraction of the height: the upper third.
    fn anchor(self) -> f32 {
        1.0 / 3.0
    }
}

/// Lines in the excerpt: the current one and the next four.
pub const EXCERPT_LINES: usize = 5;

/// What the view asks of its owner.
pub enum LyricsEvent {
    /// The excerpt was clicked: open *Testo*.
    Open,
}

impl EventEmitter<LyricsEvent> for LyricsView {}

/// The lines of the excerpt for the current line `current` out of `count`: from the current line (the first
/// before it starts), at most `EXCERPT_LINES`. Pure.
pub fn excerpt(current: Option<usize>, count: usize) -> std::ops::Range<usize> {
    let start = current.unwrap_or(0).min(count);
    start..(start + EXCERPT_LINES).min(count)
}

/// One frame of the glide: 24 per second at most.
const FRAME: Duration = Duration::from_millis(42);
/// The timer wakes just after the change, never just before.
const WAKE_MARGIN: Duration = Duration::from_millis(2);
/// Space left of the verses: the cursor (2 AP) and the gap to the text.
const GUTTER: f32 = SPACE_4;
/// Widths of the placeholder verses while the text loads, as a fraction of the line.
const SKELETON: [f32; 6] = [0.86, 0.62, 0.92, 0.48, 0.74, 0.58];

/// The art direction's ease-out, `cubic-bezier(0.2, 0, 0, 1)`: progress for a fraction of the time (the
/// curve parameter is found by bisection: x is monotonic).
pub fn ease_out(t: f32) -> f32 {
    let [x1, y1, x2, y2] = EASE_OUT_BEZIER;
    let t = t.clamp(0.0, 1.0);
    let bezier = |s: f32, p1: f32, p2: f32| {
        let r = 1.0 - s;
        3.0 * r * r * s * p1 + 3.0 * r * s * s * p2 + s * s * s
    };
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if bezier(mid, x1, x2) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bezier((lo + hi) / 2.0, y1, y2)
}

/// A value going from `from` to `to` in `ms` milliseconds with the ease-out.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    ms: u32,
}

impl Tween {
    fn rest(at: f32, now: Instant) -> Self {
        Self { from: at, to: at, start: now, ms: 0 }
    }

    fn value(&self, now: Instant) -> f32 {
        if self.ms == 0 {
            return self.to;
        }
        let t = now.saturating_duration_since(self.start).as_secs_f32() * 1000.0 / self.ms as f32;
        if t >= 1.0 { self.to } else { self.from + (self.to - self.from) * ease_out(t) }
    }

    fn running(&self, now: Instant) -> bool {
        self.ms > 0 && now.saturating_duration_since(self.start) < Duration::from_millis(u64::from(self.ms))
    }

    /// Restarts towards `to` from where it is now.
    fn retarget(&mut self, to: f32, now: Instant, ms: u32) {
        *self = Self { from: self.value(now), to, start: now, ms };
    }
}

/// The top of each verse, from the heights.
pub fn tops(heights: &[f32]) -> Vec<f32> {
    heights
        .iter()
        .scan(0.0, |y, h| {
            let top = *y;
            *y += h;
            Some(top)
        })
        .collect()
}

/// The point (from the top of the text) that sits at the anchor: the centre of the verse at `position`, which
/// may be between two verses while the text glides. Before the first verse (`-1`) the text sits one verse
/// lower (callers clamp to 0 to start at the anchor).
pub fn focus_y(tops: &[f32], heights: &[f32], position: f32) -> f32 {
    let center =
        |i: usize| tops.get(i).copied().unwrap_or(0.0) + heights.get(i).copied().unwrap_or(0.0) / 2.0;
    let last = tops.len().saturating_sub(1);
    if position < 0.0 {
        return center(0) + position * heights.first().copied().unwrap_or(0.0);
    }
    let first = (position.floor() as usize).min(last);
    let next = (first + 1).min(last);
    let t = (position - first as f32).clamp(0.0, 1.0);
    center(first) + (center(next) - center(first)) * t
}

/// The whole verses inside a frame `height` tall with the text moved up by `offset` (a verse cut by the edge
/// is not drawn).
pub fn visible(tops: &[f32], heights: &[f32], offset: f32, height: f32) -> std::ops::Range<usize> {
    let start = tops.iter().position(|top| *top >= offset - 0.5).unwrap_or(tops.len());
    let end = tops
        .iter()
        .zip(heights)
        .skip(start)
        .position(|(top, h)| top + h > offset + height + 0.5)
        .map_or(tops.len(), |n| start + n);
    start..end
}

/// What the view shows.
#[derive(Clone, Debug, PartialEq)]
enum Shown {
    /// No track in the queue.
    NoTrack,
    Loading,
    Failed(SharedString),
    /// LRCLIB has no lyrics for this track.
    Missing,
    Plain(SharedString),
    Synced(Arc<Text>),
}

impl Shown {
    fn of(view: &PlaybackView) -> Self {
        let Some(track) = view.queue.current() else { return Self::NoTrack };
        if view.lyrics.track != Some(track.id) {
            return Self::Loading;
        }
        match &view.lyrics.state {
            LyricsState::Idle | LyricsState::Loading => Self::Loading,
            LyricsState::Failed(error) => Self::Failed(error.to_string().into()),
            LyricsState::Ready(text) if text.is_synced() => Self::Synced(Arc::clone(text)),
            LyricsState::Ready(text) => {
                text.plain.as_deref().map_or(Self::Missing, |plain| Self::Plain(plain.to_owned().into()))
            }
        }
    }
}

/// Verse heights of a text at a width: redone only when either changes.
struct Layout {
    text: Arc<Text>,
    width: f32,
    heights: Vec<f32>,
    tops: Vec<f32>,
}

pub struct LyricsView {
    core: Entity<CoreModel>,
    look: Look,
    /// The view is on screen.
    visible: bool,
    view: Arc<PlaybackView>,
    shown: Shown,
    /// The verses as `SharedString`, made once per text.
    verses: Vec<SharedString>,
    /// Title, artist and cover of the playing track, for the panel header.
    header: Option<(SharedString, SharedString, Option<SharedString>)>,
    layout: Option<Layout>,
    /// The current verse and the lit dots, if it is a pause.
    line: Option<usize>,
    dots: u32,
    /// Where the text is, in verses (`-1` before the first).
    scroll: Tween,
    /// The text frame at the last draw, in dp: measured in prepaint.
    viewport: Rc<Cell<(f32, f32)>>,
    /// The next-change timer: only while visible and playing.
    follow: Option<Task<()>>,
    /// The glide frames: only while the text moves.
    frames: Option<Task<()>>,
    _changes: Subscription,
}

impl LyricsView {
    pub fn new(core: Entity<CoreModel>, look: Look, cx: &mut Context<Self>) -> Self {
        let changes = cx.subscribe(&core, |this, core, changed: &Changed, cx| {
            if changed.playback {
                let view = Arc::clone(&core.read(cx).snapshot().playback);
                this.playback_changed(view, cx);
            }
        });
        let view = Arc::clone(&core.read(cx).snapshot().playback);
        let mut this = Self {
            core,
            look,
            visible: false,
            view: Arc::default(),
            shown: Shown::NoTrack,
            verses: Vec::new(),
            header: None,
            layout: None,
            line: None,
            dots: 0,
            scroll: Tween::rest(-1.0, Instant::now()),
            viewport: Rc::new(Cell::new((0.0, 0.0))),
            follow: None,
            frames: None,
            _changes: changes,
        };
        this.playback_changed(view, cx);
        this
    }

    /// The window says whether the view is on screen: only then it has a timer.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible {
            // Back on the right verse, without gliding from where it was.
            self.follow_position(false, cx);
            cx.notify();
        } else {
            self.frames = None;
        }
        self.sync_follow(cx);
    }

    fn playback_changed(&mut self, view: Arc<PlaybackView>, cx: &mut Context<Self>) {
        let mut dirty = false;
        let shown = Shown::of(&view);
        if shown != self.shown {
            if let Shown::Synced(text) = &shown {
                self.verses = text.lines.iter().map(|line| SharedString::from(line.text.clone())).collect();
            } else {
                self.verses.clear();
            }
            self.layout = None;
            self.line = None;
            self.dots = 0;
            self.scroll = Tween::rest(-1.0, Instant::now());
            self.frames = None;
            self.shown = shown;
            dirty = true;
        }
        let header = view.queue.current().map(|track| {
            (
                SharedString::from(track.title.clone()),
                SharedString::from(track.artist.name.clone()),
                track.cover_url.clone().map(SharedString::from),
            )
        });
        if header != self.header {
            self.header = header;
            dirty = true;
        }
        let playing_changed = view.is_playing() != self.view.is_playing();
        self.view = view;
        // A new track starts on its verse without gliding; a seek or the next verse glide.
        let animate = !dirty;
        if self.follow_position(animate, cx) || playing_changed || dirty {
            // The cursor colour changes between playing and paused too.
            cx.notify();
        }
        self.sync_follow(cx);
    }

    /// The verse and the dots at the current position; `true` if they changed: then the text glides to the
    /// new verse (or jumps, without `animate`).
    fn follow_position(&mut self, animate: bool, cx: &mut Context<Self>) -> bool {
        let Shown::Synced(text) = &self.shown else { return false };
        let now = Instant::now();
        let position = self.view.position(now);
        let lines = &text.lines;
        let line = current(lines, position);
        let dots = match line.and_then(|ix| lines.get(ix).map(|l| (ix, l))) {
            Some((ix, verse)) if is_pause(verse) => {
                dots_lit(verse, lines.get(ix + 1).map(|next| next.at), position)
            }
            _ => 0,
        };
        let moved = line != self.line;
        let changed = moved || dots != self.dots;
        self.line = line;
        self.dots = dots;
        let target = line.map_or(-1.0, |ix| ix as f32);
        if moved || (!animate && self.scroll.to != target) {
            let reduce = cx.try_global::<SpriteStore>().is_some_and(SpriteStore::reduce_motion);
            let glide = animate && self.visible && self.look != Look::Excerpt;
            let ms = if glide { MOTION_LYRICS.effective_ms(reduce) } else { 0 };
            self.scroll.retarget(target, now, ms);
            if ms > 0 {
                self.run_frames(cx);
            }
        }
        changed
    }

    /// The glide frames: one every 42 ms while the text moves, then nothing.
    fn run_frames(&mut self, cx: &mut Context<Self>) {
        if self.frames.is_some() {
            return;
        }
        self.frames = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(FRAME).await;
                let going = this
                    .update(cx, |this, cx| {
                        cx.notify();
                        let going = this.scroll.running(Instant::now());
                        if !going {
                            this.frames = None;
                        }
                        going
                    })
                    .unwrap_or(false);
                if !going {
                    break;
                }
            }
        }));
    }

    /// Arms the next-change timer, or drops it: only while visible, playing and synced. Each call replaces the
    /// previous timer.
    fn sync_follow(&mut self, cx: &mut Context<Self>) {
        let Shown::Synced(text) = &self.shown else {
            self.follow = None;
            return;
        };
        if !self.visible || self.view.state != PlaybackState::Playing {
            self.follow = None;
            return;
        }
        let Some(wait) = next_change(&text.lines, self.view.position(Instant::now())) else {
            self.follow = None;
            return;
        };
        self.follow = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait + WAKE_MARGIN).await;
            let _ = this.update(cx, |this, cx| {
                this.follow = None;
                if this.follow_position(true, cx) {
                    cx.notify();
                }
                this.sync_follow(cx);
            });
        }));
    }

    fn seek(&self, at: Duration, cx: &gpui::App) {
        if self.view.is_active() {
            self.core.read(cx).playback(PlaybackCmd::SeekTo(at));
        }
    }

    /// Verse heights at `width` dp, measured by the text system as the draw will wrap them. Only when the text
    /// or the width change.
    fn ensure_layout(&mut self, text: &Arc<Text>, width: f32, window: &Window) {
        if let Some(layout) = &self.layout
            && Arc::ptr_eq(&layout.text, text)
            && (layout.width - width).abs() < 0.5
        {
            return;
        }
        let role = self.look.role();
        let font = gpui::Font {
            family: SharedString::new_static(role.family),
            weight: FontWeight(f32::from(role.weight)),
            ..gpui::font(role.family)
        };
        let heights: Vec<f32> = self
            .verses
            .iter()
            .map(|verse| {
                let rows = if verse.trim().is_empty() {
                    1
                } else {
                    let run = TextRun {
                        len: verse.len(),
                        font: font.clone(),
                        color: Hsla::default(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    window
                        .text_system()
                        .shape_text(verse.clone(), dp(role.size), &[run], Some(dp(width)), None)
                        .map_or(1, |lines| lines.iter().map(|line| line.wrap_boundaries().len() + 1).sum())
                };
                rows as f32 * role.line_height + self.look.gap()
            })
            .collect();
        self.layout = Some(Layout { text: Arc::clone(text), width, tops: tops(&heights), heights });
    }

    /// Synced lyrics in the measured frame.
    fn synced(&mut self, text: &Arc<Text>, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let (width, height) = self.viewport.get();
        if width <= GUTTER || height <= 0.0 {
            // The first draw, before prepaint measured the frame: no verses measured for nothing.
            return div().into_any_element();
        }
        let text_width = width - GUTTER;
        self.ensure_layout(text, text_width, window);
        let Some(layout) = &self.layout else { return div().into_any_element() };
        let ap = Ap::of(window);
        let scale = ap.effective();
        let role = self.look.role();
        let now = Instant::now();
        // Before the first verse the text starts at the anchor too.
        let focus = focus_y(&layout.tops, &layout.heights, self.scroll.value(now).max(0.0));
        // On physical pixels, so the text never blurs while it glides.
        let offset = ((focus - height * self.look.anchor()) * scale).round() / scale;
        let playing = self.view.is_playing();
        let range = visible(&layout.tops, &layout.heights, offset, height);
        let mut verses = Vec::with_capacity(range.len());
        for ix in range {
            let (Some(line), Some(verse)) = (text.lines.get(ix), self.verses.get(ix)) else { continue };
            let is_current = self.line == Some(ix);
            let color = match self.line {
                Some(now) if ix < now => TEXT_TERTIARY,
                Some(now) if ix == now => TEXT_PRIMARY,
                _ => TEXT_SECONDARY,
            };
            let top = layout.tops[ix] - offset;
            let cursor = is_current.then(|| cursor(role, ap, playing));
            let words = if is_pause(line) {
                dots(role, ap, if is_current { self.dots } else { 0 })
            } else {
                words(verse.clone(), role, color, text_width)
            };
            let at = line.at;
            verses.push(
                div()
                    .id(("lyric", ix))
                    .absolute()
                    .left_0()
                    .top(dp(top))
                    .w(dp(width))
                    .h(dp(layout.heights[ix]))
                    .pl(dp(GUTTER))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.seek(at, cx)))
                    .children(cursor)
                    .child(words),
            );
        }
        div().size_full().children(verses).into_any_element()
    }

    /// Grey verses the size of real ones while the text loads.
    fn loading(&self) -> AnyElement {
        let (width, _) = self.viewport.get();
        let role = self.look.role();
        div()
            .pl(dp(GUTTER))
            .flex()
            .flex_col()
            .gap(dp(role.line_height - role.size * 0.8 + self.look.gap()))
            .pt(dp(SPACE_3))
            .children(
                SKELETON
                    .iter()
                    .map(|share| Skeleton::new(((width - GUTTER) * share).max(0.0), role.size * 0.8)),
            )
            .into_any_element()
    }

    /// A sentence instead of the text (no track, no lyrics).
    fn note(text: impl Into<SharedString>) -> gpui::Div {
        div().pl(dp(GUTTER)).type_role(TYPE_BODY).text_color(col(TEXT_SECONDARY)).child(text.into())
    }

    fn body(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        match self.shown.clone() {
            Shown::NoTrack => Self::note("Scegli un brano per leggere il testo.").into_any_element(),
            Shown::Loading => self.loading(),
            Shown::Failed(message) => {
                let core = self.core.clone();
                Self::note(message)
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(dp(SPACE_4))
                    .child(Button::new("lyrics-retry", "Riprova").secondary().on_click(move |_, _, cx| {
                        // A new «showing the lyrics» makes the core try again.
                        core.read(cx).playback(PlaybackCmd::Lyrics(true));
                    }))
                    .into_any_element()
            }
            Shown::Missing => match self.look {
                // In the panel the sentence alone; on the stage Nastro listening too.
                Look::Panel | Look::Excerpt => Self::note("Nessun testo per questo brano").into_any_element(),
                Look::Stage => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(EmptyState::new(
                        // Nastro listens (the lime LED) only while the track plays.
                        if self.view.is_playing() { sprite::NASTRO_LISTEN } else { sprite::NASTRO_IDLE },
                        "Nessun testo per questo brano",
                        "LRCLIB non ha il testo di questo brano.",
                    ))
                    .into_any_element(),
            },
            Shown::Plain(plain) => div()
                .id("lyrics-plain")
                .size_full()
                .overflow_y_scroll()
                .child(
                    div()
                        .pl(dp(GUTTER))
                        .pb(dp(SPACE_6))
                        .type_role(self.look.role())
                        .text_color(col(TEXT_SECONDARY))
                        .child(plain),
                )
                .into_any_element(),
            Shown::Synced(text) => self.synced(&text, window, cx),
        }
    }

    /// The text frame, measured in prepaint: when it changes the view redraws once on the next frame.
    fn frame(&mut self, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let viewport = Rc::clone(&self.viewport);
        let measure = canvas(
            move |bounds, window, _| {
                let size = (to_dp(bounds.size.width), to_dp(bounds.size.height));
                let old = viewport.get();
                if (old.0 - size.0).abs() > 0.5 || (old.1 - size.1).abs() > 0.5 {
                    viewport.set(size);
                    window.request_animation_frame();
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .size_full();
        div().relative().size_full().overflow_hidden().child(measure).child(self.body(window, cx))
    }
}

impl LyricsView {
    /// The *Testo* header: 64 dp with the 56 dp cover, title and artist.
    fn header(&self) -> gpui::Div {
        let (title, artist, cover) = self.header.clone().unwrap_or_default();
        div()
            .flex_none()
            .h(dp(64.0))
            .px(dp(SPACE_5))
            .flex()
            .items_center()
            .gap(dp(SPACE_4))
            .child(Cover::new(ART_SHORTCUT).url(cover))
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
                            .child(title),
                    )
                    .child(
                        div().type_role(TYPE_ARTIST).text_color(col(TEXT_SECONDARY)).truncate().child(artist),
                    ),
            )
    }

    /// The excerpt: the current line and the next four, in flow; a click opens *Testo*.
    fn excerpt(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let role = self.look.role();
        let ap = Ap::of(window);
        let playing = self.view.is_playing();
        let body: AnyElement = match &self.shown {
            Shown::NoTrack => return div().into_any_element(),
            Shown::Loading => div()
                .pl(dp(GUTTER))
                .flex()
                .flex_col()
                .gap(dp(role.line_height - role.size * 0.8 + self.look.gap()))
                .children(
                    SKELETON
                        .iter()
                        .take(EXCERPT_LINES)
                        .map(|share| Skeleton::new(240.0 * share, role.size * 0.8)),
                )
                .into_any_element(),
            Shown::Failed(_) | Shown::Missing => {
                Self::note("Nessun testo per questo brano").into_any_element()
            }
            Shown::Plain(plain) => div()
                .pl(dp(GUTTER))
                .type_role(role)
                .text_color(col(TEXT_SECONDARY))
                .children(
                    plain
                        .lines()
                        .filter(|line| !line.trim().is_empty())
                        .take(EXCERPT_LINES)
                        .map(|line| div().truncate().child(SharedString::from(line.to_owned()))),
                )
                .into_any_element(),
            Shown::Synced(text) => {
                let range = excerpt(self.line, text.lines.len());
                div()
                    .flex()
                    .flex_col()
                    .gap(dp(self.look.gap()))
                    .children(range.filter_map(|ix| {
                        let line = text.lines.get(ix)?;
                        let verse = self.verses.get(ix)?;
                        let is_current = self.line == Some(ix);
                        let color = if is_current { TEXT_PRIMARY } else { TEXT_SECONDARY };
                        let words = if is_pause(line) {
                            dots(role, ap, if is_current { self.dots } else { 0 })
                        } else {
                            div()
                                .type_role(role)
                                .text_color(col(color))
                                .line_clamp(2)
                                .child(verse.clone())
                                .into_any_element()
                        };
                        Some(
                            div()
                                .relative()
                                .pl(dp(GUTTER))
                                .children(is_current.then(|| cursor(role, ap, playing)))
                                .child(words),
                        )
                    }))
                    .into_any_element()
            }
        };
        div()
            .id("lyrics-excerpt")
            .cursor_pointer()
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(LyricsEvent::Open)))
            .child(body)
            .into_any_element()
    }
}

/// The current-line cursor: a 2 x 3 AP block centred on the verse's first row (lime while playing).
fn cursor(role: TypeRole, ap: Ap, playing: bool) -> gpui::Div {
    div()
        .absolute()
        .left_0()
        .top(dp((role.line_height - to_dp(ap.n(3))) / 2.0))
        .w(ap.n(2))
        .h(ap.n(3))
        .bg(col(if playing { STATE_ON } else { STONE }))
}

/// The three dots of a pause, `lit` of them lit.
fn dots(role: TypeRole, ap: Ap, lit: u32) -> AnyElement {
    div()
        .h(dp(role.line_height))
        .flex()
        .items_center()
        .gap(ap.n(2))
        .children(
            (0..DOTS)
                .map(|dot| div().w(ap.n(2)).h(ap.n(2)).bg(col(if dot < lit { WARM_WHITE } else { STONE }))),
        )
        .into_any_element()
}

/// A verse's words at `width` dp.
fn words(
    verse: SharedString,
    role: TypeRole,
    color: psytify_design::tokens::color::Rgba,
    width: f32,
) -> AnyElement {
    div()
        .w(dp(width))
        .type_role(role)
        .text_color(col(color))
        .hover(|style| style.text_color(col(TEXT_PRIMARY)))
        .child(verse)
        .into_any_element()
}

impl Render for LyricsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.look {
            Look::Stage => div().size_full().child(self.frame(window, cx)),
            Look::Excerpt => div().w_full().child(self.excerpt(window, cx)),
            Look::Panel => div()
                .size_full()
                .flex()
                .flex_col()
                .child(self.header())
                .child(Separator::horizontal())
                .child(div().flex_1().min_h_0().mr(dp(SPACE_5)).ml(dp(SPACE_4)).child(self.frame(window, cx)))
                .child(
                    div()
                        .flex_none()
                        .px(dp(SPACE_5))
                        .py(dp(SPACE_3))
                        .type_role(TYPE_META)
                        .text_color(col(TEXT_TERTIARY))
                        .child("Testi da LRCLIB"),
                ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn the_excerpt_is_the_current_line_and_the_next_four() {
        assert_eq!(excerpt(None, 20), 0..5, "before the first verse: the first five");
        assert_eq!(excerpt(Some(7), 20), 7..12);
        assert_eq!(excerpt(Some(18), 20), 18..20, "near the end: what is left");
        assert_eq!(excerpt(Some(3), 0), 0..0);
    }

    #[test]
    fn the_ease_out_starts_fast_and_lands_on_one() {
        assert!(ease_out(0.0).abs() < 1e-4);
        assert!((ease_out(1.0) - 1.0).abs() < 1e-4);
        assert!(ease_out(0.25) > 0.5, "{}", ease_out(0.25));
        let samples: Vec<f32> = (0..=20).map(|i| ease_out(i as f32 / 20.0)).collect();
        assert!(samples.windows(2).all(|pair| pair[1] >= pair[0]), "never backwards");
    }

    #[test]
    fn the_focus_moves_smoothly_between_two_verses() {
        let (heights, tops) = ([50.0, 100.0, 50.0], tops(&[50.0, 100.0, 50.0]));
        assert_eq!(tops, vec![0.0, 50.0, 150.0]);
        assert_eq!(focus_y(&tops, &heights, 0.0), 25.0);
        assert_eq!(focus_y(&tops, &heights, 1.0), 100.0);
        assert_eq!(focus_y(&tops, &heights, 0.5), 62.5);
        assert_eq!(focus_y(&tops, &heights, -1.0), -25.0);
        assert_eq!(focus_y(&tops, &heights, 7.0), 175.0);
    }

    #[test]
    fn only_the_whole_verses_in_the_frame_are_built() {
        let heights = [40.0; 10];
        let tops = tops(&heights);
        assert_eq!(visible(&tops, &heights, 0.0, 100.0), 0..2, "the third would cross the edge");
        assert_eq!(visible(&tops, &heights, 90.0, 100.0), 3..4);
        assert_eq!(visible(&tops, &heights, -200.0, 100.0), 0..0);
        assert_eq!(visible(&tops, &heights, -60.0, 100.0), 0..1);
        assert_eq!(visible(&tops, &heights, 1_000.0, 100.0), 10..10);
    }

    #[test]
    fn a_tween_glides_and_then_rests() {
        let start = Instant::now();
        let mut tween = Tween::rest(0.0, start);
        tween.retarget(4.0, start, 280);
        assert!(tween.running(start + ms(100)));
        let mid = tween.value(start + ms(140));
        assert!(mid > 2.0 && mid < 4.0, "{mid}");
        assert_eq!(tween.value(start + ms(280)), 4.0);
        assert!(!tween.running(start + ms(280)));
        // Restarts from where it is, not from where it started.
        tween.retarget(0.0, start + ms(140), 280);
        assert!((tween.value(start + ms(140)) - mid).abs() < 1e-4);
    }
}
