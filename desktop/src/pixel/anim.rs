//! Orologio degli sprite: un solo ridisegno programmato alla volta, solo mentre qualcosa si anima.
//!
//! Nessun ciclo per frame: uno sprite animato, quando viene *dipinto*, chiede un ridisegno alla
//! scadenza del suo prossimo frame (`request_refresh`). Se nessuno sprite animato è visibile non si
//! dipinge nulla, quindi non si programma nulla e la CPU resta a zero (vedi `docs/PERFORMANCE.md`).
//!
//! Il ridisegno invalida la *vista* che ha dipinto lo sprite (`App::notify` della vista e, per GPUI, dei
//! suoi antenati), non l'intera finestra: le viste con `.cached()` che non animano nulla si riusano
//! così com'erano invece di rifare layout, prepaint e paint a ogni scatto di un'altra vista. Con
//! `Window::refresh` ogni scatto dell'equalizzatore o del marchio le rifaceva tutte (BACKLOG 23).

use std::time::{Duration, Instant};

use gpui::{App, EntityId, Window};
use psytify_design::sprite::Sprite;

use super::store::SpriteStore;

/// Millisecondi che mancano al cambio di frame di `sprite` all'istante `elapsed_ms` (0 per gli
/// sprite statici). Segue la stessa logica di `Sprite::frame_index_at`.
pub fn ms_until_next_frame(sprite: &Sprite, elapsed_ms: u64) -> u64 {
    let cycle = u64::from(sprite.cycle_ms());
    if sprite.frame_count() < 2 || cycle == 0 {
        return 0;
    }
    let mut t = elapsed_ms % cycle;
    for frame in sprite.frames() {
        let d = u64::from(frame.duration_ms);
        if t < d {
            return d - t;
        }
        t -= d;
    }
    cycle
}

/// Margine entro cui due scadenze vicine si servono con lo stesso risveglio.
const SLACK: Duration = Duration::from_millis(4);

/// Una vista che vuole essere ridisegnata a un certo istante.
struct Wanted {
    view: EntityId,
    at: Instant,
}

/// Il ridisegno programmato: ogni vista che ha dipinto uno sprite animato dice quando le serve il frame
/// dopo, e c'è un solo timer efficace per volta, puntato alla scadenza più vicina. Alla scadenza si
/// invalidano solo le viste «a scadenza» (entro `SLACK`): le altre restano nell'elenco per il timer
/// successivo. Senza questo, tre animazioni con fasi diverse (l'equalizzatore della coda da 120 ms,
/// il marchio da 160 ms, il battito da 250 ms) ridisegnavano ogni vista a ogni scatto di un'altra.
///
/// Ogni richiesta nuova che anticipa la scadenza invalida il timer precedente (numero di generazione),
/// così i timer vecchi non ridisegnano e non se ne accumulano: senza questo, due sprite con fasi diverse
/// moltiplicavano i timer vivi e la finestra si ridisegnava a 60/s.
#[derive(Default)]
pub(super) struct RefreshClock {
    deadline: Option<Instant>,
    generation: u64,
    wanted: Vec<Wanted>,
}

impl RefreshClock {
    /// `view` ha dipinto uno sprite animato che cambia frame a `at`. Più richieste della stessa vista
    /// (più sprite) tengono la più vicina.
    pub(super) fn want(&mut self, view: EntityId, at: Instant) {
        match self.wanted.iter_mut().find(|wanted| wanted.view == view) {
            Some(wanted) => wanted.at = wanted.at.min(at),
            None => self.wanted.push(Wanted { view, at }),
        }
    }

    /// Se serve un nuovo timer ne dà la generazione e la scadenza; `None` se non c'è niente da attendere
    /// o se uno già programmato scade prima (o quasi insieme): scattando, ridisegnerà le viste a scadenza
    /// e riarmerà il successivo.
    pub(super) fn next_timer(&mut self, now: Instant) -> Option<(u64, Instant)> {
        let earliest = self.wanted.iter().map(|wanted| wanted.at).min()?;
        if let Some(pending) = self.deadline
            && pending > now
            && pending <= earliest + SLACK
        {
            return None;
        }
        self.generation += 1;
        self.deadline = Some(earliest);
        Some((self.generation, earliest))
    }

    /// Un timer è scaduto: dà le viste da ridisegnare solo se è ancora quello valido. Le viste che
    /// aspettano un frame più in là restano nell'elenco.
    pub(super) fn fire(&mut self, generation: u64, now: Instant) -> Option<Vec<EntityId>> {
        if generation != self.generation {
            return None;
        }
        self.deadline = None;
        let (due, later): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.wanted).into_iter().partition(|wanted| wanted.at <= now + SLACK);
        self.wanted = later;
        Some(due.into_iter().map(|wanted| wanted.view).collect())
    }
}

/// Programma il ridisegno della vista che sta dipingendo (`Window::current_view`) fra `delay_ms`, se non
/// ce n'è già uno che scade prima. Da chiamare solo dal prepaint o dal paint di un elemento.
pub fn request_refresh(window: &Window, cx: &mut App, delay_ms: u64) {
    let at = Instant::now() + Duration::from_millis(delay_ms.max(1));
    cx.global_mut::<SpriteStore>().refresh.want(window.current_view(), at);
    arm(cx);
}

