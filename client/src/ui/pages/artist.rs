//! Pagina artista come in Spotify: un banner con la sua foto in cerchio e le azioni (Riproduci, Segui,
//! Radio), i brani popolari dal catalogo ufficiale, tutta la discografia filtrabile con le copertine
//! grandi (un clic apre l'album), poi ciò che è già nel catalogo del server, altri artisti e i dati.
//! La pagina arriva dalla cache dove si può (`app::artists`): aprirla è immediato.

use std::sync::Arc;

use iced::widget::{column, container, row, text};
use iced::{Alignment, Element, Length};
use uuid::Uuid;

use crate::api::{ArtistPage, SourcePlaylist, Track};
use crate::app::{App, ArtistExtras, Discography, Load, Message};
use crate::config::{FollowedArtist, PlayStat};
use crate::feed::{self, ArtistCard};
use crate::ui::components::{
    HERO_LINE, Origin, SUB_LINE, TableKind, count_label, empty_block, error_block, h2, line, outlined_button,
    outlined_button_owned, outlined_icon, pill, play_bar, primary_button, section, skeleton_rows, small,
    text_button, total_label, track_table,
};
use crate::ui::cover::{Art, cover_round};
use crate::ui::icons::Icon;
use crate::ui::pages::album::credits;
use crate::ui::pages::search;
use crate::ui::theme;
use crate::ui::{shelf, side_panel};

/// Lato dell'immagine tonda dell'artista nel banner.
const PORTRAIT: f32 = 188.0;
/// Brani nella sezione «Popolari»; «Mostra altri» li mostra tutti.
const POPULAR: usize = 5;
/// Altri artisti proposti in fondo.
const RELATED: usize = 12;

pub fn view(app: &App, id: Uuid, width: f32) -> Element<'_, Message> {
    match &app.artist {
        Some((current, Load::Ready(page))) if *current == id => loaded(app, page, width),
        Some((current, Load::Failed(err))) if *current == id => error_block("l'artista", err),
        _ => skeleton_rows(8, theme::ROW_PLAIN, TableKind::Mixed),
    }
}

/// L'immagine dell'artista è la sua foto ufficiale; senza, la copertina del suo primo album con
/// un'immagine, o di un suo brano.
pub fn portrait_url(page: &ArtistPage) -> Option<&str> {
    page.image_url
        .as_deref()
        .or_else(|| page.albums.iter().find_map(|a| a.cover_url.as_deref()))
        .or_else(|| page.tracks.iter().find_map(|t| t.cover_url.as_deref()))
}

fn loaded<'a>(app: &'a App, page: &'a ArtistPage, width: f32) -> Element<'a, Message> {
    let tracks: Arc<[Track]> = Arc::from(page.tracks.as_slice());
    let extras = match app.external.artists.get(&page.id) {
        Some(Load::Ready(extras)) => Some(extras),
        _ => None,
    };
    let mut content = column![banner(app, page, extras, &tracks, width)].spacing(32).width(Length::Fill);
    match app.external.artists.get(&page.id) {
        Some(Load::Ready(extras)) => {
            if let Some(popular) = popular(app, page.id, extras, width) {
                content = content.push(popular);
            }
            if let Some(discography) = discography(app, extras, width) {
                content = content.push(discography);
            }
            if extras.top.is_empty() && extras.albums.is_empty() && tracks.is_empty() {
                content = content.push(empty_block("Il catalogo non ha brani di questo artista."));
            }
        }
        Some(Load::Failed(err)) if tracks.is_empty() => content = content.push(error_block("i brani", err)),
        Some(Load::Idle | Load::Loading) | None => {
            content = content.push(skeleton_rows(6, theme::ROW_MEDIA, TableKind::Mixed));
        }
        Some(Load::Failed(_)) => {}
    }
    if !tracks.is_empty() {
        let title = if extras.is_some() { "Già nel catalogo del server" } else { "Brani" };
        content = content.push(
            column![h2(title), track_table(app, &tracks, TableKind::Mixed, width, Origin::Catalogue)]
                .spacing(12),
        );
    }
    if let Some(related) = related(app, page.id) {
        content = content.push(related);
    }
    if !tracks.is_empty() {
        content = content.push(about(app, page, &tracks));
        if let Some(credits) = credits(&tracks) {
            content = content.push(credits);
        }
    }
    content.into()
}

