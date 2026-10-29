//! Gli scaffali della Home e di «Novità»: l'uscita in evidenza (carosello), le uscite con la
//! copertina grande, gli artisti in cerchio, i mix e la classifica di ciò che si ascolta di più.
//! Sono i pezzi con cui Spotify e Apple Music fanno la loro pagina iniziale, in Iced.

use std::sync::Arc;

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Color, Element, Length, Padding, alignment};
use uuid::Uuid;

use super::components::{
    Line, MUTED_LINE, PLAYING_LINE, SUB_LINE, TITLE_LINE, Tile, faded, hot_progress, hoverable, line,
    outlined_button, outlined_icon, primary_button, scroll_row, sized_tile, wrapped,
};
use super::cover::{Art, cover};
use super::icons::{Icon, icon};
use super::motion::{self, fade};
use super::theme;
use crate::api::{MetaArtist, SourceKind, SourcePlaylist, Track};
use crate::app::{App, HERO_SWAP, MenuTarget, Message, Screen, TrackSource};
use crate::config::taste::Mix;
use crate::config::{FollowedArtist, PlayStat, SavedAlbum};
use crate::feed::{ArtistCard, Release};

/// Le playlist e gli album delle fonti esterne in una fila di tessere: il clic apre la playlist.
pub fn source_playlist_row<'a>(app: &App, playlists: &[SourcePlaylist]) -> Element<'a, Message> {
    scroll_row(playlists.iter().map(|playlist| source_playlist_tile(app, playlist)).collect())
}

/// «Album · 2003 · Autore» per il catalogo, «Playlist · YouTube · Autore» per le altre fonti.
pub fn source_playlist_kind(playlist: &SourcePlaylist) -> String {
    let kind = playlist.kind_label();
    match (playlist.source, playlist.year()) {
        (SourceKind::Deezer, Some(year)) => format!("{kind} · {year}"),
        (SourceKind::Deezer, None) => format!("{kind} · {}", playlist.owner),
        _ => format!("{kind} · {} · {}", playlist.source.label(), playlist.owner),
    }
}

/// Gli artisti del catalogo ufficiale in cerchio con la loro foto: il clic apre la pagina.
pub fn meta_artist_row<'a>(app: &App, artists: &[MetaArtist]) -> Element<'a, Message> {
    scroll_row(
        artists
            .iter()
            .map(|artist| {
                let id = crate::ui::pages::search::art_id(&format!("artist/{}", artist.id));
                let spec = Tile::new(
                    id,
                    Art { id, title: &artist.name, url: artist.image_url.as_deref() },
                    &artist.name,
                    "Artista",
                    Message::OpenMetaArtist(artist.id),
                )
                .round();
                sized_tile(app, spec, ARTIST_COVER)
            })
            .collect(),
    )
}

/// La discografia come griglia sui due bordi della pagina: l'ultima fila parte da sinistra.
pub fn source_playlist_grid<'a>(app: &App, playlists: &[SourcePlaylist], width: f32) -> Element<'a, Message> {
    let (columns, side) = super::components::grid_columns(width);
    column(playlists.chunks(columns).map(|chunk| {
        row(chunk.iter().map(|playlist| source_playlist_tile_sized(app, playlist, side)))
            .spacing(theme::GRID_GAP)
            .into()
    }))
    .spacing(theme::GRID_GAP)
    .into()
}

fn source_playlist_tile<'a>(app: &App, playlist: &SourcePlaylist) -> Element<'a, Message> {
    source_playlist_tile_sized(app, playlist, BIG_COVER)
}

fn source_playlist_tile_sized<'a>(app: &App, playlist: &SourcePlaylist, side: f32) -> Element<'a, Message> {
    let id = crate::ui::pages::search::art_id(&playlist.id);
    let subtitle = source_playlist_kind(playlist);
    let spec = Tile::new(
        id,
        Art { id, title: &playlist.title, url: playlist.cover_url.as_deref() },
        &playlist.title,
        &subtitle,
        Message::OpenSourcePlaylist(playlist.clone()),
    );
    sized_tile(app, spec, side)
}

