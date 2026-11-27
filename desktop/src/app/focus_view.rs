//! La vista immersiva (tasto F, come nel client Iced, `client/src/ui/focus.rs`): solo la tela, nessuna
//! immagine di sfondo, niente sfocature. Un blocco centrato con la copertina (fino a `COVER_FOCUS_MAX`),
//! l'avanzamento con il tempo passato e quello che manca in tipo pixel, titolo con il cuore, artista e
//! album e un trasporto minimo; accanto il testo del brano (`LyricsView` con `Look::Focus`, lo stesso
//! motore del pannello). Si chiude con Esc o con la X.
//!
//! Come la barra «in riproduzione», ha un timer da 250 ms (`PROGRESS_TICK_MS`) solo mentre è aperta e il
//! brano suona: ridisegna l'avanzamento e i tempi. Il testo è una vista a parte in `.cached()`: il battito
//! non lo rifà, e lui ha il suo timer (vedi `lyrics_view`).
//!
//! Con la feature `video` e un brano con il videoclip, la vista ha due facce, come nel client Iced: il testo
//! (sopra) o il videoclip (`set_video`). Sul videoclip il pozzo del palco (`VideoStage::well`) prende il
//! posto della copertina e del testo, in un riquadro 16:9 grande quanto la vista consente, con sotto
//! avanzamento, titolo e trasporto; in cima «Torna al testo» e la X. Sul testo, in cima, «Videoclip». Il
//! palco, ultimo figlio di `Shell`, dipinge i fotogrammi nel pozzo: un fotogramma non ridisegna questa vista.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AppContext, ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString,
    StyleRefinement, Styled, Subscription, Task, Window, div,
};
use psytify_core::api::Track;
use psytify_core::command::{LibraryCmd, PlaybackCmd};
use psytify_core::player::{PlaybackState, Repeat as QueueRepeat};
use psytify_core::snapshot::PlaybackView;
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY};
use psytify_design::tokens::motion::PROGRESS_TICK_MS;
use psytify_design::tokens::size::TITLE_BAR_H;
use psytify_design::tokens::size::cover::COVER_FOCUS_MAX;
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_5, SPACE_9};
#[cfg(feature = "video")]
use psytify_design::tokens::space::{SPACE_4, SPACE_7};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_HEADING, TYPE_META};
use uuid::Uuid;

use super::lyrics_view::{Look, LyricsView};
#[cfg(feature = "video")]
use crate::components::button::Button;
use crate::components::icon_button::{IconButton, OnStyle};
#[cfg(feature = "video")]
use crate::components::indicators::LoadingIndicator;
use crate::components::player_control::{PlayerControl, Repeat, Transport};
use crate::components::progress::ProgressBar;
use crate::components::tile::Cover;
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{Ap, TypeStyle, col, dp};
#[cfg(feature = "video")]
use crate::video::size;
#[cfg(feature = "video")]
use crate::video::stage::{Spot, StageEvent, Status, Surface, VideoStage};

/// Margine dai bordi e fra la colonna della copertina e il testo.
const MARGIN: f32 = SPACE_9;
/// Altezza della riga in cima (fonte del testo e chiusura).
const TOP_ROW: f32 = 48.0;
/// Larghezza massima del testo e minima della colonna della copertina (ci devono stare i controlli).
const LYRICS_MAX_W: f32 = 760.0;
const LEFT_MIN: f32 = 300.0;
/// Spazio sotto la copertina per avanzamento, titolo, artista e controlli.
const UNDER_COVER: f32 = 190.0;
/// La copertina più piccola.
const COVER_MIN: f32 = 140.0;

/// Quello che la vista chiede alla finestra.
pub enum FocusEvent {
    Close,
    /// Passare al videoclip (`true`) o al testo.
    #[cfg(feature = "video")]
    Video(bool),
}

impl EventEmitter<FocusEvent> for FocusView {}

/// Il brano in corso, con i testi già pronti.
struct Now {
    track: Track,
    title: SharedString,
    detail: SharedString,
    cover_url: Option<SharedString>,
    duration: Duration,
}

impl Now {
    fn of(track: &Track) -> Self {
        let detail = match &track.album {
            Some(album) => format!("{} · {}", track.artist.name, album.title),
            None => track.artist.name.clone(),
        };
        Self {
            track: track.clone(),
            title: track.title.clone().into(),
            detail: detail.into(),
            cover_url: track.cover_url.clone().map(SharedString::from),
            duration: track.duration(),
        }
    }
}

