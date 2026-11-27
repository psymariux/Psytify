//! The side panel's *In riproduzione* tab (desktop v2 §5): the media slot (the 280 dp cover, or the clip at
//! 16:9 in the same slot), the title in Title, the artist link and the badges, the live lyrics excerpt (the
//! current line and the next four; a click opens *Testo*), then «Prossimo» (the next five tracks).
//!
//! No timer of its own: the excerpt is a `LyricsView` with its next-change timer, which only runs while this
//! tab is on screen and the track plays; the clip's frames are painted by the `VideoStage` over the well.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext, ClickEvent, Context, Entity, EventEmitter, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
};
use psytify_core::api::Track;
use psytify_core::command::PlaybackCmd;
use psytify_core::player::Queue;
use psytify_design::tokens::color::{TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY};
use psytify_design::tokens::size::cover::ART_PANEL;
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_HEADING, TYPE_LABEL, TYPE_META};
use uuid::Uuid;

use super::lyrics_view::{Look, LyricsEvent, LyricsView};
use super::media::{Slot, media_slot};
use crate::components::badge::{Badge, BadgeKind};
use crate::components::now_playing::{UpNext, UpNextRow};
use crate::core_model::{Changed, CoreModel};
use crate::theme::{TypeStyle, cased, col, dp};
#[cfg(feature = "video")]
use crate::video::stage::{StageEvent, Surface, VideoStage};

/// Rows of «Prossimo» in the panel.
pub const UP_NEXT_PANEL: usize = 5;

/// What the tab asks of the window.
pub enum NowEvent {
    /// The excerpt was clicked: *Testo*.
    OpenLyrics,
    OpenArtist(Uuid),
    /// The Cover / Video switch.
    Video(bool),
}

impl EventEmitter<NowEvent> for NowPanel {}

/// The playing track's texts, ready made.
#[derive(Clone, Debug, PartialEq)]
pub struct NowTrack {
    pub id: Uuid,
    pub title: SharedString,
    pub artist: SharedString,
    pub artist_id: Uuid,
    pub cover_url: Option<SharedString>,
    pub has_video: bool,
    /// `LOSSLESS` or the format (`M4A`, `MP3`).
    pub quality: Option<(SharedString, BadgeKind)>,
}

impl NowTrack {
    pub fn of(track: &Track) -> Self {
        let lossless = matches!(track.audio_format.as_str(), "flac" | "wav" | "alac" | "aiff");
        let quality = if lossless {
            Some((SharedString::new_static("Lossless"), BadgeKind::Quality))
        } else if track.audio_format.is_empty() {
            None
        } else {
            Some((SharedString::from(track.audio_format.to_uppercase()), BadgeKind::Neutral))
        };
        Self {
            id: track.id,
            title: track.title.clone().into(),
            artist: track.artist.name.clone().into(),
            artist_id: track.artist.id,
            cover_url: track.cover_url.clone().map(SharedString::from),
            has_video: track.has_video,
            quality,
        }
    }
}

/// The next `count` tracks after the playing one, in play order. Pure.
pub fn up_next(queue: &Queue, count: usize) -> Vec<UpNext> {
    queue
        .upcoming()
        .take(count)
        .map(|(position, track)| UpNext {
            position,
            title: track.title.clone().into(),
            artist: track.artist.name.clone().into(),
            cover_url: track.cover_url.clone().map(SharedString::from),
        })
        .collect()
}

pub struct NowPanel {
    core: Entity<CoreModel>,
    queue: Arc<Queue>,
    now: Option<NowTrack>,
    next: Vec<UpNext>,
    lyrics: Entity<LyricsView>,
    /// The clip is on in the media slot.
    video: bool,
    visible: bool,
    #[cfg(feature = "video")]
    stage: Entity<VideoStage>,
    #[cfg(feature = "video")]
    spot: crate::video::stage::Spot,
    _subscriptions: Vec<Subscription>,
}

impl NowPanel {
    pub fn new(
        core: Entity<CoreModel>,
        #[cfg(feature = "video")] stage: Entity<VideoStage>,
        cx: &mut Context<Self>,
    ) -> Self {
        let lyrics = cx.new(|cx| LyricsView::new(core.clone(), Look::Excerpt, cx));
        #[cfg_attr(not(feature = "video"), allow(unused_mut))]
        let mut subscriptions = vec![
            cx.subscribe(&core, |this, core, changed: &Changed, cx| {
                if changed.playback {
                    this.refresh(&core, cx);
                }
            }),
            cx.subscribe(&lyrics, |_, _, event: &LyricsEvent, cx| match event {
                LyricsEvent::Open => cx.emit(NowEvent::OpenLyrics),
            }),
        ];
        #[cfg(feature = "video")]
        subscriptions.push(cx.subscribe(&stage, |this, _, event: &StageEvent, cx| {
            if matches!(event, StageEvent::Status) && this.visible {
                cx.notify();
            }
        }));
        let mut panel = Self {
            core: core.clone(),
            queue: Arc::default(),
            now: None,
            next: Vec::new(),
            lyrics,
            video: false,
            visible: false,
            #[cfg(feature = "video")]
            spot: stage.read(cx).spot(Surface::Panel),
            #[cfg(feature = "video")]
            stage,
            _subscriptions: subscriptions,
        };
        panel.refresh(&core, cx);
        panel
    }

