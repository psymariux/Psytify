//! The stage (F, desktop v2 §5): the record, the words and the clip in one place, instead of the old focus
//! view and full-screen video. It takes everything under the title bar.
//!
//! - Left (45 %): the media slot at the largest square that fits (up to `ART_STAGE_MAX`), or the clip at 16:9
//!   in the same slot; under it the title in Display, the artist, the seek bar with the times in pixel
//!   numerals, and the transport row: the heart, the transport with the 56 dp play button, the Cover / Video
//!   switch (V).
//! - Right: the lyrics at 32 / 40 with the current line at the upper third (`LyricsView`, `Look::Stage`); no
//!   lyrics: Nastro listening and the line.
//! - Bottom: a 72 dp «Prossimo» strip with the next three tracks as 40 dp rows.
//!
//! Entering and leaving: `Shell` eases a reveal from 0 to 1 (or back) in `MOTION_PANEL` and shares it
//! ([`Reveal`]); while it moves, the cover grows in whole-AP steps from the player bar cover to the media slot
//! and the rest fades in over the page. Reduced motion: a cut.
//!
//! A 250 ms tick redraws the seek bar and the times only while the stage is open and the track plays; the
//! lyrics are a separate cached view with their own timer.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AppContext, ClickEvent, Context, Entity, EventEmitter, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StyleRefinement, Styled, Subscription, Task, Window, canvas, div,
};
use psytify_core::command::{LibraryCmd, PlaybackCmd};
use psytify_core::player::{PlaybackState, Queue, Repeat as QueueRepeat};
use psytify_core::snapshot::PlaybackView;
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY};
use psytify_design::tokens::motion::PROGRESS_TICK_MS;
use psytify_design::tokens::size::cover::{ART_SHORTCUT, ART_STAGE_MAX};
use psytify_design::tokens::size::{PLAYER_BAR_H, frame_margin};
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{TITLE, TYPE_DISPLAY, TYPE_LABEL};
use uuid::Uuid;

use super::lyrics_view::{Look, LyricsView};
use super::media::{Slot, media_slot, slot_height};
use super::now_panel::{NowTrack, up_next};
use crate::components::icon_button::IconButton;
use crate::components::like_button::LikeButton;
use crate::components::now_playing::{MediaSwitch, UpNext, UpNextRow};
use crate::components::player_control::{PlayerControl, Repeat, Transport};
use crate::components::progress::ProgressBar;
use crate::components::separator::Separator;
use crate::components::tile::Cover;
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::motion::ap_round;
use crate::theme::{Ap, TypeStyle, cased, col, dp, to_dp};
#[cfg(feature = "video")]
use crate::video::stage::{StageEvent as VideoEvent, Surface, VideoStage};

/// The «Prossimo» strip.
pub const STRIP_H: f32 = 72.0;
/// Tracks on the strip.
const STRIP_TRACKS: usize = 3;
/// The left column's share of the width.
const LEFT_SHARE: f32 = 0.45;
/// Room under the media slot: title, artist, seek bar and times, transport, and their gaps.
const UNDER_MEDIA: f32 = 44.0 + 28.0 + 24.0 + 28.0 + 56.0 + 4.0 * SPACE_4 + SPACE_5;
/// The smallest media slot and the narrowest info column (the transport row must fit).
const MEDIA_MIN: f32 = 160.0;
const INFO_MIN: f32 = 360.0;
/// The play button on the stage.
const PLAY_SIDE: f32 = 56.0;

/// The reveal shared with `Shell`: 0 is the player bar, 1 the stage.
pub type Reveal = Rc<Cell<f32>>;

/// Where everything sits on a stage `width` x `height` dp. Pure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub margin: f32,
    /// The left column's width.
    pub left_w: f32,
    /// The media slot (a square for the cover, 16:9 for the clip) and its top-left corner.
    pub media_w: f32,
    pub media_h: f32,
    pub media_x: f32,
    pub media_y: f32,
    /// The info column under the slot.
    pub info_w: f32,
    /// The lyrics column.
    pub lyrics_x: f32,
    pub lyrics_w: f32,
    /// The height above the strip.
    pub content_h: f32,
}

