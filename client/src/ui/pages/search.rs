//! Ricerca come in un servizio di streaming. In cima il catalogo ufficiale: artisti con la loro foto,
//! brani, album e playlist con le copertine vere; un play cerca l'audio giusto sul server e lo porta
//! nel catalogo. Sotto, ciò che il server ha già (risultato principale, brani, artisti e album) e
//! Internet Archive, di cui si vedono anche le librerie (le raccolte delle netlabel) in cui stanno gli
//! album trovati: aprirne una mostra i suoi album con licenza libera al posto dei risultati.

use std::sync::Arc;

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::api::{
    DiscoverItem, DiscoverLibrary, SearchResults, SourceItem, SourceKind, SourcePlaylist, TopKind, Track,
};
use crate::app::{App, Load, Message, OpenLibrary, Screen};
use crate::ui::components::{
    H1_LINE, MUTED_LINE, SUB_LINE, TITLE_LINE, TableKind, album_art, album_tile_row, art_of, artist_art,
    artist_tile_row, centered, count_label, duration_label, empty_block, error_block, h2, icon_button, line,
    outlined_button, primary_button, skeleton_rows, small, text_button, track_rows,
};
use crate::ui::cover::{Art, cover};
use crate::ui::icons::{Icon, icon};
use crate::ui::shelf;
use crate::ui::theme;

/// Sotto questa larghezza il risultato principale va sopra i brani.
const TWO_COLUMNS_MIN: f32 = 820.0;
const COLUMN_GAP: f32 = 24.0;
/// Brani mostrati accanto al risultato principale; gli altri vanno in "Altri brani".
const TOP_TRACKS: usize = 5;
const TOP_COVER: f32 = 104.0;
const TOP_PADDING: f32 = 18.0;

/// Chiave dell'importazione in corso per un video YouTube (gli item di Archive usano il loro
/// identificatore, che non contiene `:`).
pub fn source_key(source: SourceKind, id: &str) -> String {
    format!("{}:{}", source.as_str(), id)
}

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let query = app.search.submitted.as_str();
    let title = || line(&format!("Risultati per «{query}»"), width, H1_LINE);
    let external = external_sections(app, width);
    let official = official_sections(app, width);
    match &app.search.results {
        Load::Idle => column![
            column![
                line("Cerca", width, H1_LINE),
                line(
                    "Scrivi il nome di un artista, di un album o di un brano. Ctrl+F porta qui da ogni schermata.",
                    width,
                    MUTED_LINE
                ),
            ]
            .spacing(6),
        ]
        .push_maybe(recent_searches(app))
        .push(column![h2("Sfoglia tutto"), shelf::browse_grid(app, width)].spacing(14))
        .spacing(32)
        .into(),
        Load::Loading => {
            column![title(), skeleton_rows(6, theme::ROW_MEDIA, TableKind::Mixed)].spacing(24).into()
        }
        Load::Failed(err) if official.is_empty() => {
            column![title(), error_block("i risultati", err)].spacing(24).into()
        }
        Load::Failed(_) => column![title()].extend(official).extend(external).spacing(36).into(),
        Load::Ready(results) if results.is_empty() => {
            let mut page = column![title()].spacing(24);
            if official.is_empty() && external.is_empty() && !external_loading(app) {
                page = page.push(empty_block(format!(
                    "Nessun risultato per «{query}». Controlla l'ortografia o prova con meno parole."
                )));
            }
            page.extend(official).extend(external).spacing(36).into()
        }
        Load::Ready(results) => {
            let on_server = column![h2("Già nel catalogo del server"), catalogue(app, results, width)]
                .spacing(16);
            column![title()].extend(official).push(on_server).extend(external).spacing(36).into()
        }
    }
}

