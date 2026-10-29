//! Finestre di dialogo al centro: nome di una playlist nuova, rinomina, conferma di eliminazione.
//! Sotto c'è un velo scuro; un clic sul velo, Esc o «Annulla» le chiudono, Invio conferma.

use iced::widget::{Space, button, column, container, mouse_area, opaque, row, stack, text, text_input};
use iced::{Alignment, Element, Length, Padding, alignment};

use super::components::{body, h2, muted, primary_button, text_button};
use super::{motion, theme};
use crate::app::{App, DIALOG_INPUT, Dialog, Message};

const WIDTH: f32 = 420.0;
/// Quanto sale la finestra mentre compare.
const RISE: f32 = 10.0;

/// Il livello della finestra di dialogo; `None` se non ce n'è una aperta.
pub fn view(app: &App) -> Option<Element<'_, Message>> {
    let open = app.dialog.as_ref()?;
    let shown = motion::appear(open.since, theme::MOTION_PANEL);
    let (title, content, confirm): (&str, Element<'_, Message>, Element<'_, Message>) = match &open.dialog {
        Dialog::NewPlaylist { name, seed } => {
            let hint = match seed.len() {
                0 => "Dai un nome alla playlist. Potrai aggiungere i brani dal menu del clic destro."
                    .to_string(),
                1 => "La playlist parte con il brano che hai scelto.".to_string(),
                n => format!("La playlist parte con questi {n} brani."),
            };
            (
                "Nuova playlist",
                column![muted(hint), name_field(name)].spacing(14).into(),
                primary_button("Crea", None, Message::DialogSubmit),
            )
        }
        Dialog::Rename { name, .. } => (
            "Rinomina la playlist",
            column![name_field(name)].spacing(14).into(),
            primary_button("Salva", None, Message::DialogSubmit),
        ),
        Dialog::ConfirmDelete(id) => {
            let name = app.playlist(*id).map(|p| p.name.as_str()).unwrap_or("questa playlist");
            (
                "Eliminare la playlist?",
                body(format!("«{name}» sparirà dalla libreria. I brani restano nel catalogo.")).into(),
                button(super::components::centered(
                    text("Elimina").size(theme::FS_BODY).font(theme::FONT_MEDIUM),
                ))
                .height(theme::CONTROL_HEIGHT)
                .padding([0, 18])
                .style(theme::danger)
                .on_press(Message::DialogSubmit)
                .into(),
            )
        }
    };
    let actions =
        row![Space::with_width(Length::Fill), text_button("Annulla", Message::DialogCancel), confirm]
            .spacing(18)
            .align_y(Alignment::Center);
    let card = container(column![h2(title), content, actions].spacing(18))
        .width(WIDTH)
        .padding(theme::PAD)
        .style(theme::dialog_box(shown));
    let centered = container(opaque(card))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .padding(Padding { top: RISE * (1.0 - shown) * 2.0, ..Padding::ZERO });
    Some(
        stack![
            mouse_area(container(Space::new(Length::Fill, Length::Fill)).style(theme::scrim(shown)))
                .on_press(Message::DialogCancel),
            centered,
        ]
        .into(),
    )
}

fn name_field<'a>(value: &str) -> Element<'a, Message> {
    text_input("Nome della playlist", value)
        .id(text_input::Id::new(DIALOG_INPUT))
        .on_input(Message::DialogInput)
        .on_submit(Message::DialogSubmit)
        .size(theme::FS_BODY + 1.0)
        .padding([10, 14])
        .style(theme::dialog_input)
        .width(Length::Fill)
        .into()
}
