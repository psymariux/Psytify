//! Barra laterale: due pannelli arrotondati sul nero. In alto la navigazione; sotto la libreria
//! personale, con i brani che piacciono e le playlist locali. L'elemento attivo ha un fondo, il
//! segno verde a sinistra e l'icona piena.

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length};
use uuid::Uuid;

use super::components::{
    H2_LINE, NAV_LINE, SUB_LINE, TITLE_LINE, centered, count_label, hoverable, icon_button, line,
    outlined_button, small,
};
use super::cover::{Art, cover, cover_round};
use super::icons::{Icon, icon};
use super::theme;
use crate::app::{App, Dialog, MenuTarget, Message, Screen, SidebarFilter};

/// Spazio per il testo di una voce della libreria: pannello 252, padding 8 + 8 per lato, copertina 40
/// e uno spazio da 12.
const ENTRY_TEXT: f32 = theme::SIDEBAR_WIDTH - 32.0 - 40.0 - 12.0;

pub fn view(app: &App) -> Element<'_, Message> {
    let nav = container(
        column![
            nav_item(Icon::Home, Icon::HomeFill, "Home", Screen::Home, matches!(app.screen, Screen::Home), 0),
            nav_item(
                Icon::Search,
                Icon::SearchFill,
                "Cerca",
                Screen::Search,
                matches!(app.screen, Screen::Search),
                0,
            ),
            nav_item(
                Icon::Library,
                Icon::LibraryFill,
                "Catalogo",
                Screen::Library,
                matches!(app.screen, Screen::Library | Screen::Album(_) | Screen::Artist(_)),
                0,
            ),
            nav_item(
                Icon::Bell,
                Icon::BellFill,
                "Novità",
                Screen::News,
                matches!(app.screen, Screen::News),
                app.unread_news(),
            ),
            nav_item(
                Icon::People,
                Icon::PeopleFill,
                "Jam",
                Screen::Jam,
                matches!(app.screen, Screen::Jam),
                0
            ),
            nav_item(
                Icon::Settings,
                Icon::SettingsFill,
                "Impostazioni",
                Screen::Settings,
                matches!(app.screen, Screen::Settings),
                0,
            ),
        ]
        .spacing(2),
    )
    .padding(8)
    .width(Length::Fill)
    .style(theme::panel_box);

    column![nav, library(app)].spacing(theme::OUTER_GAP).width(theme::SIDEBAR_WIDTH).into()
}

/// La libreria personale: intestazione con «+», brani che piacciono e playlist.
fn library(app: &App) -> Element<'_, Message> {
    let header = row![
        icon(Icon::LibraryFill, 20.0, theme::MUTED),
        text("La tua libreria").size(theme::FS_BODY + 1.0).font(theme::FONT_SEMIBOLD).color(theme::MUTED),
        Space::with_width(Length::Fill),
        icon_button(
            Icon::Plus,
            "Crea una playlist",
            false,
            Some(Message::OpenDialog(Dialog::NewPlaylist {
                name: app.suggested_playlist_name(),
                seed: Vec::new()
            })),
        ),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .padding([0, 8]);

    let filter = app.sidebar_filter;
    let mut list = column![].spacing(2);
    if matches!(filter, SidebarFilter::All | SidebarFilter::Playlists) {
        list = list.push(entry(
            liked_cover(40.0),
            "Brani che ti piacciono",
            format!("Playlist · {}", count_label(app.liked.len())),
            app.screen == Screen::Liked,
            Message::Navigate(Screen::Liked),
            None,
        ));
        for playlist in &app.playlists {
            let active = app.screen == Screen::Playlist(playlist.id);
            list = list.push(entry(
                cover(&app.covers, Art::drawn(playlist.id, &playlist.name), 40.0),
                &playlist.name,
                format!("Playlist · {}", count_label(playlist.tracks.len())),
                active,
                Message::Navigate(Screen::Playlist(playlist.id)),
                Some((playlist.id, MenuTarget::Playlist(playlist.id))),
            ));
        }
        if app.playlists.is_empty() && filter == SidebarFilter::Playlists {
            list = list.push(container(first_playlist(app)).padding([12, 4]));
        }
    }
    if matches!(filter, SidebarFilter::All | SidebarFilter::Albums) {
        for album in &app.saved_albums {
            list = list.push(entry(
                cover(
                    &app.covers,
                    Art { id: album.id, title: &album.title, url: album.cover_url.as_deref() },
                    40.0,
                ),
                &album.title,
                format!("Album · {}", album.artist.name),
                app.screen == Screen::Album(album.id),
                Message::Navigate(Screen::Album(album.id)),
                Some((album.id, MenuTarget::Album(Box::new(album.clone())))),
            ));
        }
        if app.saved_albums.is_empty() && filter == SidebarFilter::Albums {
            list = list.push(hint("Salva un album con «Salva» o dal menu del clic destro: lo ritrovi qui."));
        }
    }
    if matches!(filter, SidebarFilter::All | SidebarFilter::Artists) {
        for artist in &app.followed {
            list = list.push(entry(
                cover_round(
                    &app.covers,
                    Art { id: artist.id, title: &artist.name, url: artist.cover_url.as_deref() },
                    40.0,
                ),
                &artist.name,
                "Artista".to_string(),
                app.screen == Screen::Artist(artist.id),
                Message::Navigate(Screen::Artist(artist.id)),
                Some((artist.id, MenuTarget::artist(artist.id, &artist.name, artist.cover_url.as_deref()))),
            ));
        }
        if app.followed.is_empty() && filter == SidebarFilter::Artists {
            list = list
                .push(hint("Segui un artista dal suo profilo o dal menu del clic destro: lo ritrovi qui."));
        }
    }

    let chips = row(SidebarFilter::CHIPS.iter().map(|&chip| {
        button(centered(text(chip.label()).size(theme::FS_SMALL + 1.0).font(theme::FONT_MEDIUM)))
            .height(30)
            .padding([0, 10])
            .style(theme::pill(chip == filter))
            .on_press(Message::SidebarFilter(chip))
            .into()
    }))
    .spacing(6)
    .padding([0, 8]);

    container(column![header, chips, scrollable(list).height(Length::Fill).style(theme::scroll)].spacing(12))
        .padding([14, 8])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::panel_box)
        .into()
}

