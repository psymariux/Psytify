//! Amici: chi stai seguendo nella musica. In cima la ricerca di persone per nome utente, poi le
//! richieste ricevute (accetta o rifiuta), gli amici con il brano che stanno ascoltando e le
//! richieste inviate. Un tocco su una persona apre il suo profilo.

use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Element, Length};

use crate::api::social::{Friend, Profile};
use crate::app::{Account, App, Load, Message};
use crate::ui::components::{
    MUTED_LINE, SUB_LINE, TITLE_LINE, TableKind, empty_state, error_block, h1, line, outlined_button,
    primary_button, section, skeleton_rows, small,
};
use crate::ui::cover::{Art, cover_round};
use crate::ui::icons::Icon;
use crate::ui::pages::search::art_id;
use crate::ui::theme;

/// L'avatar tondo di una persona: la foto di Clerk, oppure un colore con l'iniziale.
pub fn avatar<'a>(app: &App, profile: &Profile, size: f32) -> Element<'a, Message> {
    cover_round(
        &app.covers,
        Art { id: art_id(&profile.handle), title: &profile.display_name, url: profile.avatar_url.as_deref() },
        size,
    )
}

/// Una persona su una riga: avatar, nome e `@nome`, sotto una riga facoltativa (il brano in
/// ascolto), e a destra le azioni.
pub fn person_row<'a>(
    app: &'a App,
    profile: &'a Profile,
    detail: Option<String>,
    actions: Vec<Element<'a, Message>>,
    width: f32,
) -> Element<'a, Message> {
    let text_width = (width - theme::AVATAR_ROW - 24.0 - 120.0 * actions.len() as f32).max(80.0);
    let mut lines = column![
        line(&profile.display_name, text_width, TITLE_LINE),
        line(&format!("@{}", profile.handle), text_width, SUB_LINE),
    ]
    .spacing(2)
    .width(Length::Fill);
    if let Some(detail) = detail {
        lines = lines.push(line(&detail, text_width, MUTED_LINE));
    }
    let open =
        button(row![avatar(app, profile, theme::AVATAR_ROW), lines].spacing(12).align_y(Alignment::Center))
            .width(Length::Fill)
            .padding([8, 10])
            .style(theme::row(false))
            .on_press(Message::OpenProfile(profile.handle.clone()));
    let mut item = row![open].spacing(8).align_y(Alignment::Center);
    for action in actions {
        item = item.push(action);
    }
    item.into()
}

/// «▶ Titolo · Artista» per chi sta ascoltando, «In pausa» se è fermo.
pub fn listening_line(friend: &Friend) -> Option<String> {
    let playing = friend.now_playing.as_ref()?;
    let state = if playing.paused { "In pausa" } else { "In ascolto" };
    Some(format!("{state}: {} · {}", playing.track.title, playing.track.artist.name))
}

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let title = h1("Amici");
    let Account::SignedIn(me) = &app.account else {
        return column![
            title,
            empty_state(
                "Accedi per trovare i tuoi amici",
                "Vedi cosa ascoltano, le loro playlist e gli artisti che amano.",
                Some(("Accedi", Message::SignIn)),
            ),
        ]
        .spacing(24)
        .into();
    };
    if me.profile.is_none() {
        return column![
            title,
            empty_state(
                "Scegli un nome utente",
                "Gli amici ti trovano con il tuo nome utente: sceglilo nelle impostazioni.",
                Some(("Apri le impostazioni", Message::Navigate(crate::app::Screen::Settings))),
            ),
        ]
        .spacing(24)
        .into();
    }
    let pending = |handle: &str| app.social.pending.iter().any(|h| h == handle);

    let search = row![
        text_input("Cerca per nome utente", &app.social.query)
            .on_input(Message::UserQuery)
            .on_submit(Message::UserSearch)
            .size(theme::FS_BODY)
            .padding([10, 16])
            .style(theme::search_input),
        outlined_button("Cerca", Some(Icon::Search), Some(Message::UserSearch)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let mut page = column![title, search].spacing(24).width(Length::Fill);

    match &app.social.found {
        Load::Idle => {}
        Load::Loading => page = page.push(skeleton_rows(3, theme::ROW_MEDIA, TableKind::Mixed)),
        Load::Failed(err) => page = page.push(error_block("la ricerca", err)),
        Load::Ready(found) if found.is_empty() => page = page.push(small("Nessuno con questo nome.")),
        Load::Ready(found) => {
            let rows = found.iter().map(|profile| {
                let add = outlined_button(
                    "Aggiungi",
                    Some(Icon::UserPlus),
                    (!pending(&profile.handle)).then(|| Message::FriendAction(profile.handle.clone(), true)),
                );
                person_row(app, profile, None, vec![add], width)
            });
            page = page.push(section("Persone", None, column(rows).spacing(4).into()));
        }
    }

    let friends = match &app.social.friends {
        Load::Idle | Load::Loading => {
            return page.push(skeleton_rows(4, theme::ROW_MEDIA, TableKind::Mixed)).into();
        }
        Load::Failed(err) => return page.push(error_block("gli amici", err)).into(),
        Load::Ready(friends) => friends,
    };
    if !friends.incoming.is_empty() {
        let rows = friends.incoming.iter().map(|profile| {
            let enabled = !pending(&profile.handle);
            let accept = primary_button("Accetta", None, Message::FriendAction(profile.handle.clone(), true));
            let decline = outlined_button(
                "Rifiuta",
                None,
                enabled.then(|| Message::FriendAction(profile.handle.clone(), false)),
            );
            // Il titolo «Richieste» dice già cosa vuole: la riga resta libera per nome e pulsanti.
            person_row(app, profile, None, vec![accept, decline], width)
        });
        page = page.push(section("Richieste", None, column(rows).spacing(4).into()));
    }
    let body: Element<'_, Message> = if friends.friends.is_empty() {
        empty_state(
            "Ancora nessun amico",
            "Cerca qualcuno per nome utente qui sopra e mandagli una richiesta.",
            None,
        )
    } else {
        column(
            friends
                .friends
                .iter()
                .map(|friend| person_row(app, &friend.profile, listening_line(friend), Vec::new(), width)),
        )
        .spacing(4)
        .into()
    };
    let count = text(friends.friends.len().to_string()).size(theme::FS_BODY).color(theme::MUTED);
    page = page.push(section("I tuoi amici", Some(count.into()), body));
    if !friends.outgoing.is_empty() {
        let rows = friends.outgoing.iter().map(|profile| {
            let cancel = outlined_button(
                "Ritira",
                None,
                (!pending(&profile.handle)).then(|| Message::FriendAction(profile.handle.clone(), false)),
            );
            person_row(app, profile, Some("Richiesta inviata".into()), vec![cancel], width)
        });
        page = page.push(section("In attesa", None, column(rows).spacing(4).into()));
    }
    page.push(Space::with_height(8))
        .push(container(small(format!("Sei @{}", me.profile.as_ref().map_or("", |p| p.handle.as_str())))))
        .into()
}
