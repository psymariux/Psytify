//! Il testo del brano in corso: un solo motore per il pannello laterale (`Look::Panel`) e per la vista
//! immersiva (`Look::Focus`), come nel client Iced (`client/src/ui/lyrics.rs`), con l'aspetto della
//! direzione artistica: la riga in corso è `TEXT_PRIMARY` con il cursore di 2 x 3 AP a sinistra (lime
//! mentre suona, `STONE` in pausa), quelle che arrivano `TEXT_SECONDARY`, quelle passate `TEXT_TERTIARY`;
//! niente ingrandimento della riga e niente parole che si accendono. Le pause strumentali sono tre puntini
//! di 2 x 2 AP che si accendono uno alla volta (`STONE` → `WARM_WHITE`). Un clic su un verso porta lì la
//! riproduzione. Il testo semplice (senza tempi) si scorre a mano.
//!
//! Il testo sincronizzato non è una lista che si scorre: ogni verso sta alla posizione che gli dà la riga in
//! corso (un `Tween` misurato in righe), così il testo scivola fino alla riga nuova in `MOTION_LYRICS`
//! (280 ms, ease-out), a 24 fotogrammi al secondo al massimo e allineato ai pixel fisici. Le altezze dei
//! versi (quante righe occupano andando a capo) si misurano con il sistema di testo una volta per testo e
//! per larghezza, mai a ogni disegno; si costruiscono solo i versi dentro il riquadro.
//!
//! Timer: la vista ne ha uno solo **mentre è visibile e il brano suona**, e non batte a intervalli: dorme
//! fino al prossimo cambio (l'inizio del verso dopo, o il prossimo puntino di una pausa), calcolato da
//! `clock.at(now)`, e ridisegna solo se la riga o i puntini sono cambiati davvero. In pausa, nascosta o
//! senza testo sincronizzato non c'è nessun timer. Un seek, una pausa o un brano nuovo arrivano come
//! pubblicazione del core e riarmano il timer da capo. Lo scorrimento ha un suo timer da 42 ms che vive
//! solo per i 280 ms del movimento.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, ClickEvent, Context, Entity, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, TextRun,
    Window, canvas, div, px,
};
use psytify_core::command::PlaybackCmd;
use psytify_core::player::PlaybackState;
use psytify_core::player::lyrics::{Line, Text, current};
use psytify_core::snapshot::{LyricsState, PlaybackView};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    STATE_ON, STONE, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY, WARM_WHITE,
};
use psytify_design::tokens::motion::{EASE_OUT_BEZIER, MOTION_LYRICS};
use psytify_design::tokens::space::{SPACE_3, SPACE_4, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{
    TYPE_BODY, TYPE_HEADING, TYPE_LYRIC_FOCUS, TYPE_LYRIC_PANEL, TYPE_META, TypeRole,
};

use crate::components::button::Button;
use crate::components::indicators::Skeleton;
use crate::components::overlay::EmptyState;
use crate::components::separator::Separator;
use crate::core_model::{Changed, CoreModel};
use crate::pixel::SpriteStore;
use crate::theme::{Ap, TypeStyle, col, dp};

/// Dove sta il testo: nel pannello laterale o nella vista immersiva.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Panel,
    Focus,
}

impl Look {
    /// `TYPE_LYRIC` 18/26 nel pannello, 24/32 nella vista immersiva.
    fn role(self) -> TypeRole {
        match self {
            Look::Panel => TYPE_LYRIC_PANEL,
            Look::Focus => TYPE_LYRIC_FOCUS,
        }
    }

    /// Spazio fra due versi.
    fn gap(self) -> f32 {
        match self {
            Look::Panel => SPACE_4,
            Look::Focus => SPACE_5,
        }
    }

    /// Dove sta la riga in corso, dall'alto, in frazione dell'altezza: nel pannello in alto (si legge
    /// quello che arriva), nella vista immersiva un po' sopra il centro. Come nel client Iced.
    fn anchor(self) -> f32 {
        match self {
            Look::Panel => 0.3,
            Look::Focus => 0.4,
        }
    }
}

/// Puntini di una pausa strumentale.
const DOTS: u32 = 3;
/// Un fotogramma dello scorrimento: 24 al secondo al massimo.
const FRAME: Duration = Duration::from_millis(42);
/// Il timer si sveglia appena dopo il cambio, non appena prima.
const WAKE_MARGIN: Duration = Duration::from_millis(2);
/// Spazio a sinistra dei versi: il cursore (2 AP) e lo stacco dal testo.
const GUTTER: f32 = SPACE_4;
/// Larghezze dei versi finti mentre il testo arriva, in frazione della riga.
const SKELETON: [f32; 6] = [0.86, 0.62, 0.92, 0.48, 0.74, 0.58];

