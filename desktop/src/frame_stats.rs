//! Dove vanno i millisecondi di un fotogramma: il profilo di GPUI letto da noi, per misurare prima di
//! ottimizzare (`docs/PERFORMANCE.md`, BACKLOG 23). Esiste solo con la feature `profile` e si accende con
//! `PSYTIFY_FRAME_STATS=SECONDI` (senza il numero, 10): ogni intervallo scrive su stderr una riga con i
//! fotogrammi disegnati, il tempo medio di `Window::draw` diviso per fase (layout dei widget, prepaint,
//! paint), il tempo di consegna alla GPU, i task che hanno girato più di 100 µs (con il punto del codice
//! che li ha lanciati), le viste che da sole hanno pesato più di 500 µs e quanti sono i task piccoli.
//! Senza la variabile non si accende niente.
//!
//! La somma di queste voci è il lavoro del thread della finestra; il resto della CPU del processo sta nei
//! thread del core, dell'audio e di GPUI (`scripts/perf-sample.py` li separa).

use std::collections::HashMap;
use std::time::Duration;

use gpui::App;
use gpui::profiler::journal::{ForegroundEvent, ForegroundJournalEntry};

/// Intervallo di stampa predefinito, in secondi.
const DEFAULT_SECS: u64 = 10;
/// Quante voci dei task e delle viste si stampano per intervallo.
const TOP: usize = 6;

/// Somma e conteggio di una voce.
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
            // `ForegroundEvent` può crescere: ciò che non conosciamo non si conta.
            _ => {}
        }
    }

    /// Una riga di riepilogo e, sotto, le voci più pesanti.
    fn report(&self, secs: f64) -> String {
        let per_frame = |total: Duration| {
            if self.draw.count == 0 { 0.0 } else { ms(total) / self.draw.count as f64 }
        };
        // Il lavoro del thread della finestra in questo intervallo, per secondo di orologio.
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

/// Accende le misure se c'è `PSYTIFY_FRAME_STATS`: il profilo di GPUI registra e un task lo legge a
/// intervalli. Va chiamato una volta all'avvio.
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
