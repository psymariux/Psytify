//! La barra «in riproduzione» (72 dp in basso): copertina, titolo e artista, «mi piace»; trasporto e
//! barra di avanzamento al centro; testo, coda, volume e vista immersiva a destra (l'ordine della direzione
//! artistica). Tutti i comandi vanno al core.
//!
//! Con la feature `video` e un brano con il videoclip, a sinistra del testo, «Videoclip»: il pannello laterale
//! sul videoclip (acceso mentre è aperto).
//!
//! In una jam, a sinistra del testo, il segno della jam: il gruppo e il badge `LIVE` (il lime della direzione
//! artistica per «jam in progress»), che apre la pagina Jam; mentre ci si collega il gruppo ha il loader.
//! Fuori da una jam non c'è niente. Cambia solo quando cambia la fase, il codice o il numero di persone.
//!
//! La posizione non sta nell'istantanea: si calcola da `clock.at(now)` a ogni disegno. Un timer da 250 ms
//! (`PROGRESS_TICK_MS`) la ridisegna, e **esiste solo mentre suona**: in pausa, fermo o in caricamento
//! non c'è nessun timer e quindi nessun frame (la barra non si muove, non c'è niente da aggiornare). Neanche
//! quando la vista immersiva copre la barra (`set_hidden`).

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
use psytify_design::tokens::motion::PROGRESS_TICK_MS;
use psytify_design::tokens::size::{PLAYER_BAR_H, cover::COVER_S};
use psytify_design::tokens::space::{SPACE_3, SPACE_4, SPACE_5};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_NUMERIC, TYPE_TRACK_TITLE};
use uuid::Uuid;

use super::SidePanel;
use crate::components::badge::{Badge, BadgeKind};
use crate::components::icon_button::{IconButton, OnStyle};
use crate::components::player_control::{PlayerControl, Repeat, Transport};
use crate::components::progress::ProgressBar;
use crate::components::tile::Cover;
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{TypeStyle, col, dp};

/// Quello che la barra chiede alla finestra.
pub enum BarEvent {
    /// Il pulsante della coda è stato premuto.
    ToggleQueue,
    /// Il pulsante del testo (il pannello laterale sul testo).
    ToggleLyrics,
    /// Il pulsante della vista immersiva.
    ToggleFocus,
    /// Il nome dell'artista del brano in corso: apre la sua pagina (come nel client Iced).
    OpenArtist(uuid::Uuid),
    /// Il segno della jam: apre la pagina Jam.
    OpenJam,
    /// Il pulsante del videoclip (il pannello laterale sul videoclip), solo con la feature `video`.
    #[cfg(feature = "video")]
    ToggleVideo,
}

/// Il segno della jam nella barra.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum JamChip {
    /// Nessuna jam: niente.
    #[default]
    None,
    Connecting,
    /// In stanza: il codice e quante persone (per il suggerimento).
    Live {
        tip: SharedString,
    },
}

/// Il segno della jam dalla sua vista. Puro.
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
    /// «Videoclip», solo se il brano in corso ne ha uno.
    #[cfg(feature = "video")]
    fn video_button(&self, cx: &mut Context<Self>) -> Option<IconButton> {
        self.now.as_ref().filter(|now| now.track.has_video)?;
        Some(
            IconButton::new("video-toggle", sprite::ICON_VIDEO)
                .selected(self.panel == Some(SidePanel::Video))
                .on_style(OnStyle::Active)
                .tooltip("Videoclip", None)
                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(BarEvent::ToggleVideo))),
        )
    }

    #[cfg(not(feature = "video"))]
    fn video_button(&self, _: &mut Context<Self>) -> Option<IconButton> {
        None
    }
}

/// Il brano in corso, con i testi già pronti.
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
    /// Il pannello laterale aperto (il suo pulsante è acceso).
    panel: Option<SidePanel>,
    /// La vista immersiva copre la barra: niente battito.
    hidden: bool,
    /// Il timer da 250 ms: c'è solo mentre il brano suona.
    tick: Option<Task<()>>,
    /// Il testo del tempo trascorso, rifatto solo quando cambia il secondo.
    elapsed: (u64, SharedString),
    /// Il segno della jam.
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
            tick: None,
            elapsed: (u64::MAX, SharedString::default()),
            jam: JamChip::None,
            _changes: changes,
        };
        bar.jam = jam_chip(&bar.core.read(cx).jam_view());
        bar.rebuild_now();
        bar.sync_tick(cx);
        bar
    }

    /// La finestra dice quale pannello è aperto (il suo pulsante si accende).
    pub fn set_panel(&mut self, panel: Option<SidePanel>, cx: &mut Context<Self>) {
        if self.panel != panel {
            self.panel = panel;
            cx.notify();
        }
    }

    /// La vista immersiva copre la barra (o la scopre): coperta non ha il battito.
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
                self.rebuild_now();
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

    fn rebuild_now(&mut self) {
        self.now = self.view.queue.current().map(Now::of);
    }

    /// Accende il timer mentre suona (e la barra si vede) e lo spegne altrimenti.
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

        // Sinistra: copertina, testi, «mi piace».
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
                div()
                    .flex()
                    .items_center()
                    .gap(dp(SPACE_4))
                    .child(Cover::new(COVER_S).url(now.cover_url.clone()))
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
                                    .child(now.title.clone()),
                            )
                            .child(status),
                    )
                    .child(
                        IconButton::new("like", sprite::ICON_HEART)
                            .icon_on(sprite::ICON_HEART_FILL)
                            .on_style(OnStyle::Active)
                            .selected(liked)
                            .tooltip(
                                if liked { "Rimuovi dai preferiti" } else { "Aggiungi ai preferiti" },
                                None,
                            )
                            .on_click(move |_, _, cx| {
                                core.read(cx).library(LibraryCmd::ToggleLike(track.clone()));
                            }),
                    )
                    .into_any_element()
            }
            None => div()
                .flex()
                .items_center()
                .gap(dp(SPACE_4))
                .child(Cover::new(COVER_S))
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
                    .tooltip("Testo a tutto schermo", Some("F"))
                    .on_click(toggle_focus),
            );

        // Il filo sopra la barra lo disegna `Shell`: la barra entra in `.cached()` alla sua altezza esatta.
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
