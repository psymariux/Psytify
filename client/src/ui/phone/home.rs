//! La Home del telefono. In cima il marchio e le impostazioni, poi le scorciatoie in due colonne
//! (ciò che piace e gli ultimi ascolti), l'uscita in evidenza e gli scaffali. Le file di copertine
//! partono dal margine e scorrono fino al bordo dello schermo: quella tagliata invita a scorrere.

use std::sync::Arc;

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length, Padding};

use super::{settings_icon, touch_icon};
use crate::api::Track;
use crate::app::{App, LibraryFilter, Load, Message, Screen};
use crate::config::{FollowedArtist, SavedAlbum};
use crate::ui::components::{
    Line, MUTED_LINE, album_tile_row, empty_block, error_block, faded, line, scroll_row, skeleton,
    skeleton_tiles, text_button, tile_row, wrapped,
};
use crate::ui::cover::cover;
use crate::ui::icons::{Icon, icon, mark};
use crate::ui::shelf::{self, release_art, release_play};
use crate::ui::{motion, pages, theme};

/// Copertina dell'uscita in evidenza.
const FEATURED_COVER: f32 = 128.0;
/// Il Play dell'uscita in evidenza.
const FEATURED_PLAY: f32 = 52.0;
/// Il titolo dell'uscita in evidenza: due righe al massimo.
const FEATURED_TITLE: Line = Line { size: 22.0, font: theme::FONT_SEMIBOLD, color: Some(theme::TEXT) };
/// Le tessere hanno 8px di fondo attorno: il margine della fila li toglie, così la prima
/// copertina sta sul margine della pagina.
const ROW_INSET: f32 = theme::PHONE_GUTTER - 8.0;
/// Brani nella classifica degli ascolti.
const TOP: usize = 5;

/// `width` è la larghezza dello schermo: i margini li mette ogni sezione.
pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let inner = width - 2.0 * theme::PHONE_GUTTER;
    let feed = &app.home.feed;
    let mut page = column![padded(header(app))].spacing(28);

    if let Some(grid) = pages::home::shortcuts(app, inner) {
        page = page.push(padded(grid));
    }
    page = page.push(padded(match featured(app, inner) {
        Some(card) => card,
        None => match &app.home.recent {
            Load::Failed(err) => error_block("le novità", err),
            _ => skeleton(Length::Fill, FEATURED_COVER + 2.0 * 18.0 + FEATURED_PLAY),
        },
    }));

    page = page.push(match &app.home.recent {
        Load::Idle | Load::Loading => shelf_section("Aggiunti di recente", None, skeleton_tiles(3)),
        Load::Failed(_) => Space::with_height(0).into(),
        Load::Ready(_) if feed.releases.is_empty() => {
            padded(empty_block("Il catalogo non ha ancora brani pubblicati."))
        }
        Load::Ready(_) => {
            let latest = &feed.releases[..feed.releases.len().min(16)];
            shelf_section(
                "Aggiunti di recente",
                Some(Message::Navigate(Screen::News)),
                shelf::release_row(app, latest, theme::PHONE_COVER, None),
            )
        }
    });
    if !feed.news.is_empty() {
        let latest = &feed.news[..feed.news.len().min(12)];
        page = page.push(shelf_section(
            "Dai tuoi artisti",
            Some(Message::Navigate(Screen::News)),
            shelf::release_row(app, latest, theme::PHONE_COVER, Some(&app.news_seen)),
        ));
    }
    if !feed.mixes.is_empty() {
        let cards = feed
            .mixes
            .iter()
            .enumerate()
            .map(|(index, mix)| shelf::mix_card(app, index, mix, theme::PHONE_COVER));
        page = page.push(shelf_section("Mix per te", None, scroll_row(cards.collect())));
    }
    if !feed.artists.is_empty() {
        let title = if feed.artists_are_yours { "I tuoi artisti" } else { "Artisti nel catalogo" };
        let cards = feed.artists.iter().map(|card| shelf::artist_tile(app, card, theme::PHONE_ARTIST));
        page = page.push(shelf_section(title, None, scroll_row(cards.collect())));
    }
    if !app.recent.is_empty() {
        let recent: Arc<[Track]> = Arc::from(app.recent.as_slice());
        page = page.push(shelf_section("Ascoltati di recente", None, tile_row(app, &recent)));
    }
    if !feed.top.is_empty() {
        let top = &feed.top[..feed.top.len().min(TOP)];
        page = page.push(
            column![heading("Più ascoltati da te", None), padded(shelf::chart(app, top, inner))].spacing(8),
        );
    }
    match &app.home.albums {
        Load::Ready(albums) if !albums.is_empty() => {
            page = page.push(shelf_section(
                "Album",
                Some(Message::OpenLibrary(LibraryFilter::Albums)),
                album_tile_row(app, albums),
            ));
        }
        Load::Idle | Load::Loading => page = page.push(shelf_section("Album", None, skeleton_tiles(3))),
        Load::Ready(_) | Load::Failed(_) => {}
    }
    if let Load::Ready(tracks) = &app.home.alphabetical
        && !tracks.is_empty()
    {
        page = page.push(shelf_section(
            "Dalla A alla Z",
            Some(Message::Navigate(Screen::Library)),
            tile_row(app, tracks),
        ));
    }
    page.into()
}

