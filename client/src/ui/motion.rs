//! Animazioni. Iced 0.13 non ne ha: ogni movimento è un [`Tween`] che ricorda l'istante di
//! partenza, e la vista ne legge il valore al momento del disegno. `App::subscription` chiede i
//! frame della finestra solo mentre qualcosa si muove, così da fermo l'app non ridisegna.
//!
//! Le durate sono token di `theme.rs`. Due andamenti: l'ease-out, che rallenta verso la fine (pagine,
//! menu, avvisi), e la molla ([`spring`]), che supera di poco l'arrivo e torna indietro (copertina che
//! respira con il Play, scheda del pannello, marchio dello splash). La molla è riservata a ciò che
//! si tocca o che rappresenta il brano: le liste e le pagine restano calme.

use std::f32::consts::TAU;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iced::Color;

use super::theme;

/// Un valore che va da `from` a `to` in `duration`, a partire da `start`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    duration: Duration,
    curve: Curve,
}

/// L'andamento di un [`Tween`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    EaseOut,
    Spring,
}

impl Tween {
    /// Fermo su `value`.
    pub fn rest(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            start: Instant::now(),
            duration: Duration::ZERO,
            curve: Curve::EaseOut,
        }
    }

    /// Da `from` a `to`, da adesso.
    pub fn new(from: f32, to: f32, duration: Duration) -> Self {
        Self { from, to, start: Instant::now(), duration, curve: Curve::EaseOut }
    }

    /// Lo stesso movimento con un altro andamento.
    pub fn with_curve(self, curve: Curve) -> Self {
        Self { curve, ..self }
    }

    /// Verso `to` dal punto in cui si trova adesso: un pannello che si chiude mentre si sta aprendo
    /// torna indietro da lì, senza scatti. Se va già verso `to`, non cambia niente.
    pub fn retarget(&mut self, to: f32, duration: Duration) {
        if to == self.to {
            return;
        }
        let now = Instant::now();
        *self = Self { from: self.value_at(now), to, start: now, duration, curve: self.curve };
    }

    pub fn value(&self) -> f32 {
        self.value_at(Instant::now())
    }

    /// Da quanto è partito: serve a far entrare gli elementi di una lista uno dopo l'altro.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    #[cfg(test)]
    pub(crate) fn target(&self) -> f32 {
        self.to
    }

    pub fn is_running(&self) -> bool {
        self.is_running_at(Instant::now())
    }

    fn progress_at(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        (now.saturating_duration_since(self.start).as_secs_f32() / self.duration.as_secs_f32())
            .clamp(0.0, 1.0)
    }

    fn value_at(&self, now: Instant) -> f32 {
        let t = self.progress_at(now);
        let eased = match self.curve {
            Curve::EaseOut => ease_out(t),
            Curve::Spring => spring(t),
        };
        self.from + (self.to - self.from) * eased
    }

    fn is_running_at(&self, now: Instant) -> bool {
        self.progress_at(now) < 1.0
    }
}

/// Ease-out cubico: parte veloce e si posa piano.
pub fn ease_out(t: f32) -> f32 {
    let rest = 1.0 - t.clamp(0.0, 1.0);
    1.0 - rest * rest * rest
}

/// Una molla smorzata: arriva a 1 verso il 40% del tempo, lo supera di circa il 6% e si posa con
/// un'oscillazione che si spegne. Vale esattamente 0 all'inizio e 1 alla fine.
pub fn spring(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t >= 1.0 {
        return 1.0;
    }
    // Smorzamento e frequenza scelti a occhio: un solo rimbalzo visibile, mai più di uno.
    let decay = (-6.5 * t).exp();
    1.0 - decay * (TAU * 1.15 * t).cos() * (1.0 - t)
}

/// Quanto è avanzata, da 0 a 1 con ease-out, un'apparizione cominciata a `since`.
pub fn appear(since: Instant, duration: Duration) -> f32 {
    Tween { from: 0.0, to: 1.0, start: since, duration, curve: Curve::EaseOut }.value()
}

/// Come [`appear`], con la molla: per ciò che entra «saltando» (barre del marchio).
pub fn spring_in(since: Instant, delay: Duration, duration: Duration) -> f32 {
    let elapsed = since.elapsed().saturating_sub(delay).as_secs_f32();
    spring(elapsed / duration.as_secs_f32().max(f32::EPSILON))
}

/// Quanto è avanzato l'ingresso dell'elemento `index` di una lista (da 0 a 1 con ease-out) a `elapsed`
/// dalla comparsa della lista: ognuno parte `STAGGER` dopo il precedente, i primi quattordici.
pub fn stagger(elapsed: Duration, index: usize) -> f32 {
    let delay = theme::STAGGER * u32::try_from(index.min(14)).unwrap_or(14);
    let progress = elapsed.saturating_sub(delay).as_secs_f32() / theme::MOTION_BASE.as_secs_f32();
    ease_out(progress)
}

/// Un piccolo rimbalzo che parte da 0, sale a 1 e torna a 0 in `duration` (il cuore appena premuto).
pub fn pop(since: Instant, duration: Duration) -> f32 {
    let t = (since.elapsed().as_secs_f32() / duration.as_secs_f32().max(f32::EPSILON)).clamp(0.0, 1.0);
    (std::f32::consts::PI * t).sin() * (1.0 - 0.35 * t)
}