impl Layout {
    pub fn of(width: f32, height: f32, video: bool) -> Self {
        let margin = frame_margin(width);
        let content_h = (height - STRIP_H - 1.0).max(MEDIA_MIN + UNDER_MEDIA);
        let left_w = (width * LEFT_SHARE).max(INFO_MIN + 2.0 * margin);
        let room_w = left_w - 2.0 * margin;
        let room_h = (content_h - 2.0 * margin - UNDER_MEDIA).max(MEDIA_MIN * 9.0 / 16.0);
        let (media_w, media_h) = if video {
            let w = room_w.min(room_h * 16.0 / 9.0).max(MEDIA_MIN);
            (w, slot_height(w, true))
        } else {
            let side = room_w.min(room_h).clamp(MEDIA_MIN, ART_STAGE_MAX);
            (side, side)
        };
        let info_w = media_w.max(INFO_MIN).min(room_w);
        let block_h = media_h + UNDER_MEDIA;
        let media_y = ((content_h - block_h) / 2.0).max(margin);
        let media_x = (left_w - media_w) / 2.0;
        let lyrics_x = left_w;
        let lyrics_w = (width - left_w - margin).max(240.0);
        Self { margin, left_w, media_w, media_h, media_x, media_y, info_w, lyrics_x, lyrics_w, content_h }
    }
}

/// `a` to `b` at `t`, on whole APs (`ap_dp` is one AP in dp).
fn step(a: f32, b: f32, t: f32, ap_dp: f32) -> f32 {
    let value = a + (b - a) * t;
    if ap_dp > 0.0 { ap_round(value / ap_dp) as f32 * ap_dp } else { value }
}

/// What the stage asks of the window.
pub enum StageViewEvent {
    Close,
    /// The Cover / Video switch.
    Video(bool),
}

impl EventEmitter<StageViewEvent> for StageView {}

pub struct StageView {
    core: Entity<CoreModel>,
    view: Arc<PlaybackView>,
    queue: Arc<Queue>,
    liked: Arc<HashSet<Uuid>>,
    now: Option<NowTrack>,
    duration: Duration,
    next: Vec<UpNext>,
    lyrics: Entity<LyricsView>,
    open: bool,
    /// The clip is on in the media slot.
    video: bool,
    reveal: Reveal,
    /// The stage's top-left corner in the window, measured in prepaint (for the cover's flight).
    origin: Rc<Cell<(f32, f32)>>,
    tick: Option<Task<()>>,
    /// Elapsed and remaining, redone only when the second changes.
    times: (u64, SharedString, SharedString),
    #[cfg(feature = "video")]
    stage: Entity<VideoStage>,
    #[cfg(feature = "video")]
    spot: crate::video::stage::Spot,
    _subscriptions: Vec<Subscription>,
}

impl StageView {
    pub fn new(
        core: Entity<CoreModel>,
        reveal: Reveal,
        #[cfg(feature = "video")] stage: Entity<VideoStage>,
        cx: &mut Context<Self>,
    ) -> Self {
        let lyrics = cx.new(|cx| LyricsView::new(core.clone(), Look::Stage, cx));
        #[cfg_attr(not(feature = "video"), allow(unused_mut))]
        let mut subscriptions = vec![cx.subscribe(&core, |this, core, changed: &Changed, cx| {
            let mut dirty = false;
            if changed.playback {
                this.take_playback(&core, cx);
                this.sync_tick(cx);
                dirty = true;
            }
            if changed.library {
                this.liked = core.read(cx).liked();
                dirty = true;
            }
            if dirty && this.open {
                cx.notify();
            }
        })];
        #[cfg(feature = "video")]
        subscriptions.push(cx.subscribe(&stage, |this, _, event: &VideoEvent, cx| {
            if matches!(event, VideoEvent::Status) && this.open {
                cx.notify();
            }
        }));
        let mut view = Self {
            view: Arc::default(),
            queue: Arc::default(),
            liked: core.read(cx).liked(),
            now: None,
            duration: Duration::ZERO,
            next: Vec::new(),
            lyrics,
            open: false,
            video: false,
            reveal,
            origin: Rc::new(Cell::new((0.0, 0.0))),
            tick: None,
            times: (u64::MAX, SharedString::default(), SharedString::default()),
            #[cfg(feature = "video")]
            spot: stage.read(cx).spot(Surface::Stage),
            #[cfg(feature = "video")]
            stage,
            core: core.clone(),
            _subscriptions: subscriptions,
        };
        view.take_playback(&core, cx);
        view
    }

