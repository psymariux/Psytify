//! Il profilo di una persona: avatar grande e nome, il pulsante d'amicizia, il brano che sta
//! ascoltando, gli ascolti recenti, gli artisti che ascolta di più, le playlist pubbliche e gli
//! amici in comune. L'attività la vedono solo gli amici (lo decide il server).

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::api::social::{Relation, UserPage};
use crate::app::{App, Load, Message, Screen};
use crate::ui::components::{
    MUTED_LINE, Origin, SUB_LINE, TITLE_LINE, TableKind, art_of, count_label, empty_block, empty_state,
    error_block, line, muted, outlined_button, primary_button, section, skeleton_rows, small, track_rows,
    track_table,
};
use crate::ui::cover::{Art, cover};
use crate::ui::icons::Icon;
use crate::ui::pages::friends::avatar;
use crate::ui::theme;

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let Some((handle, load)) = &app.social.open else {
        return empty_block("Nessun profilo aperto.");
    };
    let open = match load {
        Load::Idle | Load::Loading => return skeleton_rows(6, theme::ROW_MEDIA, TableKind::Mixed),
        Load::Failed(err) if err.is_not_found() => return empty_block("Questa persona non esiste più."),
        Load::Failed(err) => return error_block("il profilo", err),
        Load::Ready(open) => open,
    };
    let page = &open.page;
    let pending = app.social.pending.iter().any(|h| h == handle);
    let mut body = column![header(app, page, pending, width)].spacing(32).width(Length::Fill);

    match &page.activity {
        Some(activity) => {
            if let Some(now) = &activity.now_playing {
                let state = if now.paused { "In pausa" } else { "Sta ascoltando" };
                let text_width = width - 2.0 * 12.0 - 64.0 - 160.0;
                let card = button(
                    row![
                        cover(&app.covers, art_of(&now.track), 64.0),
                        column![
                            text(state).size(theme::FS_SMALL).font(theme::FONT_SEMIBOLD).color(theme::ACCENT),
                            line(&now.track.title, text_width, TITLE_LINE),
                            line(&now.track.artist.name, text_width, SUB_LINE),
                        ]
                        .spacing(3)
                        .width(Length::Fill),
                        primary_button(
                            "Ascolta",
                            Some(Icon::Play),
                            Message::PlayTracks(std::sync::Arc::from([now.track.clone()]), 0)
                        ),
                    ]
                    .spacing(14)
                    .align_y(Alignment::Center),
                )
                .padding(12)
                .width(Length::Fill)
                .style(theme::row(false))
                .on_press(Message::PlayTracks(std::sync::Arc::from([now.track.clone()]), 0));
                body = body.push(container(card).style(theme::card));
            }
            if !activity.top_artists.is_empty() {
                let chips = row(activity.top_artists.iter().map(|artist| {
                    button(
                        column![
                            text(artist.name.as_str()).size(theme::FS_BODY).font(theme::FONT_MEDIUM),
                            small(if artist.plays == 1 {
                                "1 ascolto".to_owned()
                            } else {
                                format!("{} ascolti", artist.plays)
                            }),
                        ]
                        .spacing(2),
                    )
                    .padding([10, 16])
                    .style(theme::pill(false))
                    .on_press(Message::Navigate(Screen::Artist(artist.id)))
                    .into()
                }))
                .spacing(8)
                .wrap();
                body = body.push(section(
                    "Artisti più ascoltati",
                    Some(small("ultimi 30 giorni").into()),
                    chips.into(),
                ));
            }
            let recent: Element<'_, Message> = if open.recent.is_empty() {
                small("Ancora nessun ascolto.").into()
            } else {
                track_rows(app, &open.recent, width)
            };
            body = body.push(section("Ascolti recenti", None, recent));
        }
        None if page.relation != Relation::Me => {
            body = body.push(empty_state(
                "L'attività è per gli amici",
                "Quando siete amici vedi cosa ascolta, gli ascolti recenti e i suoi artisti preferiti.",
                None,
            ));
        }
        None => {}
    }

    if !page.playlists.is_empty() {
        let rows = page.playlists.iter().map(|playlist| {
            button(
                row![
                    cover(&app.covers, Art::drawn(playlist.id, &playlist.name), 48.0),
                    column![
                        line(&playlist.name, width - 120.0, TITLE_LINE),
                        line(&count_label(playlist.track_count as usize), width - 120.0, MUTED_LINE),
                    ]
                    .spacing(2),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([6, 10])
            .style(theme::row(false))
            .on_press(Message::OpenFriendPlaylist(handle.clone(), playlist.id))
            .into()
        });
        body = body.push(section("Playlist pubbliche", None, column(rows).spacing(2).into()));
    }
    if !page.mutual_friends.is_empty() {
        let people = row(page.mutual_friends.iter().map(|friend| {
            button(
                column![avatar(app, friend, 72.0), line(&friend.display_name, 88.0, SUB_LINE)]
                    .spacing(6)
                    .align_x(Alignment::Center),
            )
            .width(96)
            .padding(4)
            .style(theme::bare)
            .on_press(Message::OpenProfile(friend.handle.clone()))
            .into()
        }))
        .spacing(8)
        .wrap();
        body = body.push(section("Amici in comune", None, people.into()));
    }
    body.push(Space::with_height(8)).into()
}

/// Avatar, nome, `@nome`, quanti amici e il pulsante giusto per la relazione.
fn header<'a>(app: &'a App, page: &'a UserPage, pending: bool, width: f32) -> Element<'a, Message> {
    let profile = &page.profile;
    let handle = profile.handle.clone();
    let action: Element<'a, Message> = match page.relation {
        Relation::Me => {
            outlined_button("Modifica profilo", Some(Icon::Edit), Some(Message::Navigate(Screen::Settings)))
        }
        Relation::Friend => outlined_button(
            "Amici",
            Some(Icon::Check),
            (!pending).then(|| Message::FriendAction(handle, false)),
        ),
        Relation::Incoming => {
            primary_button("Accetta la richiesta", Some(Icon::UserPlus), Message::FriendAction(handle, true))
        }
        Relation::Outgoing => outlined_button(
            "Richiesta inviata",
            None,
            (!pending).then(|| Message::FriendAction(handle, false)),
        ),
        Relation::None => {
            primary_button("Aggiungi agli amici", Some(Icon::UserPlus), Message::FriendAction(handle, true))
        }
    };
    let friends = match page.friend_count {
        1 => "1 amico".to_owned(),
        n => format!("{n} amici"),
    };
    let size = if crate::app::is_phone() { theme::FS_PHONE_TITLE } else { theme::FS_PAGE };
    let narrow = width < theme::AVATAR_PROFILE + 24.0 + 240.0;
    let details = column![
        text(profile.display_name.as_str()).size(size).font(theme::FONT_SEMIBOLD),
        muted(format!("@{} · {friends}", profile.handle)),
        Space::with_height(6),
        action,
    ]
    .spacing(6);
    if narrow {
        column![
            container(avatar(app, profile, theme::AVATAR_PROFILE)).center_x(Length::Fill),
            details.align_x(Alignment::Center).width(Length::Fill)
        ]
        .spacing(16)
        .into()
    } else {
        row![avatar(app, profile, theme::AVATAR_PROFILE), details]
            .spacing(24)
            .align_y(Alignment::Center)
            .into()
    }
}

