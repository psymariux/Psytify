//! Lo splash d'ingresso. Sul nero le tre barre del marchio saltano su una dopo l'altra con la molla,
//! poi ondeggiano piano come un equalizzatore mentre il nome si accende lettera per lettera. Alla
//! fine il velo sfuma e scopre l'app, che intanto entra salendo (`App::advance_splash`). Un clic lo
//! chiude subito. Dura meno di due secondi e non aspetta niente: l'app si carica sotto.

use std::time::Duration;

use iced::widget::{Space, column, container, mouse_area, rich_text, row, span};
use iced::{Alignment, Element, Length, Padding, alignment};

use super::motion::{self, fade};
use super::theme;
use crate::app::{App, Message};

/// Le barre del marchio (`assets/icons/mark.svg`: larghe 4, alte 9, 13 e 17 su 24) in grande.
const BAR_WIDTH: f32 = 22.0;
const BAR_GAP: f32 = 15.0;
const BAR_HEIGHTS: [f32; 3] = [50.0, 72.0, 94.0];
/// Ogni barra parte tanto dopo la precedente, e la sua molla dura tanto.
const BAR_DELAY: Duration = Duration::from_millis(90);
const BAR_RISE: Duration = Duration::from_millis(620);
/// Quando le barre sono posate cominciano a ondeggiare, di poco.
const DANCE_FROM: Duration = Duration::from_millis(700);
const DANCE: f32 = 0.1;
const DANCE_PERIOD: Duration = Duration::from_millis(640);
/// Il nome: da quando compare, la distanza fra una lettera e l'altra e quanto ci mette ognuna.
const NAME: &str = "Psytify";
const NAME_SIZE: f32 = 40.0;
const NAME_FROM: Duration = Duration::from_millis(360);
const LETTER_STAGGER: Duration = Duration::from_millis(45);
const LETTER_IN: Duration = Duration::from_millis(280);
const NAME_RISE: f32 = 12.0;
/// Uscendo il marchio si avvicina un poco, mentre sfuma.
const EXIT_GROW: f32 = 0.06;

/// Lo splash, finché è sullo schermo.
pub fn view(app: &App) -> Option<Element<'_, Message>> {
    let elapsed = app.splash?.since.elapsed();
    let exit = (elapsed.saturating_sub(theme::SPLASH_EXIT).as_secs_f32()
        / (theme::SPLASH_TOTAL - theme::SPLASH_EXIT).as_secs_f32())
    .clamp(0.0, 1.0);
    let leaving = motion::ease_out(exit);
    let alpha = 1.0 - leaving;
    let grow = 1.0 + EXIT_GROW * leaving;

    let dance_ramp =
        (elapsed.saturating_sub(DANCE_FROM).as_secs_f32() / BAR_RISE.as_secs_f32()).clamp(0.0, 1.0);
    let since = app.splash?.since;
    let bars = BAR_HEIGHTS.iter().enumerate().map(|(lane, &height)| {
        let rise = motion::spring_in(since, BAR_DELAY * u32::try_from(lane).unwrap_or(0), BAR_RISE);
        let dance = 1.0 + DANCE * dance_ramp * (2.0 * motion::wave(lane, DANCE_PERIOD) - 1.0);
        let shown = (height * rise * dance * grow).max(0.0);
        container(Space::new(BAR_WIDTH * grow, shown))
            .style(theme::bar(fade(theme::ACCENT, alpha), BAR_WIDTH * grow / 2.0))
            .into()
    });
    // Il posto delle barre comprende il rimbalzo e l'ondeggiare, così il nome sotto non si muove.
    let tallest = BAR_HEIGHTS[2] * (1.0 + 2.0 * DANCE) * (1.0 + EXIT_GROW);
    let mark = container(row(bars).spacing(BAR_GAP * grow).align_y(Alignment::End))
        .height(tallest)
        .align_y(alignment::Vertical::Bottom);

    let name_shown = elapsed.saturating_sub(NAME_FROM);
    let letters: Vec<_> = NAME
        .char_indices()
        .enumerate()
        .map(|(index, (at, letter))| {
            let delay = LETTER_STAGGER * u32::try_from(index).unwrap_or(0);
            let t = name_shown.saturating_sub(delay).as_secs_f32() / LETTER_IN.as_secs_f32();
            let lit = motion::ease_out(t);
            span(&NAME[at..at + letter.len_utf8()]).color(fade(theme::TEXT, lit * alpha))
        })
        .collect();
    let name_rise = NAME_RISE * (1.0 - motion::ease_out(name_shown.as_secs_f32() / BAR_RISE.as_secs_f32()));
    let name = container(rich_text(letters).size(NAME_SIZE).font(theme::FONT_SEMIBOLD)).padding(Padding {
        top: name_rise,
        bottom: NAME_RISE - name_rise,
        ..Padding::ZERO
    });

    let content = column![mark, name].spacing(26).align_x(Alignment::Center);
    Some(
        mouse_area(
            container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .center(Length::Fill)
                .style(theme::splash(alpha)),
        )
        .on_press(Message::SkipSplash)
        .into(),
    )
}
