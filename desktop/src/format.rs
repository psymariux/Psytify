//! Short texts for the interface (durations, counts), computed when the data changes and not in the view.

use std::time::Duration;

/// `m:ss` (or `h:mm:ss` beyond the hour) of a duration in whole seconds.
pub fn clock(duration: Duration) -> String {
    let secs = duration.as_secs();
    let (hours, minutes, seconds) = (secs / 3600, secs / 60 % 60, secs % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{}:{seconds:02}", secs / 60)
    }
}

/// Duration of a track or a queue from milliseconds.
pub fn clock_ms(ms: u64) -> String {
    clock(Duration::from_millis(ms))
}

/// «1 brano» o «N brani».
pub fn tracks(count: usize) -> String {
    if count == 1 { "1 brano".to_owned() } else { format!("{count} brani") }
}

/// The duration of a collection in words: «48 min», «2 h», «1 h 5 min» (to the nearest minute), as in the
/// phone layout.
pub fn total(ms: u64) -> String {
    let minutes = (ms + 30_000) / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// A weight in words: «126,4 MB» (multiples of a thousand, one decimal digit).
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

/// What a licence is called in Internet Archive's results (`DiscoverItem::licence`).
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

/// The fans of an artist of the official catalogue, (`side_panel::fans_label`).
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

/// «CC0 · 20, CC BY · 3»: the tracks' licences, from the most common.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_read_minutes_and_hours() {
        assert_eq!(clock(Duration::ZERO), "0:00");
        assert_eq!(clock(Duration::from_millis(61_900)), "1:01");
        assert_eq!(clock(Duration::from_secs(3599)), "59:59");
        assert_eq!(clock(Duration::from_secs(3661)), "1:01:01");
        assert_eq!(clock_ms(273_000), "4:33");
    }

    #[test]
    fn licences_have_a_readable_name() {
        assert_eq!(licence_label("cc-by-nc-sa"), "CC BY-NC-SA");
        assert_eq!(licence_label("public-domain"), "Pubblico dominio");
        assert_eq!(licence_label("qualcosa-di-nuovo"), "Licenza libera");
    }

    #[test]
    fn totals_and_sizes_read_like_the_iced_client() {
        assert_eq!(total(0), "0 min");
        assert_eq!(total(2_880_000), "48 min");
        assert_eq!(total(7_200_000), "2 h");
        assert_eq!(total(3_900_000), "1 h 5 min");
        assert_eq!(size(512), "512 B");
        assert_eq!(size(126_400_000), "126,4 MB");
        assert_eq!(size(2_000_000_000_000), "2000,0 GB");
    }

    #[test]
    fn counts_agree_in_number() {
        assert_eq!(tracks(1), "1 brano");
        assert_eq!(tracks(0), "0 brani");
        assert_eq!(tracks(12), "12 brani");
    }

    #[test]
    fn fans_and_licences_read_like_the_iced_client() {
        assert_eq!(super::fans(12), "12 fan");
        assert_eq!(super::fans(12_400), "12.400 fan");
        assert_eq!(super::fans(3_450_000), "3,4 Mln di fan");
        assert_eq!(super::licence_summary(["CC BY", "CC0", "CC BY"]), "CC BY · 2, CC0 · 1");
    }
}
