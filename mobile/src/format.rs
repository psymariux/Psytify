//! Short texts the app shows (times, counts), the same rules as `desktop/src/format.rs` and the Iced
//! client, so the three frontends say the same thing.

/// `m:ss`, or `h:mm:ss` past the hour, of a time in milliseconds (whole seconds, rounded down).
pub fn clock(ms: u64) -> String {
    let secs = ms / 1000;
    let (hours, minutes, seconds) = (secs / 3600, secs / 60 % 60, secs % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

/// «1 brano» or «N brani».
pub fn tracks(count: usize) -> String {
    if count == 1 { "1 brano".to_owned() } else { format!("{count} brani") }
}

/// A total length in words, to the nearest minute: «48 min», «2 h», «1 h 5 min».
pub fn total(ms: u64) -> String {
    let minutes = ms.saturating_add(30_000) / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// «12.400 fan», «1,2 Mln di fan».
pub fn fans(fans: u64) -> String {
    match fans {
        0..1_000 => format!("{fans} fan"),
        1_000..1_000_000 => format!("{}.{:03} fan", fans / 1_000, fans % 1_000),
        _ => {
            let tenths = fans / 100_000;
            format!("{},{} Mln di fan", tenths / 10, tenths % 10)
        }
    }
}

/// The short name of an Internet Archive licence.
pub fn licence_label(kind: &str) -> &'static str {
    match kind {
        "cc0" => "CC0",
        "cc-by" => "CC BY",
        "cc-by-sa" => "CC BY-SA",
        "cc-by-nc" => "CC BY-NC",
        "cc-by-nc-sa" => "CC BY-NC-SA",
        "cc-by-nd" => "CC BY-ND",
        "cc-by-nc-nd" => "CC BY-NC-ND",
        "public-domain" => "Pubblico dominio",
        "live-trade" => "Concerto dal vivo",
        _ => "Licenza libera",
    }
}

/// «CC0 · 20, CC BY · 3»: the licences of some tracks, the most common first.
pub fn licence_summary<'a>(labels: impl IntoIterator<Item = &'a str>) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for label in labels {
        match counts.iter_mut().find(|(known, _)| *known == label) {
            Some((_, n)) => *n += 1,
            None => counts.push((label, 1)),
        }
    }
    counts.sort_by_key(|a| std::cmp::Reverse(a.1));
    counts.iter().map(|(label, n)| format!("{label} · {n}")).collect::<Vec<_>>().join(", ")
}

/// «12 brani · 48 min», or only the count when the length is not known.
pub fn length(count: usize, ms: u64) -> String {
    if ms > 0 { format!("{} · {}", tracks(count), total(ms)) } else { tracks(count) }
}

/// A file size in decimal units, one decimal with a comma: «12,3 MB», «980 B».
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "kB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]).replace('.', ",") }
}

/// The player's times: `m:ss` (`h:mm:ss` past the hour).
#[uniffi::export]
pub fn format_clock(ms: u64) -> String {
    clock(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_uses_minutes_and_seconds_and_hours_only_when_needed() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(59_999), "0:59");
        assert_eq!(clock(61_000), "1:01");
        assert_eq!(clock(3_599_000), "59:59");
        assert_eq!(clock(3_661_000), "1:01:01");
    }

    #[test]
    fn counts_and_totals_read_as_in_the_other_frontends() {
        assert_eq!(
            (tracks(1), tracks(0), tracks(12)),
            ("1 brano".into(), "0 brani".into(), "12 brani".into())
        );
        assert_eq!(total(29_999), "0 min");
        assert_eq!(total(48 * 60_000), "48 min");
        assert_eq!(total(120 * 60_000), "2 h");
        assert_eq!(total(65 * 60_000 + 10_000), "1 h 5 min");
        assert_eq!(total(u64::MAX), total(u64::MAX - 1));
        assert_eq!((length(3, 0), length(3, 600_000)), ("3 brani".into(), "3 brani · 10 min".into()));
    }

    #[test]
    fn fans_and_licences_read_as_on_the_desktop() {
        assert_eq!(
            (fans(999), fans(12_400), fans(1_250_000)),
            ("999 fan".into(), "12.400 fan".into(), "1,2 Mln di fan".into())
        );
        assert_eq!(licence_label("cc-by"), "CC BY");
        assert_eq!(licence_summary(["CC BY", "CC0", "CC0"]), "CC0 · 2, CC BY · 1");
    }

    #[test]
    fn sizes_read_as_on_the_desktop() {
        assert_eq!(size(980), "980 B");
        assert_eq!(size(12_300_000), "12,3 MB");
        assert_eq!(size(4_200_000_000), "4,2 GB");
    }
}
