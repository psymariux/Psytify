//! Lo splash d'ingresso, sulla tela nera. Il marchio (la faccia di Nastro, `mark`) compare in tre
//! passi da 120 ms, il nome si scrive lettera per lettera (45 ms l'una) con un cursore a blocco
//! bianco e, finita la durata (`theme::SPLASH_TOTAL`, sotto 1,2 s), l'app appare in un taglio. Niente
//! lime (non suona ancora nulla) e niente dissolvenze degli sprite. Un clic lo chiude subito. Non
//! aspetta niente: l'app si carica sotto.

use std::time::Duration;

use iced::widget::{column, container, mouse_area, rich_text, row, span};
use iced::{Alignment, Element, Length};

use super::icons::sprite_image;
use super::motion::fade;
use super::theme;
use crate::app::{App, Message};

/// Il marchio compare in tre passi (guscio, bobine, guida del nastro), uno ogni tanto.
const MARK_STEP: Duration = Duration::from_millis(120);
/// Il marchio si ingrandisce di quanti pixel d'arte per pixel: è un'immagine sola, grande, sulla
/// schermata che lo mostra (96 dp).
const MARK_ZOOM: u32 = 4;
/// Il nome: da quando si scrive e la distanza fra una lettera e l'altra.
const NAME: &str = "Psytify";
const NAME_SIZE: f32 = 40.0;
const NAME_FROM: Duration = Duration::from_millis(400);
const LETTER_STEP: Duration = Duration::from_millis(45);
/// Il cursore a blocco (U+2588), acceso e spento a questo passo.
const CURSOR: &str = "\u{2588}";
const CURSOR_BLINK: Duration = Duration::from_millis(250);

/// Lo splash, finché è sullo schermo.
pub fn view(app: &App) -> Option<Element<'_, Message>> {
    let elapsed = app.splash?.since.elapsed();

    // Tre passi netti (un terzo, due terzi, pieno), senza interpolare.
    let steps = (elapsed.as_millis() / MARK_STEP.as_millis().max(1)).min(2) as f32 + 1.0;
    let mark = sprite_image(psytify_design::sprite::MARK, 0, Some(fade(theme::TEXT, steps / 3.0)), MARK_ZOOM);

    let typed = (elapsed.saturating_sub(NAME_FROM).as_millis() / LETTER_STEP.as_millis().max(1)) as usize;
    let cursor_on =
        (elapsed.as_millis() / CURSOR_BLINK.as_millis().max(1)).is_multiple_of(2) || typed < NAME.len();
    let mut letters = Vec::with_capacity(NAME.len() + 1);
    for (index, (at, letter)) in NAME.char_indices().enumerate() {
        let shown = if index < typed { theme::TEXT } else { iced::Color::TRANSPARENT };
        let piece = &NAME[at..at + letter.len_utf8()];
        if index == typed.min(NAME.len()) {
            letters.push(span(CURSOR).color(if cursor_on { theme::FOCUS } else { iced::Color::TRANSPARENT }));
        }
        letters.push(span(piece).color(shown));
    }
    if typed >= NAME.len() {
        letters.push(span(CURSOR).color(if cursor_on { theme::FOCUS } else { iced::Color::TRANSPARENT }));
    }
    let name = rich_text(letters).size(NAME_SIZE).font(theme::FONT_SEMIBOLD);

    let content = column![mark, row![name]].spacing(24).align_x(Alignment::Center);
    // Fino a `SPLASH_TOTAL` lo splash è pieno; poi l'app lo toglie e scopre la pagina in un taglio.
    Some(
        mouse_area(
            container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .center(Length::Fill)
                .style(theme::splash(1.0)),
        )
        .on_press(Message::SkipSplash)
        .into(),
    )
}
