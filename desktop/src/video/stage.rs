//! `VideoStage`: the entity that holds the playing track's pipeline and draws its frames. One per window: the
//! clip shows in one media slot at a time, the side panel's or the stage's, and `Shell` says which
//! (`set_surface`).
//!
//! - **Where it draws.** The stage is `Shell`'s last child (after page, panels and the stage view), not a child
//!   of the view that shows it: a new frame redraws the stage and `Shell`, which GPUI redraws anyway as the
//!   root, and the cached views are reused (BACKLOG 41). The view showing the clip puts a well (`well`) in its
//!   media slot that marks the place (`Spot`) while it paints; the stage paints the frame there in the same
//!   draw, then the dissolve veil the slot asked for, if any. `Shell` must not sit in `.cached()` under another
//!   root.
//! - **What to open** comes from the core (`VideoView`, read as `ClipView`), **when** from the pure rules of
//!   `plan`: a pipeline only for the playing track, while it plays or loads and while a slot shows it. Opening
//!   and closing block and run on their own thread; a better version (360p to 720p) opens beside and takes
//!   over without stopping. Paused, the pipeline closes and the last frame stays; on resume it reopens at the
//!   audio position.
//! - **Frames.** GStreamer puts the latest in the slot (`slot`) and wakes the UI once (`Notify`): the wake
//!   notifies only this view, which takes the frame in its draw, makes a `RenderImage` and drops the previous
//!   one from the atlas. A late UI skips frames, never queues them.
//! - **Sync.** The clock is the audio's (`PlaybackView::clock`): `Alignment::follow` on every playback
//!   publication and `Alignment::align` on the frame wake, at most every 250 ms (no extra timer).
//! - **Failures.** A pipeline that does not open or stops is `VideoCmd::Failed { seq }`: the core asks for
//!   another address once, then blocks the clip, shown with «Riprova» (`retry`).

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use crate::pixel::shape::paint_dither_veil;
use crate::theme::Ap;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, Bounds, ContentMask, Context, Corners, Div, Entity, EventEmitter, IntoElement, ParentElement,
    Pixels, Render, RenderImage, SharedString, Styled, Subscription, Task, Window, canvas, div, point, px,
    size,
};
use image::{Frame as ImageFrame, RgbaImage};
use psytify_core::command::VideoCmd;
use psytify_core::player::PlaybackState;
use psytify_core::snapshot::PlaybackView;
use psytify_core::video::Alignment;
use psytify_design::tokens::color::SURFACE_WELL;
use smallvec::SmallVec;
use tokio::sync::oneshot;
use uuid::Uuid;

use super::ClipView;
use super::pipeline::{Pipeline, Shared};
use super::plan::{self, Action, Current, Inputs, Want};
use super::size::Size;
use crate::components::tile::Cover;
use crate::core_model::{Changed, CoreModel};
use crate::notices::Notice;
use crate::theme::{col, dp};

/// The media slot that shows the clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// The side panel's *In riproduzione* slot.
    Panel,
    /// The stage's slot.
    Stage,
}

/// The frame's place in a box: the well (`VideoStage::well`) marks it while it is painted, with the
/// mask that cuts it, and the stage reads it when it paints, right after. The view that contains the well
/// empties it at the start of each draw (`clear`): reused from the cache, the place stays the previous one, which is
/// still right (GPUI reuses a view only if it has the same bounds and the same mask).
#[derive(Clone, Default)]
pub struct Spot(Rc<Cell<Option<Place>>>);

/// The slot's bounds, the mask that clips it and the dissolve veil (cells covered of every 4).
type Place = (Bounds<Pixels>, ContentMask<Pixels>, u8);

impl Spot {
    pub fn clear(&self) {
        self.0.set(None);
    }
}

/// What the video box shows (it changes rarely: not at every frame).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// The track has no video, or there is no track.
    None,
    /// The cover, without waiting: paused before the first frame.
    Cover,
    /// The server is preparing the video, or the pipeline is opening.
    Loading,
    /// The video (also stopped on the last frame, paused).
    Playing,
    /// Did not open, not even at the second attempt: "Il videoclip non si è aperto." with «Riprova» (the
    /// testo dell'errore del core va nell'avviso).
    Failed,
}

/// What the stage says to whoever shows it.
pub enum StageEvent {
    /// `status` has changed.
    Status,
    /// A video stopped: the notice (the notices are shown by `Shell`).
    Notice(Notice),
}

impl EventEmitter<StageEvent> for VideoStage {}

