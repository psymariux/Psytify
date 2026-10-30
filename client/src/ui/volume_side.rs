//! Il volume dei player a schermo intero: una barra verticale sul bordo destro. A riposo è un filo
//! discreto; appena la si tocca o la si trascina si allarga con un movimento morbido, si accende di
//! lime e mostra la percentuale. Qualche istante dopo il rilascio torna a filo (`App::volume_motion`).
//!
//! Sul telefono il dito che scorre arriva come rotella e lo `slider` di Iced la ignora, quindi sopra
//! la barra sta lo strato di `touch_seek`, come per le bande dell'equalizzatore.

use iced::widget::{column, container, stack, text, vertical_slider};
use iced::{Alignment, Element, Length, alignment};

use super::components::icon_button;
use super::icons::Icon;
use super::motion::fade;
use super::pointer::pointer;
use super::theme;
use super::touch_seek::touch_slider_vertical;
use crate::app::{App, Message};

/// Larghezza della colonna: ci stanno la percentuale, la barra e il tasto dell'audio.
const WIDTH: f32 = 48.0;
/// Larghezza della zona che segue il dito o il mouse sulla barra.
const GRAB: f32 = 30.0;

/// La barra del volume, alta `bar` pixel, allineata al bordo destro del livello che la contiene.
pub fn overlay<'a>(app: &App, bar: f32, touch: bool) -> Element<'a, Message> {
    let open = app.volume_motion.value();
    let volume = app.playback.volume;
    // La percentuale entra con la barra; il posto resta riservato, così nulla si sposta.
    let percent = container(
        text(format!("{}%", (volume * 100.0).round()))
            .size(theme::FS_SMALL)
            .font(theme::FONT_MEDIUM)
            .color(fade(theme::TEXT, open)),
    )
    .width(WIDTH)
    .align_x(alignment::Horizontal::Center);

    let slider = pointer(
        vertical_slider(0.0..=1.0, volume, Message::VolumeSide)
            .step(0.01_f32)
            .width(GRAB)
            .height(bar)
            .on_release(Message::VolumeSideReleased)
            .style(theme::side_volume(open)),
    );
    let track: Element<'a, Message> = if touch {
        stack![slider, touch_slider_vertical(Message::VolumeSide, Message::VolumeSideReleased)].into()
    } else {
        slider.into()
    };

    let (glyph, label) = if volume == 0.0 {
        (Icon::Mute, "Riattiva l'audio")
    } else if volume < 0.4 {
        (Icon::VolumeLow, "Disattiva l'audio")
    } else {
        (Icon::Volume, "Disattiva l'audio")
    };
    let column = column![
        percent,
        container(track).width(GRAB).height(bar).center_x(WIDTH),
        container(icon_button(glyph, label, false, Some(Message::ToggleMute))).center_x(WIDTH),
    ]
    .spacing(8)
    .align_x(Alignment::Center);

    let pill = container(column).padding([10, 0]).width(WIDTH).style(theme::volume_pill(open));
    container(pill)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Right)
        .center_y(Length::Fill)
        .into()
}
