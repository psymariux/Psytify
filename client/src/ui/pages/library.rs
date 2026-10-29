//! Catalogo: tutto ciò che il backend pubblica. Tre schede: gli album con la copertina grande, gli
//! artisti in cerchio e l'elenco dei brani, con i filtri a pillola e l'ordine (più recenti o dalla A
//! alla Z). Domanda della schermata: "che cosa ascolto?".

use std::sync::Arc;

use iced::widget::{Space, button, column, row, text};
use iced::{Alignment, Element, Length};

use crate::api::{Sort, Track};
use crate::app::{App, LibraryFilter, Load, Message, Screen};
use crate::feed::{self, ArtistCard};
use crate::ui::components::{
    Origin, TableKind, album_grid, centered, empty_state, error_block, outlined_button, page_header, pill,
    play_controls, section, skeleton_grid, skeleton_rows, small, text_button, total_label, track_table,
};
use crate::ui::shelf;
use crate::ui::theme;

/// Quanti singoli compaiono sotto gli album.
const SINGLES_SHOWN: usize = 12;
/// Le schede principali, in ordine.
const TABS: [(LibraryFilter, &str); 3] =
    [(LibraryFilter::Albums, "Album"), (LibraryFilter::Artists, "Artisti"), (LibraryFilter::All, "Brani")];
/// I filtri dell'elenco dei brani.
const TRACK_FILTERS: [(LibraryFilter, &str); 3] = [
    (LibraryFilter::All, "Tutti"),
    (LibraryFilter::Downloadable, "Scaricabili"),
    (LibraryFilter::Singles, "Singoli"),
];

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let tracks = app.library.tracks.ready().map(Vec::as_slice);
    let filter = app.library.filter;
    let visible: Option<Arc<[Track]>> =
        tracks.map(|all| all.iter().filter(|t| filter.matches(t)).cloned().collect());
    // Riproduci e Casuale agiscono sui brani che si vedono; su album e artisti si sceglie un elemento.
    let actions = visible
        .as_ref()
        .filter(|list| filter.is_tracks() && !list.is_empty())
        .map(|list| play_controls(Message::PlayTracks(list.clone(), 0), Message::ShufflePlay(list.clone())));
    let header = page_header("Catalogo", subline(app, tracks), actions);

    let mut page = column![header, tabs(app, tracks, width)].spacing(20);
    if filter.is_tracks() {
        page = page.push(track_filters(app, tracks));
    }
    let body = match filter {
        LibraryFilter::Albums => albums(app, width),
        LibraryFilter::Artists => artists(app, width),
        _ => match (&app.library.tracks, visible) {
            (Load::Failed(err), _) => error_block("i brani", err),
            (Load::Ready(all), Some(list)) => loaded(app, all, list, width),
            _ => skeleton_rows(8, theme::ROW_PLAIN, TableKind::Mixed),
        },
    };
    page.push(body).spacing(24).into()
}

/// I numeri del catalogo, dai dati caricati: «24 album · 10 artisti · 50 brani».
fn subline(app: &App, tracks: Option<&[Track]>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Load::Ready(albums) = &app.library.albums {
        let more = if app.library.albums_truncated { "+" } else { "" };
        parts.push(format!("{}{more} album", albums.len()));
    }
    let artists = library_artists(app);
    if !artists.is_empty() {
        parts.push(format!("{} artisti", artists.len()));
    }
    if let Some(tracks) = tracks {
        let more = if app.library.next_offset.is_some() { "+" } else { "" };
        let downloadable = tracks.iter().filter(|t| t.downloadable).count();
        parts.push(format!("{}{more} brani", tracks.len()));
        parts.push(format!("{} di musica", total_label(tracks.iter().map(|t| t.duration_ms).sum())));
        if downloadable > 0 {
            parts.push(format!("{downloadable} scaricabili"));
        }
    }
    if parts.is_empty() { "Il catalogo pubblicato".into() } else { parts.join(" · ") }
}

