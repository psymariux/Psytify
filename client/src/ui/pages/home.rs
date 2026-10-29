//! Home: cosa ascoltare adesso, come la pagina iniziale di Spotify e di Apple Music. In cima
//! l'uscita in evidenza (carosello), poi le scorciatoie, le nuove uscite con la copertina grande, le
//! novità dei propri artisti, gli artisti in cerchio, i mix, la classifica di ciò che si ascolta e
//! gli album.

use std::sync::Arc;

use iced::widget::{Space, button, column, container, row};
use iced::{Alignment, Element, Length};
use uuid::Uuid;

use crate::api::Track;
use crate::app::{App, LibraryFilter, Load, MenuTarget, Message, Screen, TrackSource};
use crate::player::PlaybackState;
use crate::ui::components::{
    H2_LINE, MUTED_LINE, album_tile_row, art_of, empty_block, error_block, line, loaded, primary_button,
    section, shortcut, skeleton, skeleton_tiles, small, text_button, tile_row,
};
use crate::ui::cover::{Art, cover};
use crate::ui::icons::{Icon, icon};
use crate::ui::shelf;
use crate::ui::sidebar::liked_cover;
use crate::ui::theme;

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let feed = &app.home.feed;
    let mut page = column![].spacing(36);

    page = page.push(match shelf::hero(app, width) {
        Some(hero) => hero,
        None => loaded(
            &app.home.recent,
            "le novità",
            || skeleton(Length::Fill, 292.0),
            |_| empty_block("Il catalogo non ha ancora brani pubblicati."),
        ),
    });
    if let Some(grid) = shortcuts(app, width) {
        page = page.push(grid);
    }
    if let Some(card) = resume_card(app, width) {
        page = page.push(card);
    }

    let more = || Some(text_button("Vedi tutte", Message::Navigate(Screen::News)));
    page = page.push(match &app.home.recent {
        Load::Idle | Load::Loading => section("Aggiunti di recente", None, skeleton_tiles(6)),
        Load::Failed(err) => error_block("i brani aggiunti di recente", err),
        Load::Ready(_) if feed.releases.is_empty() => {
            section("Aggiunti di recente", None, empty_block("Il catalogo è ancora vuoto."))
        }
        Load::Ready(_) => {
            let latest = &feed.releases[..feed.releases.len().min(16)];
            section("Aggiunti di recente", more(), shelf::release_row(app, latest, shelf::BIG_COVER, None))
        }
    });
    if !feed.news.is_empty() {
        let unread = app.unread_news();
        let title =
            if unread > 0 { "Novità dai tuoi artisti · da ascoltare" } else { "Novità dai tuoi artisti" };
        let latest = &feed.news[..feed.news.len().min(12)];
        page = page.push(section(
            title,
            Some(text_button("Apri Novità", Message::Navigate(Screen::News))),
            shelf::release_row(app, latest, shelf::BIG_COVER, Some(&app.news_seen)),
        ));
    }
    if !feed.artists.is_empty() {
        let title = if feed.artists_are_yours { "I tuoi artisti" } else { "Artisti nel catalogo" };
        page = page.push(section(title, None, shelf::artist_row(app, &feed.artists)));
    }
    if !feed.mixes.is_empty() {
        page = page.push(section("Mix per te", None, shelf::mix_row(app, &feed.mixes)));
    }
    if !feed.top.is_empty() {
        let top = &feed.top[..feed.top.len().min(10)];
        page = page.push(section("Più ascoltati da te", None, shelf::chart(app, top, width)));
    }
    if !app.recent.is_empty() {
        let recent: Arc<[Track]> = Arc::from(app.recent.as_slice());
        page = page.push(section("Ascoltati di recente", None, tile_row(app, &recent)));
    }
    page = page.push(section(
        "Album",
        Some(text_button("Vedi tutti gli album", Message::OpenLibrary(LibraryFilter::Albums))),
        loaded(
            &app.home.albums,
            "gli album",
            || skeleton_tiles(6),
            |albums| {
                if albums.is_empty() {
                    empty_block("Nessun album nel catalogo: per ora solo singoli.")
                } else {
                    album_tile_row(app, albums)
                }
            },
        ),
    ));
    if let Some(card) = shelf::listening_card(app, width) {
        page = page.push(card);
    }
    page.push(section(
        "Dalla A alla Z",
        Some(text_button("Vedi tutto il catalogo", Message::Navigate(Screen::Library))),
        loaded(
            &app.home.alphabetical,
            "il catalogo",
            || skeleton_tiles(6),
            |tracks| {
                if tracks.is_empty() {
                    empty_block("Nessun brano pubblicato.")
                } else {
                    tile_row(app, tracks)
                }
            },
        ),
    ))
    .into()
}

