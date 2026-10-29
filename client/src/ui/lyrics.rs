//! Il testo del brano in riproduzione: un solo motore per il pannello laterale e per la vista a tutto
//! schermo. Il testo sincronizzato non è uno `scrollable`: ogni verso sta alla posizione che gli dà
//! la riga in corso (`App::lyrics_scroll`, un `Tween` misurato in righe), così il testo scivola da una
//! riga all'altra invece di saltare.
//!
//! - La riga in corso cresce fino al corpo pieno e si accende parola per parola. LRC dà solo l'inizio
//!   della riga: il canto si stima sui caratteri e finisce prima della riga successiva.
//! - Le altre righe restano più piccole e sfumano verso i bordi.
//! - Le pause strumentali (righe vuote) sono tre puntini che si accendono fino al verso successivo.
//! - Un clic su un verso porta lì la riproduzione.
//!
//! Il testo semplice, senza tempi, si scorre a mano.

use std::time::Duration;

use iced::widget::{Space, button, column, container, responsive, rich_text, scrollable, span, stack, text};
use iced::{Color, Element, Length, Padding};

use super::components::{empty_block, outlined_button, skeleton, small};
use super::motion::{fade, mix};
use super::theme;
use crate::app::{App, Load, Message, SideTab};
use crate::player::lyrics::{Line, Text};

/// Come si disegna il testo sincronizzato.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// Corpo della riga in corso; le altre stanno a `REST_SCALE` di questo.
    pub size: f32,
    /// Spazio fra due versi.
    pub gap: f32,
    /// Dove sta la riga in corso, dall'alto, in frazione dell'altezza disponibile.
    pub anchor: f32,
}

/// Nel pannello laterale: la riga in corso sta in alto, si legge quello che arriva.
pub const PANEL: Look = Look { size: 22.0, gap: 14.0, anchor: 0.3 };
/// A tutto schermo, e con meno spazio.
pub const STAGE: Look = Look { size: 38.0, gap: 20.0, anchor: 0.4 };
pub const STAGE_NARROW: Look = Look { size: 30.0, gap: 16.0, anchor: 0.4 };

/// Interlinea dei versi.
const LEADING: f32 = 1.2;
/// Avanzamento dei caratteri ASCII stampabili (da ' ' a '~') di Inter SemiBold, in millesimi di em,
/// letto da `assets/fonts/Inter-SemiBold.ttf`. Serve a stimare dove un verso va a capo.
const ADVANCE: [u16; 95] = [
    269, 321, 523, 644, 650, 1004, 663, 326, 391, 391, 647, 647, 269, 647, 269, 379, 647, 647, 647, 647, 647,
    647, 647, 647, 647, 647, 269, 269, 647, 647, 647, 543, 999, 728, 659, 737, 722, 605, 588, 749, 746, 277,
    580, 703, 565, 922, 759, 769, 645, 773, 652, 650, 660, 736, 728, 1020, 720, 713, 652, 391, 379, 391, 481,
    469, 351, 574, 624, 583, 624, 591, 389, 625, 612, 262, 262, 569, 262, 900, 612, 609, 624, 624, 397, 549,
    353, 612, 587, 839, 569, 588, 566, 391, 359, 391, 647,
];
/// Gli altri caratteri (lettere accentate, altri alfabeti) contano per tanto, in em: per eccesso.
const OTHER_EM: f32 = 0.7;
/// Margine sulla larghezza stimata: il kerning restringe, ma un verso troppo basso si taglierebbe.
const WRAP_SAFETY: f32 = 1.02;
/// Scala delle righe che non sono in corso.
const REST_SCALE: f32 = 0.84;
/// Opacità delle righe vicine a quella in corso; più lontano sfumano fino a sparire.
const DIM: f32 = 0.42;
/// Opacità delle parole della riga in corso non ancora cantate.
const UNSUNG: f32 = 0.5;
/// Velocità stimata del canto: un attacco e tanti secondi per carattere.
const SWEEP_LEAD: f32 = 0.3;
const PER_CHAR: f32 = 0.065;
/// Le pause strumentali.
const DOTS: &str = "•  •  •";

