//! `VideoStage`: l'entità che tiene la pipeline del brano in corso e ne disegna i fotogrammi. Una sola per
//! finestra: la mostra il pannello laterale sul videoclip o la vista immersiva sul videoclip (mai tutti e due
//! insieme), e `Shell` le dice quale (`set_surface`).
//! - **Dove disegna.** Il palco è l'ultimo figlio di `Shell` (dopo pagina, pannelli e vista immersiva), non un
//!   figlio del pannello o della vista immersiva: un fotogramma nuovo rifà il palco e `Shell`, che GPUI rifà
//!   comunque a ogni disegno perché è la radice, e le viste in cache si riusano, pannello e vista immersiva
//!   compresi (BACKLOG 41). Chi lo mostra mette nel suo riquadro un pozzo (`well`) che, mentre si dipinge,
//!   segna il posto (`Spot`); il palco dipinge il fotogramma lì sopra, nello stesso disegno, come
//!   `PlayingSpot` fa per l'equalizzatore. `Shell` non va in `.cached()` sotto un'altra radice: una vista in
//!   cache che si rifà rifà anche tutte le viste in cache che contiene (GPUI le ridisegna con `refreshing`).
//!
//! - **Che cosa aprire** lo dice il core (`VideoView`, letto come `ClipView`), **quando** le regole pure di
//!   `plan`: una pipeline solo per il brano in corso, mentre suona o si carica e mentre un posto la mostra.
//!   Aprire e chiudere bloccano e vanno su un thread a parte; una versione migliore (360p → 720p) si apre
//!   di fianco e prende il posto di quella che suona senza fermarla. In pausa la pipeline si chiude e resta
//!   l'ultimo fotogramma; alla ripresa si riapre alla posizione dell'audio.
//! - **Fotogrammi.** GStreamer mette l'ultimo nel posto (`slot`) e sveglia la UI una volta (`Notify`): la
//!   sveglia notifica solo questa vista, che nel disegno prende il fotogramma, ne fa una `RenderImage` e
//!   toglie dall'atlante quella di prima. Una UI in ritardo salta fotogrammi, non li accumula.
//! - **Sincronia.** L'orologio è quello dell'audio (`PlaybackView::clock`): `Alignment::follow` a ogni
//!   pubblicazione della riproduzione (un brano che parte, un salto, pausa o ripresa) e `Alignment::align`
//!   sulla sveglia dei fotogrammi, al massimo ogni 250 ms (nessun timer in più).
//! - **Guasti.** Una pipeline che non si apre o si ferma è `VideoCmd::Failed { seq }`: il core ne chiede un
//!   altro indirizzo una volta e poi blocca il videoclip, che la vista mostra con «Riprova» (`retry`).

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

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

/// Il posto che mostra il videoclip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// Il mini player del pannello laterale.
    Panel,
    /// La vista immersiva.
    Focus,
}

/// Il posto del fotogramma in un riquadro: lo segna il pozzo (`VideoStage::well`) mentre si dipinge, con la
/// maschera che lo taglia, e lo legge il palco quando dipinge, subito dopo. La vista che contiene il pozzo lo
/// svuota all'inizio di ogni suo disegno (`clear`): riusata dalla cache, il posto resta quello di prima, che è
/// ancora giusto (GPUI riusa una vista solo se ha gli stessi limiti e la stessa maschera).
#[derive(Clone, Default)]
pub struct Spot(Rc<Cell<Option<Place>>>);

/// I limiti del riquadro e la maschera che lo taglia.
type Place = (Bounds<Pixels>, ContentMask<Pixels>);

impl Spot {
    pub fn clear(&self) {
        self.0.set(None);
    }
}

/// Che cosa mostra il riquadro del videoclip (cambia di rado: non a ogni fotogramma).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Il brano non ha un videoclip, o non c'è un brano.
    None,
    /// La copertina, senza attese: in pausa prima del primo fotogramma.
    Cover,
    /// Il server prepara il videoclip, o la pipeline si apre.
    Loading,
    /// Il videoclip (anche fermo sull'ultimo fotogramma, in pausa).
    Playing,
    /// Non si è aperto, nemmeno al secondo tentativo: «Il videoclip non si è aperto.» con «Riprova» (il
    /// testo dell'errore del core va nell'avviso).
    Failed,
}

/// Ciò che il palco dice a chi lo mostra.
pub enum StageEvent {
    /// `status` è cambiato.
    Status,
    /// Un videoclip si è fermato: l'avviso (gli avvisi li mostra `Shell`).
    Notice(Notice),
}

impl EventEmitter<StageEvent> for VideoStage {}

/// La pipeline del videoclip che si vuole, aperta o in apertura.
struct Running {
    want: Want,
    pipeline: Option<Pipeline>,
    shared: Arc<Shared>,
    alignment: Alignment,
    last_check: Option<Instant>,
    /// Il `seq` della versione migliore che si apre di fianco.
    upgrading: Option<u64>,
    _open: Option<Task<()>>,
    _upgrade: Option<Task<()>>,
    /// Aspetta le sveglie di GStreamer: c'è solo con la pipeline aperta.
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