/// I brani più ascoltati dell'artista dal catalogo ufficiale: cinque, o tutti con «Mostra altri».
/// Un play suona la lista da quel brano in poi.
fn popular<'a>(app: &'a App, id: Uuid, extras: &'a ArtistExtras, width: f32) -> Option<Element<'a, Message>> {
    if extras.top.is_empty() {
        return None;
    }
    let shown = if app.artists.more_popular { extras.top.len() } else { POPULAR.min(extras.top.len()) };
    let text_width = width - 2.0 * 12.0 - 50.0 - 16.0 * 2.0 - 40.0;
    let rows = column(extras.top.iter().take(shown).enumerate().map(|(index, item)| {
        search::source_row_with(app, item, text_width, Message::PlayArtistTop(id, index))
    }));
    let toggle = (extras.top.len() > POPULAR).then(|| {
        text_button(
            if app.artists.more_popular { "Mostra meno" } else { "Mostra altri" },
            Message::ArtistMorePopular,
        )
    });
    Some(section("Popolari", toggle, rows.into()))
}

/// Tutta la discografia dell'artista con le schede Tutto, Album e Singoli ed EP (solo quelle che
/// hanno qualcosa). Un clic su una copertina apre l'album.
fn discography<'a>(app: &'a App, extras: &'a ArtistExtras, width: f32) -> Option<Element<'a, Message>> {
    if extras.albums.is_empty() {
        return None;
    }
    let is_album = |a: &&SourcePlaylist| a.kind == "album";
    let albums = extras.albums.iter().filter(is_album).count();
    let singles = extras.albums.len() - albums;

    // Con una sola specie di uscite le schede non servono.
    let both = albums > 0 && singles > 0;
    let filter = if both {
        app.artists.filter
    } else if albums == 0 {
        Discography::Singles
    } else {
        Discography::Albums
    };
    let chips: Option<Element<'a, Message>> = both.then(|| {
        row(Discography::ALL.iter().map(|&choice| {
            let count = match choice {
                Discography::All => albums + singles,
                Discography::Albums => albums,
                Discography::Singles => singles,
            };
            pill(choice.label(), count.to_string(), choice == filter, Message::ArtistFilter(choice))
        }))
        .spacing(8)
        .into()
    });
    let shown: Vec<SourcePlaylist> = extras
        .albums
        .iter()
        .filter(|a| match filter {
            Discography::All => true,
            Discography::Albums => a.kind == "album",
            Discography::Singles => a.kind != "album",
        })
        .cloned()
        .collect();
    let body = shelf::source_playlist_grid(app, &shown, width);
    Some(section("Discografia", None, column![].push_maybe(chips).push(body).spacing(16).into()))
}

/// Altri artisti del catalogo, escluso questo: si apre il loro profilo.
fn related(app: &App, id: Uuid) -> Option<Element<'_, Message>> {
    let cards: Vec<ArtistCard> =
        app.home.feed.artists.iter().filter(|c| c.artist.id != id).take(RELATED).cloned().collect();
    (!cards.is_empty()).then(|| section("Altri artisti nel catalogo", None, shelf::artist_row(app, &cards)))
}

/// I tuoi ascolti di questo artista.
pub fn plays_of(stats: &[PlayStat], artist: Uuid) -> u32 {
    stats.iter().filter(|s| s.track.artist.id == artist).map(|s| s.plays).sum()
}

/// «CC0 · 20, CC BY · 3»: le licenze dei brani, dalla più comune.
pub fn licence_summary(tracks: &[Track]) -> String {
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for track in tracks {
        let label = track.licence.label();
        match counts.iter_mut().find(|(known, _)| *known == label) {
            Some((_, n)) => *n += 1,
            None => counts.push((label, 1)),
        }
    }
    counts.sort_by_key(|a| std::cmp::Reverse(a.1));
    counts.iter().map(|(label, n)| format!("{label} · {n}")).collect::<Vec<_>>().join(", ")
}

/// Una scheda con i dati dell'artista e il modo di cercare altro.
fn about<'a>(app: &'a App, page: &'a ArtistPage, tracks: &[Track]) -> Element<'a, Message> {
    let total: u64 = tracks.iter().map(|t| t.duration_ms).sum();
    let mut facts =
        column![small(format!("{} · {}", count_label(tracks.len()), total_label(total))).color(theme::TEXT),]
            .spacing(6);
    let plays = plays_of(&app.stats, page.id);
    if plays > 0 {
        facts = facts.push(small(format!("Li hai ascoltati {plays} volte")).color(theme::MUTED));
    }
    facts = facts.push(small(format!("Licenze: {}", licence_summary(tracks))).color(theme::MUTED));
    let search = outlined_button_owned(
        format!("Cerca altro di {}", page.name),
        Some(Icon::Search),
        Message::SearchInput(page.name.clone()),
    );
    container(column![h2("Informazioni"), facts, search].spacing(12))
        .padding(20)
        .width(Length::Fill)
        .style(theme::card)
        .into()
}

