//! The «in riproduzione» bar (72 dp at the bottom): cover, title and artist, the heart; transport and seek bar
//! in the centre; lyrics, queue, volume and the stage on the right (the art direction's order). Every command
//! goes to the core. The cover opens the side panel on *In riproduzione*.
//!
//! With the `video` feature and a track that has a clip, «Videoclip» left of the lyrics button turns the clip
//! on in the side panel's media slot (lit while the clip is on screen).
//!
//! In a jam, left of the lyrics button, the jam mark: the group and the `LIVE` badge, which opens the Jam page;
//! while connecting the group has the loader. It changes only with the phase, the code or the head count.
//!
//! A track change cross-fades the cover in `MOTION_BASE` and raises the title and artist 2 AP, asking for
//! frames only while that runs.
//!
//! The position is not in the snapshot: it comes from `clock.at(now)` at every draw. A 250 ms timer
//! (`PROGRESS_TICK_MS`) redraws it and **exists only while playing** and the bar is visible: paused, stopped,
//! loading or under the stage there is no timer and no frame.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
};
use psytify_core::api::Track;
use psytify_core::command::{LibraryCmd, PlaybackCmd};
use psytify_core::player::{PlaybackState, Repeat as QueueRepeat};
use psytify_core::snapshot::{JamPhase, JamView, PlaybackView};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    SURFACE_CHROME, TEXT_ERROR, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
};
use psytify_design::tokens::motion::{MOTION_BASE, PROGRESS_TICK_MS, ease_out};
use psytify_design::tokens::size::{PLAYER_BAR_H, cover::ART_SHORTCUT};
use psytify_design::tokens::space::{SPACE_3, SPACE_4, SPACE_5};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_NUMERIC, TYPE_TRACK_TITLE};
use uuid::Uuid;

use super::SidePanel;
use crate::components::badge::{Badge, BadgeKind};
use crate::components::icon_button::{IconButton, OnStyle};
use crate::components::like_button::LikeButton;
use crate::components::player_control::{PlayerControl, Repeat, Transport};
use crate::components::progress::ProgressBar;
use crate::components::tile::Cover;
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::motion::{ap_round, keep_running};
use crate::theme::{Ap, TypeStyle, col, dp};

/// What the bar asks of the window.
pub enum BarEvent {
    ToggleQueue,
    /// The lyrics button: the side panel on *Testo*.
    ToggleLyrics,
    /// The cover: the side panel on *In riproduzione*.
    ToggleNow,
    /// The stage (F).
    ToggleFocus,
    /// The playing track's artist name: opens the artist page.
    OpenArtist(uuid::Uuid),
    /// The jam mark: opens the Jam page.
    OpenJam,
    /// «Videoclip»: the clip in the media slot, `video` feature only.
    #[cfg(feature = "video")]
    ToggleVideo,
}

/// The jam sign in the bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum JamChip {
    /// No jam: nothing.
    #[default]
    None,
    Connecting,
    /// In a room: the code and how many people (for the hint).
    Live {
        tip: SharedString,
    },
}

/// The jam sign from its view. Pure.
pub fn jam_chip(view: &JamView) -> JamChip {
    match view.phase {
        JamPhase::Idle => JamChip::None,
        JamPhase::Connecting => JamChip::Connecting,
        JamPhase::Open => {
            let people = match view.members.len() {
                1 => "1 persona".to_owned(),
                n => format!("{n} persone"),
            };
            let code = view.code.as_deref().unwrap_or("…");
            JamChip::Live { tip: format!("Jam {code} · {people}").into() }
        }
    }
}

impl EventEmitter<BarEvent> for PlayerBar {}

impl PlayerBar {
    /// «Videoclip», only when the playing track has one.
    #[cfg(feature = "video")]
    fn video_button(&self, cx: &mut Context<Self>) -> Option<IconButton> {
        self.now.as_ref().filter(|now| now.track.has_video)?;
        Some(
            IconButton::new("video-toggle", sprite::ICON_VIDEO)
                .selected(self.video_shown)
                .on_style(OnStyle::Active)
                .tooltip("Videoclip", Some("V"))
                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(BarEvent::ToggleVideo))),
        )
    }

    #[cfg(not(feature = "video"))]
    fn video_button(&self, _: &mut Context<Self>) -> Option<IconButton> {
        None
    }
}

/// The playing track, with the texts already ready.
struct Now {
    track: Track,
    title: SharedString,
    artist: SharedString,
    total: SharedString,
    cover_url: Option<SharedString>,
    duration: Duration,
}

impl Now {
    fn of(track: &Track) -> Self {
        Self {
            track: track.clone(),
            title: track.title.clone().into(),
            artist: track.artist.name.clone().into(),
            total: format::clock_ms(track.duration_ms).into(),
            cover_url: track.cover_url.clone().map(SharedString::from),
            duration: track.duration(),
        }
    }
}

