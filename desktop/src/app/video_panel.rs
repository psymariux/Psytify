//! Il videoclip nel pannello laterale (il pulsante «Videoclip» della barra), solo con la feature `video`: il
//! mini player del client Iced (che lo mette nella scheda «In riproduzione», che il desktop non ha: BACKLOG
//! 34). In alto «Videoclip» con «titolo · artista», come il pannello del testo; poi il riquadro 16:9 largo
//! `COVER_PANEL` con il palco (`VideoStage`), sotto l'attesa («Preparo il videoclip…») o il guasto con
//! «Riprova», e «Videoclip a tutto schermo» (V), che apre la vista immersiva sul videoclip.
//!
//! La vista non ha timer né lavoro per fotogramma: il riquadro è il pozzo del palco (`VideoStage::well`), che
//! segna dove il palco, ultimo figlio di `Shell`, dipinge i fotogrammi; un fotogramma non ridisegna questa
//! vista. Si ridisegna da sé quando cambia lo stato del palco o il brano.

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div,
};
use psytify_core::snapshot::PlaybackView;
use psytify_design::sprite;
use psytify_design::tokens::color::{TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY};
use psytify_design::tokens::size::cover::COVER_PANEL;
use psytify_design::tokens::space::{SPACE_3, SPACE_4, SPACE_5};
use psytify_design::tokens::type_::{TYPE_BODY, TYPE_HEADING, TYPE_META};

use crate::components::button::Button;
use crate::components::icon_button::IconButton;
use crate::components::indicators::LoadingIndicator;
use crate::components::separator::Separator;
use crate::core_model::{Changed, CoreModel};
use crate::theme::{TypeStyle, col, dp};
use crate::video::size;
use crate::video::stage::{Spot, StageEvent, Status, Surface, VideoStage};

/// Il videoclip che non si apre, come nel client Iced.
pub const FAILED: &str = "Il videoclip non si è aperto.";

/// Il riquadro del videoclip: 16:9, largo quanto la copertina del pannello.
pub const BOX_W: f32 = COVER_PANEL;
pub const BOX_H: f32 = COVER_PANEL * 9.0 / 16.0;

/// Quello che il pannello chiede alla finestra.
pub enum PanelEvent {
    /// «Videoclip a tutto schermo»: la vista immersiva sul videoclip.
    Fullscreen,
}

impl EventEmitter<PanelEvent> for VideoPanel {}

pub struct VideoPanel {
    stage: Entity<VideoStage>,
    /// Il posto del fotogramma nel riquadro, svuotato a ogni disegno.
    spot: Spot,
    /// «titolo · artista» del brano in corso.
    track_label: SharedString,
    /// C'è un brano in coda.
    has_track: bool,
    _subscriptions: Vec<Subscription>,
}

fn label(view: &PlaybackView) -> SharedString {
    view.queue
        .current()
        .map(|track| SharedString::from(format!("{} · {}", track.title, track.artist.name)))
        .unwrap_or_default()
}

impl VideoPanel {
    pub fn new(core: Entity<CoreModel>, stage: Entity<VideoStage>, cx: &mut Context<Self>) -> Self {
        let playback = Arc::clone(&core.read(cx).snapshot().playback);
        let subscriptions = vec![
            cx.subscribe(&core, |this, core, changed: &Changed, cx| {
                if !changed.playback {
                    return;
                }
                let playback = Arc::clone(&core.read(cx).snapshot().playback);
                let (track_label, has_track) = (label(&playback), playback.queue.current().is_some());
                if track_label != this.track_label || has_track != this.has_track {
                    this.track_label = track_label;
                    this.has_track = has_track;
                    cx.notify();
                }
            }),
            cx.subscribe(&stage, |_, _, event: &StageEvent, cx| {
                if matches!(event, StageEvent::Status) {
                    cx.notify();
                }
            }),
        ];
        Self {
            spot: stage.read(cx).spot(Surface::Panel),
            stage,
            track_label: label(&playback),
            has_track: playback.queue.current().is_some(),
            _subscriptions: subscriptions,
        }
    }

    fn note(text: impl Into<SharedString>) -> gpui::Div {
        div().type_role(TYPE_BODY).text_color(col(TEXT_SECONDARY)).child(text.into())
    }
}

impl Render for VideoPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Il riquadro lo risegna, se c'è, quando si dipinge.
        self.spot.clear();
        let status = self.stage.read(cx).status().clone();
        let scale = window.scale_factor();
        self.stage.update(cx, |stage, _| stage.set_surface_px(size::device(BOX_W, BOX_H, scale)));

        let header = div()
            .flex_none()
            .flex()
            .flex_col()
            .px(dp(SPACE_5))
            .py(dp(SPACE_4))
            .child(div().type_role(TYPE_HEADING).text_color(col(TEXT_PRIMARY)).child("Videoclip"))
            .child(
                div()
                    .type_role(TYPE_META)
                    .text_color(col(TEXT_TERTIARY))
                    .truncate()
                    .child(self.track_label.clone()),
            );

        let body = match &status {
            Status::None if !self.has_track => Self::note("Scegli un brano per vedere il videoclip."),
            Status::None => Self::note("Questo brano non ha un videoclip."),
            _ => {
                let stage = self.stage.clone();
                let line = match &status {
                    Status::Loading => {
                        Some(LoadingIndicator::new().caption("Preparo il videoclip…").into_any_element())
                    }
                    Status::Failed => Some(
                        div()
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap(dp(SPACE_4))
                            .child(Self::note(FAILED))
                            .child(
                                Button::new("video-retry", "Riprova")
                                    .secondary()
                                    .on_click(move |_, _, cx| stage.read(cx).retry(cx)),
                            )
                            .into_any_element(),
                    ),
                    _ => None,
                };
                let fullscreen = cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PanelEvent::Fullscreen));
                div()
                    .flex()
                    .flex_col()
                    .gap(dp(SPACE_4))
                    .child(self.stage.read(cx).well(Surface::Panel, BOX_W, BOX_H))
                    .children(line)
                    .child(
                        div().flex().justify_end().child(
                            IconButton::new("video-fullscreen", sprite::ICON_FULLSCREEN)
                                .tooltip("Videoclip a tutto schermo", Some("V"))
                                .on_click(fullscreen),
                        ),
                    )
            }
        };

        div().size_full().flex().flex_col().child(header).child(Separator::horizontal()).child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .items_center()
                .pt(dp(SPACE_4))
                .child(div().w(dp(BOX_W)).flex().flex_col().gap(dp(SPACE_3)).child(body)),
        )
    }
}