/// Un'onda lenta tra 0 e 1, sfasata per `lane`: le barre del visualizzatore non salgono insieme.
pub fn wave(lane: usize, period: Duration) -> f32 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let period_ms = period.as_millis().max(1);
    // Ogni barra ha il suo ritmo: periodo leggermente diverso e fase propria.
    let lane_f = lane as f32;
    let phase = (now % period_ms) as f32 / period_ms as f32 * (1.0 + 0.37 * lane_f) + lane_f * 0.31;
    0.5 - 0.5 * (TAU * phase).cos()
}

/// Un'oscillazione lenta tra -1 e 1 con il periodo dato: fa muovere le sfumature.
pub fn drift(period: Duration) -> f32 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let period_ms = period.as_millis().max(1);
    (TAU * (now % period_ms) as f32 / period_ms as f32).sin()
}

/// Pulsazione degli scheletri: da 0 a 1 e ritorno ogni `period`. Dipende solo dall'orologio,
/// così tutti gli scheletri dello schermo pulsano insieme.
pub fn pulse(period: Duration) -> f32 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let period = period.as_millis().max(1);
    let phase = (now % period) as f32 / period as f32;
    0.5 - 0.5 * (TAU * phase).cos()
}

/// Colore a metà strada tra `a` e `b` (`t` da 0 a 1).
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// Lo stesso colore con l'opacità moltiplicata per `alpha`.
pub fn fade(color: Color, alpha: f32) -> Color {
    Color { a: color.a * alpha.clamp(0.0, 1.0), ..color }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(tween: &Tween, millis: u64) -> f32 {
        tween.value_at(tween.start + Duration::from_millis(millis))
    }

    #[test]
    fn a_tween_goes_from_start_to_end() {
        let tween = Tween::new(0.0, 10.0, Duration::from_millis(100));
        assert_eq!((at(&tween, 0), at(&tween, 100), at(&tween, 500)), (0.0, 10.0, 10.0));
    }

    #[test]
    fn ease_out_is_ahead_of_linear_halfway() {
        let tween = Tween::new(0.0, 1.0, Duration::from_millis(100));
        assert!(at(&tween, 50) > 0.8);
    }

    #[test]
    fn a_tween_at_rest_is_not_running() {
        assert!(!Tween::rest(1.0).is_running());
    }

    #[test]
    fn retargeting_starts_from_the_current_value() {
        let mut tween = Tween::new(0.0, 1.0, Duration::from_secs(3600));
        tween.retarget(0.0, Duration::from_secs(3600));
        assert!(tween.value() < 0.01 && tween.target() == 0.0);
    }

    #[test]
    fn retargeting_to_the_same_end_keeps_the_motion() {
        let mut tween = Tween::new(0.0, 1.0, Duration::from_secs(3600));
        let before = tween;
        tween.retarget(1.0, Duration::from_millis(1));
        assert_eq!(tween, before);
    }

    #[test]
    fn pulse_stays_between_zero_and_one() {
        let value = pulse(Duration::from_millis(1400));
        assert!((0.0..=1.0).contains(&value));
    }

    #[test]
    fn list_items_enter_one_after_the_other() {
        let at = Duration::from_millis(100);
        assert!(stagger(at, 0) > stagger(at, 3));
        assert_eq!(stagger(Duration::ZERO, 2), 0.0);
        assert_eq!(stagger(Duration::from_secs(5), 99), 1.0);
    }

    #[test]
    fn the_pop_starts_and_ends_at_rest() {
        let now = Instant::now();
        assert!(pop(now, Duration::from_secs(60)).abs() < 0.01);
        let long_ago = now.checked_sub(Duration::from_secs(5)).unwrap_or(now);
        assert!(pop(long_ago, Duration::from_millis(200)).abs() < 0.01);
    }

    #[test]
    fn the_wave_stays_between_zero_and_one() {
        for lane in 0..4 {
            assert!((0.0..=1.0).contains(&wave(lane, Duration::from_millis(900))));
        }
    }

    #[test]
    fn the_spring_starts_at_zero_overshoots_and_settles_on_one() {
        assert_eq!((spring(0.0), spring(1.0)), (0.0, 1.0));
        let peak = (1..100).map(|i| spring(i as f32 / 100.0)).fold(0.0_f32, f32::max);
        assert!(peak > 1.02 && peak < 1.15, "picco {peak}");
    }

    #[test]
    fn a_spring_tween_keeps_its_curve_when_retargeted() {
        let mut tween = Tween::new(0.0, 1.0, Duration::from_secs(3600)).with_curve(Curve::Spring);
        tween.retarget(0.5, Duration::from_secs(1));
        assert_eq!(tween.curve, Curve::Spring);
    }

    #[test]
    fn mix_lands_on_both_ends() {
        let (a, b) = (Color::BLACK, Color::WHITE);
        assert_eq!((mix(a, b, 0.0), mix(a, b, 1.0)), (a, b));
    }
}