/// Le misure del blocco centrale per una finestra di `width` x `height` dp: lato della copertina,
/// larghezza della colonna della copertina, del testo e altezza del palco. Come nel client Iced.
pub fn stage(width: f32, height: f32) -> (f32, f32, f32, f32) {
    let stage_h = (height - TITLE_BAR_H - TOP_ROW - MARGIN / 2.0).max(240.0);
    let side = (stage_h - UNDER_COVER).min(width * 0.36).clamp(COVER_MIN, COVER_FOCUS_MAX);
    let left = side.max(LEFT_MIN);
    let lyrics = (width - left - 3.0 * MARGIN).clamp(240.0, LYRICS_MAX_W);
    (side, left, lyrics, stage_h)
}

/// Il riquadro 16:9 del videoclip in una vista di `width` x `height` dp: largo quanto la vista meno i
/// margini, alto quanto resta sopra avanzamento, titolo e trasporto (e la riga dell'attesa o del guasto).
#[cfg(feature = "video")]
pub fn video_box(width: f32, height: f32) -> (f32, f32) {
    let (.., stage_h) = stage(width, height);
    let max_w = (width - 2.0 * MARGIN).max(COVER_MIN);
    let max_h = (stage_h - UNDER_COVER - SPACE_5 - SPACE_7).max(COVER_MIN * 9.0 / 16.0);
    let w = max_w.min(max_h * 16.0 / 9.0);
    (w, w * 9.0 / 16.0)
}

pub struct FocusView {
    core: Entity<CoreModel>,
    view: Arc<PlaybackView>,
    liked: Arc<HashSet<Uuid>>,
    now: Option<Now>,
    lyrics: Entity<LyricsView>,
    open: bool,
    /// Il timer da 250 ms: c'è solo mentre la vista è aperta e il brano suona.
    tick: Option<Task<()>>,
    /// I tempi passato e mancante, rifatti solo quando cambia il secondo.
    times: (u64, SharedString, SharedString),
    /// Il palco del videoclip e la faccia scelta (videoclip o testo).
    #[cfg(feature = "video")]
    stage: Entity<VideoStage>,
    /// Il posto del fotogramma nel riquadro della faccia del videoclip, svuotato a ogni disegno.
    #[cfg(feature = "video")]
    spot: Spot,
    #[cfg(feature = "video")]
    video: bool,
    _changes: Subscription,
    #[cfg(feature = "video")]
    _stage_changes: Subscription,
}

impl FocusView {
    pub fn new(
        core: Entity<CoreModel>,
        #[cfg(feature = "video")] stage: Entity<VideoStage>,
        cx: &mut Context<Self>,
    ) -> Self {
        let lyrics = cx.new(|cx| LyricsView::new(core.clone(), Look::Focus, cx));
        #[cfg(feature = "video")]
        let stage_changes = cx.subscribe(&stage, |this, _, event: &StageEvent, cx| {
            if matches!(event, StageEvent::Status) && this.open {
                cx.notify();
            }
        });
        let changes = cx.subscribe(&core, |this, core, changed: &Changed, cx| {
            let mut dirty = false;
            if changed.playback {
                let view = Arc::clone(&core.read(cx).snapshot().playback);
                if !Arc::ptr_eq(&view.queue, &this.view.queue) {
                    this.now = view.queue.current().map(Now::of);
                }
                this.view = view;
                this.sync_tick(cx);
                // Un brano senza videoclip torna al testo (che allora si vede e ha il suo timer).
                this.sync_lyrics(cx);
                dirty = true;
            }
            if changed.library {
                this.liked = core.read(cx).liked();
                dirty = true;
            }
            if dirty && this.open {
                cx.notify();
            }
        });
        let view = Arc::clone(&core.read(cx).snapshot().playback);
        Self {
            now: view.queue.current().map(Now::of),
            view,
            liked: core.read(cx).liked(),
            core,
            lyrics,
            open: false,
            tick: None,
            times: (u64::MAX, SharedString::default(), SharedString::default()),
            #[cfg(feature = "video")]
            spot: stage.read(cx).spot(Surface::Focus),
            #[cfg(feature = "video")]
            stage,
            #[cfg(feature = "video")]
            video: false,
            _changes: changes,
            #[cfg(feature = "video")]
            _stage_changes: stage_changes,
        }
    }