    fn take_playback(&mut self, core: &Entity<CoreModel>, cx: &mut Context<Self>) {
        let view = Arc::clone(&core.read(cx).snapshot().playback);
        if !Arc::ptr_eq(&view.queue, &self.queue) {
            self.queue = Arc::clone(&view.queue);
            let current = self.queue.current();
            self.now = current.map(NowTrack::of);
            self.duration = current.map_or(Duration::ZERO, |track| track.duration());
            self.next = up_next(&self.queue, STRIP_TRACKS);
        }
        self.view = view;
    }

    /// The window opens or closes the stage: only open it has the tick, and the lyrics their timer.
    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        self.lyrics.update(cx, |lyrics, cx| lyrics.set_visible(open, cx));
        self.sync_tick(cx);
        cx.notify();
    }

    /// The clip on or off in the media slot.
    pub fn set_video(&mut self, video: bool, cx: &mut Context<Self>) {
        if self.video != video {
            self.video = video;
            cx.notify();
        }
    }

    fn sync_tick(&mut self, cx: &mut Context<Self>) {
        if !self.open || !self.view.is_playing() {
            self.tick = None;
            return;
        }
        if self.tick.is_some() {
            return;
        }
        self.tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(u64::from(PROGRESS_TICK_MS))).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        }));
    }

    fn times(&mut self, position: Duration) -> (SharedString, SharedString) {
        let secs = position.as_secs();
        if self.times.0 != secs {
            let left = self.duration.saturating_sub(position);
            self.times = (secs, format::clock(position).into(), format!("-{}", format::clock(left)).into());
        }
        (self.times.1.clone(), self.times.2.clone())
    }

    /// Title, artist, seek bar and transport under the slot.
    fn info(&mut self, now: &NowTrack, width: f32, cx: &mut Context<Self>) -> gpui::Div {
        let view = Arc::clone(&self.view);
        let active = view.is_active();
        let duration = self.duration;
        let position = if active { view.position(Instant::now()) } else { Duration::ZERO };
        let fraction = if duration.is_zero() { 0.0 } else { position.as_secs_f32() / duration.as_secs_f32() };
        let (elapsed, remaining) = self.times(position);
        let core = self.core.clone();
        let command = move |command: PlaybackCmd| {
            let core = core.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut gpui::App| core.read(cx).playback(command.clone())
        };
        let queue = &view.queue;
        let transport = match view.state {
            PlaybackState::Playing => Transport::Playing,
            PlaybackState::Loading { .. } => Transport::Loading,
            _ => Transport::Stopped,
        };
        let repeat = match queue.repeat() {
            QueueRepeat::Off => Repeat::Off,
            QueueRepeat::All => Repeat::All,
            QueueRepeat::One => Repeat::One,
        };
        let liked = self.liked.contains(&now.id);
        let like_core = self.core.clone();
        let track = self.queue.current().cloned();
        let seek_core = self.core.clone();
        let switch = now.has_video.then(|| {
            let entity = cx.entity().downgrade();
            MediaSwitch::new("stage-switch", self.video && now.has_video).on_toggle(move |video, _, cx| {
                entity.update(cx, |_, cx| cx.emit(StageViewEvent::Video(video))).ok();
            })
        });
        div()
            .w(dp(width))
            .flex()
            .flex_col()
            .gap(dp(SPACE_4))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .type_role(TYPE_DISPLAY)
                            .text_color(col(TEXT_PRIMARY))
                            .truncate()
                            .child(now.title.clone()),
                    )
                    .child(
                        div()
                            .type_role(TITLE.weight(500))
                            .text_color(col(TEXT_SECONDARY))
                            .truncate()
                            .child(now.artist.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(dp(SPACE_2))
                    .child(
                        ProgressBar::new("stage-seek", fraction)
                            .disabled(!active)
                            .label(move |fraction| format::clock(duration.mul_f32(fraction)))
                            .on_change(move |fraction, _, cx| {
                                seek_core.read(cx).playback(PlaybackCmd::SeekFraction(fraction));
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .pixel_type(true)
                            .text_color(col(TEXT_TERTIARY))
                            .child(elapsed)
                            .child(remaining),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(LikeButton::new("stage-like", liked).on_click(move |_, _, cx| {
                        if let Some(track) = track.clone() {
                            like_core.read(cx).library(LibraryCmd::ToggleLike(track));
                        }
                    }))
                    .child(
                        PlayerControl::new("stage-transport")
                            .transport(transport)
                            .play_size(PLAY_SIDE)
                            .shuffle(queue.shuffle())
                            .repeat(repeat)
                            .disabled(queue.is_empty())
                            .on_play(command(PlaybackCmd::Toggle))
                            .on_previous(command(PlaybackCmd::Previous))
                            .on_next(command(PlaybackCmd::Next))
                            .on_shuffle(command(PlaybackCmd::ToggleShuffle))
                            .on_repeat(command(PlaybackCmd::CycleRepeat)),
                    )
                    .child(div().w(dp(96.0)).flex().justify_end().children(switch)),
            )
    }

    /// The 72 dp «Prossimo» strip.
    fn strip(&self, margin: f32) -> gpui::Div {
        let core = self.core.clone();
        let rows = self.next.iter().enumerate().map(|(ix, item)| {
            let position = item.position;
            let core = core.clone();
            div().flex_1().min_w_0().child(
                UpNextRow::new(("stage-next", ix), item.clone())
                    .height(40.0)
                    .on_click(move |_, _, cx| core.read(cx).playback(PlaybackCmd::PlayAt(position))),
            )
        });
        div()
            .flex_none()
            .h(dp(STRIP_H))
            .px(dp(margin))
            .flex()
            .items_center()
            .gap(dp(SPACE_6))
            .child(
                div().flex_none().type_role(TYPE_LABEL).text_color(col(TEXT_TERTIARY)).child(cased(
                    if self.next.is_empty() { "Fine della coda" } else { "Prossimo" },
                    TYPE_LABEL,
                )),
            )
            .children(rows)
    }
}

/// The lyrics in `.cached()`: the view measures itself.
fn fill() -> StyleRefinement {
    StyleRefinement::default().size_full()
}

impl Render for StageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(feature = "video")]
        self.spot.clear();
        let viewport = window.viewport_size();
        let (vw, vh) = (to_dp(viewport.width), to_dp(viewport.height));
        let (ox, oy) = self.origin.get();
        let (width, height) = (vw - ox, vh - oy);
        let Some(now) = self.now.clone() else {
            return div().size_full().bg(col(SURFACE_CANVAS)).into_any_element();
        };
        let video = self.video && now.has_video;
        let layout = Layout::of(width, height, video);
        let reveal = self.reveal.get().clamp(0.0, 1.0);
        let moving = reveal < 1.0;

        let origin = Rc::clone(&self.origin);
        let measure = canvas(
            move |bounds, window, _| {
                let at = (to_dp(bounds.origin.x), to_dp(bounds.origin.y));
                let old = origin.get();
                if (old.0 - at.0).abs() > 0.5 || (old.1 - at.1).abs() > 0.5 {
                    origin.set(at);
                    window.request_animation_frame();
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .size_full();

        let entity = cx.entity().downgrade();
        let media = if moving {
            // The flying cover stands in for the slot while the stage moves.
            div().w(dp(layout.media_w)).h(dp(layout.media_h)).into_any_element()
        } else {
            media_slot(
                Slot {
                    id: "stage-media",
                    width: layout.media_w,
                    cover_url: now.cover_url.clone(),
                    large: true,
                    has_video: false,
                    video,
                    on_toggle: Rc::new(move |video, _, cx| {
                        entity.update(cx, |_, cx| cx.emit(StageViewEvent::Video(video))).ok();
                    }),
                },
                #[cfg(feature = "video")]
                Some((&self.stage, Surface::Stage)),
                window,
                cx,
            )
        };
        #[cfg(feature = "video")]
        let status = (video && !moving)
            .then(|| super::media::video_status(&self.stage, "stage-video-retry", cx))
            .flatten();
        #[cfg(not(feature = "video"))]
        let status: Option<gpui::AnyElement> = None;
        let info = self.info(&now, layout.info_w, cx);
        let left = div()
            .flex_none()
            .w(dp(layout.left_w))
            .h_full()
            .pt(dp(layout.media_y))
            .flex()
            .flex_col()
            .items_center()
            .gap(dp(SPACE_5))
            .child(div().relative().child(media).children(status.map(|line| {
                div().absolute().left_0().right_0().bottom(dp(-SPACE_6)).flex().justify_center().child(line)
            })))
            .child(info);
        let right = div()
            .flex_none()
            .w(dp(layout.lyrics_w))
            .h_full()
            .py(dp(layout.margin))
            .child(self.lyrics.clone().cached(fill()));
        let close = cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(StageViewEvent::Close));
        let content = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(col(SURFACE_CANVAS))
            .opacity(if moving { reveal } else { 1.0 })
            .child(div().flex_none().h(dp(layout.content_h)).flex().child(left).child(right))
            .child(Separator::horizontal())
            .child(self.strip(layout.margin))
            .child(
                div().absolute().top(dp(SPACE_3)).right(dp(SPACE_3)).child(
                    IconButton::new("stage-close", sprite::ICON_CLOSE)
                        .tooltip("Chiudi il palco", Some("Esc"))
                        .on_click(close),
                ),
            );

        let mut root = div().id("stage").relative().size_full().child(measure).child(content);
        if moving {
            // The cover grows in whole APs from the player bar cover to the slot.
            let ap_dp = to_dp(Ap::of(window).one());
            let from =
                (SPACE_5 - ox, vh - PLAYER_BAR_H + (PLAYER_BAR_H - ART_SHORTCUT) / 2.0 - oy, ART_SHORTCUT);
            let side = layout.media_w.min(layout.media_h);
            let to = (layout.media_x + (layout.media_w - side) / 2.0, layout.media_y, side);
            let x = step(from.0, to.0, reveal, ap_dp);
            let y = step(from.1, to.1, reveal, ap_dp);
            let s = step(from.2, to.2, reveal, ap_dp).max(ART_SHORTCUT);
            root = root.child(
                div()
                    .absolute()
                    .left(dp(x))
                    .top(dp(y))
                    .child(Cover::new(s).url(now.cover_url.clone()).large()),
            );
        }
        root.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stage_fits_the_window_and_caps_the_cover() {
        let wide = Layout::of(2560.0, 1440.0 - 41.0, false);
        assert_eq!(wide.media_w, ART_STAGE_MAX, "the cover stops at 720 dp");
        assert_eq!(wide.media_w, wide.media_h);
        assert!(wide.media_y + wide.media_h + UNDER_MEDIA <= wide.content_h + 0.01);
        assert!(wide.lyrics_x + wide.lyrics_w <= 2560.0 + 0.01);
        let small = Layout::of(1280.0, 800.0 - 41.0, false);
        assert!(small.media_w >= MEDIA_MIN && small.media_w < ART_STAGE_MAX);
        assert!(small.media_y + small.media_h + UNDER_MEDIA <= small.content_h + 0.01);
        assert!(small.info_w >= INFO_MIN);
    }

    #[test]
    fn the_clip_takes_the_slot_at_sixteen_by_nine() {
        for (w, h) in [(1280.0, 759.0), (2560.0, 1399.0), (1000.0, 560.0)] {
            let layout = Layout::of(w, h, true);
            assert!((layout.media_w / layout.media_h - 16.0 / 9.0).abs() < 0.02, "{w} x {h}");
            assert!(layout.media_w <= layout.left_w - 2.0 * layout.margin + 0.01);
        }
    }

    #[test]
    fn the_flight_lands_on_whole_aps() {
        assert_eq!(step(0.0, 100.0, 0.0, 2.0), 0.0);
        assert_eq!(step(0.0, 100.0, 1.0, 2.0), 100.0);
        let mid = step(0.0, 100.0, 0.37, 2.0);
        assert_eq!(mid % 2.0, 0.0);
    }
}