/// Lato delle copertine grandi degli scaffali.
pub const BIG_COVER: f32 = 184.0;
/// Lato delle copertine tonde degli artisti.
pub const ARTIST_COVER: f32 = 148.0;
/// Il titolo dell'uscita in evidenza.
/// Testo secondario sopra una scheda colorata: il bianco, un po' attenuato.
const ON_TINT_LINE: Line =
    Line { size: theme::FS_SMALL, font: theme::FONT, color: Some(Color { a: 0.75, ..Color::WHITE }) };
const HERO_TITLE: Line =
    Line { size: theme::FS_FEATURE, font: theme::FONT_SEMIBOLD, color: Some(theme::TEXT) };

pub fn release_art(release: &Release) -> Art<'_> {
    Art { id: release.id, title: &release.title, url: release.cover_url.as_deref() }
}

/// Il messaggio che fa partire un'uscita: l'album intero (si chiede al server), o il singolo.
pub fn release_play(release: &Release) -> Message {
    match release.tracks.first() {
        Some(single) if !release.is_album => Message::PlayTracks(Arc::from([single.clone()]), 0),
        _ => Message::PlayAlbum(release.id),
    }
}

/// Una tessera di uscita: copertina grande, titolo, «Album · Artista». `new` mette l'etichetta.
pub fn release_tile<'a>(app: &App, release: &Release, side: f32, new: bool) -> Element<'a, Message> {
    let subtitle = format!("{} · {}", release.kind_label(), release.artist.name);
    let mut spec = if release.is_album {
        Tile::new(
            release.id,
            release_art(release),
            &release.title,
            &subtitle,
            Message::Navigate(Screen::Album(release.id)),
        )
        .menu(MenuTarget::Album(Box::new(SavedAlbum {
            id: release.id,
            title: release.title.clone(),
            artist: release.artist.clone(),
            cover_url: release.cover_url.clone(),
        })))
        .play_button(release_play(release))
    } else {
        let single =
            Tile::new(release.id, release_art(release), &release.title, &subtitle, release_play(release));
        let single = match release.tracks.first() {
            Some(track) => single.menu(MenuTarget::track(track, TrackSource::Catalogue)),
            None => single,
        };
        single.play_badge()
    };
    if new {
        spec = spec.badge("Nuovo");
    }
    sized_tile(app, spec, side)
}

/// Una fila orizzontale di uscite. Quelle più recenti di `seen` portano l'etichetta «Nuovo»
/// (`None`: nessuna).
pub fn release_row<'a>(
    app: &App,
    releases: &[Release],
    side: f32,
    seen: Option<&str>,
) -> Element<'a, Message> {
    scroll_row(
        releases
            .iter()
            .map(|release| {
                let new = seen.is_some_and(|seen| release.created_at.as_str() > seen);
                release_tile(app, release, side, new)
            })
            .collect(),
    )
}

/// Una tessera tonda di artista.
pub fn artist_tile<'a>(app: &App, card: &ArtistCard, side: f32) -> Element<'a, Message> {
    let spec = Tile::new(
        card.artist.id,
        Art { id: card.artist.id, title: &card.artist.name, url: card.cover_url.as_deref() },
        &card.artist.name,
        if app.is_following(card.artist.id) { "Seguito" } else { "Artista" },
        Message::Navigate(Screen::Artist(card.artist.id)),
    )
    .menu(MenuTarget::artist(card.artist.id, &card.artist.name, card.cover_url.as_deref()))
    .round();
    sized_tile(app, spec, side)
}

/// I brani come tessere in griglia, sui due bordi della pagina come quella degli album.
pub fn track_grid<'a>(app: &App, tracks: &Arc<[Track]>, width: f32) -> Element<'a, Message> {
    let (columns, side) = super::components::grid_columns(width);
    let indices: Vec<usize> = (0..tracks.len()).collect();
    column(indices.chunks(columns).map(|chunk| {
        row(chunk.iter().map(|&index| super::components::track_tile(app, tracks, index, side)))
            .spacing(theme::GRID_GAP)
            .into()
    }))
    .spacing(theme::GRID_GAP)
    .into()
}

/// Gli artisti in griglia, sui due bordi della pagina come quella degli album.
pub fn artist_grid<'a>(app: &App, cards: &[ArtistCard], width: f32) -> Element<'a, Message> {
    let (columns, side) = super::components::grid_columns(width);
    column(cards.chunks(columns).map(|chunk| {
        row(chunk.iter().map(|card| artist_tile(app, card, side))).spacing(theme::GRID_GAP).into()
    }))
    .spacing(theme::GRID_GAP)
    .into()
}