    /// La finestra apre o chiude la vista: solo aperta ha il battito, e il testo ha il suo timer.
    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        self.sync_lyrics(cx);
        self.sync_tick(cx);
        cx.notify();
    }

    /// La faccia della vista: il videoclip (se il brano ce l'ha) o il testo.
    #[cfg(feature = "video")]
    pub fn set_video(&mut self, video: bool, cx: &mut Context<Self>) {
        if self.video != video {
            self.video = video;
            self.sync_lyrics(cx);
            cx.notify();
        }
    }

    /// La vista mostra il videoclip: scelto, e il brano ne ha uno.
    fn shows_video(&self) -> bool {
        #[cfg(feature = "video")]
        let shows = self.video && self.has_video();
        #[cfg(not(feature = "video"))]
        let shows = false;
        shows
    }

    #[cfg(feature = "video")]
    fn has_video(&self) -> bool {
        self.now.as_ref().is_some_and(|now| now.track.has_video)
    }

    /// Il testo si vede (e ha il suo timer) solo con la vista aperta sulla faccia del testo.
    fn sync_lyrics(&mut self, cx: &mut Context<Self>) {
        let visible = self.open && !self.shows_video();
        self.lyrics.update(cx, |lyrics, cx| lyrics.set_visible(visible, cx));
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

    /// «m:ss» passato e «-m:ss» mancante, rifatti solo quando cambia il secondo.
    fn times(&mut self, position: Duration, duration: Duration) -> (SharedString, SharedString) {
        let secs = position.as_secs();
        if self.times.0 != secs {
            let left = duration.saturating_sub(position);
            self.times = (secs, format::clock(position).into(), format!("-{}", format::clock(left)).into());
        }
        (self.times.1.clone(), self.times.2.clone())
    }
}

/// Il testo in `.cached()`: la vista lo misura da sé (riquadro pieno).
fn fill() -> StyleRefinement {
    StyleRefinement::default().size_full()
}

