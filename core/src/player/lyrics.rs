//! Testi sincronizzati: lettura del formato LRC e riga in corso per la posizione di riproduzione.
//! Tutto è una funzione pura: il testo arriva già dal backend.

use std::time::Duration;

use crate::api::Lyrics;

/// Una riga del testo, dal momento in cui va cantata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub at: Duration,
    pub text: String,
}

/// Testo pronto per la vista: le righe con i tempi, se il testo è sincronizzato, e il testo semplice.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Text {
    pub lines: Vec<Line>,
    pub plain: Option<String>,
}

impl Text {
    pub fn of(lyrics: &Lyrics) -> Self {
        Self {
            lines: lyrics.synced.as_deref().map(parse_lrc).unwrap_or_default(),
            plain: lyrics.plain.clone(),
        }
    }

    pub fn is_synced(&self) -> bool {
        !self.lines.is_empty()
    }
}

/// Legge un testo in formato LRC. Una riga può avere più tempi (`[00:10.00][01:40.00] ritornello`);
/// i tag di metadati (`[ar:...]`, `[ti:...]`) si ignorano, `[offset:±ms]` sposta tutte le righe.
/// Il risultato è ordinato per tempo.
pub fn parse_lrc(text: &str) -> Vec<Line> {
    let mut offset_ms: i64 = 0;
    let mut stamped: Vec<(u64, String)> = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some((tag, after)) = rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
            if let Some(ms) = parse_stamp(tag) {
                stamps.push(ms);
            } else if let Some(value) = tag.strip_prefix("offset:") {
                offset_ms = value.trim().parse().unwrap_or(0);
            }
            rest = after;
        }
        stamped.extend(stamps.into_iter().map(|ms| (ms, rest.trim().to_owned())));
    }
    let mut lines: Vec<Line> = stamped
        .into_iter()
        .map(|(ms, text)| {
            let shifted = (i64::try_from(ms).unwrap_or(i64::MAX) - offset_ms).max(0);
            Line { at: Duration::from_millis(u64::try_from(shifted).unwrap_or(0)), text }
        })
        .collect();
    lines.sort_by_key(|line| line.at);
    lines
}

/// Un tempo `mm:ss`, `mm:ss.xx` o `mm:ss.xxx` in millisecondi. `None` per ogni altro tag.
fn parse_stamp(tag: &str) -> Option<u64> {
    let (minutes, seconds) = tag.split_once(':')?;
    let minutes: u64 = digits(minutes)?;
    let (whole, fraction) = match seconds.split_once(['.', ':']) {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (seconds, None),
    };
    let seconds: u64 = digits(whole)?;
    let millis = match fraction {
        // I centesimi valgono dieci millisecondi: `.5` è mezzo secondo, `.45` è 450 ms.
        Some(fraction) if (1..=3).contains(&fraction.len()) => digits(&format!("{fraction:0<3}"))?,
        Some(_) => return None,
        None => 0,
    };
    Some(minutes * 60_000 + seconds * 1000 + millis)
}

/// Solo cifre ASCII, altrimenti il tag non è un tempo (`ar`, `ti`, `+1` ecc.).
fn digits(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Indice della riga in corso: l'ultima che inizia prima o nel momento `position`. `None` prima
/// della prima riga.
pub fn current(lines: &[Line], position: Duration) -> Option<usize> {
    lines.partition_point(|line| line.at <= position).checked_sub(1)
}

/// Puntini di una pausa strumentale.
pub const DOTS: u32 = 3;

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

/// Fra quanto cambia qualcosa da mostrare alla posizione `position`: l'inizio del verso dopo o, dentro
/// una pausa, il prossimo puntino. `None` dopo l'ultimo verso: non c'è niente da aspettare.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn verse(at_ms: u64, text: &str) -> Line {
        Line { at: Duration::from_millis(at_ms), text: text.into() }
    }

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
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
        assert!(is_pause(&lines[1]) && !is_pause(&lines[0]));
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
    fn reads_timed_lines() {
        let lines = parse_lrc("[00:01.00] Hello\n[00:05.20] World");
        assert_eq!(
            lines,
            vec![
                Line { at: secs(1), text: "Hello".into() },
                Line { at: Duration::from_millis(5200), text: "World".into() },
            ]
        );
    }

    #[test]
    fn one_line_can_have_several_times() {
        let lines = parse_lrc("[00:10.00][01:40.50] ritornello");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].at, secs(10));
        assert_eq!(lines[1].at, Duration::from_millis(100_500));
        assert!(lines.iter().all(|l| l.text == "ritornello"));
    }

    #[test]
    fn metadata_tags_are_not_lines() {
        let lines = parse_lrc("[ar:Marea]\n[ti:Banchi]\n[00:02.00] onde");
        assert_eq!(lines, vec![Line { at: secs(2), text: "onde".into() }]);
    }

    #[test]
    fn offset_shifts_every_line_and_never_goes_negative() {
        let lines = parse_lrc("[offset:+500]\n[00:01.00] a\n[00:00.20] b");
        assert_eq!(lines[0].at, Duration::from_millis(0));
        assert_eq!(lines[1].at, Duration::from_millis(500));
        let later = parse_lrc("[00:02.00] a\n[offset:-1000]");
        assert_eq!(later[0].at, Duration::from_millis(3000));
    }

    #[test]
    fn fractions_are_centiseconds_or_milliseconds() {
        assert_eq!(parse_stamp("00:01.5"), Some(1500));
        assert_eq!(parse_stamp("00:01.45"), Some(1450));
        assert_eq!(parse_stamp("00:01.456"), Some(1456));
        assert_eq!(parse_stamp("00:01.4567"), None);
        assert_eq!(parse_stamp("ar:Marea"), None);
    }

    #[test]
    fn lines_are_sorted_by_time() {
        let lines = parse_lrc("[00:09.00] nove\n[00:03.00] tre");
        assert_eq!(lines[0].text, "tre");
        assert_eq!(lines[1].text, "nove");
    }

    #[test]
    fn current_line_follows_the_position() {
        let lines = parse_lrc("[00:01.00] a\n[00:05.00] b");
        assert_eq!(current(&lines, Duration::ZERO), None);
        assert_eq!(current(&lines, secs(1)), Some(0));
        assert_eq!(current(&lines, secs(4)), Some(0));
        assert_eq!(current(&lines, secs(5)), Some(1));
        assert_eq!(current(&lines, secs(999)), Some(1));
    }

    #[test]
    fn text_without_timings_is_plain_only() {
        let lyrics = Lyrics {
            track_id: uuid::Uuid::nil(),
            found: true,
            synced: None,
            plain: Some("solo testo".into()),
        };
        let text = Text::of(&lyrics);
        assert!(!text.is_synced());
        assert_eq!(text.plain.as_deref(), Some("solo testo"));
    }
}