pub fn artist_row<'a>(app: &App, cards: &[ArtistCard]) -> Element<'a, Message> {
    scroll_row(cards.iter().map(|card| artist_tile(app, card, ARTIST_COVER)).collect())
}

// ---------- Mix ----------

/// Una scheda di mix: quattro copertine in un quadrato, il nome e chi c'è dentro.
pub fn mix_card<'a>(app: &App, index: usize, mix: &Mix, side: f32) -> Element<'a, Message> {
    let id = Uuid::from_u128(0xfeed_0000 + index as u128);
    const GAP: f32 = 2.0;
    let half = ((side - GAP) / 2.0).floor();
    // Quattro copertine diverse, se il mix ne ha: quelle di un solo album si ripeterebbero.
    let mut arts: Vec<Art<'_>> = Vec::new();
    for track in &mix.tracks {
        let art = super::components::art_of(track);
        if arts.len() < 4 && !arts.iter().any(|a| a.url == art.url && a.id == art.id) {
            arts.push(art);
        }
    }
    let pick = |n: usize| cover(&app.covers, arts[n % arts.len()], half);
    let collage =
        column![row![pick(0), pick(1)].spacing(GAP), row![pick(2), pick(3)].spacing(GAP)].spacing(GAP);

    let progress = hot_progress(app, id);
    let mut layers = iced::widget::stack![collage];
    if progress > 0.0 {
        let play = container(icon(Icon::Play, 18.0, fade(theme::ON_PRIMARY, progress)))
            .center(super::components::FAB)
            .style(theme::fab(progress));
        layers = layers.push(
            container(play)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(alignment::Horizontal::Right)
                .align_y(alignment::Vertical::Bottom)
                .padding(Padding { right: 8.0, bottom: 8.0 - 6.0 * (1.0 - progress), ..Padding::ZERO }),
        );
    }
    let item = button(
        column![layers, line(&mix.title, side, TITLE_LINE), line(&mix.subtitle, side, SUB_LINE)]
            .spacing(6)
            .width(side),
    )
    .padding(8)
    .style(theme::tile)
    .on_press(Message::PlayMix(index));
    hoverable(id, false, None, item.into())
}

pub fn mix_row<'a>(app: &App, mixes: &[Mix]) -> Element<'a, Message> {
    scroll_row(mixes.iter().enumerate().map(|(index, mix)| mix_card(app, index, mix, BIG_COVER)).collect())
}

// ---------- Classifica ----------

/// Larghezza del numero d'ordine di una riga della classifica.
const RANK_WIDTH: f32 = 30.0;

