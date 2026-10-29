//! Jam: ascolto condiviso con altre persone, fino a 8 per stanza. Si crea una stanza e si
//! condivide il codice, oppure si entra con quello di qualcun altro.

use iced::widget::{column, container, row, text_input};
use iced::{Alignment, Element, Length};

use crate::app::{App, JamSession, Message};
use crate::jam::{CODE_LEN, MAX_MEMBERS};
use crate::ui::components::{
    BODY_LINE, h1, h2, line, muted, outlined_button, primary_button, section, small,
};
use crate::ui::theme;

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let inner = width - 2.0 * theme::PAD;
    let body = match &app.jam {
        Some(session) => joined(session, inner),
        None => start(app, inner),
    };
    column![h1("Jam"), body].spacing(36).width(Length::Fill).into()
}

fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content).padding(theme::PAD).width(Length::Fill).style(theme::card).into()
}

/// Nessuna jam: il nome da mostrare nella stanza, la creazione e l'ingresso con un codice.
fn start(app: &App, width: f32) -> Element<'_, Message> {
    let name = text_input("Il tuo nome (facoltativo)", &app.jam_name)
        .on_input(Message::JamName)
        .size(theme::FS_BODY)
        .padding([8, 14])
        .style(theme::search_input);
    let code = text_input("Codice di 6 caratteri", &app.jam_code)
        .on_input(Message::JamCode)
        .on_submit(Message::JamJoin)
        .size(theme::FS_BODY)
        .padding([8, 14])
        .style(theme::search_input);
    let join = row![code, outlined_button("Entra", None, Some(Message::JamJoin))]
        .spacing(8)
        .align_y(Alignment::Center);
    section(
        "Ascolta insieme",
        None,
        card(
            column![
                // Due righe: la frase è lunga e tagliarla a metà non direbbe che cosa si condivide.
                muted("Fino a 8 persone ascoltano la stessa coda. Ognuno suona dal proprio dispositivo:")
                    .width(width),
                muted("la jam condivide solo il brano e la posizione."),
                name,
                primary_button("Crea una jam", None, Message::JamCreate),
                small(format!("Per entrare serve il codice di {CODE_LEN} caratteri.")),
                join,
            ]
            .spacing(12),
        ),
    )
}

/// Con una jam aperta: il codice da condividere, chi c'è e l'uscita.
fn joined(session: &JamSession, width: f32) -> Element<'_, Message> {
    let code = session.code.as_deref().unwrap_or("…");
    let people = session.members.iter().map(|member| {
        let label = if session.you == Some(member.id) {
            format!("{} (tu)", member.name)
        } else {
            member.name.clone()
        };
        line(&label, width, BODY_LINE)
    });
    let status = if session.code.is_some() { "Collegato alla stanza" } else { "Connessione in corso…" };
    column![
        section(
            "Codice",
            None,
            card(
                column![
                    h2(code),
                    muted("Condividilo con chi vuoi far entrare. Vale finché la jam è aperta."),
                ]
                .spacing(8),
            ),
        ),
        section(
            "Persone",
            Some(small(format!("{} di {MAX_MEMBERS}", session.members.len())).into()),
            card(column(people).spacing(8)),
        ),
        section(
            "Stato",
            None,
            card(
                column![small(status), outlined_button("Esci dalla jam", None, Some(Message::JamLeave)),]
                    .spacing(12),
            ),
        ),
    ]
    .spacing(36)
    .into()
}
