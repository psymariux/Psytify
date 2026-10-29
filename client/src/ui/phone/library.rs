//! La Libreria del telefono: ciò che è tuo in una lista sola, come nelle app da telefono. In cima
//! il titolo con «Nuova playlist» e i filtri; poi i brani che piacciono, il catalogo del server, le
//! playlist, gli album salvati e gli artisti seguiti. Sul desktop la stessa lista sta nella barra
//! laterale.

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};
use uuid::Uuid;

use super::{settings_icon, title_row, touch_icon};
use crate::app::{App, Dialog, MenuTarget, Message, Screen, SidebarFilter};
use crate::ui::components::{
    PLAYING_LINE, SUB_LINE, TITLE_LINE, centered, count_label, hoverable, line, small,
};
use crate::ui::cover::{Art, cover, cover_round};
use crate::ui::icons::{Icon, icon};
use crate::ui::sidebar::liked_cover;
use crate::ui::theme;

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let new_playlist =
        Message::OpenDialog(Dialog::NewPlaylist { name: app.suggested_playlist_name(), seed: Vec::new() });
    let header = title_row(
        "Libreria",
        vec![touch_icon(Icon::Plus, theme::TEXT, new_playlist), super::friends_icon(app), settings_icon()],
    );
    let text_width = width - theme::PHONE_ROW_COVER - 14.0 - 16.0;
    let filter = app.sidebar_filter;
    let mut list = column![];

    if matches!(filter, SidebarFilter::All | SidebarFilter::Playlists) {
        list = list.push(entry(
            liked_cover(theme::PHONE_ROW_COVER),
            "Brani che ti piacciono",
            &format!("Playlist · {}", count_label(app.liked.len())),
            text_width,
            app.screen == Screen::Liked,
            Message::Navigate(Screen::Liked),
            None,
        ));
    }
    if filter == SidebarFilter::All {
        let art = container(icon(Icon::LibraryFill, 26.0, theme::ACCENT))
            .center(theme::PHONE_ROW_COVER)
            .style(theme::cover(theme::RAISED, theme::RADIUS_ROW));
        list = list.push(entry(
            art.into(),
            "Il catalogo",
            "Album, artisti e brani del server",
            text_width,
            false,
            Message::Navigate(Screen::Library),
            None,
        ));
    }
    if matches!(filter, SidebarFilter::All | SidebarFilter::Playlists) {
        for playlist in &app.playlists {
            list = list.push(entry(
                cover(&app.covers, Art::drawn(playlist.id, &playlist.name), theme::PHONE_ROW_COVER),
                &playlist.name,
                &format!("Playlist · {}", count_label(playlist.tracks.len())),
                text_width,
                false,
                Message::Navigate(Screen::Playlist(playlist.id)),
                Some((playlist.id, MenuTarget::Playlist(playlist.id))),
            ));
        }
    }
    if matches!(filter, SidebarFilter::All | SidebarFilter::Albums) {
        for album in &app.saved_albums {
            list = list.push(entry(
                cover(
                    &app.covers,
                    Art { id: album.id, title: &album.title, url: album.cover_url.as_deref() },
                    theme::PHONE_ROW_COVER,
                ),
                &album.title,
                &format!("Album · {}", album.artist.name),
                text_width,
                false,
                Message::Navigate(Screen::Album(album.id)),
                Some((album.id, MenuTarget::Album(Box::new(album.clone())))),
            ));
        }
    }
    if matches!(filter, SidebarFilter::All | SidebarFilter::Artists) {
        for artist in &app.followed {
            list = list.push(entry(
                cover_round(
                    &app.covers,
                    Art { id: artist.id, title: &artist.name, url: artist.cover_url.as_deref() },
                    theme::PHONE_ROW_COVER,
                ),
                &artist.name,
                "Artista",
                text_width,
                false,
                Message::Navigate(Screen::Artist(artist.id)),
                Some((artist.id, MenuTarget::artist(artist.id, &artist.name, artist.cover_url.as_deref()))),
            ));
        }
    }
    if let Some(hint) = empty_hint(app, filter) {
        list = list.push(container(small(hint)).padding([16, 0]));
    }

    column![header, chips(filter), list].spacing(16).width(Length::Fill).into()
}

/// I filtri: un tocco sul filtro acceso torna a «Tutto».
fn chips<'a>(filter: SidebarFilter) -> Element<'a, Message> {
    row(SidebarFilter::CHIPS.iter().map(|&chip| {
        button(centered(text(chip.label()).size(theme::FS_BODY).font(theme::FONT_MEDIUM)))
            .height(36)
            .padding([0, 16])
            .style(theme::pill(chip == filter))
            .on_press(Message::SidebarFilter(chip))
            .into()
    }))
    .spacing(8)
    .into()
}

/// Una lista filtrata e vuota dice come riempirla.
fn empty_hint(app: &App, filter: SidebarFilter) -> Option<&'static str> {
    match filter {
        SidebarFilter::Playlists if app.playlists.is_empty() => {
            Some("Crea una playlist con + in alto, o da «Aggiungi a playlist» nel menu di un brano.")
        }
        SidebarFilter::Albums if app.saved_albums.is_empty() => {
            Some("Salva un album dalla sua pagina o dalla pressione lunga: lo ritrovi qui.")
        }
        SidebarFilter::Artists if app.followed.is_empty() => {
            Some("Segui un artista dal suo profilo: lo ritrovi qui, e le sue uscite in Novità.")
        }
        _ => None,
    }
}

/// Una voce: copertina, nome e «Playlist · n brani». La pressione lunga apre il menu.
fn entry<'a>(
    art: Element<'a, Message>,
    title: &str,
    subtitle: &str,
    width: f32,
    active: bool,
    press: Message,
    menu: Option<(Uuid, MenuTarget)>,
) -> Element<'a, Message> {
    let item = button(centered(
        row![
            art,
            column![
                line(title, width, if active { PLAYING_LINE } else { TITLE_LINE }),
                line(subtitle, width, SUB_LINE)
            ]
            .spacing(4)
            .width(Length::Fill),
            Space::with_width(0),
        ]
        .spacing(14)
        .align_y(Alignment::Center),
    ))
    .height(theme::PHONE_ROW)
    .width(Length::Fill)
    .padding(0)
    .style(theme::bare)
    .on_press(press);
    match menu {
        Some((id, target)) => hoverable(id, false, Some(target), item.into()),
        None => item.into(),
    }
}
