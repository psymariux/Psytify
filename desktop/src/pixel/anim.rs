//! Sprite clock: only one scheduled redraw at a time, and only while something animates.
//!
//! No loop per frame: an animated sprite, when it is *painted*, asks for a redraw at the
//! deadline of its next frame (`request_refresh`). If no animated sprite is visible nothing is
//! painted, so nothing is scheduled and the CPU stays at zero (see `docs/PERFORMANCE.md`).
//!
//! The redraw invalidates the *view* that painted the sprite (`App::notify` of the view and, for GPUI, of its
//! ancestors), not the whole window: the `.cached()` views that animate nothing are reused
//! as they were instead of redoing layout, prepaint and paint at every frame of another view. With
//! `Window::refresh` every frame of the equaliser or the mark redid them all (BACKLOG 23).

use std::time::{Duration, Instant};

use gpui::{App, EntityId, Window};
use psytify_design::sprite::Sprite;

use super::store::SpriteStore;

/// Milliseconds left until the frame of `sprite` changes at the instant `elapsed_ms` (0 for the
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

/// Margin within which two near deadlines are served by the same wake-up.
const SLACK: Duration = Duration::from_millis(4);

/// A view that wants to be redrawn at a certain instant.
struct Wanted {
    view: EntityId,
    at: Instant,
}

/// The scheduled redraw: each view that painted an animated sprite says when it needs the next frame,
/// and there is only one effective timer at a time, aimed at the nearest deadline. At the deadline only the
/// views "due" (within `SLACK`) are invalidated; the others stay in the list for the next
/// timer. Without this, three animations with different phases (the queue equaliser at 120 ms,
/// the mark at 160 ms, the beat at 250 ms) would redraw every view at every frame of another.
///
/// Every new request that brings the deadline forward invalidates the previous timer (generation number),
/// so the old timers do not redraw and do not pile up: without this, two sprites with different phases
/// moltiplicavano i timer vivi e la finestra si ridisegnava a 60/s.
#[derive(Default)]
pub(super) struct RefreshClock {
    deadline: Option<Instant>,
    generation: u64,
    wanted: Vec<Wanted>,
}

impl RefreshClock {
    /// `view` painted an animated sprite that changes frame at `at`. Several requests from the same view
    /// (several sprites) keep the nearest.
    pub(super) fn want(&mut self, view: EntityId, at: Instant) {
        match self.wanted.iter_mut().find(|wanted| wanted.view == view) {
            Some(wanted) => wanted.at = wanted.at.min(at),
            None => self.wanted.push(Wanted { view, at }),
        }
    }

    /// If a new timer is needed, gives its generation and deadline; `None` if there is nothing to wait for
    /// or if one already scheduled expires earlier (or almost together): when it fires, it redraws the views due
    /// and re-arms the next one.
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

    /// A timer has expired: gives the views to redraw only if it is still the valid one. The views that
    /// wait for a later frame stay in the list.
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

/// Schedules the redraw of the view being painted (`Window::current_view`) within `delay_ms`, if there is not
/// already one that expires earlier. Call only from the prepaint or paint of an element.
pub fn request_refresh(window: &Window, cx: &mut App, delay_ms: u64) {
    let at = Instant::now() + Duration::from_millis(delay_ms.max(1));
    cx.global_mut::<SpriteStore>().refresh.want(window.current_view(), at);
    arm(cx);
}

/// Starts the timer of the nearest deadline, if needed. When it fires it invalidates the due views (a view
/// that no longer exists, for a changed page or a closed window, redraws nothing) and re-arms for the others.
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
        // At the frame's edge the next one fires.
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
        // The queue's equaliser (120 ms) and the mark (160 ms) have different phases.
        clock.want(queue, t0 + MS(120));
        clock.want(mark, t0 + MS(160));
        let (first, at) = clock.next_timer(t0).expect("timer");
        assert_eq!(at, t0 + MS(120));
        assert_eq!(clock.fire(first, t0 + MS(121)), Some(vec![queue]), "il marchio non è ancora a scadenza");
        // The mark waits: the next timer is its own.
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
        // The first timer (500 ms) and then a nearer one (120 ms): the old one no longer counts.
        clock.want(view, t0 + MS(500));
        let (late, _) = clock.next_timer(t0).expect("first timer");
        clock.want(other, t0 + MS(120));
        let (soon, _) = clock.next_timer(t0).expect("an earlier deadline replaces the pending one");
        assert_eq!(clock.fire(late, t0 + MS(500)), None, "il timer superato non ridisegna");
        assert_eq!(clock.fire(soon, t0 + MS(120)), Some(vec![other]));
        // A request that expires after the one already scheduled does not create another timer.
        let mut clock = RefreshClock::default();
        let t1 = t0 + MS(1000);
        clock.want(other, t1 + MS(120));
        let (first, _) = clock.next_timer(t1).expect("timer");
        clock.want(other, t1 + MS(300));
        assert!(clock.next_timer(t1).is_none());
        assert!(clock.fire(first, t1 + MS(120)).is_some());
        // After the deadline it can be scheduled again.
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
