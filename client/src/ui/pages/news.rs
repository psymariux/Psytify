//! Novità: le uscite degli artisti che si seguono, come «Novità» di Spotify e «Aggiunti di
//! recente» di Apple Music. Il catalogo non sa chi segue chi: l'elenco si ricava dai brani recenti
//! (`created_at`) e da ciò che questo computer ricorda.

use iced::widget::{column, container, row};
use iced::{Alignment, Element, Length};

use crate::app::{App, Load, Message, Screen};
use crate::app::{MenuTarget, TrackSource};
use crate::feed::{Release, date_label};
use crate::ui::components::{
    META_LINE, MUTED_LINE, PLAYING_LINE, TITLE_LINE, TableKind, centered, empty_state, error_block, h1, h2,
    hoverable, icon_button, line, muted, skeleton_rows,
};
use crate::ui::cover::cover;
use crate::ui::icons::Icon;
use crate::ui::shelf;
use crate::ui::theme;

/// Altezza di una riga del feed e lato della sua copertina.
const ROW: f32 = 88.0;
const ROW_COVER: f32 = 68.0;

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let unread = app.unread_news();
    let subline = match (app.followed.len(), unread) {
        (0, _) => "Segui gli artisti per ritrovare qui le loro uscite".to_string(),
        (_, 0) => "Sei in pari con le uscite degli artisti che segui".to_string(),
        (_, 1) => "1 uscita da ascoltare".to_string(),
        (_, n) => format!("{n} uscite da ascoltare"),
    };
    let header = column![h1("Novità"), muted(subline)].spacing(6);

    let body: Element<'_, Message> = match &app.home.recent {
        Load::Idle | Load::Loading => skeleton_rows(6, ROW, TableKind::Mixed),
        Load::Failed(err) => error_block("le novità", err),
        Load::Ready(_) if app.followed.is_empty() => suggestions(app),
        Load::Ready(_) => feed_list(app, width),
    };
    column![header, body].spacing(28).into()
}

/// Nessun artista seguito: si spiega e si propongono quelli del catalogo.
fn suggestions(app: &App) -> Element<'_, Message> {
    let mut content = column![empty_state(
        "Non segui ancora nessuno",
        "Quando segui un artista, le sue nuove uscite arrivano qui e la Home le mette in primo piano.",
        None,
    )]
    .spacing(28);
    if !app.home.feed.artists.is_empty() {
        content = content
            .push(column![h2("Da cui iniziare"), shelf::artist_row(app, &app.home.feed.artists)].spacing(12));
    }
    content.into()
}

fn feed_list(app: &App, width: f32) -> Element<'_, Message> {
    let feed = &app.home.feed;
    let followed: Vec<_> = app
        .followed
        .iter()
        .map(|a| crate::feed::ArtistCard {
            artist: crate::api::models::ArtistRef { id: a.id, name: a.name.clone() },
            cover_url: a.cover_url.clone(),
            tracks: 0,
        })
        .collect();
    let mut page =
        column![column![h2("Artisti che segui"), shelf::artist_row(app, &followed)].spacing(12)].spacing(28);

    if feed.news.is_empty() {
        return page
            .push(empty_state(
                "Ancora nessuna uscita",
                "Gli artisti che segui non hanno brani recenti nel catalogo. Appena ne esce uno, lo trovi qui.",
                Some(("Vai alla Home", Message::Navigate(Screen::Home))),
            ))
            .into();
    }
    let (fresh, earlier): (Vec<&Release>, Vec<&Release>) =
        feed.news.iter().partition(|r| r.created_at.as_str() > app.news_seen.as_str());
    if !fresh.is_empty() {
        page = page.push(group(app, "Da ascoltare", &fresh, true, width));
    }
    if !earlier.is_empty() {
        page = page.push(group(app, "Uscite precedenti", &earlier, false, width));
    }
    page.into()
}

fn group<'a>(
    app: &App,
    title: &'a str,
    releases: &[&Release],
    new: bool,
    width: f32,
) -> Element<'a, Message> {
    let entered = app.page_motion.elapsed();
    let rows =
        releases.iter().enumerate().map(|(index, release)| item(app, release, new, width, entered, index));
    column![h2(title), column(rows)].spacing(12).into()
}

fn item<'a>(
    app: &App,
    release: &Release,
    new: bool,
    width: f32,
    entered: std::time::Duration,
    index: usize,
) -> Element<'a, Message> {
    let progress = crate::ui::motion::stagger(entered, index);
    let text_width = width - 24.0 - ROW_COVER - 2.0 * 16.0 - 34.0 - 90.0;
    let when = date_label(&release.created_at).unwrap_or_default();
    let detail = format!("{} · {} · {}", release.kind_label(), release.artist.name, when);
    let playing = release.tracks.iter().any(|t| app.is_current(t.id));
    let mut title = row![
        container(line(
            &release.title,
            text_width,
            crate::ui::components::faded(if playing { PLAYING_LINE } else { TITLE_LINE }, progress),
        ))
        .width(Length::Shrink)
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if new {
        title = title.push(
            container(
                iced::widget::text("Nuovo")
                    .size(theme::FS_SMALL)
                    .font(theme::FONT_SEMIBOLD)
                    .color(theme::ON_PRIMARY),
            )
            .padding([2, 8])
            .style(theme::badge),
        );
    }
    let open = if release.is_album {
        Message::Navigate(Screen::Album(release.id))
    } else {
        shelf::release_play(release)
    };
    let menu = if release.is_album {
        Some(MenuTarget::Album(Box::new(crate::config::SavedAlbum {
            id: release.id,
            title: release.title.clone(),
            artist: release.artist.clone(),
            cover_url: release.cover_url.clone(),
        })))
    } else {
        release.tracks.first().map(|t| MenuTarget::track(t, TrackSource::Catalogue))
    };
    let content = button_row(
        row![
            cover(&app.covers, shelf::release_art(release), ROW_COVER),
            column![
                title,
                line(&detail, text_width, crate::ui::components::faded(MUTED_LINE, progress)),
                line(
                    &crate::ui::components::count_label(release.tracks.len().max(1)),
                    text_width,
                    crate::ui::components::faded(META_LINE, progress)
                ),
            ]
            .spacing(4)
            .width(Length::Fill),
            icon_button(Icon::Play, "Riproduci", false, Some(shelf::release_play(release))),
        ]
        .spacing(16)
        .align_y(Alignment::Center)
        .into(),
        open,
        playing,
    );
    hoverable(release.id, false, menu, content)
}

fn button_row<'a>(content: Element<'a, Message>, on_press: Message, playing: bool) -> Element<'a, Message> {
    iced::widget::button(centered(content))
        .height(ROW)
        .width(Length::Fill)
        .padding([0, 12])
        .style(theme::row(playing))
        .on_press(on_press)
        .into()
}