/// Risultati del catalogo: risultato principale e brani, poi artisti, album e altri brani.
fn catalogue<'a>(app: &'a App, results: &'a SearchResults, width: f32) -> Element<'a, Message> {
    let tracks: Arc<[Track]> = Arc::from(results.tracks.as_slice());
    let first: Arc<[Track]> = tracks.iter().take(TOP_TRACKS).cloned().collect();
    let rest: Arc<[Track]> = tracks.iter().skip(TOP_TRACKS).cloned().collect();

    let wide = width >= TWO_COLUMNS_MIN;
    let top_width = if wide { (width - COLUMN_GAP) * 0.4 } else { width };
    let tracks_width = if wide { width - COLUMN_GAP - top_width } else { width };
    let top = top_result(app, results, &tracks, top_width)
        .map(|card| column![h2("Risultato principale"), card].spacing(12));
    let songs =
        (!first.is_empty()).then(|| column![h2("Brani"), track_rows(app, &first, tracks_width)].spacing(12));

    let head: Element<'a, Message> = match (top, songs) {
        (Some(top), Some(songs)) if wide => {
            row![top.width(Length::Fixed(top_width)), songs.width(Length::Fill),].spacing(COLUMN_GAP).into()
        }
        (top, songs) => column![].push_maybe(top).push_maybe(songs).spacing(28).into(),
    };

    let mut page = column![head].spacing(32);
    if !results.artists.is_empty() {
        page = page.push(column![h2("Artisti"), artist_tile_row(app, &results.artists)].spacing(12));
    }
    if !results.albums.is_empty() {
        page = page.push(column![h2("Album"), album_tile_row(app, &results.albums)].spacing(12));
    }
    if !rest.is_empty() {
        page = page.push(column![h2("Altri brani"), track_rows(app, &rest, width)].spacing(12));
    }
    page.into()
}

/// La carta del risultato principale: copertina grande, nome, tipo e azione.
fn top_result<'a>(
    app: &'a App,
    results: &'a SearchResults,
    tracks: &Arc<[Track]>,
    width: f32,
) -> Option<Element<'a, Message>> {
    let top = results.top?;
    let (art, name, kind, action): (Art<'_>, &str, String, Element<'a, Message>) = match top.kind {
        TopKind::Artist => {
            let artist = results.artists.iter().find(|a| a.id == top.id)?;
            let mut detail = vec!["Artista".to_owned(), count_label(artist.track_count as usize)];
            if artist.album_count > 0 {
                detail.push(format!("{} album", artist.album_count));
            }
            let open = primary_button("Apri", None, Message::Navigate(Screen::Artist(artist.id)));
            (artist_art(artist), &artist.name, detail.join(" · "), open)
        }
        TopKind::Album => {
            let album = results.albums.iter().find(|a| a.id == top.id)?;
            let detail =
                format!("Album · {} · {}", album.artist.name, count_label(album.track_count as usize));
            let open = primary_button("Apri", None, Message::Navigate(Screen::Album(album.id)));
            (album_art(album), &album.title, detail, open)
        }
        TopKind::Track => {
            let index = tracks.iter().position(|t| t.id == top.id)?;
            let track = &tracks[index];
            let detail = format!("Brano · {} · {}", track.artist.name, duration_label(track.duration_ms));
            let play =
                primary_button("Riproduci", Some(Icon::Play), Message::PlayTracks(tracks.clone(), index));
            (art_of(track), &track.title, detail, play)
        }
    };
    let text_width = width - 2.0 * TOP_PADDING;
    Some(
        container(
            column![
                cover(&app.covers, art, TOP_COVER),
                column![line(name, text_width, H1_LINE), line(&kind, text_width, SUB_LINE)].spacing(6),
                action,
            ]
            .spacing(16),
        )
        .padding(TOP_PADDING)
        .width(Length::Fill)
        .style(theme::card)
        .into(),
    )
}

/// Colore stabile per la copertina disegnata di un risultato esterno (FNV-1a sull'identificatore).
pub fn art_id(identifier: &str) -> uuid::Uuid {
    let hash = identifier
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    uuid::Uuid::from_u128(u128::from(hash))
}