/// Ease-out della direzione artistica, `cubic-bezier(0.2, 0, 0, 1)`: per una frazione del tempo dà
/// l'avanzamento (si cerca il parametro della curva per bisezione: x è monotona).
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

/// Un valore che va da `from` a `to` in `ms` millisecondi con l'ease-out.
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

    /// Riparte verso `to` da dove si trova adesso.
    fn retarget(&mut self, to: f32, now: Instant, ms: u32) {
        *self = Self { from: self.value(now), to, start: now, ms };
    }
}

/// Un verso è una pausa strumentale se non ha parole.
pub fn is_pause(line: &Line) -> bool {
    line.text.trim().is_empty()
}

/// Quanti puntini sono accesi in una pausa che va da `line.at` a `next`, alla posizione `position`: uno
/// all'inizio, tutti e tre nell'ultimo terzo. Senza un verso dopo resta acceso il primo.
pub fn dots_lit(line: &Line, next: Option<Duration>, position: Duration) -> u32 {
    let Some(next) = next else { return 1 };
    let span = next.saturating_sub(line.at).as_secs_f32();
    if span <= 0.0 {
        return DOTS;
    }
    let fraction = position.saturating_sub(line.at).as_secs_f32() / span;
    ((fraction * DOTS as f32).floor() as u32 + 1).min(DOTS)
}

/// Fra quanto cambia qualcosa da mostrare alla posizione `position`: l'inizio del verso dopo o, dentro una
/// pausa, il prossimo puntino. `None` dopo l'ultimo verso: non c'è niente da aspettare.
pub fn next_change(lines: &[Line], position: Duration) -> Option<Duration> {
    let now = current(lines, position);
    let next_line = lines.get(now.map_or(0, |ix| ix + 1)).map(|line| line.at);
    let mut soonest = next_line.map(|at| at.saturating_sub(position));
    if let (Some(ix), Some(next)) = (now, next_line)
        && let Some(line) = lines.get(ix)
        && is_pause(line)
    {
        let span = next.saturating_sub(line.at);
        let step = (1..DOTS).map(|k| line.at + span * k / DOTS).find(|step| *step > position);
        if let Some(step) = step {
            soonest = Some(soonest.map_or(step - position, |s| s.min(step - position)));
        }
    }
    soonest
}

/// La cima di ogni verso, dalle altezze.
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

/// Il punto (dalla cima del testo) che deve stare all'altezza dell'ancora: il centro del verso in
/// `position`, che può essere fra due versi mentre il testo scorre. Prima del primo verso (`-1`) il testo
/// sta più in basso di un verso. Come `focus_y` del client Iced.
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

/// I versi interi dentro un riquadro alto `height` con il testo spostato in su di `offset` (come nel client
/// Iced: un verso tagliato a metà dal bordo non si disegna).
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

/// Cosa mostra la vista.
#[derive(Clone, Debug, PartialEq)]
enum Shown {
    /// Nessun brano in coda.
    NoTrack,
    Loading,
    Failed(SharedString),
    /// LRCLIB non ha un testo per questo brano.
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

/// Le altezze dei versi di un testo a una larghezza: si rifanno solo se cambia uno dei due.
struct Layout {
    text: Arc<Text>,
    width: f32,
    heights: Vec<f32>,
    tops: Vec<f32>,
}

pub struct LyricsView {
    core: Entity<CoreModel>,
    look: Look,
    /// La vista è sullo schermo (pannello aperto sul testo, o vista immersiva aperta).
    visible: bool,
    view: Arc<PlaybackView>,
    shown: Shown,
    /// I versi come `SharedString`, fatti una volta per testo.
    verses: Vec<SharedString>,
    /// «Titolo · Artista» per la testata del pannello.
    track_label: SharedString,
    layout: Option<Layout>,
    /// Il verso in corso e i puntini accesi, se è una pausa.
    line: Option<usize>,
    dots: u32,
    /// Dove sta il testo, in versi (`-1` prima del primo).
    scroll: Tween,
    /// Il riquadro del testo all'ultimo disegno, in dp: lo misura il prepaint.
    viewport: Rc<Cell<(f32, f32)>>,
    /// Il timer del prossimo cambio: c'è solo mentre la vista si vede e il brano suona.
    follow: Option<Task<()>>,
    /// I fotogrammi dello scorrimento: ci sono solo mentre il testo si muove.
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
            track_label: SharedString::default(),
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