/// Il marchio a sinistra, gli amici e le impostazioni a destra.
fn header(app: &App) -> Element<'_, Message> {
    row![
        mark(26.0),
        text("Psytify").size(theme::FS_PHONE_TITLE - 4.0).font(theme::FONT_SEMIBOLD),
        Space::with_width(Length::Fill),
        super::friends_icon(app),
        settings_icon(),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .height(theme::TOUCH_TARGET)
    .into()
}

fn padded<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    container(content).padding([0.0, theme::PHONE_GUTTER]).width(Length::Fill).into()
}

/// Il titolo di uno scaffale e, a destra, «Vedi tutto» se porta a una pagina intera.
fn heading<'a>(title: &'a str, more: Option<Message>) -> Element<'a, Message> {
    let mut bar = row![text(title).size(theme::FS_TITLE).font(theme::FONT_SEMIBOLD).width(Length::Fill)]
        .align_y(Alignment::Center);
    if let Some(message) = more {
        bar = bar.push(text_button("Vedi tutto", message));
    }
    padded(bar.into())
}

/// Uno scaffale: il titolo fra i margini e la fila di copertine da bordo a bordo.
fn shelf_section<'a>(
    title: &'a str,
    more: Option<Message>,
    tiles: Element<'a, Message>,
) -> Element<'a, Message> {
    column![
        heading(title, more),
        container(tiles).padding(Padding { left: ROW_INSET, ..Padding::ZERO }).width(Length::Fill),
    ]
    .spacing(4)
    .into()
}

/// L'uscita in evidenza, sulla tinta della sua copertina: copertina e titolo, sotto il Play, Salva
/// e Segui e i puntini del carosello.
fn featured(app: &App, width: f32) -> Option<Element<'_, Message>> {
    let releases = &app.home.feed.hero;
    let release = releases.get(app.home.hero.index)?;
    let shown = app.home.hero.since.map_or(1.0, |since| motion::appear(since, theme::MOTION_PANEL));
    let tint = app.covers.tint_of(release_art(release));
    let pad = 18.0;
    let text_width = width - 2.0 * pad - FEATURED_COVER - 16.0;

    let mut meta = vec![release.kind_label().to_string(), release.artist.name.clone()];
    if release.is_album {
        meta.push(crate::ui::components::count_label(release.tracks.len().max(1)));
    }
    let open =
        if release.is_album { Message::Navigate(Screen::Album(release.id)) } else { release_play(release) };
    let top = button(
        row![
            container(cover(&app.covers, release_art(release), FEATURED_COVER)).style(theme::lifted),
            column![
                wrapped(&release.title, text_width, faded(FEATURED_TITLE, shown), 2),
                line(&meta.join(" · "), text_width, faded(MUTED_LINE, shown)),
            ]
            .spacing(6)
            .width(Length::Fill),
        ]
        .spacing(16)
        .align_y(Alignment::Center),
    )
    .padding(0)
    .style(theme::bare)
    .on_press(open);

    let play = button(container(icon(Icon::Play, 22.0, theme::ON_PRIMARY)).center(Length::Fill))
        .width(FEATURED_PLAY)
        .height(FEATURED_PLAY)
        .padding(0)
        .style(theme::play_fab)
        .on_press(release_play(release));
    let mut actions = row![play].spacing(4).align_y(Alignment::Center);
    if release.is_album {
        let saved = app.has_saved(release.id);
        actions = actions.push(touch_icon(
            if saved { Icon::Check } else { Icon::Plus },
            theme::TEXT,
            Message::ToggleSaveAlbum(SavedAlbum {
                id: release.id,
                title: release.title.clone(),
                artist: release.artist.clone(),
                cover_url: release.cover_url.clone(),
            }),
        ));
    }
    let following = app.is_following(release.artist.id);
    actions = actions.push(touch_icon(
        if following { Icon::Check } else { Icon::UserPlus },
        theme::TEXT,
        Message::ToggleFollow(FollowedArtist {
            id: release.artist.id,
            name: release.artist.name.clone(),
            cover_url: release.cover_url.clone(),
        }),
    ));
    actions = actions.push(Space::with_width(Length::Fill));
    if releases.len() > 1 {
        actions = actions.push(dots(releases.len(), app.home.hero.index));
    }
    Some(
        container(column![top, actions].spacing(16))
            .padding(pad)
            .width(Length::Fill)
            .style(theme::hero_card(tint))
            .into(),
    )
}

/// I puntini del carosello: quello acceso è una barretta lunga. Ognuno è largo quanto un tocco.
fn dots<'a>(count: usize, active: usize) -> Element<'a, Message> {
    let items = (0..count).map(|n| {
        let on = n == active;
        let (width, color) = if on { (20.0, theme::TEXT) } else { (6.0, theme::FAINT) };
        button(container(Space::new(Length::Fixed(width), Length::Fixed(6.0))).style(theme::bar(color, 3.0)))
            .padding([14, 3])
            .style(theme::bare)
            .on_press(Message::HeroGo(n))
            .into()
    });
    row(items).align_y(Alignment::Center).into()
}