/// Gli artisti del catalogo che si conoscono: quelli degli album e dei brani caricati.
fn library_artists(app: &App) -> Vec<ArtistCard> {
    let albums = app.library.albums.ready().map_or(&[][..], |a| &a[..]);
    let tracks = app.library.tracks.ready().map_or(&[][..], Vec::as_slice);
    let mut found = feed::artists_from(albums, tracks);
    for card in &app.home.feed.artists {
        if !found.iter().any(|a| a.artist.id == card.artist.id) {
            found.push(card.clone());
        }
    }
    if app.library.sort == Sort::Title {
        found.sort_by_key(|a| a.artist.name.to_lowercase());
    }
    found
}

/// Le schede con il loro conteggio, e a destra l'ordine.
fn tabs<'a>(app: &App, tracks: Option<&[Track]>, width: f32) -> Element<'a, Message> {
    let more = if app.library.next_offset.is_some() { "+" } else { "" };
    let selected = |tab: LibraryFilter| {
        tab == app.library.filter || (tab == LibraryFilter::All && app.library.filter.is_tracks())
    };
    let tabs = row(TABS.iter().map(|&(tab, label)| {
        let count = match (tab, &app.library.albums, tracks) {
            (LibraryFilter::Albums, Load::Ready(albums), _) => {
                format!("{}{}", albums.len(), if app.library.albums_truncated { "+" } else { "" })
            }
            (LibraryFilter::Artists, ..) => {
                let n = library_artists(app).len();
                if n == 0 { "—".to_string() } else { n.to_string() }
            }
            (LibraryFilter::All, _, Some(all)) => format!("{}{more}", all.len()),
            _ => "—".to_string(),
        };
        pill(label, count, selected(tab), Message::LibraryFilter(tab))
    }))
    .spacing(8);

    let sort = row![
        text("Ordina").size(theme::FS_SMALL).color(theme::FAINT),
        sort_chip("Recenti", Sort::Recent, app.library.sort),
        sort_chip("A–Z", Sort::Title, app.library.sort),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if width < 640.0 {
        return column![tabs, sort].spacing(12).into();
    }
    row![tabs, Space::with_width(Length::Fill), sort].align_y(Alignment::Center).into()
}

fn sort_chip<'a>(label: &'a str, sort: Sort, current: Sort) -> Element<'a, Message> {
    button(centered(text(label).size(theme::FS_SMALL + 1.0).font(theme::FONT_MEDIUM)))
        .height(28)
        .padding([0, 12])
        .style(theme::pill(sort == current))
        .on_press(Message::LibrarySort(sort))
        .into()
}

/// Tutti, scaricabili, singoli: ognuno dice quanti brani mostra.
fn track_filters<'a>(app: &App, tracks: Option<&[Track]>) -> Element<'a, Message> {
    let more = if app.library.next_offset.is_some() { "+" } else { "" };
    row(TRACK_FILTERS.iter().map(|&(filter, label)| {
        let count = match tracks {
            Some(all) => format!("{}{more}", all.iter().filter(|t| filter.matches(t)).count()),
            None => "—".to_string(),
        };
        pill(label, count, filter == app.library.filter, Message::LibraryFilter(filter))
    }))
    .spacing(8)
    .into()
}

/// Gli album del catalogo in griglia, con la copertina grande, e sotto i singoli.
fn albums(app: &App, width: f32) -> Element<'_, Message> {
    let body: Element<'_, Message> = match &app.library.albums {
        Load::Idle | Load::Loading => skeleton_grid(width, 2),
        Load::Failed(err) => error_block("gli album", err),
        Load::Ready(albums) if albums.is_empty() => empty_state(
            "Nessun album, per ora",
            "Il catalogo ha solo singoli. Un album entra quando ascolti un item di Internet Archive con più brani.",
            Some(("Vedi i brani", Message::LibraryFilter(LibraryFilter::All))),
        ),
        Load::Ready(albums) => album_grid(app, albums, width),
    };
    let mut page = column![body].spacing(16);
    if app.library.albums_truncated {
        let order = if app.library.sort == Sort::Title { "dalla A alla Z" } else { "dai più recenti" };
        page = page.push(small(format!("Sono mostrati i primi 100 album, {order}.")));
    }
    if let Some(singles) = singles(app, width) {
        page = page.push(singles);
    }
    page.spacing(32).into()
}