/// Larghezza che servono Riproduci, Segui e Radio con le etichette.
const ACTIONS_WIDTH: f32 = 340.0;

/// Il banner: immagine tonda, nome enorme, dati e le azioni (Riproduci, Segui, Radio).
fn banner<'a>(
    app: &'a App,
    page: &'a ArtistPage,
    extras: Option<&'a ArtistExtras>,
    tracks: &Arc<[Track]>,
    width: f32,
) -> Element<'a, Message> {
    let url = portrait_url(page).or_else(|| extras.and_then(|e| e.artist.image_url.as_deref()));
    let art = Art { id: page.id, title: &page.name, url };
    let tint = app.covers.tint_of(art);
    let pad = 24.0;
    let side = if width >= 620.0 { PORTRAIT } else { 120.0 };
    let text_width = (width - 2.0 * pad - side - 28.0).max(160.0);

    let mut meta = Vec::new();
    match extras {
        Some(extras) => {
            let albums = extras.albums.iter().filter(|a| a.kind == "album").count();
            if albums > 0 {
                meta.push(format!("{albums} album"));
            }
            let others = extras.albums.len() - albums;
            if others > 0 {
                meta.push(format!("{others} singoli ed EP"));
            }
            if extras.artist.fans > 0 {
                meta.push(side_panel::fans_label(extras.artist.fans));
            }
        }
        None => meta.push(count_label(tracks.len())),
    }

    // Con il banner stretto (finestra piccola, pannello aperto) Segui e Radio restano solo icone:
    // con le etichette uscirebbero dal banner.
    let compact = text_width < ACTIONS_WIDTH;
    let action = move |label: &'a str, glyph: Icon, message: Message| {
        if compact {
            outlined_icon(glyph, label, Some(message))
        } else {
            outlined_button(label, Some(glyph), Some(message))
        }
    };
    let following = app.is_following(page.id);
    let follow = action(
        if following { "Seguito" } else { "Segui" },
        if following { Icon::Check } else { Icon::UserPlus },
        Message::ToggleFollow(FollowedArtist {
            id: page.id,
            name: page.name.clone(),
            cover_url: url.map(str::to_string),
        }),
    );
    // La radio parte dal brano più ascoltato di questo artista già nel catalogo del server.
    let radio = feed::popular(tracks, &app.stats, 1)
        .into_iter()
        .next()
        .map(|track| action("Radio", Icon::Radio, Message::StartRadio(Box::new(track))));
    let mut extra = vec![follow];
    extra.extend(radio);
    // Riproduci suona i brani più ascoltati dal catalogo ufficiale; finché non arrivano, quelli del server.
    let actions: Element<'a, Message> = match extras.filter(|e| !e.top.is_empty()) {
        Some(_) => {
            let mut bar =
                row![primary_button("Riproduci", Some(Icon::Play), Message::PlayArtistTop(page.id, 0))]
                    .spacing(10)
                    .align_y(Alignment::Center);
            for button in extra {
                bar = bar.push(button);
            }
            bar.into()
        }
        None => play_bar(tracks, extra),
    };

    let body = column![
        text("ARTISTA").size(theme::FS_SMALL).font(theme::FONT_SEMIBOLD).color(theme::TEXT),
        line(&page.name, text_width, HERO_LINE),
        line(&meta.join(" · "), text_width, SUB_LINE),
        actions,
    ]
    .spacing(12)
    .width(Length::Fill);
    container(row![cover_round(&app.covers, art, side), body].spacing(28).align_y(Alignment::Center))
        .padding(pad)
        .width(Length::Fill)
        .style(theme::hero_card(tint))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures::track;

    #[test]
    fn licences_are_counted_from_the_most_common() {
        let mut tracks: Vec<Track> = (1..=3).map(track).collect();
        tracks[2].licence.kind = "cc0".into();
        assert_eq!(licence_summary(&tracks), "CC BY · 2, CC0 · 1");
        assert_eq!(licence_summary(&[]), "");
    }

    #[test]
    fn plays_are_summed_for_one_artist() {
        let mine = track(3);
        let other = track(4);
        let stats = vec![
            PlayStat { track: mine.clone(), plays: 4, last_played: 10 },
            PlayStat { track: track(6), plays: 9, last_played: 9 },
            PlayStat { track: other, plays: 2, last_played: 8 },
        ];
        // track(3) e track(6) sono dello stesso artista (n % 3 == 0).
        assert_eq!(plays_of(&stats, mine.artist.id), 13);
        assert_eq!(plays_of(&stats, Uuid::nil()), 0);
    }
}