pub fn view(app: &App) -> Element<'_, Message> {
    let body: Element<'_, Message> = match &app.lyrics {
        Load::Idle => empty_block("Scegli un brano per leggere il testo."),
        Load::Loading => loading(),
        Load::Failed(err) => column![
            small(err.to_string()),
            outlined_button("Riprova", None, Some(Message::SideTab(SideTab::Lyrics))),
        ]
        .spacing(12)
        .into(),
        Load::Ready(text) => ready(app, text),
    };
    column![body, small("Testi da LRCLIB")].spacing(12).height(Length::Fill).into()
}

/// In attesa del testo: versi grigi della misura di quelli veri.
fn loading<'a>() -> Element<'a, Message> {
    const WIDTHS: [f32; 6] = [0.86, 0.62, 0.92, 0.48, 0.74, 0.58];
    let inner = theme::SIDE_PANEL_WIDTH - 32.0;
    let rows = WIDTHS.iter().map(|share| skeleton(Length::Fixed(inner * share), PANEL.size * 0.8));
    column(rows).spacing(PANEL.gap + 8.0).padding(Padding { top: 12.0, ..Padding::ZERO }).into()
}

fn ready<'a>(app: &'a App, shown: &'a Text) -> Element<'a, Message> {
    if shown.is_synced() {
        responsive(move |size| synced(app, shown, PANEL, size.width, size.height)).into()
    } else if let Some(plain) = &shown.plain {
        scrollable(text(plain.as_str()).size(theme::FS_BODY).color(theme::MUTED))
            .height(Length::Fill)
            .style(theme::scroll)
            .into()
    } else {
        empty_block("LRCLIB non ha un testo per questo brano.")
    }
}

/// Altezza di un verso a `width` e corpo `size`: le righe che occupa, l'interlinea e lo spazio.
fn verse_height(verse: &Line, width: f32, size: f32, gap: f32) -> f32 {
    rows(&verse.text, width, size) * size * LEADING + gap
}

/// Altezza di ogni verso mentre il testo è a `position`: fra quella a riposo e quella a corpo pieno,
/// secondo quanto il verso è in corso. Così lo spazio cresce con la riga, senza scatti e senza
/// lasciare vuoti sotto i versi che andrebbero a capo solo a corpo pieno.
pub fn verse_heights(lines: &[Line], width: f32, look: Look, position: f32) -> Vec<f32> {
    lines
        .iter()
        .enumerate()
        .map(|(index, verse)| {
            let rest = verse_height(verse, width, look.size * REST_SCALE, look.gap);
            let current = (1.0 - (index as f32 - position).abs()).clamp(0.0, 1.0);
            if current == 0.0 {
                return rest;
            }
            rest + (verse_height(verse, width, look.size, look.gap) - rest) * current
        })
        .collect()
}

/// Larghezza di un testo in em.
pub(super) fn em_width(text: &str) -> f32 {
    text.chars().map(char_em).sum()
}

/// La larghezza di un carattere in em (Inter SemiBold; per i pesi più leggeri è per eccesso).
pub(super) fn char_em(c: char) -> f32 {
    match (c as usize).checked_sub(32).and_then(|i| ADVANCE.get(i)) {
        Some(&advance) => f32::from(advance) / 1000.0,
        None => OTHER_EM,
    }
}

/// Quante righe occupa `text` a corpo `size` in `width`, andando a capo fra le parole come il testo
/// di Iced; una parola più lunga della riga si spezza.
fn rows(text: &str, width: f32, size: f32) -> f32 {
    let limit = (width / (size * WRAP_SAFETY)).max(1.0);
    let space = f32::from(ADVANCE[0]) / 1000.0;
    let (mut rows, mut line) = (1.0_f32, 0.0_f32);
    for word in text.split_whitespace() {
        let word = em_width(word);
        if line > 0.0 && line + space + word <= limit {
            line += space + word;
            continue;
        }
        if line > 0.0 {
            rows += 1.0;
        }
        // La parola apre una riga nuova; se non ci sta, occupa le righe che servono.
        let extra = (word / limit).ceil().max(1.0) - 1.0;
        rows += extra;
        line = word - extra * limit;
    }
    rows
}

/// Il punto (in pixel dalla cima del testo) che deve stare all'altezza dell'ancora: il centro della
/// riga in `position`, che può essere fra due righe mentre il testo scorre. Prima della prima riga
/// (`-1`) il testo sta ancora più in basso.
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