impl Render for FocusView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // La faccia del videoclip, se c'è, lo risegna quando si dipinge.
        #[cfg(feature = "video")]
        self.spot.clear();
        let viewport = window.viewport_size();
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        let (side, left_w, lyrics_w, stage_h) = stage(vw, vh);
        let ap = Ap::of(window);
        let view = Arc::clone(&self.view);
        let active = view.is_active();
        let duration = self.now.as_ref().map_or(Duration::ZERO, |now| now.duration);
        let position = if active { view.position(Instant::now()) } else { Duration::ZERO };
        let fraction = if duration.is_zero() { 0.0 } else { position.as_secs_f32() / duration.as_secs_f32() };
        let (elapsed, remaining) = self.times(position, duration);
        let video = self.shows_video();

        let close = cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(FocusEvent::Close));
        let source = if video { "Videoclip" } else { "Testi da LRCLIB" };
        let mut actions = div().flex().items_center().gap(dp(SPACE_3));
        #[cfg(feature = "video")]
        if self.has_video() {
            // L'altra faccia della vista, a sinistra della X (come nel client Iced).
            let to_video = !video;
            let switch = cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(FocusEvent::Video(to_video)));
            actions = actions.child(if video {
                IconButton::new("focus-face", sprite::ICON_LYRICS)
                    .tooltip("Torna al testo", None)
                    .on_click(switch)
            } else {
                IconButton::new("focus-face", sprite::ICON_VIDEO).tooltip("Videoclip", None).on_click(switch)
            });
        }
        actions = actions.child(
            IconButton::new("focus-close", sprite::ICON_CLOSE).tooltip("Chiudi", Some("Esc")).on_click(close),
        );
        let top = div()
            .flex_none()
            .h(dp(TOP_ROW))
            .px(dp(SPACE_5))
            .flex()
            .items_center()
            .justify_between()
            .child(div().type_role(TYPE_META).text_color(col(TEXT_TERTIARY)).child(source))
            .child(actions);

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

        let mut left = div().w(dp(left_w)).flex_none().flex().flex_col().items_center().gap(dp(SPACE_5));
        if let Some(now) = &self.now {
            let liked = self.liked.contains(&now.track.id);
            let track = now.track.clone();
            let like_core = self.core.clone();
            let seek_core = self.core.clone();
            let progress_w = side.max(LEFT_MIN - 40.0);
            if !video {
                left = left.child(Cover::new(side).url(now.cover_url.clone()).large());
            }
            left = left
                .child(
                    div()
                        .w(dp(progress_w))
                        .flex()
                        .flex_col()
                        .gap(dp(SPACE_2))
                        .child(
                            ProgressBar::new("focus-seek", fraction)
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
                                .pixel_type(ap, true)
                                .text_color(col(TEXT_TERTIARY))
                                .child(elapsed)
                                .child(remaining),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(dp(SPACE_2))
                        .child(
                            div()
                                .max_w_full()
                                .flex()
                                .items_center()
                                .gap(dp(SPACE_3))
                                .child(
                                    div()
                                        .min_w_0()
                                        .type_role(TYPE_HEADING)
                                        .text_color(col(TEXT_PRIMARY))
                                        .truncate()
                                        .child(now.title.clone()),
                                )
                                .child(
                                    IconButton::new("focus-like", sprite::ICON_HEART)
                                        .icon_on(sprite::ICON_HEART_FILL)
                                        .on_style(OnStyle::Active)
                                        .selected(liked)
                                        .tooltip(
                                            if liked {
                                                "Rimuovi dai preferiti"
                                            } else {
                                                "Aggiungi ai preferiti"
                                            },
                                            None,
                                        )
                                        .on_click(move |_, _, cx| {
                                            like_core.read(cx).library(LibraryCmd::ToggleLike(track.clone()));
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .max_w_full()
                                .type_role(TYPE_ARTIST)
                                .text_color(col(TEXT_SECONDARY))
                                .truncate()
                                .child(now.detail.clone()),
                        ),
                )
                .child(
                    PlayerControl::new("focus-transport")
                        .transport(transport)
                        .shuffle(queue.shuffle())
                        .repeat(repeat)
                        .disabled(queue.is_empty())
                        .on_play(command(PlaybackCmd::Toggle))
                        .on_previous(command(PlaybackCmd::Previous))
                        .on_next(command(PlaybackCmd::Next))
                        .on_shuffle(command(PlaybackCmd::ToggleShuffle))
                        .on_repeat(command(PlaybackCmd::CycleRepeat)),
                );
        }

        #[cfg(feature = "video")]
        if video {
            return self.video_face(top, left, vw, vh, window, cx).into_any_element();
        }

        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .gap(dp(MARGIN))
            .pb(dp(MARGIN / 2.0))
            .child(left)
            .child(
                div().flex_none().w(dp(lyrics_w)).h(dp(stage_h)).child(self.lyrics.clone().cached(fill())),
            );

        div().size_full().flex().flex_col().bg(col(SURFACE_CANVAS)).child(top).child(body).into_any_element()
    }
}

#[cfg(feature = "video")]
impl FocusView {
    /// La faccia del videoclip: il palco in un riquadro 16:9, sotto l'attesa o il guasto con «Riprova», poi
    /// avanzamento, titolo e trasporto (`info`, la colonna della faccia del testo senza la copertina).
    fn video_face(
        &self,
        top: gpui::Div,
        info: gpui::Div,
        width: f32,
        height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let (box_w, box_h) = video_box(width, height);
        let scale = window.scale_factor();
        self.stage.update(cx, |stage, _| stage.set_surface_px(size::device(box_w, box_h, scale)));
        let line = match self.stage.read(cx).status().clone() {
            Status::Loading => {
                Some(LoadingIndicator::new().caption("Preparo il videoclip…").into_any_element())
            }
            Status::Failed => {
                let stage = self.stage.clone();
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(dp(SPACE_4))
                        .child(
                            div()
                                .type_role(TYPE_ARTIST)
                                .text_color(col(TEXT_SECONDARY))
                                .child(super::video_panel::FAILED),
                        )
                        .child(
                            Button::new("focus-video-retry", "Riprova")
                                .secondary()
                                .on_click(move |_, _, cx| stage.read(cx).retry(cx)),
                        )
                        .into_any_element(),
                )
            }
            _ => None,
        };
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(dp(SPACE_5))
            .pb(dp(MARGIN / 2.0))
            .child(self.stage.read(cx).well(Surface::Focus, box_w, box_h))
            .children(line)
            .child(info);
        div().size_full().flex().flex_col().bg(col(SURFACE_CANVAS)).child(top).child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stage_fits_the_window() {
        // Finestra grande: copertina al massimo del suo spazio, testo entro il suo massimo.
        let (side, left, lyrics, stage_h) = stage(1920.0, 1080.0);
        assert!((COVER_MIN..=COVER_FOCUS_MAX).contains(&side));
        assert!(side <= stage_h - UNDER_COVER + 0.01);
        assert_eq!(left, side.max(LEFT_MIN));
        assert!(lyrics <= LYRICS_MAX_W);
        assert!(left + lyrics + 3.0 * MARGIN <= 1920.0 + 0.01);
        // Finestra piccola: la copertina non scende sotto il minimo e la colonna tiene i controlli.
        let (side, left, lyrics, _) = stage(600.0, 400.0);
        assert_eq!(side, COVER_MIN);
        assert_eq!(left, LEFT_MIN);
        assert_eq!(lyrics, 240.0);
    }

    #[cfg(feature = "video")]
    #[test]
    fn the_video_box_is_sixteen_by_nine_inside_the_view() {
        for (width, height) in [(1280.0, 800.0), (1920.0, 1080.0), (900.0, 600.0), (2560.0, 700.0)] {
            let (w, h) = video_box(width, height);
            assert!((w / h - 16.0 / 9.0).abs() < 1e-3, "{w} x {h}");
            assert!(w <= width - 2.0 * MARGIN + 0.01, "{w} in {width}");
            let (.., stage_h) = stage(width, height);
            assert!(h + UNDER_COVER <= stage_h + 0.01, "{h} in {stage_h}");
        }
        // Nella finestra di 1280 x 800 il riquadro è più grande del mini player e sotto il 720p a scala 1.
        let (w, _) = video_box(1280.0, 800.0);
        assert!(w > 288.0 && w <= 1280.0);
    }
}