    /// La finestra dice se la vista è sullo schermo: solo allora ha un timer.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible {
            // Riappare sul verso giusto, senza scorrere da dove era rimasta.
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
        let label = view
            .queue
            .current()
            .map(|track| SharedString::from(format!("{} · {}", track.title, track.artist.name)))
            .unwrap_or_default();
        if label != self.track_label {
            self.track_label = label;
            dirty = true;
        }
        let playing_changed = view.is_playing() != self.view.is_playing();
        self.view = view;
        // Un brano nuovo parte dal suo verso senza scorrere; un seek o il verso dopo scorrono.
        let animate = !dirty;
        if self.follow_position(animate, cx) || playing_changed || dirty {
            // Anche il cursore cambia colore fra suona e pausa.
            cx.notify();
        }
        self.sync_follow(cx);
    }

    /// Il verso e i puntini alla posizione di adesso. `true` se sono cambiati: allora il testo scorre
    /// verso il verso nuovo (o ci salta, senza `animate`).
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
            let ms = if animate && self.visible { MOTION_LYRICS.effective_ms(reduce) } else { 0 };
            self.scroll.retarget(target, now, ms);
            if ms > 0 {
                self.run_frames(cx);
            }
        }
        changed
    }

    /// I fotogrammi dello scorrimento: uno ogni 42 ms finché il testo si muove, poi niente.
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

    /// Arma il timer del prossimo cambio, o lo spegne: c'è solo se la vista si vede, il brano suona e il
    /// testo è sincronizzato. Ogni chiamata sostituisce il timer di prima.
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

    /// Le altezze dei versi alla larghezza `width` (dp), misurate dal sistema di testo come le andrà a capo
    /// il disegno. Solo quando cambiano il testo o la larghezza.
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
                        .shape_text(verse.clone(), px(role.size), &[run], Some(px(width)), None)
                        .map_or(1, |lines| lines.iter().map(|line| line.wrap_boundaries().len() + 1).sum())
                };
                rows as f32 * role.line_height + self.look.gap()
            })
            .collect();
        self.layout = Some(Layout { text: Arc::clone(text), width, tops: tops(&heights), heights });
    }

    /// Il testo sincronizzato nel riquadro misurato.
    fn synced(&mut self, text: &Arc<Text>, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let (width, height) = self.viewport.get();
        if width <= GUTTER || height <= 0.0 {
            // Il primo disegno, prima che il prepaint misuri il riquadro: niente versi misurati a vuoto.
            return div().into_any_element();
        }
        let text_width = width - GUTTER;
        self.ensure_layout(text, text_width, window);
        let Some(layout) = &self.layout else { return div().into_any_element() };
        let scale = window.scale_factor();
        let ap = Ap::for_scale(scale);
        let role = self.look.role();
        let now = Instant::now();
        let focus = focus_y(&layout.tops, &layout.heights, self.scroll.value(now));
        // Allineato ai pixel fisici, così il testo non sfoca mentre scorre.
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
            // Il cursore: un blocco di 2 x 3 AP centrato sulla prima riga del verso.
            let cursor = is_current.then(|| {
                div()
                    .absolute()
                    .left_0()
                    .top(px((role.line_height - f32::from(ap.n(3))) / 2.0))
                    .w(ap.n(2))
                    .h(ap.n(3))
                    .bg(col(if playing { STATE_ON } else { STONE }))
            });
            let words: AnyElement = if is_pause(line) {
                let lit = if is_current { self.dots } else { 0 };
                div()
                    .h(px(role.line_height))
                    .flex()
                    .items_center()
                    .gap(ap.n(2))
                    .children((0..DOTS).map(|dot| {
                        div().w(ap.n(2)).h(ap.n(2)).bg(col(if dot < lit { WARM_WHITE } else { STONE }))
                    }))
                    .into_any_element()
            } else {
                div()
                    .w(px(text_width))
                    .type_role(role)
                    .text_color(col(color))
                    .hover(|style| style.text_color(col(TEXT_PRIMARY)))
                    .child(verse.clone())
                    .into_any_element()
            };
            let at = line.at;
            verses.push(
                div()
                    .id(("lyric", ix))
                    .absolute()
                    .left_0()
                    .top(px(top))
                    .w(px(width))
                    .h(px(layout.heights[ix]))
                    .pl(dp(GUTTER))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.seek(at, cx)))
                    .children(cursor)
                    .child(words),
            );
        }
        div().size_full().children(verses).into_any_element()
    }

    /// Versi grigi della misura di quelli veri, mentre il testo arriva.
    fn loading(&self) -> AnyElement {
        let (width, _) = self.viewport.get();
        let role = self.look.role();
        div()
            .pl(dp(GUTTER))
            .flex()
            .flex_col()
            .gap(px(role.line_height - role.size * 0.8 + self.look.gap()))
            .pt(dp(SPACE_3))
            .children(
                SKELETON
                    .iter()
                    .map(|share| Skeleton::new(((width - GUTTER) * share).max(0.0), role.size * 0.8)),
            )
            .into_any_element()
    }

    /// Una frase al posto del testo (nessun brano, testo che non c'è).
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
                        // Un nuovo «mostro il testo» fa riprovare il core.
                        core.read(cx).playback(PlaybackCmd::Lyrics(true));
                    }))
                    .into_any_element()
            }
            Shown::Missing => match self.look {
                // Nel pannello solo la frase; nella vista immersiva anche Nastro che ascolta.
                Look::Panel => Self::note("Nessun testo per questo brano").into_any_element(),
                Look::Focus => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(EmptyState::new(
                        // Nastro ascolta (il LED lime) solo mentre il brano suona.
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

    /// Il riquadro del testo, con la misura presa nel prepaint: se cambia, la vista si ridisegna una volta
    /// al fotogramma dopo con le misure nuove.
    fn frame(&mut self, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let viewport = Rc::clone(&self.viewport);
        let measure = canvas(
            move |bounds, window, _| {
                let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
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

impl Render for LyricsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.look {
            Look::Focus => div().size_full().child(self.frame(window, cx)),
            Look::Panel => div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .px(dp(SPACE_5))
                        .py(dp(SPACE_4))
                        .child(div().type_role(TYPE_HEADING).text_color(col(TEXT_PRIMARY)).child("Testo"))
                        .child(
                            div()
                                .type_role(TYPE_META)
                                .text_color(col(TEXT_TERTIARY))
                                .truncate()
                                .child(self.track_label.clone()),
                        ),
                )
                .child(Separator::horizontal())
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .mt(dp(SPACE_4))
                        .mr(dp(SPACE_5))
                        .ml(dp(SPACE_4))
                        .child(self.frame(window, cx)),
                )
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

    fn verse(at_ms: u64, text: &str) -> Line {
        Line { at: Duration::from_millis(at_ms), text: text.into() }
    }

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn the_ease_out_starts_fast_and_lands_on_one() {
        assert!(ease_out(0.0).abs() < 1e-4);
        assert!((ease_out(1.0) - 1.0).abs() < 1e-4);
        assert!(ease_out(0.25) > 0.5, "{}", ease_out(0.25));
        let samples: Vec<f32> = (0..=20).map(|i| ease_out(i as f32 / 20.0)).collect();
        assert!(samples.windows(2).all(|pair| pair[1] >= pair[0]), "mai indietro");
    }

    #[test]
    fn the_next_change_is_the_start_of_the_next_verse() {
        let lines = [verse(1_000, "a"), verse(5_000, "b")];
        assert_eq!(next_change(&lines, ms(0)), Some(ms(1_000)));
        assert_eq!(next_change(&lines, ms(1_000)), Some(ms(4_000)));
        assert_eq!(next_change(&lines, ms(4_900)), Some(ms(100)));
        assert_eq!(next_change(&lines, ms(5_000)), None, "dopo l'ultimo verso non si aspetta niente");
    }

    #[test]
    fn a_pause_wakes_for_each_dot() {
        let lines = [verse(0, "a"), verse(3_000, ""), verse(9_000, "b")];
        assert_eq!(next_change(&lines, ms(3_000)), Some(ms(2_000)));
        assert_eq!(next_change(&lines, ms(5_500)), Some(ms(1_500)));
        assert_eq!(next_change(&lines, ms(7_000)), Some(ms(2_000)), "l'ultimo terzo aspetta il verso");
        assert_eq!(dots_lit(&lines[1], Some(ms(9_000)), ms(3_000)), 1);
        assert_eq!(dots_lit(&lines[1], Some(ms(9_000)), ms(5_000)), 2);
        assert_eq!(dots_lit(&lines[1], Some(ms(9_000)), ms(8_999)), 3);
        assert_eq!(dots_lit(&lines[1], None, ms(8_000)), 1);
    }

    #[test]
    fn the_wait_is_never_zero() {
        // Due versi con lo stesso tempo: si aspetta quello dopo, mai un timer da 0 che gira a vuoto.
        let lines = [verse(1_000, "a"), verse(1_000, "a bis"), verse(2_000, "b")];
        assert_eq!(next_change(&lines, ms(1_000)), Some(ms(1_000)));
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
        assert_eq!(visible(&tops, &heights, 0.0, 100.0), 0..2, "il terzo uscirebbe dal bordo");
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
        // Riparte da dove si trova, non da dove era partito.
        tween.retarget(0.0, start + ms(140), 280);
        assert!((tween.value(start + ms(140)) - mid).abs() < 1e-4);
    }
}