/// Quanto della riga è già cantato, da 0 a 1, a `now`. Una pausa strumentale dura fino al verso
/// successivo; un verso si canta in un tempo stimato dai caratteri, mai oltre il verso successivo.
pub fn sung_fraction(verse: &Line, next: Option<Duration>, now: Duration) -> f32 {
    let elapsed = now.saturating_sub(verse.at).as_secs_f32();
    let gap = next.map_or(f32::INFINITY, |next| next.saturating_sub(verse.at).as_secs_f32());
    let span = if verse.text.trim().is_empty() {
        gap
    } else {
        (SWEEP_LEAD + verse.text.chars().count() as f32 * PER_CHAR).min(gap * 0.95)
    };
    if span.is_infinite() {
        return 0.0;
    }
    if span <= 0.0 {
        return 1.0;
    }
    (elapsed / span).clamp(0.0, 1.0)
}

/// Le parole di un verso, accese fino a `fraction`: quelle cantate in `lit`, le altre in `unlit`, e
/// quella a metà fra i due.
fn sung_spans<'a>(words: &'a str, fraction: f32, lit: Color, unlit: Color) -> Vec<text::Span<'a, Message>> {
    let total = words.chars().count().max(1) as f32;
    let mut done = 0.0;
    words
        .split_inclusive(' ')
        .map(|word| {
            let start = done / total;
            done += word.chars().count() as f32;
            let end = done / total;
            let t = ((fraction - start) / (end - start).max(f32::EPSILON)).clamp(0.0, 1.0);
            span(word).color(mix(unlit, lit, t))
        })
        .collect()
}

