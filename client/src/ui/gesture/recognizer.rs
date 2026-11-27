//! Il riconoscitore dei gesti: logica pura, senza Iced e senza orologio proprio.
//!
//! Riceve gli eventi di un dito con il loro istante (`Duration` da un'origine qualsiasi) e dice
//! che gesto sono: un tocco, una pressione lunga, un trascinamento su un asse, un lancio. Le soglie
//! stanno in dp e si convertono con la scala dello schermo (`Config::scaled`). Per i test e per
//! un eventuale altro frontend (GPUI, Kotlin) non serve altro che questo file.
//!
//! Regole, in ordine di arrivo:
//!
//! - Il dito scende: comincia un gesto *incerto*.
//! - Se si muove oltre la soglia (`slop`) prima di `long_press`, il gesto diventa un
//!   trascinamento e l'asse si blocca: orizzontale solo se lo spostamento orizzontale supera quello
//!   verticale di `horizontal_bias` volte (un gesto in diagonale è verticale, cioè scorre la pagina).
//!   L'asse non cambia più fino al rilascio (isteresi).
//! - Se resta fermo (entro la soglia) per `long_press`, è una pressione lunga. Da quel momento ogni
//!   movimento è un trascinamento dopo pressione lunga (riordino) e non un trascinamento normale.
//! - Se si stacca prima, entro la soglia, è un tocco.
//! - Dopo un trascinamento, il rilascio è un lancio se la velocità lungo l'asse, misurata sugli
//!   ultimi `velocity_window`, supera `min_fling`; se il dito si era fermato prima di staccarsi
//!   (più di `stale_after` dall'ultimo movimento) la velocità è zero.
//! - Un secondo dito, o l'annullamento del sistema, chiude il gesto con `Cancel`; finché non si
//!   stacca l'ultimo dito, niente di nuovo comincia.

use std::time::Duration;

/// Un punto, in dp (o in pixel: le soglie si convertono con la scala).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

impl Pt {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl std::ops::Sub for Pt {
    type Output = Pt;

    fn sub(self, other: Pt) -> Pt {
        Pt::new(self.x - other.x, self.y - other.y)
    }
}

impl std::ops::Add for Pt {
    type Output = Pt;

    fn add(self, other: Pt) -> Pt {
        Pt::new(self.x + other.x, self.y + other.y)
    }
}

/// Lungo quale asse va un trascinamento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    /// La componente di `v` lungo questo asse.
    pub fn of(self, v: Pt) -> f32 {
        match self {
            Self::Horizontal => v.x,
            Self::Vertical => v.y,
        }
    }
}

/// Le soglie, in dp. `scaled` le converte nell'unità delle coordinate in ingresso.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    /// Quanto può muoversi il dito prima che il tocco diventi un trascinamento.
    pub slop: f32,
    /// Quanto deve restare fermo per una pressione lunga.
    pub long_press: Duration,
    /// Quanto deve superare lo spostamento verticale quello orizzontale perché l'asse sia
    /// orizzontale (1 = il più grande vince; più alto = la diagonale va in verticale).
    pub horizontal_bias: f32,
    /// Finestra su cui si misura la velocità del rilascio.
    pub velocity_window: Duration,
    /// Se l'ultimo movimento è più vecchio di così al rilascio, il dito si era fermato.
    pub stale_after: Duration,
    /// Sotto questa velocità (dp/s lungo l'asse) il rilascio non è un lancio.
    pub min_fling: f32,
    /// Il lancio più veloce che si riporta (dp/s).
    pub max_fling: f32,
}

/// Le soglie di Psytify, in dp. La stessa regola dell'asse e la stessa soglia sono ripetute in
/// `android/vendor/iced_winit/src/program/touch.rs`: vanno tenute uguali, altrimenti la pagina
/// scorrerebbe su un asse mentre la riga si trascina sull'altro.
pub const SLOP_DP: f32 = 8.0;
pub const HORIZONTAL_BIAS: f32 = 1.25;
pub const LONG_PRESS: Duration = Duration::from_millis(450);

impl Config {
    /// Le soglie in dp, per coordinate espresse in dp.
    pub const DP: Self = Self {
        slop: SLOP_DP,
        long_press: LONG_PRESS,
        horizontal_bias: HORIZONTAL_BIAS,
        velocity_window: Duration::from_millis(90),
        stale_after: Duration::from_millis(100),
        min_fling: 300.0,
        max_fling: 8000.0,
    };