/// Avvia il timer della scadenza più vicina, se serve. Scattando invalida le viste a scadenza (una vista
/// che non c'è più, per pagina cambiata o finestra chiusa, non ridisegna niente) e riarma per le altre.
fn arm(cx: &mut App) {
    let now = Instant::now();
    let Some((generation, at)) = cx.global_mut::<SpriteStore>().refresh.next_timer(now) else {
        return;
    };
    let delay = at.saturating_duration_since(now);
    cx.spawn(async move |cx| {
        cx.background_executor().timer(delay).await;
        cx.update(|cx| {
            let Some(due) = cx.global_mut::<SpriteStore>().refresh.fire(generation, Instant::now()) else {
                return;
            };
            for view in due {
                cx.notify(view);
            }
            arm(cx);
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use psytify_design::sprite;

    #[test]
    fn next_frame_delay_counts_down_inside_a_frame() {
        let Ok(sheet) = sprite::sheet() else { return };
        let Some(eq) = sheet.get(sprite::EQ) else { return };
        let first = u64::from(eq.frames()[0].duration_ms);
        assert_eq!(ms_until_next_frame(eq, 0), first);
        assert_eq!(ms_until_next_frame(eq, 30), first - 30);
        // Al confine del frame scatta il successivo.
        assert_eq!(ms_until_next_frame(eq, first), u64::from(eq.frames()[1].duration_ms));
    }

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn a_view_asking_twice_keeps_the_nearest_deadline() {
        let mut clock = RefreshClock::default();
        let (queue, t0) = (EntityId::from(7), Instant::now());
        clock.want(queue, t0 + MS(240));
        clock.want(queue, t0 + MS(120));
        clock.want(queue, t0 + MS(300));
        let (_, at) = clock.next_timer(t0).expect("timer");
        assert_eq!(at, t0 + MS(120));
        assert_eq!(clock.fire(1, t0 + MS(120)), Some(vec![queue]), "una voce sola per vista");
    }

    #[test]
    fn only_the_views_whose_frame_is_due_are_redrawn_and_the_rest_wait() {
        let mut clock = RefreshClock::default();
        let (queue, mark, t0) = (EntityId::from(7), EntityId::from(9), Instant::now());
        // L'equalizzatore della coda (120 ms) e il marchio (160 ms) hanno fasi diverse.
        clock.want(queue, t0 + MS(120));
        clock.want(mark, t0 + MS(160));
        let (first, at) = clock.next_timer(t0).expect("timer");
        assert_eq!(at, t0 + MS(120));
        assert_eq!(clock.fire(first, t0 + MS(121)), Some(vec![queue]), "il marchio non è ancora a scadenza");
        // Il marchio resta in attesa: il prossimo timer è il suo.
        let (second, at) = clock.next_timer(t0 + MS(121)).expect("timer for the waiting view");
        assert_eq!(at, t0 + MS(160));
        assert_eq!(clock.fire(second, t0 + MS(160)), Some(vec![mark]));
        assert_eq!(clock.next_timer(t0 + MS(160)), None, "nessuno chiede più niente: nessun timer");
    }

    #[test]
    fn deadlines_within_a_few_milliseconds_share_one_wakeup() {
        let mut clock = RefreshClock::default();
        let (queue, page, t0) = (EntityId::from(7), EntityId::from(9), Instant::now());
        clock.want(queue, t0 + MS(120));
        let (timer, _) = clock.next_timer(t0).expect("timer");
        clock.want(page, t0 + MS(122));
        assert!(clock.next_timer(t0).is_none(), "entro 4 ms: una sola scadenza");
        let mut due = clock.fire(timer, t0 + MS(120)).expect("valid timer");
        due.sort_by_key(|view| view.as_u64());
        assert_eq!(due, vec![queue, page]);
    }

    #[test]
    fn only_the_latest_timer_may_redraw_and_stale_ones_are_ignored() {
        let mut clock = RefreshClock::default();
        let (view, other, t0) = (EntityId::from(7), EntityId::from(9), Instant::now());
        // Il primo timer (500 ms) e poi uno più vicino (120 ms): il vecchio non vale più.
        clock.want(view, t0 + MS(500));
        let (late, _) = clock.next_timer(t0).expect("first timer");
        clock.want(other, t0 + MS(120));
        let (soon, _) = clock.next_timer(t0).expect("an earlier deadline replaces the pending one");
        assert_eq!(clock.fire(late, t0 + MS(500)), None, "il timer superato non ridisegna");
        assert_eq!(clock.fire(soon, t0 + MS(120)), Some(vec![other]));
        // Una richiesta che scade dopo quella già programmata non crea un altro timer.
        let mut clock = RefreshClock::default();
        let t1 = t0 + MS(1000);
        clock.want(other, t1 + MS(120));
        let (first, _) = clock.next_timer(t1).expect("timer");
        clock.want(other, t1 + MS(300));
        assert!(clock.next_timer(t1).is_none());
        assert!(clock.fire(first, t1 + MS(120)).is_some());
        // Dopo la scadenza si può programmare di nuovo.
        clock.want(other, t1 + MS(240));
        assert!(clock.next_timer(t1 + MS(120)).is_some());
    }

    #[test]
    fn static_sprites_never_schedule_a_refresh() {
        let Ok(sheet) = sprite::sheet() else { return };
        let Some(play) = sheet.get(sprite::ICON_PLAY) else { return };
        assert_eq!(ms_until_next_frame(play, 12_345), 0);
    }
}