    fn refresh(&mut self, core: &Entity<CoreModel>, cx: &mut Context<Self>) {
        let queue = Arc::clone(&core.read(cx).snapshot().playback.queue);
        if Arc::ptr_eq(&queue, &self.queue) {
            return;
        }
        self.now = queue.current().map(NowTrack::of);
        self.next = up_next(&queue, UP_NEXT_PANEL);
        self.queue = queue;
        cx.notify();
    }

    /// On screen or not: the excerpt has its timer only while the tab shows.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible != visible {
            self.visible = visible;
            self.lyrics.update(cx, |lyrics, cx| lyrics.set_visible(visible, cx));
        }
    }

    /// The clip on or off in the media slot.
    pub fn set_video(&mut self, video: bool, cx: &mut Context<Self>) {
        if self.video != video {
            self.video = video;
            cx.notify();
        }
    }
}

impl Render for NowPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(feature = "video")]
        self.spot.clear();
        let Some(now) = self.now.clone() else {
            return div()
                .size_full()
                .p(dp(SPACE_5))
                .type_role(TYPE_META)
                .text_color(col(TEXT_SECONDARY))
                .child("Scegli un brano: qui vedrai copertina, testo e cosa viene dopo.")
                .into_any_element();
        };
        let entity = cx.entity().downgrade();
        let video = self.video && now.has_video;
        let slot = Slot {
            id: "panel-media",
            width: ART_PANEL,
            cover_url: now.cover_url.clone(),
            large: false,
            has_video: now.has_video,
            video,
            on_toggle: Rc::new(move |video, _, cx| {
                entity.update(cx, |_, cx| cx.emit(NowEvent::Video(video))).ok();
            }),
        };
        let media = media_slot(
            slot,
            #[cfg(feature = "video")]
            Some((&self.stage, Surface::Panel)),
            window,
            cx,
        );
        #[cfg(feature = "video")]
        let status =
            video.then(|| super::media::video_status(&self.stage, "panel-video-retry", cx)).flatten();
        #[cfg(not(feature = "video"))]
        let status: Option<gpui::AnyElement> = None;

        let artist_id = now.artist_id;
        let badges = div()
            .flex()
            .gap(dp(SPACE_2))
            .children(now.quality.clone().map(|(text, kind)| Badge::new(text, kind)));
        let info = div()
            .flex()
            .flex_col()
            .gap(dp(SPACE_2))
            .child(
                div()
                    .type_role(TYPE_HEADING)
                    .text_color(col(TEXT_PRIMARY))
                    .line_clamp(2)
                    .child(now.title.clone()),
            )
            .child(
                div().flex().min_w_0().child(
                    div()
                        .id("now-artist")
                        .type_role(TYPE_ARTIST)
                        .text_color(col(TEXT_SECONDARY))
                        .truncate()
                        .cursor_pointer()
                        .hover(|style| style.underline().text_color(col(TEXT_PRIMARY)))
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(NowEvent::OpenArtist(artist_id))
                        }))
                        .child(now.artist.clone()),
                ),
            )
            .child(badges);

        let core = self.core.clone();
        let next = self.next.iter().enumerate().map(|(ix, item)| {
            let position = item.position;
            let core = core.clone();
            UpNextRow::new(("now-next", ix), item.clone())
                .on_click(move |_, _, cx| core.read(cx).playback(PlaybackCmd::PlayAt(position)))
        });
        let up_next = (!self.next.is_empty()).then(|| {
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
                .child(div().flex().flex_col().mx(dp(-SPACE_2)).children(next))
        });

        div()
            .id("now-panel")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(dp(SPACE_5))
                    .px(dp(SPACE_5 + 4.0))
                    .pt(dp(SPACE_5 + 4.0))
                    .pb(dp(SPACE_6))
                    .child(div().flex().flex_col().gap(dp(SPACE_3)).child(media).children(status))
                    .child(info)
                    .child(self.lyrics.clone())
                    .children(up_next),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn up_next_follows_the_play_order_and_stops_at_the_count() {
        let queue = Queue::default();
        assert!(up_next(&queue, UP_NEXT_PANEL).is_empty());
    }
}