/// Le scorciatoie in cima: i brani che piacciono, le playlist, poi gli ultimi ascolti. Due colonne,
/// al massimo sei.
fn shortcuts(app: &App, width: f32) -> Option<Element<'_, Message>> {
    let cell = (width - theme::GRID_GAP) / 2.0;
    let mut cards: Vec<Element<'_, Message>> = Vec::new();
    if !app.liked.is_empty() {
        cards.push(shortcut(
            app,
            Uuid::nil(),
            liked_cover(56.0),
            "Brani che ti piacciono",
            cell,
            Message::Navigate(Screen::Liked),
            None,
        ));
    }
    for playlist in app.playlists.iter().take(3) {
        cards.push(shortcut(
            app,
            playlist.id,
            cover(&app.covers, Art::drawn(playlist.id, &playlist.name), 56.0),
            &playlist.name,
            cell,
            Message::Navigate(Screen::Playlist(playlist.id)),
            Some(MenuTarget::Playlist(playlist.id)),
        ));
    }
    for track in &app.recent {
        if cards.len() >= 6 {
            break;
        }
        cards.push(shortcut(
            app,
            track.id,
            cover(&app.covers, art_of(track), 56.0),
            &track.title,
            cell,
            Message::PlayTracks(Arc::from([track.clone()]), 0),
            Some(MenuTarget::track(track, TrackSource::Catalogue)),
        ));
    }
    if cards.is_empty() {
        return None;
    }
    let mut rows = column![].spacing(theme::GRID_GAP);
    let mut cards = cards.into_iter();
    while let Some(first) = cards.next() {
        let second = cards.next().unwrap_or_else(|| Space::with_width(Length::Fill).into());
        rows = rows.push(row![first, second].spacing(theme::GRID_GAP));
    }
    Some(rows.into())
}

/// Sotto questa larghezza la card «Continua ad ascoltare» usa il pulsante tondo senza scritta.
const NARROW_CARD: f32 = 520.0;

/// Il brano rimasto in coda dall'ultima sessione, con l'azione per riprenderlo.
fn resume_card(app: &App, width: f32) -> Option<Element<'_, Message>> {
    let track = app.queue.current()?;
    let (label, glyph) = match app.playback.state {
        PlaybackState::Playing => ("Pausa", Icon::Pause),
        PlaybackState::Paused => ("Riprendi", Icon::Play),
        _ => ("Riproduci", Icon::Play),
    };
    let upcoming = app.queue.upcoming().count();
    let context = if upcoming == 0 {
        "ultimo brano in coda".to_string()
    } else {
        format!("poi altri {upcoming} in coda")
    };
    // Sul telefono il pulsante perde la scritta e diventa un tondo, per lasciare spazio al titolo.
    let narrow = width < NARROW_CARD;
    let (action, action_width): (Element<'_, Message>, f32) = if narrow {
        let round = button(container(icon(glyph, 20.0, theme::ON_PRIMARY)).center(Length::Fill))
            .width(theme::TOUCH_TARGET)
            .height(theme::TOUCH_TARGET)
            .padding(0)
            .style(theme::play_fab)
            .on_press(Message::TogglePlay);
        (round.into(), theme::TOUCH_TARGET)
    } else {
        (primary_button(label, Some(glyph), Message::TogglePlay), 140.0)
    };
    // Card: padding 14 per lato, copertina 62, due spazi da 16, poi il pulsante.
    let text_width = width - 28.0 - 62.0 - 32.0 - action_width;
    Some(
        container(
            row![
                cover(&app.covers, art_of(track), 62.0),
                column![
                    small("Continua ad ascoltare"),
                    line(&track.title, text_width, H2_LINE),
                    line(&format!("{} · {context}", track.artist.name), text_width, MUTED_LINE),
                ]
                .spacing(4)
                .width(Length::Fill),
                action,
            ]
            .spacing(16)
            .align_y(Alignment::Center),
        )
        .padding(14)
        .width(Length::Fill)
        .style(theme::card)
        .into(),
    )
}