fn licence_label(kind: &str) -> &'static str {
    match kind {
        "cc0" => "CC0",
        "cc-by" => "CC BY",
        "cc-by-sa" => "CC BY-SA",
        "cc-by-nc" => "CC BY-NC",
        "cc-by-nc-sa" => "CC BY-NC-SA",
        "cc-by-nd" => "CC BY-ND",
        "cc-by-nc-nd" => "CC BY-NC-ND",
        "public-domain" => "Pubblico dominio",
        "live-trade" => "Concerto dal vivo",
        _ => "Licenza libera",
    }
}

fn external_loading(app: &App) -> bool {
    matches!(app.search.discover, Load::Loading) || matches!(app.search.sources, Load::Loading)
}

/// Brani mostrati nella ricerca del catalogo ufficiale.
const OFFICIAL_TRACKS: usize = 12;

/// Il catalogo ufficiale: brani, artisti, album e playlist. Un play cerca l'audio giusto sul server.
fn official_sections<'a>(app: &'a App, width: f32) -> Vec<Element<'a, Message>> {
    let text_width = width - 2.0 * 12.0 - 50.0 - 16.0 * 2.0 - 40.0;
    let mut sections: Vec<Element<'a, Message>> = Vec::new();
    match &app.search.sources {
        Load::Loading => {
            sections.push(
                column![h2("Brani"), skeleton_rows(4, theme::ROW_MEDIA, TableKind::Mixed)].spacing(12).into(),
            );
            return sections;
        }
        Load::Failed(err) => {
            sections.push(small(format!("Catalogo: {err}")).into());
            return sections;
        }
        Load::Idle | Load::Ready(_) => {}
    }
    if !app.search.artists.is_empty() {
        sections.push(
            column![h2("Artisti"), shelf::meta_artist_row(app, &app.search.artists)].spacing(12).into(),
        );
    }
    if let Load::Ready(tracks) = &app.search.sources
        && !tracks.is_empty()
    {
        let rows = column(tracks.iter().take(OFFICIAL_TRACKS).map(|item| source_row(app, item, text_width)));
        sections.push(column![h2("Brani"), rows].spacing(12).into());
    }
    if let Load::Ready(lists) = &app.search.playlists {
        let (albums, playlists): (Vec<SourcePlaylist>, Vec<SourcePlaylist>) =
            lists.iter().cloned().partition(|l| l.kind != "playlist");
        if !albums.is_empty() {
            sections.push(column![h2("Album"), shelf::source_playlist_row(app, &albums)].spacing(12).into());
        }
        if !playlists.is_empty() {
            sections.push(
                column![h2("Playlist"), shelf::source_playlist_row(app, &playlists)].spacing(12).into(),
            );
        }
    }
    sections
}

/// Sezioni di Internet Archive, solo quelle con qualcosa da mostrare (risultati,
/// caricamento o errore).
fn external_sections<'a>(app: &'a App, width: f32) -> Vec<Element<'a, Message>> {
    let text_width = width - 2.0 * 12.0 - 50.0 - 16.0 * 2.0 - 40.0;
    let mut sections = Vec::new();
    if let Some(open) = &app.search.open_library {
        sections.push(open_library(app, open, text_width));
    } else {
        if !app.search.libraries.is_empty() {
            let rows =
                column(app.search.libraries.iter().map(|library| library_row(app, library, text_width)));
            sections.push(
                column![
                    h2("Librerie su Internet Archive"),
                    small("Raccolte di netlabel: dentro trovi solo gli album con licenza libera."),
                    rows
                ]
                .spacing(12)
                .into(),
            );
        }
        if let Some(body) =
            external(&app.search.discover, "Internet Archive", |item| discover_row(app, item, text_width))
        {
            sections.push(column![h2("Da Internet Archive"), body].spacing(12).into());
        }
    }
    sections
}

fn external<'a, T>(
    load: &'a Load<Arc<[T]>>,
    source: &str,
    item_row: impl Fn(&'a T) -> Element<'a, Message>,
) -> Option<Element<'a, Message>> {
    match load {
        Load::Idle => None,
        Load::Loading => Some(skeleton_rows(3, theme::ROW_MEDIA, TableKind::Mixed)),
        Load::Failed(err) => Some(small(format!("{source}: {err}")).into()),
        Load::Ready(items) if items.is_empty() => None,
        Load::Ready(items) => Some(column(items.iter().map(item_row)).into()),
    }
}