/// La playlist pubblica di qualcuno: come una playlist locale, senza modifiche.
pub fn playlist_view(app: &App, width: f32) -> Element<'_, Message> {
    let playlist = match &app.social.playlist {
        None => return empty_block("Nessuna playlist aperta."),
        Some(Load::Idle | Load::Loading) => return skeleton_rows(8, theme::ROW_MEDIA, TableKind::Mixed),
        Some(Load::Failed(err)) => return error_block("la playlist", err),
        Some(Load::Ready(playlist)) => playlist,
    };
    let art = match playlist.tracks.first() {
        Some(first) => cover(&app.covers, art_of(first), crate::ui::components::HERO_COVER),
        None => {
            cover(&app.covers, Art::drawn(playlist.id, &playlist.name), crate::ui::components::HERO_COVER)
        }
    };
    let meta = format!("{} · {}", playlist.owner.display_name, count_label(playlist.tracks.len()));
    let owner = outlined_button(
        "Profilo",
        Some(Icon::Account),
        Some(Message::OpenProfile(playlist.owner.handle.clone())),
    );
    let mut page = column![
        crate::ui::components::hero(art, "Playlist pubblica", &playlist.name, &meta, width),
        crate::ui::components::play_bar(app, &playlist.tracks, vec![owner]),
    ]
    .spacing(24);
    page = if playlist.tracks.is_empty() {
        page.push(small("I brani di questa playlist non sono più nel catalogo."))
    } else {
        page.push(track_table(app, &playlist.tracks, TableKind::Mixed, width, Origin::Catalogue))
    };
    page.into()
}