    #[allow(dead_code, reason = "API per altri frontend e per i test")]
    /// Le stesse soglie per coordinate in pixel fisici: `scale` sono i pixel per dp.
    pub fn scaled(scale: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        Self {
            slop: Self::DP.slop * scale,
            min_fling: Self::DP.min_fling * scale,
            max_fling: Self::DP.max_fling * scale,
            ..Self::DP
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::DP
    }
}

/// Che asse sceglie uno spostamento `moved` rispetto al punto di partenza.
pub fn lock_axis(moved: Pt, bias: f32) -> Axis {
    if moved.x.abs() > moved.y.abs() * bias { Axis::Horizontal } else { Axis::Vertical }
}

/// Un evento del dito.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    Down {
        id: u64,
        at: Pt,
        time: Duration,
    },
    Move {
        id: u64,
        at: Pt,
        time: Duration,
    },
    Up {
        id: u64,
        at: Pt,
        time: Duration,
    },
    /// Il sistema (o chi contiene il gesto, per esempio una lista che prende lo scorrimento) lo
    /// annulla.
    Cancel,
}

/// Il gesto riconosciuto.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Output {
    /// Dito sceso e risalito qui, senza muoversi e prima della pressione lunga.
    Tap { at: Pt },
    /// Il dito è fermo da `long_press`: da qui in poi `HeldMove` e `HeldEnd`.
    LongPress { at: Pt },
    /// Dopo la pressione lunga il dito si muove (`delta` dall'ultima posizione).
    HeldMove { at: Pt, delta: Pt },
    /// Dopo la pressione lunga il dito si stacca.
    HeldEnd { at: Pt },
    /// Il trascinamento comincia: l'asse è deciso. `total` è lo spostamento dal punto di partenza.
    PanStart { axis: Axis, from: Pt, total: Pt },
    /// Il trascinamento prosegue (`delta` dall'ultimo evento, `total` dalla partenza).
    PanMove { axis: Axis, at: Pt, delta: Pt, total: Pt },
    /// Il dito si stacca piano o fermo: niente lancio.
    PanEnd { axis: Axis, total: Pt },
    /// Il dito si stacca in movimento: `velocity` in unità al secondo (componenti x, y).
    Fling { axis: Axis, velocity: Pt, total: Pt },
    /// Il gesto è annullato (secondo dito, sistema, chi contiene).
    Cancel,
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    at: Pt,
    time: Duration,
}

/// Quanti campioni si tengono per la velocità: bastano gli ultimi, a 120 Hz sono ~12 in 100 ms.
const SAMPLES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Nessun dito.
    Idle,
    /// Dito giù, ancora incerto.
    Pending,
    /// Pressione lunga scattata.
    Held,
    /// Trascinamento con asse bloccato.
    Panning(Axis),
    /// Gesto chiuso (cancellato, o secondo dito): si aspetta che tutte le dita salgano.
    Dead,
}

/// Il riconoscitore di un dito alla volta.
#[derive(Debug, Clone)]
pub struct Recognizer {
    config: Config,
    phase: Phase,
    id: u64,
    start: Pt,
    started: Duration,
    last: Pt,
    /// Quante dita sono giù (serve a `Dead`).
    down: u32,
    samples: [Option<Sample>; SAMPLES],
    next_sample: usize,
}

impl Default for Recognizer {
    fn default() -> Self {
        Self::new(Config::DP)
    }
}