pub struct PlayerBar {
    core: Entity<CoreModel>,
    view: Arc<PlaybackView>,
    liked: Arc<HashSet<Uuid>>,
    now: Option<Now>,
    /// The open side panel (its button is lit).
    panel: Option<SidePanel>,
    /// The stage covers the bar: no tick.
    hidden: bool,
    /// The clip is on screen somewhere (the «Videoclip» button is lit).
    #[cfg(feature = "video")]
    video_shown: bool,
    /// The cover of the track before a change and when the change began: the cross-fade.
    swap: Option<(Option<SharedString>, Instant)>,
    /// The 250 ms timer: it runs only while the track plays.
    tick: Option<Task<()>>,
    /// The elapsed time text, rebuilt only when the second changes.
    elapsed: (u64, SharedString),
    /// The jam sign.
    jam: JamChip,
    _changes: Subscription,
}

impl PlayerBar {
    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let snapshot = core.read(cx).snapshot();
        let changes =
            cx.subscribe(&core, |this, core, changed: &Changed, cx| this.core_changed(&core, *changed, cx));
        let mut bar = Self {
            now: None,
            view: Arc::clone(&snapshot.playback),
            liked: core.read(cx).liked(),
            core,
            panel: None,
            hidden: false,
            #[cfg(feature = "video")]
            video_shown: false,
            swap: None,
            tick: None,
            elapsed: (u64::MAX, SharedString::default()),
            jam: JamChip::None,
            _changes: changes,
        };
        bar.jam = jam_chip(&bar.core.read(cx).jam_view());
        bar.rebuild_now(false);
        bar.sync_tick(cx);
        bar
    }

    /// The window says which panel is open (its button lights up).
    pub fn set_panel(&mut self, panel: Option<SidePanel>, cx: &mut Context<Self>) {
        if self.panel != panel {
            self.panel = panel;
            cx.notify();
        }
    }

    /// The clip is on screen (or not): lights «Videoclip».
    #[cfg(feature = "video")]
    pub fn set_video_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.video_shown != shown {
            self.video_shown = shown;
            cx.notify();
        }
    }

    /// The immersive view covers the bar (or uncovers it): covered, it has no beat.
    pub fn set_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
        if self.hidden != hidden {
            self.hidden = hidden;
            self.sync_tick(cx);
            cx.notify();
        }
    }

    fn core_changed(&mut self, core: &Entity<CoreModel>, changed: Changed, cx: &mut Context<Self>) {
        let mut dirty = false;
        if changed.playback {
            let snapshot = core.read(cx).snapshot();
            let same_queue = Arc::ptr_eq(&snapshot.playback.queue, &self.view.queue);
            self.view = Arc::clone(&snapshot.playback);
            if !same_queue {
                let animate = !crate::motion::reduced(cx);
                self.rebuild_now(animate);
            }
            self.sync_tick(cx);
            dirty = true;
        }
        if changed.library {
            self.liked = core.read(cx).liked();
            dirty = true;
        }
        if changed.jam {
            let chip = jam_chip(&core.read(cx).jam_view());
            if chip != self.jam {
                self.jam = chip;
                dirty = true;
            }
        }
        if dirty {
            cx.notify();
        }
    }

    /// The playing track's texts; a different track starts the cross-fade from the old cover.
    fn rebuild_now(&mut self, animate: bool) {
        let next = self.view.queue.current().map(Now::of);
        let old = self.now.as_ref().map(|now| (now.track.id, now.cover_url.clone()));
        let changed = old.as_ref().map(|(id, _)| *id) != next.as_ref().map(|now| now.track.id);
        if animate
            && changed
            && let Some((_, cover)) = old
        {
            self.swap = Some((cover, Instant::now()));
        }
        self.now = next;
    }

    /// Progress of the track-change move (0..=1), or `None` at rest.
    fn swap_progress(&mut self, now: Instant) -> Option<f32> {
        let (_, start) = self.swap.as_ref()?;
        let t = now.saturating_duration_since(*start).as_secs_f32() * 1000.0 / MOTION_BASE.ms as f32;
        if t >= 1.0 {
            self.swap = None;
            return None;
        }
        Some(ease_out(t.max(0.0)))
    }

    /// Turns the timer on while it plays (and the bar is visible) and off otherwise.
    fn sync_tick(&mut self, cx: &mut Context<Self>) {
        if !self.view.is_playing() || self.hidden {
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

    fn elapsed_label(&mut self, position: Duration) -> SharedString {
        let secs = position.as_secs();
        if self.elapsed.0 != secs {
            self.elapsed = (secs, format::clock(position).into());
        }
        self.elapsed.1.clone()
    }
}

impl Render for PlayerBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let swap = self.swap_progress(Instant::now());
        keep_running(window, swap.is_some());
        let ap = Ap::of(window);
        let view = Arc::clone(&self.view);
        let active = view.is_active();
        let position = if active { view.position(Instant::now()) } else { Duration::ZERO };
        let duration = self.now.as_ref().map_or(Duration::ZERO, |now| now.duration);
        let fraction = if duration.is_zero() { 0.0 } else { position.as_secs_f32() / duration.as_secs_f32() };
        let elapsed = self.elapsed_label(position);
        let total = self.now.as_ref().map_or_else(|| SharedString::from("0:00"), |now| now.total.clone());

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
        let volume = if view.muted { 0.0 } else { view.volume };
        let volume_icon = if volume <= 0.0 {
            sprite::ICON_MUTE
        } else if volume < 0.5 {
            sprite::ICON_VOLUME_LOW
        } else {
            sprite::ICON_VOLUME
        };

        let core = self.core.clone();
        let command = move |command: PlaybackCmd| {
            let core = core.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| core.read(cx).playback(command.clone())
        };

        // Left: cover, texts, heart.
        let open_now = cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(BarEvent::ToggleNow));
        let left = match &self.now {
            Some(now) => {
                let liked = self.liked.contains(&now.track.id);
                let track = now.track.clone();
                let core = self.core.clone();
                let status: gpui::AnyElement = match &view.state {
                    PlaybackState::Error(message) => div()
                        .type_role(TYPE_ARTIST)
                        .text_color(col(TEXT_ERROR))
                        .truncate()
                        .child(SharedString::from(message.clone()))
                        .into_any_element(),
                    _ => {
                        let artist = now.track.artist.id;
                        div()
                            .flex()
                            .min_w_0()
                            .child(
                                div()
                                    .id("bar-artist")
                                    .type_role(TYPE_ARTIST)
                                    .text_color(col(TEXT_SECONDARY))
                                    .truncate()
                                    .cursor_pointer()
                                    .hover(|style| style.underline())
                                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                        cx.emit(BarEvent::OpenArtist(artist));
                                    }))
                                    .child(now.artist.clone()),
                            )
                            .into_any_element()
                    }
                };
                // The new cover fades in over the old one; title and artist rise 2 AP.
                let mut cover = div()
                    .id("bar-cover")
                    .relative()
                    .flex_none()
                    .size(dp(ART_SHORTCUT))
                    .cursor_pointer()
                    .on_click(open_now);
                let fade = swap.unwrap_or(1.0);
                if let (Some(t), Some((old, _))) = (swap, self.swap.as_ref()) {
                    cover = cover.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .opacity(1.0 - t)
                            .child(Cover::new(ART_SHORTCUT).url(old.clone())),
                    );
                }
                cover = cover.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .opacity(fade)
                        .child(Cover::new(ART_SHORTCUT).url(now.cover_url.clone())),
                );
                let rise = swap.map_or(0, |t| ap_round((1.0 - t) * 2.0));
                div()
                    .flex()
                    .items_center()
                    .gap(dp(SPACE_4))
                    .child(cover)
                    .child(
                        div()
                            .relative()
                            .top(ap.n(rise))
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .type_role(TYPE_TRACK_TITLE)
                                    .text_color(col(TEXT_PRIMARY))
                                    .truncate()
                                    .child(now.title.clone()),
                            )
                            .child(status),
                    )
                    .child(LikeButton::new("like", liked).on_click(move |_, _, cx| {
                        core.read(cx).library(LibraryCmd::ToggleLike(track.clone()));
                    }))
                    .into_any_element()
            }
            None => div()
                .flex()
                .items_center()
                .gap(dp(SPACE_4))
                .child(Cover::new(ART_SHORTCUT))
                .child(div().type_role(TYPE_ARTIST).text_color(col(TEXT_TERTIARY)).child("Nessun brano"))
                .into_any_element(),
        };

        let seek_core = self.core.clone();
        let volume_core = self.core.clone();
        let commit_core = self.core.clone();
        let mute_core = self.core.clone();
        let toggle_queue = cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(BarEvent::ToggleQueue));
        let toggle_lyrics = cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(BarEvent::ToggleLyrics));
        let toggle_focus = cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(BarEvent::ToggleFocus));

        let center = div()
            .flex_1()
            .min_w_0()
            .max_w(dp(640.0))
            .flex()
            .flex_col()
            .items_center()
            .child(
                PlayerControl::new("transport")
                    .transport(transport)
                    .shuffle(queue.shuffle())
                    .repeat(repeat)
                    .disabled(queue.is_empty())
                    .on_play(command(PlaybackCmd::Toggle))
                    .on_previous(command(PlaybackCmd::Previous))
                    .on_next(command(PlaybackCmd::Next))
                    .on_shuffle(command(PlaybackCmd::ToggleShuffle))
                    .on_repeat(command(PlaybackCmd::CycleRepeat)),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(dp(SPACE_3))
                    .child(
                        div()
                            .w(dp(40.0))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .type_role(TYPE_NUMERIC)
                            .text_color(col(TEXT_TERTIARY))
                            .child(elapsed),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            ProgressBar::new("seek", fraction)
                                .disabled(!active)
                                .label(move |fraction| format::clock(duration.mul_f32(fraction)))
                                .on_change(move |fraction, _, cx| {
                                    seek_core.read(cx).playback(PlaybackCmd::SeekFraction(fraction));
                                }),
                        ),
                    )
                    .child(
                        div()
                            .w(dp(40.0))
                            .flex_none()
                            .type_role(TYPE_NUMERIC)
                            .text_color(col(TEXT_TERTIARY))
                            .child(total),
                    ),
            );

        let jam = match &self.jam {
            JamChip::None => None,
            JamChip::Connecting => Some(
                IconButton::new("jam-chip", sprite::ICON_PEOPLE)
                    .loading(true)
                    .tooltip("Jam: connessione in corso…", None)
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(BarEvent::OpenJam)))
                    .into_any_element(),
            ),
            JamChip::Live { tip } => Some(
                div()
                    .flex()
                    .items_center()
                    .child(
                        IconButton::new("jam-chip", sprite::ICON_PEOPLE)
                            .tooltip(tip.clone(), None)
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(BarEvent::OpenJam))),
                    )
                    .child(Badge::new("LIVE", BadgeKind::Live))
                    .into_any_element(),
            ),
        };
        let right = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(dp(SPACE_3))
            .children(jam)
            .children(self.video_button(cx))
            .child(
                IconButton::new("lyrics-toggle", sprite::ICON_LYRICS)
                    .selected(self.panel == Some(SidePanel::Lyrics))
                    .on_style(OnStyle::Active)
                    .tooltip("Testo", Some("T"))
                    .on_click(toggle_lyrics),
            )
            .child(
                IconButton::new("queue-toggle", sprite::ICON_QUEUE)
                    .selected(self.panel == Some(SidePanel::Queue))
                    .on_style(OnStyle::Active)
                    .tooltip("Coda", Some("Q"))
                    .on_click(toggle_queue),
            )
            .child(
                IconButton::new("mute", volume_icon)
                    .tooltip(if volume <= 0.0 { "Riattiva l'audio" } else { "Muto" }, None)
                    .on_click(move |_, _, cx| mute_core.read(cx).playback(PlaybackCmd::ToggleMute)),
            )
            .child(
                ProgressBar::slider("volume", volume)
                    .label(|fraction| format!("{}%", (fraction * 100.0).round()))
                    .on_change(move |level, _, cx| volume_core.read(cx).playback(PlaybackCmd::Volume(level)))
                    .on_commit(move |_, _, cx| commit_core.read(cx).playback(PlaybackCmd::Save)),
            )
            .child(
                IconButton::new("focus-toggle", sprite::ICON_FOCUS)
                    .disabled(self.now.is_none())
                    .tooltip("Palco", Some("F"))
                    .on_click(toggle_focus),
            );

        // The line above the bar is drawn by `Shell`: the bar enters `.cached()` at its exact height.
        div()
            .size_full()
            .h(dp(PLAYER_BAR_H))
            .bg(col(SURFACE_CHROME))
            .flex()
            .items_center()
            .gap(dp(SPACE_5))
            .px(dp(SPACE_5))
            // The sides share what the centre (640 dp at most) leaves, so the transport stays centred and
            // the right-hand controls reach the edge on wide windows.
            .child(div().flex_1().min_w_0().child(div().max_w(dp(360.0)).child(left)))
            .child(center)
            .child(div().flex_1().min_w_0().flex().justify_end().child(right))
    }
}

#[cfg(test)]
mod tests {
    use psytify_core::jam::Member;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn the_jam_mark_follows_the_phase_of_the_jam() {
        assert_eq!(jam_chip(&JamView::default()), JamChip::None);
        let connecting = JamView { phase: JamPhase::Connecting, ..JamView::default() };
        assert_eq!(jam_chip(&connecting), JamChip::Connecting);
        let member = |n: u128| Member { id: Uuid::from_u128(n), name: format!("m{n}") };
        let mut open = JamView {
            phase: JamPhase::Open,
            code: Some("K7Q2MX".to_owned()),
            members: Arc::from(vec![member(1)]),
            ..JamView::default()
        };
        assert_eq!(jam_chip(&open), JamChip::Live { tip: "Jam K7Q2MX · 1 persona".into() });
        open.members = Arc::from(vec![member(1), member(2), member(3)]);
        assert_eq!(jam_chip(&open), JamChip::Live { tip: "Jam K7Q2MX · 3 persone".into() });
    }
}