/// Invito a creare la prima playlist.
fn first_playlist(app: &App) -> Element<'_, Message> {
    container(
        column![
            line("Crea una playlist", ENTRY_TEXT + 40.0, H2_LINE),
            small("Raccogli i brani che vuoi riascoltare. Si salvano su questo computer."),
            outlined_button(
                "Nuova playlist",
                None,
                Some(Message::OpenDialog(Dialog::NewPlaylist {
                    name: app.suggested_playlist_name(),
                    seed: Vec::new(),
                })),
            ),
        ]
        .spacing(10),
    )
    .padding(14)
    .width(Length::Fill)
    .style(theme::card)
    .into()
}

/// La copertina di «Brani che ti piacciono»: sfumatura rosa con il cuore.
pub fn liked_cover<'a>(side: f32) -> Element<'a, Message> {
    container(icon(Icon::HeartFill, side * 0.42, theme::TEXT))
        .center(side)
        .style(theme::cover_gradient(theme::LIKED_TINT, theme::NEGATIVE))
        .into()
}

fn nav_item<'a>(
    glyph: Icon,
    filled: Icon,
    label: &'a str,
    target: Screen,
    active: bool,
    badge: usize,
) -> Element<'a, Message> {
    let tint = if active { theme::TEXT } else { theme::NAV_IDLE };
    // Il segno verde a sinistra: acceso sulla voce attiva, invisibile sulle altre (stesso spazio).
    let mark = container(Space::new(3.0, 16.0))
        .style(theme::bar(if active { theme::ACCENT } else { iced::Color::TRANSPARENT }, 1.5));
    let mut content =
        row![mark, icon(if active { filled } else { glyph }, 20.0, tint), line(label, 150.0, NAV_LINE)]
            .spacing(12)
            .align_y(Alignment::Center);
    if badge > 0 {
        let shown = if badge > 99 { "99+".to_string() } else { badge.to_string() };
        content = content.push(Space::with_width(Length::Fill)).push(
            container(text(shown).size(theme::FS_SMALL).font(theme::FONT_SEMIBOLD).color(theme::ON_PRIMARY))
                .padding([1, 7])
                .style(theme::badge),
        );
    }
    button(centered(content))
        .height(theme::NAV_PITCH)
        .width(Length::Fill)
        .padding([0, 8])
        .style(theme::nav_item(active))
        .on_press(Message::Navigate(target))
        .into()
}

/// Una voce della libreria: copertina, nome, «Playlist · n brani». Il clic destro apre il menu.
fn entry<'a>(
    art: Element<'a, Message>,
    title: &str,
    subtitle: String,
    active: bool,
    press: Message,
    menu: Option<(Uuid, MenuTarget)>,
) -> Element<'a, Message> {
    let item = button(centered(
        row![
            art,
            column![
                line(title, ENTRY_TEXT, if active { super::components::PLAYING_LINE } else { TITLE_LINE }),
                line(&subtitle, ENTRY_TEXT, SUB_LINE),
            ]
            .spacing(3)
            .width(Length::Fill),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    ))
    .height(56)
    .width(Length::Fill)
    .padding([0, 8])
    .style(theme::row(active))
    .on_press(press);
    match menu {
        Some((id, target)) => hoverable(id, false, Some(target), item.into()),
        None => item.into(),
    }
}

/// Una nota sotto una lista vuota della libreria.
fn hint<'a>(message: &'a str) -> Element<'a, Message> {
    container(small(message)).padding([12, 8]).into()
}