    /// Chiude tutto su un thread a parte; i task finiscono con lui.
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
    /// Il core sa che un posto mostra il videoclip (`VideoCmd::Show`).
    shown_sent: bool,
    running: Option<Running>,
    /// L'ultimo fotogramma disegnato e il brano a cui appartiene.
    image: Option<(Uuid, Arc<RenderImage>)>,
    /// Immagini da togliere dall'atlante al prossimo disegno.
    retired: Vec<Arc<RenderImage>>,
    /// I posti del pannello e della vista immersiva.
    panel_spot: Spot,
    focus_spot: Spot,
    status: Status,
    /// I videoclip fermati già detti con un avviso.
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
            focus_spot: Spot::default(),
            status: Status::None,
            _changes: changes,
        };
        stage.status = stage.compute_status();
        stage
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Il posto che mostra il videoclip (o nessuno). Il core chiede il videoclip al server solo finché un
    /// posto lo mostra, e prepara in anticipo quelli dei brani successivi solo allora.
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

    /// I pixel fisici del riquadro: chi mostra il palco li dice a ogni disegno (la pipeline rinegozia la
    /// misura solo quando cambia abbastanza).
    pub fn set_surface_px(&mut self, surface: Size) {
        self.surface_px = surface;
    }

    /// «Riprova» su un videoclip che non si apriva.
    pub fn retry(&self, cx: &App) {
        self.send(VideoCmd::Retry, cx);
    }

    /// Il posto del fotogramma in `surface`: la vista che mostra il riquadro lo svuota a ogni suo disegno.
    pub fn spot(&self, surface: Surface) -> Spot {
        match surface {
            Surface::Panel => self.panel_spot.clone(),
            Surface::Focus => self.focus_spot.clone(),
        }
    }

    /// Il pozzo del riquadro (`width` x `height` dp) in `surface`: il fondo, la copertina finché non c'è un
    /// fotogramma, e il segno del posto dove il palco dipinge il fotogramma.
    pub fn well(&self, surface: Surface, width: f32, height: f32) -> Div {
        let spot = self.spot(surface);
        let marker = canvas(
            |_, _, _| (),
            move |bounds, (), window, _| spot.0.set(Some((bounds, window.content_mask()))),
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
            // Il video segue l'audio: un brano che parte, un salto, la ripresa.
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

    /// Dalla pipeline che c'è a quella che serve (`plan`), poi lo stato.
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
        // Un altro brano: il fotogramma del brano di prima non resta sul display.
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

    /// Apre su un thread a parte l'indirizzo del videoclip `want`, alla posizione dell'audio.
    fn spawn_open(&self, want: Want) -> Option<(Arc<Shared>, oneshot::Receiver<Option<Pipeline>>)> {
        let url = self.view.ready_for(want.track)?.url().to_owned();
        let clock = self.playback.clock;
        let surface = Some(self.surface_px).filter(|size| size.width > 0 && size.height > 0);
        let shared = Arc::new(Shared::default());
        let (sender, receiver) = oneshot::channel();
        let for_thread = Arc::clone(&shared);
        let spawned = std::thread::Builder::new().name("video-open".into()).spawn(move || {
            let opened = Pipeline::open(&url, clock, surface, for_thread).ok();
            // Nessuno aspetta più (il brano è cambiato): si chiude qui, su questo thread.
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

    /// La pipeline di `want` si è aperta (o no).
    fn opened(&mut self, want: Want, pipeline: Option<Pipeline>, cx: &mut Context<Self>) {
        let Some(running) = self.running.as_mut().filter(|r| r.want == want && r.pipeline.is_none()) else {
            // Arrivata tardi: il brano è cambiato o il videoclip è stato chiuso.
            if let Some(pipeline) = pipeline {
                pipeline.discard();
            }
            return;
        };
        let Some(pipeline) = pipeline else {
            // Non si è aperta: il core ne chiede un altro indirizzo una volta, poi la ferma.
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

    /// Una versione migliore dello stesso brano: si apre di fianco e prende il posto di quella che suona.
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
            // Un altro indirizzo ha già preso il suo posto: questo non serve più.
            Some(pipeline) => pipeline.discard(),
            // Non si è aperta: resta la versione che suona, il core riprova più tardi.
            None => self.send(VideoCmd::Failed { seq: want.seq }, cx),
        }
    }

    /// GStreamer ha svegliato la UI: un fotogramma nuovo, un errore o la fine.
    fn woke(&mut self, cx: &mut Context<Self>) {
        let playback = Arc::clone(&self.playback);
        let Some(running) = self.running.as_mut() else { return };
        if running.shared.failed() {
            // Lo stream è caduto: il core ne chiede un altro indirizzo una volta.
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

    /// C'è un fotogramma del brano in corso: già disegnato, o pronto nel posto della pipeline.
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

    /// Prende il fotogramma più recente, se ce n'è uno nuovo, e lo fa diventare l'immagine da disegnare.
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

/// Il fotogramma dentro `bounds`, con le sue proporzioni, centrato, sulla griglia dei pixel fisici.
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
    /// Un velo grande quanto la finestra, senza zone attive: lascia l'atlante alle immagini di prima e dipinge
    /// il fotogramma nel posto che il pozzo del riquadro ha segnato in questo disegno (il palco si dipinge
    /// dopo pagina e pannelli). Senza un posto che lo mostra non dipinge niente.
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
                // Le immagini di prima lasciano l'atlante nello stesso disegno della nuova.
                for old in retired {
                    let _ = window.drop_image(old);
                }
                let (Some(image), Some((bounds, mask))) = (image, spot.and_then(|spot| spot.0.get())) else {
                    return;
                };
                let target = fitted(bounds, &image, scale);
                let mask = ContentMask { bounds: mask.bounds.intersect(&bounds) };
                window.with_content_mask(Some(mask), |window| {
                    // Se il fotogramma non entra nell'atlante resta il fondo.
                    let _ = window.paint_image(target, target, Corners::default(), image, 0, false);
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}