/// I brani più ascoltati, numerati: una colonna se la pagina è stretta, due se c'è spazio.
pub fn chart<'a>(app: &App, top: &[PlayStat], width: f32) -> Element<'a, Message> {
    let tracks: Arc<[Track]> = top.iter().map(|s| s.track.clone()).collect();
    let columns = if width >= 760.0 { 2 } else { 1 };
    let per_column = tracks.len().div_ceil(columns).max(1);
    let cell = (width - theme::GRID_GAP * (columns as f32 - 1.0)) / columns as f32;
    let entered = app.page_motion.elapsed();
    let rows: Vec<Element<'a, Message>> = top
        .iter()
        .enumerate()
        .map(|(index, stat)| {
            let track = &stat.track;
            let playing = app.is_current(track.id);
            let progress = motion::stagger(entered, index);
            let text_width = cell - 24.0 - RANK_WIDTH - theme::ROW_COVER - 90.0 - 32.0;
            let plays =
                if stat.plays == 1 { "1 ascolto".to_string() } else { format!("{} ascolti", stat.plays) };
            let item = button(super::components::centered(
                row![
                    container(
                        text((index + 1).to_string())
                            .size(theme::FS_TITLE)
                            .font(theme::FONT_SEMIBOLD)
                            .color(fade(if index == 0 { theme::ACCENT } else { theme::MUTED }, progress)),
                    )
                    .width(RANK_WIDTH),
                    cover(&app.covers, super::components::art_of(track), theme::ROW_COVER),
                    column![
                        line(
                            &track.title,
                            text_width,
                            faded(if playing { PLAYING_LINE } else { TITLE_LINE }, progress)
                        ),
                        line(&track.artist.name, text_width, faded(SUB_LINE, progress)),
                    ]
                    .spacing(4)
                    .width(Length::Fill),
                    text(plays).size(theme::FS_SMALL).color(fade(theme::FAINT, progress)),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            ))
            .height(theme::ROW_PLAIN)
            .width(Length::Fill)
            .padding([0, 12])
            .style(theme::row(playing))
            .on_press(if playing {
                Message::TogglePlay
            } else {
                Message::PlayTracks(tracks.clone(), index)
            });
            hoverable(track.id, true, Some(MenuTarget::track(track, TrackSource::Catalogue)), item.into())
        })
        .collect();
    let mut grid = row![].spacing(theme::GRID_GAP);
    let mut rows = rows.into_iter();
    for _ in 0..columns {
        let part: Vec<Element<'a, Message>> = rows.by_ref().take(per_column).collect();
        grid = grid.push(column(part).width(Length::FillPortion(1)));
    }
    grid.into()
}

// ---------- Carosello ----------

/// L'uscita in evidenza: copertina grande, titolo enorme, Ascolta / Salva / Segui, e i puntini
/// per passare alle altre. Gira da solo ogni `HERO_PERIOD`.
/// Sotto questa larghezza il riquadro in evidenza mette i pulsanti sotto la copertina.
const NARROW_HERO: f32 = 420.0;

pub fn hero<'a>(app: &'a App, width: f32) -> Option<Element<'a, Message>> {
    let releases = &app.home.feed.hero;
    let release = releases.get(app.home.hero.index)?;
    let shown = app.home.hero.since.map_or(1.0, |since| motion::appear(since, HERO_SWAP));
    let tint = app.covers.tint_of(release_art(release));

    // Sul telefono (stretto) copertina più piccola e pulsanti sotto, a tutta larghezza: il titolo
    // ha lo spazio per due righe intere.
    let narrow = width < NARROW_HERO;
    let side = if width >= 620.0 {
        220.0
    } else if width >= 460.0 {
        150.0
    } else if narrow {
        96.0
    } else {
        112.0
    };
    let pad = if narrow { 18.0 } else { 24.0 };
    let gap = if narrow { 16.0 } else { 28.0 };
    let text_width = (width - 2.0 * pad - side - gap).max(140.0);
    // Con poco spazio il titolo rimpicciolisce e va a capo, e i pulsanti secondari restano icone:
    // niente «Blindin…» né «S».
    let compact = text_width < 320.0;
    let title_style = if compact { Line { size: theme::FS_PAGE, ..HERO_TITLE } } else { HERO_TITLE };
    let following = app.is_following(release.artist.id);
    let artist = FollowedArtist {
        id: release.artist.id,
        name: release.artist.name.clone(),
        cover_url: release.cover_url.clone(),
    };

    let mut meta = vec![release.artist.name.clone(), release.kind_label().to_string()];
    if release.is_album {
        meta.push(super::components::count_label(release.tracks.len().max(1)));
    }
    // L'anno del catalogo è quello in cui il brano è entrato, non quello di uscita: non si mostra.
    let kicker = if app.unread_news() > 0 && app.home.feed.news.iter().any(|n| n.id == release.id) {
        "NOVITÀ DAI TUOI ARTISTI"
    } else {
        "AGGIUNTO DI RECENTE"
    };

    let mut actions = row![primary_button("Ascolta", Some(Icon::Play), release_play(release))]
        .spacing(10)
        .align_y(Alignment::Center);
    let secondary = |label: &'static str, glyph: Icon, message: Message| {
        if compact {
            outlined_icon(glyph, label, Some(message))
        } else {
            outlined_button(label, Some(glyph), Some(message))
        }
    };
    if release.is_album {
        let saved = app.has_saved(release.id);
        actions = actions.push(secondary(
            if saved { "Salvato" } else { "Salva" },
            if saved { Icon::Check } else { Icon::Plus },
            Message::ToggleSaveAlbum(SavedAlbum {
                id: release.id,
                title: release.title.clone(),
                artist: release.artist.clone(),
                cover_url: release.cover_url.clone(),
            }),
        ));
    }
    actions = actions.push(secondary(
        if following { "Segui già" } else { "Segui" },
        if following { Icon::Check } else { Icon::UserPlus },
        Message::ToggleFollow(artist),
    ));

    let open =
        if release.is_album { Message::Navigate(Screen::Album(release.id)) } else { release_play(release) };
    let title_button = button(wrapped(&release.title, text_width, faded(title_style, shown), 2))
        .padding(0)
        .style(theme::text_action)
        .on_press(open);

    let mut body = column![
        text(kicker).size(theme::FS_SMALL).font(theme::FONT_SEMIBOLD).color(fade(theme::TEXT, shown)),
        title_button,
        line(&meta.join(" · "), text_width, faded(MUTED_LINE, shown)),
    ]
    .spacing(8)
    .width(Length::Fill);
    // Sul telefono i pulsanti vanno sotto la copertina, a tutta larghezza.
    let below = if narrow {
        Some(actions)
    } else {
        body = body.push(Space::with_height(8)).push(actions);
        None
    };

    // La copertina sale di qualche pixel mentre entra.
    let art = container(container(cover(&app.covers, release_art(release), side)).style(theme::lifted))
        .padding(Padding { top: 10.0 * (1.0 - shown), bottom: 10.0 * shown, ..Padding::ZERO });
    let top = row![art, body].spacing(gap).align_y(Alignment::Center);

    let mut card = column![top].spacing(16);
    if let Some(actions) = below {
        card = card.push(actions);
    }
    if releases.len() > 1 {
        card = card.push(dots(releases.len(), app.home.hero.index));
    }
    Some(container(card).padding(pad).width(Length::Fill).style(theme::hero_card(tint)).into())
}