/// Il testo sincronizzato in un riquadro di `width` per `height`, con la riga in corso all'ancora.
pub fn synced<'a>(
    app: &'a App,
    shown: &'a Text,
    look: Look,
    width: f32,
    height: f32,
) -> Element<'a, Message> {
    let position = app.lyrics_scroll.value();
    let heights = verse_heights(&shown.lines, width, look, position);
    let tops: Vec<f32> = heights
        .iter()
        .scan(0.0, |y, h| {
            let top = *y;
            *y += h;
            Some(top)
        })
        .collect();
    let focus = focus_y(&tops, &heights, position);
    let anchor = height * look.anchor;
    // Prima del primo verso il testo entra dal basso; in un riquadro basso (telefono piccolo) il
    // primo verso resta comunque tutto dentro, invece di sparire.
    let offset = (focus - anchor).max(heights.first().copied().unwrap_or(0.0) - height);
    let now = app.shown_position();

    // Solo i versi interi dentro il riquadro: quelli che escono sono già trasparenti.
    let mut verses = column![].width(width);
    let mut first_top = None;
    for (index, verse) in shown.lines.iter().enumerate() {
        let (top, verse_height) = (tops[index] - offset, heights[index]);
        if top < 0.0 {
            continue;
        }
        if top + verse_height > height {
            break;
        }
        first_top.get_or_insert(top);
        let distance = tops[index] + verse_height / 2.0 - focus;
        let reach = if distance < 0.0 { anchor } else { height - anchor };
        let near = (1.0 - distance.abs() / reach.max(1.0)).clamp(0.0, 1.0).powf(0.7);
        // Da 0 a 1 mentre il testo arriva su questa riga: ne fa crescere il corpo e l'opacità.
        let current = (1.0 - (index as f32 - position).abs()).clamp(0.0, 1.0);
        let dim = DIM * near;
        let alpha = dim + (1.0 - dim) * current;
        // Al pixel: mentre la riga cresce il testo si reimpagina solo quando il corpo cambia davvero.
        let size = (look.size * (REST_SCALE + (1.0 - REST_SCALE) * current)).round();
        let pause = verse.text.trim().is_empty();
        let words = if pause { DOTS } else { verse.text.as_str() };

        let label: Element<'a, Message> = if app.lyrics_line == Some(index) {
            let fraction = sung_fraction(verse, shown.lines.get(index + 1).map(|next| next.at), now);
            let unlit = fade(theme::TEXT, (alpha * UNSUNG).max(dim));
            rich_text(sung_spans(words, fraction, fade(theme::TEXT, alpha), unlit))
                .size(size)
                .font(theme::FONT_SEMIBOLD)
                .line_height(LEADING)
                .width(Length::Fill)
                .into()
        } else {
            text(words)
                .size(size)
                .font(theme::FONT_SEMIBOLD)
                .line_height(LEADING)
                .color(fade(theme::TEXT, alpha))
                .width(Length::Fill)
                .into()
        };
        // Il pulsante sta sopra il testo: le parole accese catturerebbero il clic.
        let seek = button(Space::new(Length::Fill, Length::Fill))
            .padding(0)
            .style(theme::bare)
            .on_press(Message::LyricSeek(verse.at));
        verses = verses.push(
            stack![container(label).width(width).height(verse_height), seek]
                .width(width)
                .height(verse_height),
        );
    }
    let top = first_top.unwrap_or(0.0);
    container(verses).padding(Padding { top, ..Padding::ZERO }).width(width).height(height).clip(true).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verse(at_ms: u64, text: &str) -> Line {
        Line { at: Duration::from_millis(at_ms), text: text.into() }
    }

    #[test]
    fn long_verses_get_more_rows() {
        let look = Look { size: 30.0, gap: 18.0, anchor: 0.4 };
        let heights = verse_heights(&[verse(0, "Hello"), verse(0, &"word ".repeat(30))], 600.0, look, 0.0);
        assert_eq!(heights[0], 30.0 * LEADING + 18.0);
        assert!(heights[1] >= 3.0 * 30.0 * LEADING + 18.0);
    }

    #[test]
    fn only_the_current_verse_takes_the_full_size_space() {
        let look = Look { size: 38.0, gap: 20.0, anchor: 0.4 };
        let wide = verse(0, "It doesn't even matter how hard you try");
        let lines = [wide.clone(), wide];
        let heights = verse_heights(&lines, 711.0, look, 0.0);
        assert_eq!(heights[0], 2.0 * 38.0 * LEADING + 20.0);
        assert!(heights[1] < 38.0 * LEADING + 20.0);
    }

    #[test]
    fn a_verse_that_fits_stays_on_one_row() {
        // Misurato sullo schermo: a 31,9 px questo verso è largo 610 px.
        assert_eq!(rows("Watch it fly by as the pendulum swings", 711.0, 31.92), 1.0);
        assert_eq!(rows("It doesn't even matter how hard you try", 711.0, 38.0), 2.0);
    }

    #[test]
    fn a_word_longer_than_the_row_breaks() {
        assert_eq!(rows(&"m".repeat(40), 100.0, 10.0), 4.0);
        assert_eq!(rows("", 100.0, 10.0), 1.0);
    }

    #[test]
    fn the_focus_moves_smoothly_between_two_verses() {
        let (tops, heights) = ([0.0, 50.0, 150.0], [50.0, 100.0, 50.0]);
        assert_eq!(focus_y(&tops, &heights, 0.0), 25.0);
        assert_eq!(focus_y(&tops, &heights, 1.0), 100.0);
        assert_eq!(focus_y(&tops, &heights, 0.5), 62.5);
        // Prima della prima riga il testo aspetta più in basso; oltre l'ultima resta sull'ultima.
        assert_eq!(focus_y(&tops, &heights, -1.0), -25.0);
        assert_eq!(focus_y(&tops, &heights, 7.0), 175.0);
    }

    #[test]
    fn a_verse_is_sung_before_the_next_one_starts() {
        let line = verse(10_000, "Twenty characters ok");
        let next = Some(Duration::from_millis(11_000));
        assert_eq!(sung_fraction(&line, next, Duration::from_millis(9_000)), 0.0);
        assert_eq!(sung_fraction(&line, next, Duration::from_millis(10_960)), 1.0);
    }

    #[test]
    fn a_long_gap_does_not_slow_the_singing_down() {
        let line = verse(0, "Short");
        let half_way = Duration::from_secs_f32((SWEEP_LEAD + 5.0 * PER_CHAR) / 2.0);
        let fraction = sung_fraction(&line, Some(Duration::from_secs(30)), half_way);
        assert!((fraction - 0.5).abs() < 0.01, "{fraction}");
    }

    #[test]
    fn a_pause_fills_up_until_the_next_verse() {
        let pause = verse(0, "");
        let fraction = sung_fraction(&pause, Some(Duration::from_secs(10)), Duration::from_secs(5));
        assert!((fraction - 0.5).abs() < 0.01);
    }

    #[test]
    fn words_light_up_in_order() {
        let (lit, unlit) = (Color::WHITE, Color::BLACK);
        let spans = sung_spans("one two three", 0.5, lit, unlit);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].color, Some(lit));
        assert_eq!(spans[2].color, Some(unlit));
    }
}