/// Play di un risultato esterno: "Apro…" mentre il server lo porta nel catalogo.
pub fn play_action<'a>(importing: bool, play: Message) -> (Element<'a, Message>, Option<Message>) {
    if importing {
        (small("Apro…").into(), None)
    } else {
        (icon_button(Icon::Play, "Ascolta", false, Some(play.clone())), Some(play))
    }
}

/// Riga di un risultato esterno: copertina, titolo, dettagli e azione (play o apertura).
pub fn external_row<'a>(
    app: &'a App,
    art: Art<'_>,
    title: &str,
    detail: &str,
    (action, press): (Element<'a, Message>, Option<Message>),
    text_width: f32,
) -> Element<'a, Message> {
    button(centered(
        row![
            cover(&app.covers, art, 50.0),
            column![line(title, text_width, TITLE_LINE), line(detail, text_width, SUB_LINE)]
                .spacing(4)
                .width(Length::Fill),
            action,
        ]
        .spacing(16)
        .align_y(Alignment::Center),
    ))
    .height(theme::ROW_MEDIA)
    .width(Length::Fill)
    .padding([0, 12])
    .style(theme::row(false))
    .on_press_maybe(press)
    .into()
}

fn discover_row<'a>(app: &'a App, item: &'a DiscoverItem, text_width: f32) -> Element<'a, Message> {
    let mut detail = Vec::new();
    if let Some(creator) = &item.creator {
        detail.push(creator.clone());
    }
    detail.push(licence_label(&item.licence).to_owned());
    if let Some(year) = &item.year {
        detail.push(year.clone());
    }
    if item.lossless {
        detail.push("Lossless".to_owned());
    }
    let importing = app.search.importing.as_deref() == Some(item.identifier.as_str());
    let art = Art { id: art_id(&item.identifier), title: &item.title, url: item.cover_url.as_deref() };
    external_row(
        app,
        art,
        &item.title,
        &detail.join(" · "),
        play_action(importing, Message::PlayDiscovered(item.identifier.clone())),
        text_width,
    )
}

/// "Netlabel · Autore · 3 album trovati".
fn library_detail(library: &DiscoverLibrary) -> String {
    let mut detail = vec!["Netlabel".to_owned()];
    if let Some(creator) = &library.creator {
        detail.push(creator.clone());
    }
    detail.push(if library.found == 1 {
        "1 album trovato".to_owned()
    } else {
        format!("{} album trovati", library.found)
    });
    detail.join(" · ")
}

fn library_row<'a>(app: &'a App, library: &'a DiscoverLibrary, text_width: f32) -> Element<'a, Message> {
    let art =
        Art { id: art_id(&library.identifier), title: &library.title, url: library.cover_url.as_deref() };
    let open = Message::ArchiveLibraryOpen(library.clone());
    let action = icon_button(Icon::List, "Apri la libreria", false, Some(open.clone()));
    external_row(app, art, &library.title, &library_detail(library), (action, Some(open)), text_width)
}

/// Una libreria aperta: intestazione con il ritorno ai risultati, poi i suoi album con licenza
/// libera, una pagina alla volta.
fn open_library<'a>(app: &'a App, open: &'a OpenLibrary, text_width: f32) -> Element<'a, Message> {
    let library = &open.library;
    let head = row![
        column![h2(library.title.as_str()), small(library_detail(library))].spacing(4).width(Length::Fill),
        outlined_button("Torna ai risultati", Some(Icon::Close), Some(Message::ArchiveLibraryClose)),
    ]
    .spacing(16)
    .align_y(Alignment::Center);
    let body: Element<'a, Message> = match &open.items {
        Load::Idle | Load::Loading => skeleton_rows(4, theme::ROW_MEDIA, TableKind::Mixed),
        Load::Failed(err) => small(format!("Internet Archive: {err}")).into(),
        Load::Ready(items) if items.is_empty() => {
            small("In questa libreria non ci sono album con licenza libera.").into()
        }
        Load::Ready(items) => {
            let rows = column(items.iter().map(|item| discover_row(app, item, text_width)));
            let more = match (open.next_page, open.loading_more) {
                (None, _) => None,
                (Some(_), true) => Some(outlined_button("Carico…", None, None)),
                (Some(_), false) => {
                    Some(outlined_button("Altri album", None, Some(Message::ArchiveLibraryMore)))
                }
            };
            column![rows].push_maybe(more).spacing(12).into()
        }
    };
    column![head, body].spacing(12).into()
}