/// The track's video pipeline that is wanted, open or opening.
struct Running {
    want: Want,
    pipeline: Option<Pipeline>,
    shared: Arc<Shared>,
    alignment: Alignment,
    last_check: Option<Instant>,
    /// The `seq` of the better version that opens alongside.
    upgrading: Option<u64>,
    _open: Option<Task<()>>,
    _upgrade: Option<Task<()>>,
    /// Waits for GStreamer's wake-ups: there only with the pipeline open.
    _wake: Option<Task<()>>,
}

impl Running {
    fn current(&self) -> Current {
        Current {
            track: self.want.track,
            seq: self.want.seq,
            open: self.pipeline.is_some(),
            upgrading: self.upgrading,
        }
    }

    /// Closes everything on a thread of its own; the tasks end with it.
    fn discard(self) {
        if let Some(pipeline) = self.pipeline {
            pipeline.discard();
        }
    }
}

pub struct VideoStage {
    core: Entity<CoreModel>,
    playback: Arc<PlaybackView>,
    view: Arc<ClipView>,
    /// Il brano in corso, se ha un videoclip, con la sua copertina.
    track: Option<(Uuid, Option<SharedString>)>,
    surface: Option<Surface>,
    /// I pixel fisici del riquadro: li dice chi lo mostra, servono a scegliere quanto decodificare.
    surface_px: Size,
    /// The core knows that a spot shows the video (`VideoCmd::Show`).
    shown_sent: bool,
    running: Option<Running>,
    /// The last frame drawn and the track it belongs to.
    image: Option<(Uuid, Arc<RenderImage>)>,
    /// Immagini da togliere dall'atlante al prossimo disegno.
    retired: Vec<Arc<RenderImage>>,
    /// The places of the panel and of the immersive view.
    panel_spot: Spot,
    stage_spot: Spot,
    status: Status,
    /// The stopped videos already announced with a notice.
    failures_seen: u64,
    _changes: Subscription,
}

impl VideoStage {
    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let changes = cx.subscribe(&core, |this, core, changed: &Changed, cx| {
            if changed.playback || changed.video {
                this.core_changed(&core, *changed, cx);
            }
        });
        let playback = Arc::clone(&core.read(cx).snapshot().playback);
        let view = core.read(cx).clip_view();
        let mut stage = Self {
            failures_seen: view.failures,
            track: video_track(&playback),
            playback,
            view,
            core,
            surface: None,
            surface_px: Size::new(0, 0),
            shown_sent: false,
            running: None,
            image: None,
            retired: Vec::new(),
            panel_spot: Spot::default(),
            stage_spot: Spot::default(),
            status: Status::None,
            _changes: changes,
        };
        stage.status = stage.compute_status();
        stage
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// The spot that shows the video (or none). The core asks the server for the video only while a
    /// spot shows it, and prepares those of the next tracks in advance only then.
    pub fn set_surface(&mut self, surface: Option<Surface>, cx: &mut Context<Self>) {
        if self.surface == surface {
            return;
        }
        self.surface = surface;
        let shown = surface.is_some();
        if shown != self.shown_sent {
            self.shown_sent = shown;
            self.send(VideoCmd::PrepareAhead(shown), cx);
            self.send(VideoCmd::Show(shown), cx);
        }
        if !shown {
            // Nessuno lo guarda: l'ultimo fotogramma lascia l'atlante al prossimo disegno del palco.
            if let Some((_, image)) = self.image.take() {
                self.retired.push(image);
            }
        }
        self.reconcile(cx);
        cx.notify();
    }

    /// The physical pixels of the box: whoever shows the stage says them at every draw (the pipeline renegotiates the
    /// measure only when it changes enough).
    pub fn set_surface_px(&mut self, surface: Size) {
        self.surface_px = surface;
    }

    /// "Riprova" on a video that did not open.
    pub fn retry(&self, cx: &App) {
        self.send(VideoCmd::Retry, cx);
    }

    /// The frame's place in `surface`: the view that shows the box empties it at each of its draws.
    pub fn spot(&self, surface: Surface) -> Spot {
        match surface {
            Surface::Panel => self.panel_spot.clone(),
            Surface::Stage => self.stage_spot.clone(),
        }
    }

