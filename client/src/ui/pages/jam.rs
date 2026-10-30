//! Jam: ascolto condiviso con altre persone, fino a 8 per stanza. Si crea una stanza e si
//! condivide il codice, oppure si entra con quello di qualcun altro.

use iced::widget::{column, container, row, text_input};
use iced::{Alignment, Element, Length};

use crate::app::{Account, App, JamSession, Message};
use crate::jam::{CODE_LEN, MAX_MEMBERS};
use crate::ui::components::{
    BODY_LINE, h1, h2, line, muted, outlined_button, primary_button, section, small,
};
use crate::ui::icons::Icon;
use crate::ui::pages::friends::person_row;
use crate::ui::theme;

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let inner = width - 2.0 * theme::PAD;
    let body = match &app.jam {
        Some(session) => joined(app, session, inner),
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
    let start = section(
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
    );
    if app.social.jam_invites.is_empty() {
        return start;
    }
    column![invites(app, width), start].spacing(36).into()
}

/// Gli inviti degli amici: un tocco per entrare nella loro jam, oppure per rifiutare.
fn invites(app: &App, width: f32) -> Element<'_, Message> {
    let rows = app.social.jam_invites.iter().map(|invite| {
        let join = primary_button("Entra", None, Message::JamInviteAccept(invite.code.clone()));
        let decline = outlined_button("Ignora", None, Some(Message::JamInviteDismiss(invite.code.clone())));
        let text_width = (width - 2.0 * theme::PAD - 2.0 * 120.0).max(80.0);
        row![
            column![
                line(&invite.from_name, text_width, BODY_LINE),
                line(&format!("@{} ti invita in una jam", invite.from_handle), text_width, BODY_LINE),
            ]
            .spacing(2)
            .width(Length::Fill),
            join,
            decline,
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    });
    section("Inviti", None, card(column(rows).spacing(12)))
}

/// Con una jam aperta: il codice da condividere, chi c'è e l'uscita.
fn joined<'a>(app: &'a App, session: &'a JamSession, width: f32) -> Element<'a, Message> {
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
        invite_friends(app, width),
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

/// Gli amici da invitare nella jam in corso: se ne scelgono quanti si vuole e si manda un invito
/// a tutti insieme. Chi lo riceve lo vede nella propria schermata Jam.
fn invite_friends(app: &App, width: f32) -> Element<'_, Message> {
    let body: Element<'_, Message> = match (&app.account, app.social.friends.ready()) {
        (Account::SignedIn(me), Some(friends)) if me.profile.is_some() => {
            if friends.friends.is_empty() {
                small("Non hai ancora amici da invitare: trovali dalla pagina Amici.").into()
            } else {
                let rows = friends.friends.iter().map(|friend| {
                    let handle = friend.profile.handle.clone();
                    let chosen = app.social.invitees.contains(&handle);
                    let toggle = if chosen {
                        primary_button("Scelto", Some(Icon::Check), Message::JamInviteToggle(handle))
                    } else {
                        outlined_button("Scegli", None, Some(Message::JamInviteToggle(handle)))
                    };
                    person_row(app, &friend.profile, None, vec![toggle], width - 2.0 * theme::PAD)
                });
                let ready = !app.social.invitees.is_empty() && !app.social.inviting;
                let send = if ready {
                    primary_button("Invia l'invito", Some(Icon::UserPlus), Message::JamInviteSend)
                } else {
                    outlined_button("Invia l'invito", Some(Icon::UserPlus), None)
                };
                column![column(rows).spacing(4), send].spacing(12).into()
            }
        }
        (Account::SignedIn(_), None) => small("Carico i tuoi amici…").into(),
        _ => small("Accedi con un profilo per invitare i tuoi amici.").into(),
    };
    let chosen = app.social.invitees.len();
    let trailing = (chosen > 0).then(|| small(format!("{chosen} scelti")).into());
    section("Invita amici", trailing, card(body))
}