/// I puntini del carosello: quello acceso è una barretta lunga.
fn dots<'a>(count: usize, active: usize) -> Element<'a, Message> {
    let items = (0..count).map(|n| {
        let on = n == active;
        let (width, color) = if on { (26.0, theme::TEXT) } else { (7.0, theme::FAINT) };
        button(container(Space::new(Length::Fixed(width), Length::Fixed(7.0))).style(theme::bar(color, 3.5)))
            .padding([6, 3])
            .style(theme::bare)
            .on_press(Message::HeroGo(n))
            .into()
    });
    row(items).align_y(Alignment::Center).into()
}

/// Un'etichetta di conteggio sotto un numero grande: «12 h», «ore ascoltate».
pub fn stat<'a>(value: String, label: &'a str, width: f32) -> Element<'a, Message> {
    column![
        text(value).size(32.0).font(theme::FONT_SEMIBOLD).color(theme::TEXT),
        line(label, width, ON_TINT_LINE),
    ]
    .spacing(4)
    .width(Length::FillPortion(1))
    .into()
}

/// Ascolti e tempo ascoltato, in stile «Replay»: tre numeri grandi su una scheda sfumata.
pub fn listening_card<'a>(app: &App, width: f32) -> Option<Element<'a, Message>> {
    if app.stats.is_empty() && app.listened_ms == 0 {
        return None;
    }
    let plays = crate::config::taste::total_plays(&app.stats);
    let top = crate::config::taste::top_artists(&app.stats);
    let cell = (width - 2.0 * theme::PAD) / 3.0 - 16.0;
    let hours = app.listened_ms as f64 / 3_600_000.0;
    let time =
        if hours >= 1.0 { format!("{hours:.1} h") } else { format!("{} min", (hours * 60.0).round() as u64) };
    let favourite = top.first().map_or_else(|| "—".to_string(), |a| a.artist.name.clone());
    let tint = top
        .first()
        .and_then(|a| {
            a.cover_url
                .as_deref()
                .map(|url| app.covers.tint_of(Art { id: a.artist.id, title: &a.artist.name, url: Some(url) }))
        })
        .unwrap_or(theme::BRAND_TINT);
    let body = row![
        stat(time, "ascoltati", cell),
        stat(plays.to_string(), if plays == 1 { "brano riprodotto" } else { "brani riprodotti" }, cell),
        stat(super::components::fit(&favourite, cell, 32.0 * 0.62), "l'artista che ascolti di più", cell),
    ]
    .spacing(16);
    Some(
        container(
            column![
                row![
                    icon(Icon::Chart, 18.0, theme::TEXT),
                    text("Il tuo ascolto").size(theme::FS_SECTION).font(theme::FONT_SEMIBOLD),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                body,
                text("Resta su questo computer: il catalogo non sa chi ascolta cosa.")
                    .size(theme::FS_SMALL)
                    .color(fade(theme::TEXT, 0.7)),
            ]
            .spacing(16),
        )
        .padding(theme::PAD)
        .width(Length::Fill)
        .style(theme::hero_card(tint))
        .into(),
    )
}

// ---------- Sfoglia tutto ----------

/// Altezza di una scheda di «Sfoglia tutto».
const BROWSE_HEIGHT: f32 = 104.0;

/// Una scheda: nome, riga di spiegazione e una grande icona sull'angolo.
fn browse_card<'a>(
    index: usize,
    glyph: Icon,
    title: &'a str,
    detail: &'a str,
    on_press: Message,
    width: f32,
) -> Element<'a, Message> {
    let color = theme::BROWSE[index % theme::BROWSE.len()];
    let label = column![
        line(
            title,
            width - 90.0,
            Line { size: theme::FS_TITLE, font: theme::FONT_SEMIBOLD, color: Some(theme::TEXT) }
        ),
        line(detail, width - 90.0, ON_TINT_LINE),
    ]
    .spacing(4)
    .width(Length::Fill);
    let art = container(icon(glyph, 52.0, Color { a: 0.42, ..Color::WHITE }))
        .height(Length::Fill)
        .align_y(alignment::Vertical::Bottom);
    button(row![label, art].spacing(8).align_y(Alignment::Start))
        .width(Length::Fill)
        .height(BROWSE_HEIGHT)
        .padding(16)
        .style(theme::browse_card(color))
        .on_press(on_press)
        .into()
}