impl Recognizer {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            phase: Phase::Idle,
            id: 0,
            start: Pt::ZERO,
            started: Duration::ZERO,
            last: Pt::ZERO,
            down: 0,
            samples: [None; SAMPLES],
            next_sample: 0,
        }
    }

    #[allow(dead_code, reason = "API per altri frontend e per i test")]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// C'è un gesto in corso (dito giù e non annullato).
    pub fn is_active(&self) -> bool {
        matches!(self.phase, Phase::Pending | Phase::Held | Phase::Panning(_))
    }

    #[allow(dead_code, reason = "API per altri frontend e per i test")]
    /// L'asse del trascinamento in corso, se c'è.
    pub fn axis(&self) -> Option<Axis> {
        match self.phase {
            Phase::Panning(axis) => Some(axis),
            _ => None,
        }
    }

    #[allow(dead_code, reason = "API per altri frontend e per i test")]
    /// Dopo la pressione lunga e prima del rilascio.
    pub fn is_held(&self) -> bool {
        self.phase == Phase::Held
    }

    /// Quando scatterà la pressione lunga, se il dito è giù e fermo: serve a chiedere un
    /// ridisegno a quell'istante senza un timer che gira.
    pub fn deadline(&self) -> Option<Duration> {
        (self.phase == Phase::Pending).then(|| self.started + self.config.long_press)
    }

    #[allow(dead_code, reason = "API per altri frontend e per i test")]
    /// Il punto da cui è partito il gesto.
    pub fn origin(&self) -> Pt {
        self.start
    }

    fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.samples = [None; SAMPLES];
        self.next_sample = 0;
    }

    fn push_sample(&mut self, at: Pt, time: Duration) {
        self.samples[self.next_sample] = Some(Sample { at, time });
        self.next_sample = (self.next_sample + 1) % SAMPLES;
    }

    /// La velocità (unità/s) sugli ultimi `velocity_window` prima di `now`, zero se il dito si era
    /// fermato.
    fn velocity(&self, now: Duration) -> Pt {
        let newest = self.samples.iter().flatten().max_by_key(|s| s.time).copied();
        let Some(newest) = newest else {
            return Pt::ZERO;
        };
        if now.saturating_sub(newest.time) > self.config.stale_after {
            return Pt::ZERO;
        }
        let from = newest.time.saturating_sub(self.config.velocity_window);
        let oldest = self.samples.iter().flatten().filter(|s| s.time >= from).min_by_key(|s| s.time).copied();
        let Some(oldest) = oldest else {
            return Pt::ZERO;
        };
        let dt = newest.time.saturating_sub(oldest.time).as_secs_f32();
        if dt <= f32::EPSILON {
            return Pt::ZERO;
        }
        Pt::new((newest.at.x - oldest.at.x) / dt, (newest.at.y - oldest.at.y) / dt)
    }

    /// Fa scattare ciò che dipende dal tempo (la pressione lunga) senza un nuovo evento.
    pub fn poll(&mut self, now: Duration) -> Option<Output> {
        if self.phase == Phase::Pending && now.saturating_sub(self.started) >= self.config.long_press {
            self.phase = Phase::Held;
            return Some(Output::LongPress { at: self.start });
        }
        None
    }

    /// Un evento del dito: al più un gesto in uscita.
    pub fn feed(&mut self, input: Input) -> Option<Output> {
        match input {
            Input::Down { id, at, time } => self.down_at(id, at, time),
            Input::Move { id, at, time } => self.move_to(id, at, time),
            Input::Up { id, at, time } => self.up_at(id, at, time),
            Input::Cancel => {
                let was_active = self.is_active();
                self.down = 0;
                self.reset();
                was_active.then_some(Output::Cancel)
            }
        }
    }

    fn down_at(&mut self, id: u64, at: Pt, time: Duration) -> Option<Output> {
        self.down += 1;
        match self.phase {
            Phase::Idle => {
                self.phase = Phase::Pending;
                self.id = id;
                self.start = at;
                self.last = at;
                self.started = time;
                self.samples = [None; SAMPLES];
                self.next_sample = 0;
                self.push_sample(at, time);
                None
            }
            Phase::Dead => None,
            _ => {
                // Un secondo dito: il gesto del primo è annullato.
                self.phase = Phase::Dead;
                Some(Output::Cancel)
            }
        }
    }

    fn move_to(&mut self, id: u64, at: Pt, time: Duration) -> Option<Output> {
        if id != self.id {
            return None;
        }
        match self.phase {
            Phase::Pending => {
                if time.saturating_sub(self.started) >= self.config.long_press {
                    // La pressione lunga è scaduta prima che arrivasse questo movimento: scatta e,
                    // se il dito è ancora entro la soglia, il movimento è già del dopo.
                    let moved = at - self.start;
                    if moved.x.hypot(moved.y) < self.config.slop {
                        self.phase = Phase::Held;
                        self.last = at;
                        return Some(Output::LongPress { at: self.start });
                    }
                }
                self.push_sample(at, time);
                let moved = at - self.start;
                if moved.x.hypot(moved.y) < self.config.slop {
                    return None;
                }
                let axis = lock_axis(moved, self.config.horizontal_bias);
                self.phase = Phase::Panning(axis);
                self.last = at;
                Some(Output::PanStart { axis, from: self.start, total: moved })
            }
            Phase::Held => {
                let delta = at - self.last;
                self.last = at;
                Some(Output::HeldMove { at, delta })
            }
            Phase::Panning(axis) => {
                self.push_sample(at, time);
                let delta = at - self.last;
                self.last = at;
                Some(Output::PanMove { axis, at, delta, total: at - self.start })
            }
            Phase::Idle | Phase::Dead => None,
        }
    }

    fn up_at(&mut self, id: u64, at: Pt, time: Duration) -> Option<Output> {
        self.down = self.down.saturating_sub(1);
        match self.phase {
            Phase::Dead => {
                if self.down == 0 {
                    self.reset();
                }
                None
            }
            _ if id != self.id => None,
            Phase::Pending => {
                self.reset();
                // Il dito può essersi staccato dopo la scadenza senza che nessuno chiedesse `poll`:
                // anche allora è una pressione lunga che finisce subito.
                let moved = at - self.start;
                if moved.x.hypot(moved.y) >= self.config.slop {
                    // Si è mosso senza che sia arrivato un Move: un trascinamento di un solo passo.
                    let axis = lock_axis(moved, self.config.horizontal_bias);
                    return Some(Output::PanEnd { axis, total: moved });
                }
                if time.saturating_sub(self.started) >= self.config.long_press {
                    return Some(Output::LongPress { at: self.start });
                }
                Some(Output::Tap { at })
            }
            Phase::Held => {
                self.reset();
                Some(Output::HeldEnd { at })
            }
            Phase::Panning(axis) => {
                self.push_sample(at, time);
                let velocity = self.velocity(time);
                let total = at - self.start;
                self.reset();
                let along = axis.of(velocity);
                if along.abs() >= self.config.min_fling {
                    let clamp = |v: f32| v.clamp(-self.config.max_fling, self.config.max_fling);
                    let velocity = Pt::new(clamp(velocity.x), clamp(velocity.y));
                    Some(Output::Fling { axis, velocity, total })
                } else {
                    Some(Output::PanEnd { axis, total })
                }
            }
            Phase::Idle => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: u64 = 7;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn down(x: f32, y: f32, t: u64) -> Input {
        Input::Down { id: ID, at: Pt::new(x, y), time: ms(t) }
    }
    fn mv(x: f32, y: f32, t: u64) -> Input {
        Input::Move { id: ID, at: Pt::new(x, y), time: ms(t) }
    }
    fn up(x: f32, y: f32, t: u64) -> Input {
        Input::Up { id: ID, at: Pt::new(x, y), time: ms(t) }
    }

    fn run(r: &mut Recognizer, inputs: &[Input]) -> Vec<Output> {
        inputs.iter().filter_map(|i| r.feed(*i)).collect()
    }

    #[test]
    fn a_quick_touch_is_a_tap() {
        let mut r = Recognizer::default();
        let out = run(&mut r, &[down(10.0, 10.0, 0), up(10.0, 10.0, 80)]);
        assert_eq!(out, vec![Output::Tap { at: Pt::new(10.0, 10.0) }]);
        assert!(!r.is_active());
    }

    #[test]
    fn jitter_below_the_slop_is_still_a_tap() {
        let mut r = Recognizer::default();
        let out = run(
            &mut r,
            &[
                down(10.0, 10.0, 0),
                mv(12.0, 11.0, 20),
                mv(9.0, 13.0, 40),
                mv(14.0, 9.0, 60),
                up(13.0, 10.0, 90),
            ],
        );
        assert_eq!(out, vec![Output::Tap { at: Pt::new(13.0, 10.0) }]);
    }

    #[test]
    fn a_slow_tap_just_before_the_long_press_is_still_a_tap() {
        let mut r = Recognizer::default();
        let out = run(&mut r, &[down(0.0, 0.0, 0), up(0.0, 0.0, 449)]);
        assert_eq!(out, vec![Output::Tap { at: Pt::ZERO }]);
    }

    #[test]
    fn holding_still_fires_one_long_press_through_poll() {
        let mut r = Recognizer::default();
        r.feed(down(20.0, 30.0, 0));
        assert_eq!(r.deadline(), Some(ms(450)));
        assert_eq!(r.poll(ms(449)), None);
        assert_eq!(r.poll(ms(450)), Some(Output::LongPress { at: Pt::new(20.0, 30.0) }));
        // Una volta sola.
        assert_eq!(r.poll(ms(900)), None);
        assert_eq!(r.deadline(), None);
        assert!(r.is_held());
    }

    #[test]
    fn no_tap_follows_a_long_press() {
        let mut r = Recognizer::default();
        r.feed(down(5.0, 5.0, 0));
        assert!(matches!(r.poll(ms(500)), Some(Output::LongPress { .. })));
        let end = r.feed(up(5.0, 5.0, 700));
        assert_eq!(end, Some(Output::HeldEnd { at: Pt::new(5.0, 5.0) }));
    }

    #[test]
    fn a_release_after_the_deadline_without_poll_is_a_long_press_not_a_tap() {
        let mut r = Recognizer::default();
        let out = run(&mut r, &[down(5.0, 5.0, 0), up(5.0, 5.0, 600)]);
        assert_eq!(out, vec![Output::LongPress { at: Pt::new(5.0, 5.0) }]);
    }

    #[test]
    fn a_late_move_within_the_slop_triggers_the_long_press() {
        let mut r = Recognizer::default();
        r.feed(down(0.0, 0.0, 0));
        assert_eq!(r.feed(mv(2.0, 1.0, 460)), Some(Output::LongPress { at: Pt::ZERO }));
        assert!(r.is_held());
    }

    #[test]
    fn moving_past_the_slop_cancels_the_long_press_and_starts_a_pan() {
        let mut r = Recognizer::default();
        r.feed(down(0.0, 0.0, 0));
        let start = r.feed(mv(0.0, 12.0, 100));
        assert_eq!(
            start,
            Some(Output::PanStart { axis: Axis::Vertical, from: Pt::ZERO, total: Pt::new(0.0, 12.0) })
        );
        // Il tempo passa, ma il trascinamento non diventa una pressione lunga.
        assert_eq!(r.poll(ms(1000)), None);
        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn held_moves_report_the_delta_for_reordering() {
        let mut r = Recognizer::default();
        r.feed(down(10.0, 10.0, 0));
        r.poll(ms(450));
        assert_eq!(
            r.feed(mv(10.0, 40.0, 500)),
            Some(Output::HeldMove { at: Pt::new(10.0, 40.0), delta: Pt::new(0.0, 30.0) })
        );
        assert_eq!(
            r.feed(mv(10.0, 35.0, 520)),
            Some(Output::HeldMove { at: Pt::new(10.0, 35.0), delta: Pt::new(0.0, -5.0) })
        );
        assert_eq!(r.feed(up(10.0, 35.0, 540)), Some(Output::HeldEnd { at: Pt::new(10.0, 35.0) }));
    }

    #[test]
    fn a_horizontal_start_locks_the_horizontal_axis() {
        let mut r = Recognizer::default();
        r.feed(down(0.0, 0.0, 0));
        let out = r.feed(mv(14.0, 3.0, 30));
        assert!(matches!(out, Some(Output::PanStart { axis: Axis::Horizontal, .. })));
        assert_eq!(r.axis(), Some(Axis::Horizontal));
    }

    #[test]
    fn a_diagonal_start_goes_vertical_because_scrolling_wins_ties() {
        // 45 gradi: lo spostamento orizzontale non supera 1,25 volte il verticale.
        let mut r = Recognizer::default();
        r.feed(down(0.0, 0.0, 0));
        let out = r.feed(mv(10.0, 10.0, 30));
        assert!(matches!(out, Some(Output::PanStart { axis: Axis::Vertical, .. })));
        // Anche un diagonale un po' più orizzontale (1,2 volte) è verticale.
        let mut r = Recognizer::default();
        r.feed(down(0.0, 0.0, 0));
        assert!(matches!(r.feed(mv(12.0, 10.0, 30)), Some(Output::PanStart { axis: Axis::Vertical, .. })));
        // Oltre la soglia di bias diventa orizzontale.
        let mut r = Recognizer::default();
        r.feed(down(0.0, 0.0, 0));
        assert!(matches!(r.feed(mv(13.0, 10.0, 30)), Some(Output::PanStart { axis: Axis::Horizontal, .. })));
    }

    #[test]
    fn the_axis_never_changes_once_locked() {
        let mut r = Recognizer::default();
        r.feed(down(0.0, 0.0, 0));
        r.feed(mv(20.0, 0.0, 20));
        // Il dito poi scende molto: l'asse resta orizzontale.
        let out = r.feed(mv(22.0, 200.0, 40));
        assert!(matches!(out, Some(Output::PanMove { axis: Axis::Horizontal, .. })));
        assert_eq!(r.axis(), Some(Axis::Horizontal));
    }

    #[test]
    fn pan_moves_report_delta_and_total() {
        let mut r = Recognizer::default();
        r.feed(down(100.0, 100.0, 0));
        r.feed(mv(100.0, 112.0, 16));
        let out = r.feed(mv(101.0, 130.0, 32));
        assert_eq!(
            out,
            Some(Output::PanMove {
                axis: Axis::Vertical,
                at: Pt::new(101.0, 130.0),
                delta: Pt::new(1.0, 18.0),
                total: Pt::new(1.0, 30.0)
            })
        );
    }

    #[test]
    fn a_fast_release_is_a_fling_with_the_velocity_of_the_last_samples() {
        let mut r = Recognizer::default();
        // 10 dp ogni 10 ms = 1000 dp/s, verso l'alto.
        let mut inputs = vec![down(0.0, 500.0, 0)];
        for i in 1..=10u64 {
            inputs.push(mv(0.0, 500.0 - 10.0 * i as f32, 10 * i));
        }
        inputs.push(up(0.0, 400.0, 105));
        let out = run(&mut r, &inputs);
        match out.last() {
            Some(Output::Fling { axis: Axis::Vertical, velocity, total }) => {
                assert!((velocity.y + 1000.0).abs() < 60.0, "{velocity:?}");
                assert_eq!(velocity.x, 0.0);
                assert_eq!(total.y, -100.0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn velocity_comes_from_the_last_window_not_the_whole_gesture() {
        let mut r = Recognizer::default();
        // Prima lentissimo per 600 ms, poi uno scatto veloce negli ultimi 50 ms.
        let mut inputs = vec![down(0.0, 0.0, 0)];
        for i in 1..=6u64 {
            inputs.push(mv(i as f32 * 3.0, 0.0, 100 * i));
        }
        inputs.push(mv(38.0, 0.0, 620));
        inputs.push(mv(58.0, 0.0, 640));
        inputs.push(mv(78.0, 0.0, 660));
        inputs.push(up(98.0, 0.0, 670));
        let out = run(&mut r, &inputs);
        match out.last() {
            Some(Output::Fling { axis: Axis::Horizontal, velocity, .. }) => {
                assert!(velocity.x > 1000.0, "{velocity:?}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_slow_release_is_a_pan_end_not_a_fling() {
        let mut r = Recognizer::default();
        // 100 dp/s: sotto i 300 dp/s di soglia.
        let mut inputs = vec![down(0.0, 0.0, 0)];
        for i in 1..=10u64 {
            inputs.push(mv(i as f32 * 2.0, 0.0, 20 * i));
        }
        inputs.push(up(21.0, 0.0, 210));
        let out = run(&mut r, &inputs);
        assert!(matches!(out.last(), Some(Output::PanEnd { axis: Axis::Horizontal, .. })), "{out:?}");
    }

    #[test]
    fn stopping_before_lifting_cancels_the_fling() {
        let mut r = Recognizer::default();
        let mut inputs = vec![down(0.0, 0.0, 0)];
        for i in 1..=5u64 {
            inputs.push(mv(0.0, 30.0 * i as f32, 10 * i));
        }
        // Fermo 300 ms, poi si stacca: il dito si era fermato.
        inputs.push(up(0.0, 150.0, 350));
        let out = run(&mut r, &inputs);
        assert!(matches!(out.last(), Some(Output::PanEnd { axis: Axis::Vertical, .. })), "{out:?}");
    }

    #[test]
    fn the_fling_speed_is_capped() {
        let mut r = Recognizer::default();
        let out =
            run(&mut r, &[down(0.0, 0.0, 0), mv(0.0, 500.0, 5), mv(0.0, 1000.0, 10), up(0.0, 1500.0, 15)]);
        match out.last() {
            Some(Output::Fling { velocity, .. }) => assert_eq!(velocity.y, 8000.0),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_fling_needs_speed_along_the_locked_axis() {
        // Gesto verticale veloce con una piccola deriva laterale veloce: conta l'asse.
        let mut r = Recognizer::default();
        let out = run(
            &mut r,
            &[down(0.0, 0.0, 0), mv(1.0, 20.0, 10), mv(2.0, 40.0, 20), mv(3.0, 60.0, 30), up(4.0, 80.0, 40)],
        );
        assert!(matches!(out.last(), Some(Output::Fling { axis: Axis::Vertical, .. })));
    }

    #[test]
    fn a_system_cancel_mid_pan_reports_cancel_and_is_idle() {
        let mut r = Recognizer::default();
        r.feed(down(0.0, 0.0, 0));
        r.feed(mv(0.0, 30.0, 20));
        assert_eq!(r.feed(Input::Cancel), Some(Output::Cancel));
        assert!(!r.is_active());
        // Una mossa e un rilascio a gesto annullato non producono niente.
        assert_eq!(r.feed(mv(0.0, 60.0, 40)), None);
        assert_eq!(r.feed(up(0.0, 60.0, 60)), None);
    }

    #[test]
    fn cancel_while_idle_is_silent() {
        let mut r = Recognizer::default();
        assert_eq!(r.feed(Input::Cancel), None);
    }

    #[test]
    fn cancel_during_a_long_press_hold_reports_cancel() {
        let mut r = Recognizer::default();
        r.feed(down(0.0, 0.0, 0));
        r.poll(ms(450));
        assert_eq!(r.feed(Input::Cancel), Some(Output::Cancel));
    }

    #[test]
    fn a_second_finger_cancels_and_blocks_until_all_fingers_are_up() {
        let mut r = Recognizer::default();
        r.feed(down(0.0, 0.0, 0));
        r.feed(mv(0.0, 30.0, 20));
        let second = r.feed(Input::Down { id: 9, at: Pt::new(50.0, 50.0), time: ms(30) });
        assert_eq!(second, Some(Output::Cancel));
        // Il primo dito continua e si stacca: niente.
        assert_eq!(r.feed(mv(0.0, 60.0, 40)), None);
        assert_eq!(r.feed(up(0.0, 60.0, 50)), None);
        // Il secondo si stacca: tutto libero.
        assert_eq!(r.feed(Input::Up { id: 9, at: Pt::new(50.0, 50.0), time: ms(60) }), None);
        let tap = run(&mut r, &[down(1.0, 1.0, 100), up(1.0, 1.0, 140)]);
        assert_eq!(tap, vec![Output::Tap { at: Pt::new(1.0, 1.0) }]);
    }

    #[test]
    fn a_second_finger_before_any_movement_also_cancels() {
        let mut r = Recognizer::default();
        r.feed(down(0.0, 0.0, 0));
        assert_eq!(r.feed(Input::Down { id: 2, at: Pt::new(9.0, 9.0), time: ms(5) }), Some(Output::Cancel));
        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn events_of_another_finger_are_ignored() {
        let mut r = Recognizer::default();
        r.feed(down(0.0, 0.0, 0));
        assert_eq!(r.feed(Input::Move { id: 99, at: Pt::new(100.0, 100.0), time: ms(10) }), None);
        assert_eq!(r.feed(Input::Up { id: 99, at: Pt::new(100.0, 100.0), time: ms(20) }), None);
        assert!(r.is_active());
    }

    #[test]
    fn the_recognizer_can_be_reused_for_the_next_gesture() {
        let mut r = Recognizer::default();
        run(&mut r, &[down(0.0, 0.0, 0), mv(0.0, 40.0, 10), up(0.0, 40.0, 400)]);
        let out = run(&mut r, &[down(5.0, 5.0, 1000), up(5.0, 5.0, 1050)]);
        assert_eq!(out, vec![Output::Tap { at: Pt::new(5.0, 5.0) }]);
    }

    #[test]
    fn scaled_thresholds_follow_the_density() {
        // A 2,5 px/dp la soglia di 8 dp è 20 px: 15 px di spostamento sono ancora un tocco.
        let mut r = Recognizer::new(Config::scaled(2.5));
        let out = run(&mut r, &[down(0.0, 0.0, 0), mv(15.0, 0.0, 20), up(15.0, 0.0, 60)]);
        assert_eq!(out, vec![Output::Tap { at: Pt::new(15.0, 0.0) }]);
        assert_eq!(Config::scaled(2.5).slop, 20.0);
        assert_eq!(Config::scaled(2.5).min_fling, 750.0);
        // Una scala senza senso ricade su 1.
        assert_eq!(Config::scaled(0.0).slop, 8.0);
        assert_eq!(Config::scaled(f32::NAN).slop, 8.0);
    }

    // I conflitti fra un gesto orizzontale dentro una lista che scorre in verticale: la riga col
    // trascinamento orizzontale (riga scorrevole) e la lista (scorrimento verticale) leggono lo
    // stesso gesto e devono decidere in modo opposto.

    #[test]
    fn nested_horizontal_swipe_in_a_vertical_list_never_fires_while_scrolling() {
        let mut r = Recognizer::default();
        r.feed(down(100.0, 300.0, 0));
        // Il dito scorre la lista: quasi verticale con un po' di deriva orizzontale.
        let mut axes = Vec::new();
        for (i, (x, y)) in
            [(104.0, 280.0), (110.0, 250.0), (118.0, 210.0), (130.0, 160.0)].into_iter().enumerate()
        {
            if let Some(Output::PanStart { axis, .. } | Output::PanMove { axis, .. }) =
                r.feed(mv(x, y, 16 * (i as u64 + 1)))
            {
                axes.push(axis);
            }
        }
        assert!(axes.iter().all(|a| *a == Axis::Vertical), "{axes:?}");
        assert_eq!(r.axis(), Some(Axis::Vertical));
    }

    #[test]
    fn nested_a_clear_horizontal_swipe_wins_even_with_some_vertical_drift() {
        let mut r = Recognizer::default();
        r.feed(down(100.0, 300.0, 0));
        let out = r.feed(mv(118.0, 306.0, 16));
        assert!(matches!(out, Some(Output::PanStart { axis: Axis::Horizontal, .. })));
        // Anche se poi scende di più, la riga resta agganciata (isteresi).
        let out = r.feed(mv(125.0, 380.0, 32));
        assert!(matches!(out, Some(Output::PanMove { axis: Axis::Horizontal, .. })));
    }

    #[test]
    fn nested_scroll_taking_over_cancels_a_pending_gesture() {
        // Il contenitore dice «scorro io» prima che il dito superi la soglia: il gesto muore e il
        // rilascio non è un tocco.
        let mut r = Recognizer::default();
        r.feed(down(0.0, 0.0, 0));
        assert_eq!(r.feed(Input::Cancel), Some(Output::Cancel));
        assert_eq!(r.feed(up(0.0, 0.0, 50)), None);
        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn a_vertical_pan_followed_by_a_horizontal_fling_stays_vertical() {
        let mut r = Recognizer::default();
        let out = run(
            &mut r,
            &[
                down(0.0, 0.0, 0),
                mv(0.0, 20.0, 10),
                mv(40.0, 22.0, 20),
                mv(80.0, 24.0, 30),
                up(120.0, 26.0, 40),
            ],
        );
        // L'asse resta verticale: la velocità orizzontale non conta per il lancio.
        assert!(
            matches!(
                out.last(),
                Some(
                    Output::PanEnd { axis: Axis::Vertical, .. } | Output::Fling { axis: Axis::Vertical, .. }
                )
            ),
            "{out:?}"
        );
    }

    #[test]
    fn a_one_step_release_beyond_the_slop_is_a_pan_end() {
        // Il sistema ha consegnato solo Down e Up lontani: non è un tocco.
        let mut r = Recognizer::default();
        let out = run(&mut r, &[down(0.0, 0.0, 0), up(60.0, 0.0, 30)]);
        assert!(matches!(out.last(), Some(Output::PanEnd { axis: Axis::Horizontal, .. })));
    }

    #[test]
    fn lock_axis_is_the_shared_rule() {
        assert_eq!(lock_axis(Pt::new(10.0, 0.0), 1.25), Axis::Horizontal);
        assert_eq!(lock_axis(Pt::new(0.0, 10.0), 1.25), Axis::Vertical);
        assert_eq!(lock_axis(Pt::new(5.0, 5.0), 1.25), Axis::Vertical);
        assert_eq!(lock_axis(Pt::new(-20.0, 3.0), 1.25), Axis::Horizontal);
    }
}