/// I singoli (brani senza album) già caricati, in tessere: tengono piena la pagina anche quando gli
/// album sono pochi.
fn singles(app: &App, width: f32) -> Option<Element<'_, Message>> {
    let all = app.library.tracks.ready()?;
    let singles: Arc<[Track]> =
        all.iter().filter(|t| t.album.is_none()).take(SINGLES_SHOWN).cloned().collect();
    if singles.is_empty() {
        return None;
    }
    let total = all.iter().filter(|t| t.album.is_none()).count();
    let more = (total > SINGLES_SHOWN || app.library.next_offset.is_some())
        .then(|| text_button("Vedi tutti i singoli", Message::LibraryFilter(LibraryFilter::Singles)));
    Some(section("Singoli", more, shelf::track_grid(app, &singles, width)))
}

/// Gli artisti in cerchio, in griglia.
fn artists(app: &App, width: f32) -> Element<'_, Message> {
    let known = library_artists(app);
    match (&app.library.albums, &app.library.tracks) {
        (Load::Failed(err), _) => error_block("gli artisti", err),
        _ if known.is_empty() && !matches!(app.library.albums, Load::Ready(_)) => skeleton_grid(width, 2),
        _ if known.is_empty() => empty_state(
            "Nessun artista, per ora",
            "Gli artisti compaiono quando il catalogo ha dei brani: cerca su Internet Archive per aggiungerne.",
            Some(("Cerca musica", Message::Navigate(Screen::Search))),
        ),
        _ => shelf::artist_grid(app, &known, width),
    }
}

fn loaded<'a>(app: &'a App, all: &[Track], visible: Arc<[Track]>, width: f32) -> Element<'a, Message> {
    let filter = app.library.filter;
    let list: Element<'a, Message> = match (visible.is_empty(), all.is_empty()) {
        (false, _) => track_table(app, &visible, TableKind::Mixed, width, Origin::Catalogue),
        (true, true) => empty_state(
            "Il catalogo è vuoto",
            "Cerca un artista o un genere: ogni album di Internet Archive che ascolti entra qui.",
            Some(("Cerca su Internet Archive", Message::Navigate(Screen::Search))),
        ),
        (true, false) if filter == LibraryFilter::Downloadable => empty_state(
            "Niente da scaricare tra questi brani",
            "Si scaricano solo i brani la cui licenza lo consente: nessuno di quelli caricati finora.",
            Some(("Vedi tutti i brani", Message::LibraryFilter(LibraryFilter::All))),
        ),
        (true, false) => empty_state(
            "Nessun singolo",
            "Tutti i brani caricati fanno parte di un album.",
            Some(("Vedi gli album", Message::LibraryFilter(LibraryFilter::Albums))),
        ),
    };

    let mut page = column![list].spacing(16).width(Length::Fill);
    if let Some(err) = &app.library.more_error {
        page =
            page.push(row![small(err.to_string()), text_button("Riprova", Message::LibraryMore)].spacing(12));
    } else if app.library.loading_more {
        page = page.push(small("Caricamento di altri brani…"));
    } else if app.library.next_offset.is_some() {
        page = page.push(outlined_button("Carica altri brani", None, Some(Message::LibraryMore)));
    }
    page.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_track_tab_is_selected_for_every_track_filter() {
        for filter in [LibraryFilter::All, LibraryFilter::Downloadable, LibraryFilter::Singles] {
            assert!(filter.is_tracks());
        }
        assert!(!LibraryFilter::Albums.is_tracks() && !LibraryFilter::Artists.is_tracks());
    }

    #[test]
    fn the_singles_filter_keeps_only_tracks_without_an_album() {
        let mut single = crate::api::models::fixtures::track(1);
        single.album = None;
        let mut on_album = crate::api::models::fixtures::track(2);
        on_album.album =
            Some(crate::api::models::AlbumRef { id: uuid::Uuid::from_u128(9), title: "Banchi".into() });
        assert!(LibraryFilter::Singles.matches(&single));
        assert!(!LibraryFilter::Singles.matches(&on_album));
        assert!(LibraryFilter::All.matches(&on_album));
    }
}
