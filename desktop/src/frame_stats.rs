//! Where the milliseconds of a frame go: GPUI's profile read by us, to measure before
//! optimising (`docs/PERFORMANCE.md`, BACKLOG 23). It exists only with the `profile` feature and is switched on with
//! `PSYTIFY_FRAME_STATS=SECONDI` (without the number, 10): each interval writes on stderr a line with the
//! frames drawn, the average time of `Window::draw` split by phase (widget layout, prepaint,
//! paint), the delivery time to the GPU, the tasks that ran for more than 100 µs (with the point of the code
//! that launched them), the views that alone weighed more than 500 µs and how many small tasks there are.
//! Without the variable nothing is switched on.
//!
//! The sum of these entries is the window thread's work; the rest of the process's CPU is in
//! thread del core, dell'audio e di GPUI (`scripts/perf-sample.py` li separa).

use std::collections::HashMap;
use std::time::Duration;

use gpui::App;
use gpui::profiler::journal::{ForegroundEvent, ForegroundJournalEntry};

/// Intervallo di stampa predefinito, in secondi.
const DEFAULT_SECS: u64 = 10;
/// How many task and view entries are printed per interval.
const TOP: usize = 6;

/// Sum and count of an entry.
#[derive(Clone, Copy, Default)]
struct Sum {
    count: u64,
    total: Duration,
    max: Duration,
}

impl Sum {
    fn add(&mut self, duration: Duration) {
        self.count += 1;
        self.total += duration;
        self.max = self.max.max(duration);
    }

    fn mean_ms(self) -> f64 {
        if self.count == 0 { 0.0 } else { ms(self.total) / self.count as f64 }
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[derive(Default)]
struct Stats {
    draw: Sum,
    layout: Duration,
    prepaint: Duration,
    paint: Duration,
    present: Sum,
    inputs: Sum,
    small_polls: Sum,
    tasks: HashMap<String, Sum>,
    views: HashMap<String, Sum>,
    lost: u64,
}

impl Stats {
    fn take(&mut self, entries: Vec<ForegroundJournalEntry>) {
        for entry in entries {
            match entry {
                ForegroundJournalEntry::Event(event) => self.event(event),
                ForegroundJournalEntry::Discontinuity { lost } => self.lost += lost,
                _ => {}
            }
        }
    }

    fn event(&mut self, event: ForegroundEvent) {
        match event {
            ForegroundEvent::Draw(frame) => {
                self.draw.add(event.duration());
                self.layout += frame.phases.request_layout;
                self.prepaint += frame.phases.prepaint;
                self.paint += frame.phases.paint;
            }
            ForegroundEvent::Present(present) => self.present.add(present.present_duration()),
            ForegroundEvent::Input(_) => self.inputs.add(event.duration()),
            ForegroundEvent::TaskPoll(poll) => {
                let place = format!("{}:{}", poll.location.file(), poll.location.line());
                self.tasks.entry(place).or_default().add(poll.poll_duration());
            }
            ForegroundEvent::View(view) => {
                let name = format!("{} {}", view.name, view.phase.as_str());
                self.views.entry(name).or_default().add(view.self_duration);
            }
            ForegroundEvent::SmallPolls(flush) => {
                self.small_polls.count += flush.summary.count;
                self.small_polls.total += flush.summary.total;
            }
            // `ForegroundEvent` may grow: what we do not know is not counted.
            _ => {}
        }
    }

    /// A summary line and, under it, the heaviest entries.
    fn report(&self, secs: f64) -> String {
        let per_frame = |total: Duration| {
            if self.draw.count == 0 { 0.0 } else { ms(total) / self.draw.count as f64 }
        };
        // The window thread's work in this interval, per second of clock.
        let busy = self.draw.total
            + self.present.total
            + self.inputs.total
            + self.small_polls.total
            + self.tasks.values().map(|sum| sum.total).sum::<Duration>();
        let mut text = format!(
            "[frame-stats] {secs:.0}s  draw {n} ({rate:.1}/s) mean {mean:.2} ms max {max:.2} ms \
             [layout {layout:.2} prepaint {prepaint:.2} paint {paint:.2}]  present {pn} mean {pmean:.2} ms  \
             input {inputs}  small tasks {sn} ({st:.1} ms)  busy {busy:.1} ms/s",
            n = self.draw.count,
            rate = self.draw.count as f64 / secs,
            mean = self.draw.mean_ms(),
            max = ms(self.draw.max),
            layout = per_frame(self.layout),
            prepaint = per_frame(self.prepaint),
            paint = per_frame(self.paint),
            pn = self.present.count,
            pmean = self.present.mean_ms(),
            inputs = self.inputs.count,
            sn = self.small_polls.count,
            st = ms(self.small_polls.total),
            busy = ms(busy) / secs,
        );
        if self.lost > 0 {
            text.push_str(&format!("  (voci perse dal registro: {})", self.lost));
        }
        for (title, map) in [("task > 100 us", &self.tasks), ("viste > 500 us", &self.views)] {
            let mut rows: Vec<_> = map.iter().collect();
            rows.sort_by_key(|(_, sum)| std::cmp::Reverse(sum.total));
            for (name, sum) in rows.into_iter().take(TOP) {
                text.push_str(&format!(
                    "\n    {title}: {name}  x{} totale {:.1} ms medio {:.2} ms max {:.2} ms",
                    sum.count,
                    ms(sum.total),
                    sum.mean_ms(),
                    ms(sum.max)
                ));
            }
        }
        text
    }
}

/// Switches the measures on if `PSYTIFY_FRAME_STATS` is set: GPUI's profile records and a task reads it at
/// intervals. It must be called once at start-up.
pub fn install(cx: &mut App) {
    let Some(value) = std::env::var_os("PSYTIFY_FRAME_STATS") else { return };
    let secs = value.to_str().and_then(|text| text.parse::<u64>().ok()).unwrap_or(DEFAULT_SECS).max(1);
    gpui::profiler::set_trace_enabled(true);
    let mut collector = cx.foreground_journal().collector();
    cx.spawn(async move |cx| {
        let mut stats = Stats::default();
        loop {
            cx.background_executor().timer(Duration::from_secs(secs)).await;
            stats.take(collector.collect_unseen().entries);
            eprintln!("{}", stats.report(secs as f64));
            stats = Stats::default();
        }
    })
    .detach();
}