/// «YouTube · Autore · 3:35» (Bandcamp non dichiara la durata).
pub fn source_detail(item: &SourceItem) -> String {
    if item.source == SourceKind::Deezer {
        return match item.duration_ms {
            Some(ms) => format!("{} · {}", item.artist, duration_label(ms)),
            None => item.artist.clone(),
        };
    }
    match item.duration_ms {
        Some(ms) => format!("{} · {} · {}", item.source.label(), item.artist, duration_label(ms)),
        None => format!("{} · {}", item.source.label(), item.artist),
    }
}

pub fn source_row<'a>(app: &'a App, item: &'a SourceItem, text_width: f32) -> Element<'a, Message> {
    source_row_with(app, item, text_width, Message::PlaySource(item.source, item.id.clone()))
}

/// Come `source_row`, con il messaggio da mandare al play (la lista da cui si suona).
pub fn source_row_with<'a>(
    app: &'a App,
    item: &'a SourceItem,
    text_width: f32,
    play: Message,
) -> Element<'a, Message> {
    let detail = source_detail(item);
    let importing = app.search.importing.as_deref() == Some(source_key(item.source, &item.id).as_str())
        || app.external.importing(item.source, &item.id);
    external_row(
        app,
        Art { id: art_id(&item.id), title: &item.title, url: item.cover_url.as_deref() },
        &item.title,
        &detail,
        play_action(importing, play),
        text_width,
    )
}

/// Le ultime ricerche, come pillole: il clic la ripete, la × la dimentica.
fn recent_searches(app: &App) -> Option<Element<'_, Message>> {
    if app.search_history.is_empty() {
        return None;
    }
    let chips = app.search_history.iter().map(|query| {
        container(
            row![
                button(text(query.as_str()).size(theme::FS_BODY).font(theme::FONT_MEDIUM))
                    .padding(0)
                    .style(theme::text_action)
                    .on_press(Message::SearchInput(query.clone())),
                button(icon(Icon::Close, 12.0, theme::MUTED))
                    .padding(2)
                    .style(theme::icon_plain)
                    .on_press(Message::ForgetSearch(Some(query.clone()))),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([7, 12])
        .style(theme::chip)
        .into()
    });
    let header = row![
        h2("Ricerche recenti"),
        Space::with_width(Length::Fill),
        text_button("Cancella tutto", Message::ForgetSearch(None)),
    ]
    .align_y(Alignment::Center);
    Some(column![header, wrap_row(chips.collect())].spacing(14).into())
}

/// Le pillole vanno a capo quando la riga è piena: Iced 0.13 non ha un `wrap`, quindi si dividono
/// a gruppi di quattro.
fn wrap_row(chips: Vec<Element<'_, Message>>) -> Element<'_, Message> {
    let mut rows = column![].spacing(8);
    let mut chips = chips.into_iter().peekable();
    while chips.peek().is_some() {
        rows = rows.push(row(chips.by_ref().take(4)).spacing(8));
    }
    rows.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_keys_never_collide_with_archive_identifiers() {
        // Gli identificatori di Archive sono lettere, cifre, `.`, `_` e `-`: mai `:`.
        assert_eq!(source_key(SourceKind::YouTube, "dQw4w9WgXcQ"), "youtube:dQw4w9WgXcQ");
    }

    #[test]
    fn external_art_colour_is_stable() {
        assert_eq!(art_id("marea-banchi"), art_id("marea-banchi"));
        assert_ne!(art_id("marea-banchi"), art_id("marea-onde"));
    }
}