    /// The well of the slot (`width` x `height` dp) in `surface`: the ground, the cover until a frame exists,
    /// and the mark of the place where the stage paints the frame and then `veil` (a dissolve step, 0..=4).
    pub fn well(&self, surface: Surface, width: f32, height: f32, veil: u8) -> Div {
        let spot = self.spot(surface);
        let marker = canvas(
            |_, _, _| (),
            move |bounds, (), window, _| spot.0.set(Some((bounds, window.content_mask(), veil))),
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let well = div()
            .relative()
            .flex_none()
            .w(dp(width))
            .h(dp(height))
            .overflow_hidden()
            .bg(col(SURFACE_WELL))
            .flex()
            .items_center()
            .justify_center();
        let cover = (!self.has_frame()).then(|| self.track.as_ref().and_then(|(_, url)| url.clone()));
        well.when_some(cover, |well, cover| well.child(Cover::new(height).url(cover))).child(marker)
    }

    fn send(&self, command: VideoCmd, cx: &App) {
        self.core.read(cx).video(command);
    }

    fn core_changed(&mut self, core: &Entity<CoreModel>, changed: Changed, cx: &mut Context<Self>) {
        if changed.playback {
            let next = Arc::clone(&core.read(cx).snapshot().playback);
            let previous = std::mem::replace(&mut self.playback, next);
            self.track = video_track(&self.playback);
            // The video follows the audio: a track starting, a seek, the resume.
            if let Some(running) = self.running.as_mut()
                && let Some(pipeline) = running.pipeline.as_mut()
                && let Some(adjust) = running.alignment.follow(&previous, &self.playback, Instant::now())
            {
                plan::apply(pipeline, adjust);
            }
        }
        if changed.video {
            self.view = core.read(cx).clip_view();
            if self.view.failures > self.failures_seen {
                self.failures_seen = self.view.failures;
                if let super::Clip::Failed(error) = &self.view.clip {
                    cx.emit(StageEvent::Notice(Notice::error(error.clone())));
                }
            }
        }
        self.reconcile(cx);
    }

    /// From the pipeline there is to the one needed (`plan`), then the state.
    fn reconcile(&mut self, cx: &mut Context<Self>) {
        let current = self.playback.queue.current().map(|track| track.id);
        let running = current.filter(|_| {
            matches!(self.playback.state, PlaybackState::Playing | PlaybackState::Loading { .. })
        });
        let ready = current
            .and_then(|track| self.view.ready_for(track).map(|source| Want { track, seq: source.seq }));
        let want = plan::wanted(Inputs { shown: self.surface.is_some(), running, ready });
        match plan::plan(self.running.as_ref().map(Running::current), want) {
            Action::Keep => {}
            Action::Close => self.close(),
            Action::Open(want) => {
                self.close();
                self.open(want, cx);
            }
            Action::Upgrade(want) => self.upgrade(want, cx),
        }
        // Another track: the frame of the previous track does not stay on the display.
        if let Some((track, _)) = &self.image
            && Some(*track) != current
            && let Some((_, image)) = self.image.take()
        {
            self.retired.push(image);
            cx.notify();
        }
        self.refresh_status(cx);
    }

    fn close(&mut self) {
        if let Some(running) = self.running.take() {
            running.discard();
        }
    }

    /// Opens on a thread of its own the address of the video `want`, at the audio's position.
    fn spawn_open(&self, want: Want) -> Option<(Arc<Shared>, oneshot::Receiver<Option<Pipeline>>)> {
        let url = self.view.ready_for(want.track)?.url().to_owned();
        let clock = self.playback.clock;
        let surface = Some(self.surface_px).filter(|size| size.width > 0 && size.height > 0);
        let shared = Arc::new(Shared::default());
        let (sender, receiver) = oneshot::channel();
        let for_thread = Arc::clone(&shared);
        let spawned = std::thread::Builder::new().name("video-open".into()).spawn(move || {
            let opened = Pipeline::open(&url, clock, surface, for_thread).ok();
            // Nobody waits any more (the track has changed): it is closed here, on this thread.
            if let Err(Some(pipeline)) = sender.send(opened) {
                drop(pipeline);
            }
        });
        spawned.ok()?;
        Some((shared, receiver))
    }

    fn open(&mut self, want: Want, cx: &mut Context<Self>) {
        let Some((shared, receiver)) = self.spawn_open(want) else {
            self.send(VideoCmd::Failed { seq: want.seq }, cx);
            return;
        };
        let task = cx.spawn(async move |this, cx| {
            let pipeline = receiver.await.ok().flatten();
            let _ = this.update(cx, |stage, cx| stage.opened(want, pipeline, cx));
        });
        self.running = Some(Running {
            want,
            pipeline: None,
            shared,
            alignment: Alignment::new(Instant::now()),
            last_check: None,
            upgrading: None,
            _open: Some(task),
            _upgrade: None,
            _wake: None,
        });
    }

    /// The pipeline of `want` has opened (or not).
    fn opened(&mut self, want: Want, pipeline: Option<Pipeline>, cx: &mut Context<Self>) {
        let Some(running) = self.running.as_mut().filter(|r| r.want == want && r.pipeline.is_none()) else {
            // Arrived late: the track has changed or the video has been closed.
            if let Some(pipeline) = pipeline {
                pipeline.discard();
            }
            return;
        };
        let Some(pipeline) = pipeline else {
            // It did not open: the core asks for another address once, then stops it.
            self.close();
            self.send(VideoCmd::Failed { seq: want.seq }, cx);
            self.refresh_status(cx);
            return;
        };
        running.pipeline = Some(pipeline);
        running._open = None;
        self.start(cx);
    }

    /// La pipeline appena entrata in scena: subito allineata e con la sua sveglia.
    fn start(&mut self, cx: &mut Context<Self>) {
        let playback = Arc::clone(&self.playback);
        let Some(running) = self.running.as_mut() else { return };
        let Some(pipeline) = running.pipeline.as_mut() else { return };
        let now = Instant::now();
        running.alignment = Alignment::new(now);
        let adjust = running.alignment.align(&playback, pipeline.measure(), true, now);
        plan::apply(pipeline, adjust);
        running.last_check = Some(now);
        let shared = Arc::clone(pipeline.shared());
        running.shared = Arc::clone(&shared);
        running._wake = Some(cx.spawn(async move |this, cx| {
            loop {
                shared.wake.notified().await;
                if this.update(cx, |stage, cx| stage.woke(cx)).is_err() {
                    break;
                }
            }
        }));
        self.refresh_status(cx);
        cx.notify();
    }

    /// A better version of the same track: it opens alongside and takes the place of the one playing.
    fn upgrade(&mut self, want: Want, cx: &mut Context<Self>) {
        let Some((_, receiver)) = self.spawn_open(want) else {
            self.send(VideoCmd::Failed { seq: want.seq }, cx);
            return;
        };
        let task = cx.spawn(async move |this, cx| {
            let pipeline = receiver.await.ok().flatten();
            let _ = this.update(cx, |stage, cx| stage.upgraded(want, pipeline, cx));
        });
        if let Some(running) = self.running.as_mut() {
            running.upgrading = Some(want.seq);
            running._upgrade = Some(task);
        }
    }

    fn upgraded(&mut self, want: Want, pipeline: Option<Pipeline>, cx: &mut Context<Self>) {
        let still_current = self.view.ready_for(want.track).is_some_and(|source| source.seq == want.seq);
        let Some(running) = self
            .running
            .as_mut()
            .filter(|r| r.want.track == want.track && r.pipeline.is_some() && r.upgrading == Some(want.seq))
        else {
            if let Some(pipeline) = pipeline {
                pipeline.discard();
            }
            return;
        };
        running.upgrading = None;
        running._upgrade = None;
        match pipeline {
            Some(pipeline) if still_current => {
                if let Some(old) = running.pipeline.replace(pipeline) {
                    old.discard();
                }
                running.want = want;
                self.start(cx);
            }
            // Another address has already taken its place: this one is no longer needed.
            Some(pipeline) => pipeline.discard(),
            // It did not open: the playing version stays, the core tries again later.
            None => self.send(VideoCmd::Failed { seq: want.seq }, cx),
        }
    }

    /// GStreamer ha svegliato la UI: un fotogramma nuovo, un errore o la fine.
    fn woke(&mut self, cx: &mut Context<Self>) {
        let playback = Arc::clone(&self.playback);
        let Some(running) = self.running.as_mut() else { return };
        if running.shared.failed() {
            // The stream dropped: the core asks for another address once.
            let seq = running.want.seq;
            self.close();
            self.send(VideoCmd::Failed { seq }, cx);
            self.refresh_status(cx);
            return;
        }
        let now = Instant::now();
        if let Some(pipeline) = running.pipeline.as_mut()
            && plan::check_due(running.last_check, now)
        {
            running.last_check = Some(now);
            let adjust = running.alignment.align(&playback, pipeline.measure(), false, now);
            plan::apply(pipeline, adjust);
        }
        if running.shared.slot.is_full() {
            self.refresh_status(cx);
            cx.notify();
        }
    }

    /// There is a frame of the track playing: already drawn, or ready in the pipeline's place.
    fn has_frame(&self) -> bool {
        let Some(track) = self.track.as_ref().map(|(track, _)| *track) else { return false };
        self.image.as_ref().is_some_and(|(of, _)| *of == track)
            || self.running.as_ref().is_some_and(|r| r.want.track == track && r.shared.slot.is_full())
    }

    fn compute_status(&self) -> Status {
        let Some(track) = self.track.as_ref().map(|(track, _)| *track) else { return Status::None };
        if self.view.failed_for(track).is_some() {
            return Status::Failed;
        }
        if self.has_frame() {
            return Status::Playing;
        }
        let playing = matches!(self.playback.state, PlaybackState::Playing | PlaybackState::Loading { .. });
        if self.surface.is_some() && playing { Status::Loading } else { Status::Cover }
    }

    fn refresh_status(&mut self, cx: &mut Context<Self>) {
        let status = self.compute_status();
        if status != self.status {
            self.status = status;
            cx.emit(StageEvent::Status);
        }
    }

    /// Takes the most recent frame, if there is a new one, and makes it the image to draw.
    fn take_frame(&mut self) {
        let Some(running) = self.running.as_mut() else { return };
        if let Some(pipeline) = running.pipeline.as_mut() {
            pipeline.fit_to(self.surface_px);
        }
        let Some(frame) = running.shared.slot.take() else { return };
        running.shared.mark_shown();
        let Some(bgra) = RgbaImage::from_raw(frame.width, frame.height, frame.bgra) else { return };
        let image = Arc::new(RenderImage::new(SmallVec::from_elem(ImageFrame::new(bgra), 1)));
        if let Some((_, old)) = self.image.replace((running.want.track, image)) {
            self.retired.push(old);
        }
    }
}

/// Il brano in corso, se ha un videoclip, con la sua copertina.
fn video_track(playback: &PlaybackView) -> Option<(Uuid, Option<SharedString>)> {
    let track = playback.queue.current().filter(|track| track.has_video)?;
    Some((track.id, track.cover_url.clone().map(SharedString::from)))
}

/// The frame inside `bounds`, with its proportions, centred, on the grid of physical pixels.
fn fitted(bounds: Bounds<Pixels>, image: &RenderImage, scale: f32) -> Bounds<Pixels> {
    let frame = image.size(0);
    let (fw, fh) = (i32::from(frame.width) as f32, i32::from(frame.height) as f32);
    let (bw, bh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    if fw <= 0.0 || fh <= 0.0 || bw <= 0.0 || bh <= 0.0 {
        return bounds;
    }
    let ratio = (bw / fw).min(bh / fh);
    let (w, h) = (fw * ratio, fh * ratio);
    let snap = |value: f32| (value * scale).round() / scale;
    let x = snap(f32::from(bounds.origin.x) + (bw - w) / 2.0);
    let y = snap(f32::from(bounds.origin.y) + (bh - h) / 2.0);
    Bounds::new(point(px(x), px(y)), size(px(snap(w)), px(snap(h))))
}

impl Render for VideoStage {
    /// A veil as large as the window, without active areas: it leaves the atlas to the previous images and paints
    /// the frame in the place that the box's well marked in this draw (the stage is painted
    /// after the page and panels). Without a place that shows it, it paints nothing.
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.take_frame();
        let retired = std::mem::take(&mut self.retired);
        let current = self.track.as_ref().map(|(track, _)| *track);
        let image =
            self.image.as_ref().filter(|(of, _)| Some(*of) == current).map(|(_, image)| Arc::clone(image));
        let spot = self.surface.map(|surface| self.spot(surface));
        let scale = window.scale_factor();
        canvas(
            |_, _, _| (),
            move |_, (), window, _| {
                // The previous images leave the atlas in the same draw as the new one.
                for old in retired {
                    let _ = window.drop_image(old);
                }
                let Some((bounds, mask, veil)) = spot.and_then(|spot| spot.0.get()) else { return };
                let mask = ContentMask { bounds: mask.bounds.intersect(&bounds) };
                window.with_content_mask(Some(mask), |window| {
                    if let Some(image) = image {
                        let target = fitted(bounds, &image, scale);
                        // A frame that does not fit the atlas leaves the ground.
                        let _ = window.paint_image(target, target, Corners::default(), image, 0, false);
                    }
                    // The dissolve covers the frame or the cover under it.
                    paint_dither_veil(window, bounds, Ap::of(window), veil, col(SURFACE_WELL));
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}