/// «Sfoglia tutto»: le porte d'ingresso al catalogo, a colori, due o tre per riga.
pub fn browse_grid<'a>(app: &App, width: f32) -> Element<'a, Message> {
    use crate::app::LibraryFilter;
    let mut cards: Vec<(Icon, &str, &str, Message)> = vec![
        (Icon::Bell, "Novità", "Le uscite dei tuoi artisti", Message::Navigate(Screen::News)),
        (Icon::Library, "Album", "Con la copertina grande", Message::OpenLibrary(LibraryFilter::Albums)),
        (Icon::Account, "Artisti", "Tutti quelli del catalogo", Message::OpenLibrary(LibraryFilter::Artists)),
        (Icon::Note, "Brani", "Dalla A alla Z", Message::OpenLibrary(LibraryFilter::All)),
        (
            Icon::Download,
            "Scaricabili",
            "Quelli che puoi tenere",
            Message::OpenLibrary(LibraryFilter::Downloadable),
        ),
        (Icon::Play, "Singoli", "Un brano alla volta", Message::OpenLibrary(LibraryFilter::Singles)),
        (Icon::HeartFill, "Ti piacciono", "I brani con il cuore", Message::Navigate(Screen::Liked)),
        (Icon::People, "Jam", "Ascolta insieme ad altri", Message::Navigate(Screen::Jam)),
    ];
    if !app.home.feed.mixes.is_empty() {
        cards.insert(1, (Icon::Sparkle, "Mix per te", "Fatti con ciò che ascolti", Message::PlayMix(0)));
    }
    let columns = if width >= 700.0 { 3 } else { 2 };
    let cell = (width - theme::GRID_GAP * (columns as f32 - 1.0)) / columns as f32;
    let mut rows = column![].spacing(theme::GRID_GAP);
    let mut cards = cards.into_iter().enumerate();
    loop {
        let mut line_cells = row![].spacing(theme::GRID_GAP);
        let mut filled = 0;
        for (index, (glyph, title, detail, message)) in cards.by_ref().take(columns) {
            line_cells = line_cells.push(browse_card(index, glyph, title, detail, message, cell));
            filled += 1;
        }
        if filled == 0 {
            break;
        }
        for _ in filled..columns {
            line_cells = line_cells.push(Space::with_width(Length::Fill));
        }
        rows = rows.push(line_cells);
    }
    rows.into()
}
