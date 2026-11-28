//! «Cerca»: the results of `CatalogView::search` in one list of 48 dp rows. Typing happens in the title
//! bar field, which sends `CatalogCmd::SearchInput` on every key: the 300 ms wait and «the last search wins»
//! belong to the core, there is no timer here.
//!
//! Three groups, in this order: the official catalogue (the artists of `search.artists`, which a
//! click brings to the server and opens; the tracks of `search.sources`, which the server imports on play;
//! the albums and playlists of `search.playlists`, which open `source_page`), what the server already has
//! (`search.results`: tracks, then artists and albums that open their pages) and Internet Archive
//! (`search.discover`). Artists, albums and playlists are `ListRow`s, at most [`CARDS`] per section, and
//! flow into columns (desktop v2 §2: they are not an ordered list); the track sections keep one column.
//! Official and Archive tracks are `SourceRow`s: play is one core command (`ExternalCmd::PlaySource` or
//! `PlayArchive`) and while the core imports the row says «Apro…». Server tracks are `TrackRow`s: double
//! click or Enter sends `PlaybackCmd::Play`; «Altro» opens `Shell`'s track menu.
//!
//! The list plan is a pure function of the three loads (`plan`), flowed into lines for the page's column
//! count (`flow_rows`), and rebuilt only when a load or the column count changes. Internet Archive's netlabel
//! libraries stay out: they are a screen, not a row.

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, UniformListScrollHandle, Window, div, uniform_list,
};
use psytify_core::api::{
    AlbumSummary, ArtistSummary, DiscoverItem, MetaArtist, SearchResults, SourceItem, SourceKind,
    SourcePlaylist, Track,
};
use psytify_core::catalog::Load;
use psytify_core::command::{CatalogCmd, ExternalCmd, LibraryCmd, PlaybackCmd};
use psytify_core::snapshot::{CatalogView, DownloadsView, ExternalView, SearchView};
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_ERROR};
use psytify_design::tokens::size::TRACK_ROW_H;
use psytify_design::tokens::space::{SPACE_3, SPACE_5};
use psytify_design::tokens::type_::TYPE_BODY;
use uuid::Uuid;

use super::page_frame::PageFrame;
use super::track_list::{download_command, download_state, with_links};
use super::track_menu::TrackMenu;
use super::{Page, PageEvent, play_state};
use crate::components::button::Button;
use crate::components::frame::{Flow, Frame};
use crate::components::indicators::{LoadingIndicator, PlayState};
use crate::components::list_row::{ListRow, RowArt};
use crate::components::overlay::EmptyState;
use crate::components::playing_spot::PlayingSpot;
use crate::components::section::{note_at, section_heading_at};
use crate::components::source_row::{SourceData, SourceRow};
use crate::components::track_row::{TrackData, TrackRow};
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{TypeStyle, col, dp};

/// Official catalogue tracks shown: the others are found by refining the search.
const OFFICIAL_TRACKS: usize = 12;
/// Artists, albums and playlists shown per section.
const CARDS: usize = 6;

/// The sections, for the headers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    OfficialArtists,
    Official,
    OfficialAlbums,
    OfficialPlaylists,
    Server,
    ServerArtists,
    ServerAlbums,
    Archive,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Self::OfficialArtists => "Artisti",
            Self::Official => "Catalogo ufficiale",
            Self::OfficialAlbums => "Album",
            Self::OfficialPlaylists => "Playlist",
            Self::Server => "Già nel catalogo",
            Self::ServerArtists => "Artisti nel catalogo",
            Self::ServerAlbums => "Album nel catalogo",
            Self::Archive => "Da Internet Archive",
        }
    }

    /// The count next to the title: tracks, artists, albums or playlists.
    fn count(self, count: usize) -> String {
        match self {
            Self::OfficialArtists | Self::ServerArtists if count == 1 => "1 artista".to_owned(),
            Self::OfficialArtists | Self::ServerArtists => format!("{count} artisti"),
            Self::OfficialAlbums | Self::ServerAlbums => format!("{count} album"),
            Self::OfficialPlaylists => format!("{count} playlist"),
            Self::Official | Self::Server | Self::Archive => format::tracks(count),
        }
    }

    fn waiting(self) -> &'static str {
        match self {
            Self::Official => "Cerco nel catalogo ufficiale…",
            Self::Archive => "Cerco su Internet Archive…",
            // Only the track sections wait: artists, albums and playlists appear when there are some.
            _ => "Cerco…",
        }
    }
}

/// A row of the list, in the order it is drawn. The indices are into the core's lists.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Row {
    Heading(Section, usize),
    /// The section is still searching.
    Waiting(Section),
    /// The section did not arrive: the error text.
    Problem(Section, String),
    Source(usize),
    Track(usize),
    Archive(usize),
    OfficialArtist(usize),
    OfficialAlbum(usize),
    OfficialPlaylist(usize),
    ServerArtist(usize),
    ServerAlbum(usize),
    /// A line of cards in flowing columns (artists, albums, playlists of one section).
    Line(Vec<Row>),
}

impl Row {
    fn is_card(&self) -> bool {
        matches!(
            self,
            Self::OfficialArtist(_)
                | Self::OfficialAlbum(_)
                | Self::OfficialPlaylist(_)
                | Self::ServerArtist(_)
                | Self::ServerAlbum(_)
        )
    }

    /// A stable number for a card's element id.
    fn card_key(&self) -> usize {
        match *self {
            Self::OfficialArtist(ix) => ix,
            Self::OfficialAlbum(ix) => 100 + ix,
            Self::OfficialPlaylist(ix) => 200 + ix,
            Self::ServerArtist(ix) => 300 + ix,
            Self::ServerAlbum(ix) => 400 + ix,
            _ => usize::MAX,
        }
    }
}

/// The card runs of `rows` flowed into lines of `flow.columns`, filled column by column; other rows stay.
fn flow_rows(rows: Vec<Row>, flow: Flow) -> Vec<Row> {
    if flow.columns <= 1 {
        return rows;
    }
    let mut out = Vec::with_capacity(rows.len());
    let mut run: Vec<Row> = Vec::new();
    let flush = |run: &mut Vec<Row>, out: &mut Vec<Row>| {
        let count = run.len();
        for line in 0..flow.rows(count) {
            out.push(Row::Line(flow.cells(line, count).filter_map(|ix| run.get(ix).cloned()).collect()));
        }
        run.clear();
    };
    for row in rows {
        if row.is_card() {
            run.push(row);
        } else {
            flush(&mut run, &mut out);
            out.push(row);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Artists, albums and playlists besides the tracks (already cut to [`CARDS`] by the counter).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Cards {
    official_artists: usize,
    official_albums: usize,
    official_playlists: usize,
    server_artists: usize,
    server_albums: usize,
}

/// What the page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Screen {
    /// No search.
    Idle,
    /// The server's tracks are arriving (the other sections wait).
    Loading,
    /// No row and the server gave an error.
    Failed(String),
    /// No result anywhere.
    Empty,
    List,
}

/// The state of a load, reduced to what the plan needs: how many items there are.
#[derive(Clone, Copy, Debug)]
enum Counted<'a> {
    Idle,
    Loading,
    Ready(usize),
    Failed(&'a psytify_core::error::AppError),
}

impl<'a> Counted<'a> {
    fn of<T>(load: &'a Load<T>, len: impl FnOnce(&T) -> usize) -> Self {
        match load {
            Load::Idle => Self::Idle,
            Load::Loading => Self::Loading,
            Load::Ready(value) => Self::Ready(len(value)),
            Load::Failed(error) => Self::Failed(error),
        }
    }

    fn rows(self) -> usize {
        match self {
            Self::Ready(count) => count,
            _ => 0,
        }
    }
}

struct Plan {
    screen: Screen,
    rows: Vec<Row>,
}

/// The rows of a simple section (title and rows), if there is at least one.
fn cards(rows: &mut Vec<Row>, which: Section, count: usize, row: fn(usize) -> Row) {
    if count > 0 {
        rows.push(Row::Heading(which, count));
        rows.extend((0..count).map(row));
    }
}

#[cfg(test)]
fn plan(results: Counted, sources: Counted, discover: Counted) -> Plan {
    plan_all(results, sources, discover, Cards::default())
}

/// Which rows to show and in what order. Pure: it depends only on the loads and the counts.
fn plan_all(results: Counted, sources: Counted, discover: Counted, extra: Cards) -> Plan {
    match results {
        Counted::Idle => return Plan { screen: Screen::Idle, rows: Vec::new() },
        Counted::Loading => return Plan { screen: Screen::Loading, rows: Vec::new() },
        Counted::Ready(_) | Counted::Failed(_) => {}
    }
    let mut rows = Vec::new();
    // The official catalogue first: it is where discovery happens; then what the server plays at once. As in the
    // While the official tracks arrive its artists are not shown.
    if !matches!(sources, Counted::Loading) {
        cards(&mut rows, Section::OfficialArtists, extra.official_artists, Row::OfficialArtist);
    }
    let shown = sources.rows().min(OFFICIAL_TRACKS);
    section(&mut rows, Section::Official, sources, shown, Row::Source);
    cards(&mut rows, Section::OfficialAlbums, extra.official_albums, Row::OfficialAlbum);
    cards(&mut rows, Section::OfficialPlaylists, extra.official_playlists, Row::OfficialPlaylist);
    let tracks = results.rows();
    if tracks > 0 {
        rows.push(Row::Heading(Section::Server, tracks));
        rows.extend((0..tracks).map(Row::Track));
    }
    cards(&mut rows, Section::ServerArtists, extra.server_artists, Row::ServerArtist);
    cards(&mut rows, Section::ServerAlbums, extra.server_albums, Row::ServerAlbum);
    section(&mut rows, Section::Archive, discover, discover.rows(), Row::Archive);
    let screen = match results {
        Counted::Failed(error) if rows.is_empty() => Screen::Failed(error.to_string()),
        _ if rows.is_empty() => Screen::Empty,
        _ => Screen::List,
    };
    Plan { screen, rows }
}

/// Header and rows of a section that comes from an external source, or its state.
fn section(rows: &mut Vec<Row>, which: Section, load: Counted, shown: usize, row: fn(usize) -> Row) {
    match load {
        Counted::Idle | Counted::Ready(0) => {}
        Counted::Loading => {
            rows.push(Row::Heading(which, 0));
            rows.push(Row::Waiting(which));
        }
        Counted::Failed(error) => {
            rows.push(Row::Heading(which, 0));
            rows.push(Row::Problem(which, error.to_string()));
        }
        Counted::Ready(_) => {
            rows.push(Row::Heading(which, shown));
            rows.extend((0..shown).map(row));
        }
    }
}

/// The texts of a track row, ready: the view does not format or allocate strings per row at every draw.
struct RowText {
    title: SharedString,
    artist: SharedString,
    duration: SharedString,
    cover_url: Option<SharedString>,
}

impl RowText {
    fn of(track: &Track) -> Self {
        Self {
            title: track.title.clone().into(),
            artist: track.artist.name.clone().into(),
            duration: format::clock_ms(track.duration_ms).into(),
            cover_url: track.cover_url.clone().map(SharedString::from),
        }
    }
}

/// «Artista» for the official catalogue, «YouTube · Artista» for the other sources.
fn source_detail(item: &SourceItem) -> String {
    if item.source == SourceKind::Deezer {
        item.artist.clone()
    } else {
        format!("{} · {}", item.source.label(), item.artist)
    }
}

pub(super) fn source_data(position: usize, item: &SourceItem) -> SourceData {
    SourceData {
        index: u32::try_from(position + 1).unwrap_or(u32::MAX),
        title: item.title.clone().into(),
        detail: source_detail(item).into(),
        duration: item.duration_ms.map(format::clock_ms).unwrap_or_default().into(),
        cover_url: item.cover_url.clone().map(SharedString::from),
    }
}

/// «Autore · CC BY · 2019 · Lossless».
fn archive_data(position: usize, item: &DiscoverItem) -> SourceData {
    let mut detail: Vec<&str> = Vec::new();
    if let Some(creator) = &item.creator {
        detail.push(creator);
    }
    detail.push(format::licence_label(&item.licence));
    if let Some(year) = &item.year {
        detail.push(year);
    }
    if item.lossless {
        detail.push("Lossless");
    }
    SourceData {
        index: u32::try_from(position + 1).unwrap_or(u32::MAX),
        title: item.title.clone().into(),
        detail: detail.join(" · ").into(),
        duration: SharedString::default(),
        cover_url: item.cover_url.clone().map(SharedString::from),
    }
}

/// The texts of an artist, album or playlist row, ready.
#[derive(Clone, Debug, PartialEq)]
struct Card {
    title: SharedString,
    detail: SharedString,
    cover_url: Option<SharedString>,
}

fn card(title: &str, detail: String, cover_url: Option<&str>) -> Card {
    Card {
        title: title.to_owned().into(),
        detail: detail.into(),
        cover_url: cover_url.map(|u| u.to_owned().into()),
    }
}

/// «Artista · 12 brani · 2 album».
fn server_artist_card(artist: &ArtistSummary) -> Card {
    let mut detail = vec!["Artista".to_owned(), format::tracks(artist.track_count as usize)];
    if artist.album_count > 0 {
        detail.push(format!("{} album", artist.album_count));
    }
    card(&artist.name, detail.join(" · "), artist.cover_url.as_deref())
}

/// «Album · Artista · 10 brani».
fn server_album_card(album: &AlbumSummary) -> Card {
    let detail = format!("Album · {} · {}", album.artist.name, format::tracks(album.track_count as usize));
    card(&album.title, detail, album.cover_url.as_deref())
}

/// «Artista · 12.400 fan».
fn official_artist_card(artist: &MetaArtist) -> Card {
    let detail = if artist.fans > 0 {
        format!("Artista · {}", format::fans(artist.fans))
    } else {
        "Artista".to_owned()
    };
    card(&artist.name, detail, artist.image_url.as_deref())
}

/// «Album · Autore · 2021».
fn release_card(release: &SourcePlaylist) -> Card {
    let mut detail = vec![release.kind_label(), release.owner.as_str()];
    if let Some(year) = release.year() {
        detail.push(year);
    }
    detail.retain(|part| !part.is_empty());
    card(&release.title, detail.join(" · "), release.cover_url.as_deref())
}

pub struct SearchPage {
    core: Entity<CoreModel>,
    catalog: Arc<CatalogView>,
    downloads: Arc<DownloadsView>,
    external: Arc<ExternalView>,
    liked: Arc<HashSet<Uuid>>,
    /// The playing track and how to show it, for the equaliser on its row.
    now: Option<(Uuid, PlayState)>,
    screen: Screen,
    rows: Vec<Row>,
    /// For each row, in parallel to `rows`: the header count («3 brani»), empty for the
    /// others. Together with each row's texts it is rebuilt when the results change.
    labels: Vec<SharedString>,
    tracks: Vec<RowText>,
    sources: Vec<SourceData>,
    archive: Vec<SourceData>,
    /// The albums and playlists of the official catalogue (at most [`CARDS`] per kind) and the texts of each row
    /// of artist, album and playlist.
    official_albums: Vec<SourcePlaylist>,
    official_playlists: Vec<SourcePlaylist>,
    cards: Vec<(Row, Card)>,
    scroll: UniformListScrollHandle,
    frame: PageFrame,
    /// The enter step of each row (a heading starts one).
    sections: Vec<usize>,
    /// `--play-first-official`: plays the first track of the official catalogue as soon as it is there, once only.
    autoplay_first: bool,
    /// The equaliser spot of the playing track: `Shell` paints it (BACKLOG 23).
    spot: PlayingSpot,
    _changes: Subscription,
}

impl gpui::EventEmitter<PageEvent> for SearchPage {}

impl SearchPage {
    /// The equaliser spot of this view, for `Shell`'s layer.
    pub fn spot(&self) -> PlayingSpot {
        self.spot.clone()
    }

    pub fn new(core: Entity<CoreModel>, autoplay_first: bool, cx: &mut Context<Self>) -> Self {
        let changes =
            cx.subscribe(&core, |this, core, changed: &Changed, cx| this.refresh(&core, *changed, cx));
        let snapshot = core.read(cx).snapshot();
        let mut page = Self {
            catalog: Arc::clone(&snapshot.catalog),
            downloads: Arc::clone(&snapshot.downloads),
            external: Arc::clone(&snapshot.external),
            liked: core.read(cx).liked(),
            now: play_state(&snapshot.playback)
                .and_then(|state| snapshot.playback.queue.current().map(|track| (track.id, state))),
            screen: Screen::Idle,
            rows: Vec::new(),
            labels: Vec::new(),
            tracks: Vec::new(),
            sources: Vec::new(),
            archive: Vec::new(),
            official_albums: Vec::new(),
            official_playlists: Vec::new(),
            cards: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            frame: PageFrame::default(),
            sections: Vec::new(),
            autoplay_first,
            core,
            spot: PlayingSpot::new(),
            _changes: changes,
        };
        page.rebuild_rows();
        page.autoplay(cx);
        page
    }

    /// The page comes into view (`Shell::page_shown`).
    pub fn enter(&mut self, rise: bool, cx: &mut Context<Self>) {
        self.frame.enter(rise, cx);
        cx.notify();
    }

    /// A new width: a new column count flows the cards again.
    fn set_frame(&mut self, frame: Frame, cx: &mut Context<Self>) {
        if frame == self.frame.frame {
            return;
        }
        let reflow = frame.flow != self.frame.frame.flow;
        self.frame.frame = frame;
        if reflow {
            self.rebuild_rows();
        }
        cx.notify();
    }

    fn refresh(&mut self, core: &Entity<CoreModel>, changed: Changed, cx: &mut Context<Self>) {
        let snapshot = core.read(cx).snapshot();
        let mut dirty = false;
        if changed.catalog && !Arc::ptr_eq(&snapshot.catalog, &self.catalog) {
            let same = same_search(&snapshot.catalog.search, &self.catalog.search);
            self.catalog = Arc::clone(&snapshot.catalog);
            if !same {
                self.rebuild_rows();
                self.autoplay(cx);
                dirty = true;
            }
        }
        if changed.external && !Arc::ptr_eq(&snapshot.external, &self.external) {
            // Only the imports change what is seen here: the events and the artist cards do not.
            dirty |= snapshot.external.imports != self.external.imports;
            self.external = Arc::clone(&snapshot.external);
        }
        if changed.playback {
            let now = play_state(&snapshot.playback)
                .and_then(|state| snapshot.playback.queue.current().map(|track| (track.id, state)));
            if now != self.now {
                self.now = now;
                dirty = true;
            }
        }
        if changed.library {
            self.liked = core.read(cx).liked();
            dirty = true;
        }
        if changed.downloads {
            self.downloads = Arc::clone(&snapshot.downloads);
            dirty = true;
        }
        if dirty {
            cx.notify();
        }
    }

    /// With `--play-first-official`, the first result of the official catalogue starts as soon as it arrives.
    fn autoplay(&mut self, cx: &gpui::App) {
        if self.autoplay_first && !self.sources.is_empty() {
            self.autoplay_first = false;
            self.play_source(0, cx);
        }
    }

    fn rebuild_rows(&mut self) {
        let search = &self.catalog.search;
        let lists = search.playlists.ready().map_or(&[][..], |lists| &lists[..]);
        self.official_albums = lists.iter().filter(|l| l.kind != "playlist").take(CARDS).cloned().collect();
        self.official_playlists =
            lists.iter().filter(|l| l.kind == "playlist").take(CARDS).cloned().collect();
        let results = search.results.ready();
        let server_artists = results.map_or(&[][..], |r| &r.artists[..]);
        let server_albums = results.map_or(&[][..], |r| &r.albums[..]);
        let extra = Cards {
            official_artists: search.artists.len().min(CARDS),
            official_albums: self.official_albums.len(),
            official_playlists: self.official_playlists.len(),
            server_artists: server_artists.len().min(CARDS),
            server_albums: server_albums.len().min(CARDS),
        };
        self.cards = search
            .artists
            .iter()
            .take(CARDS)
            .enumerate()
            .map(|(ix, artist)| (Row::OfficialArtist(ix), official_artist_card(artist)))
            .chain(
                self.official_albums
                    .iter()
                    .enumerate()
                    .map(|(ix, r)| (Row::OfficialAlbum(ix), release_card(r))),
            )
            .chain(
                self.official_playlists
                    .iter()
                    .enumerate()
                    .map(|(ix, r)| (Row::OfficialPlaylist(ix), release_card(r))),
            )
            .chain(
                server_artists
                    .iter()
                    .take(CARDS)
                    .enumerate()
                    .map(|(ix, a)| (Row::ServerArtist(ix), server_artist_card(a))),
            )
            .chain(
                server_albums
                    .iter()
                    .take(CARDS)
                    .enumerate()
                    .map(|(ix, a)| (Row::ServerAlbum(ix), server_album_card(a))),
            )
            .collect();
        let Plan { screen, rows } = plan_all(
            Counted::of(&search.results, |results| results.tracks.len()),
            Counted::of(&search.sources, |sources| sources.len()),
            Counted::of(&search.discover, |items| items.len()),
            extra,
        );
        self.screen = screen;
        self.rows = flow_rows(rows, self.frame.frame.flow);
        let mut section = 0;
        self.sections = self
            .rows
            .iter()
            .map(|row| {
                section += usize::from(matches!(row, Row::Heading(..)));
                section
            })
            .collect();
        self.tracks =
            search.results.ready().map_or_else(Vec::new, |r| r.tracks.iter().map(RowText::of).collect());
        self.sources = search.sources.ready().map_or_else(Vec::new, |items| {
            items.iter().take(OFFICIAL_TRACKS).enumerate().map(|(ix, item)| source_data(ix, item)).collect()
        });
        self.archive = search.discover.ready().map_or_else(Vec::new, |items| {
            items.iter().enumerate().map(|(ix, item)| archive_data(ix, item)).collect()
        });
        self.labels = self
            .rows
            .iter()
            .map(|row| match row {
                Row::Heading(section, count) if *count > 0 => section.count(*count).into(),
                _ => SharedString::default(),
            })
            .collect();
    }

    fn results(&self) -> Option<&Arc<SearchResults>> {
        self.catalog.search.results.ready()
    }

    /// Double click or Enter on a server track: the queue becomes the results and the `start` row plays.
    fn play(&self, start: usize, cx: &gpui::App) {
        let Some(results) = self.results() else { return };
        let tracks: Arc<[Track]> = results.tracks.as_slice().into();
        self.core.read(cx).playback(PlaybackCmd::Play { tracks, start });
    }

    /// Double click or Enter on a result of the official catalogue (or of another source): the core
    /// imports and plays it. One request at a time: while one is in progress the core ignores the others.
    fn play_source(&self, ix: usize, cx: &gpui::App) {
        let Some(item) = self.catalog.search.sources.ready().and_then(|items| items.get(ix)) else { return };
        self.core.read(cx).external(ExternalCmd::PlaySource {
            source: item.source,
            id: item.id.clone(),
            video: false,
        });
    }

    fn play_archive(&self, ix: usize, cx: &gpui::App) {
        let Some(item) = self.catalog.search.discover.ready().and_then(|items| items.get(ix)) else { return };
        self.core.read(cx).external(ExternalCmd::PlayArchive { identifier: item.identifier.clone() });
    }

    /// List row `ix`, in its enter move.
    fn row(&self, ix: usize, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let row = self.row_at(ix, cx)?;
        // A fixed height: the uniform list measures only the first row.
        let row = div().h(dp(TRACK_ROW_H)).w_full().child(row).into_any_element();
        Some(self.frame.wrap(row, self.sections.get(ix).copied().unwrap_or(0), window))
    }

    fn row_at(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let margin = self.frame.margin();
        Some(match self.rows.get(ix)? {
            Row::Heading(section, _) => {
                section_heading_at(section.title().into(), self.labels.get(ix)?.clone(), margin)
                    .into_any_element()
            }
            Row::Waiting(section) => {
                note_at(LoadingIndicator::new().caption(section.waiting()), margin).into_any_element()
            }
            Row::Problem(section, text) => note_at(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_ERROR))
                    .child(format!("{}: {text}", section.title())),
                margin,
            )
            .into_any_element(),
            Row::Line(cells) => {
                let flow = self.frame.frame.flow;
                let cells = cells.iter().filter_map(|card| self.card_row(card, cx));
                flow.row(margin - SPACE_3, cells).into_any_element()
            }
            Row::Source(position) => {
                let position = *position;
                let item = self.catalog.search.sources.ready()?.get(position)?;
                let busy = self.external.importing(item.source, &item.id);
                self.frame.row(
                    SourceRow::new(("source-row", position), self.sources.get(position)?.clone())
                        .busy(busy)
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            // As for tracks: a click does nothing; double click or Enter plays.
                            if event.click_count() >= 2 || matches!(event, ClickEvent::Keyboard(_)) {
                                this.play_source(position, cx);
                            }
                        })),
                )
            }
            Row::Archive(position) => {
                let position = *position;
                let item = self.catalog.search.discover.ready()?.get(position)?;
                let busy = self.external.importing_archive(&item.identifier);
                self.frame.row(
                    SourceRow::new(("archive-row", position), self.archive.get(position)?.clone())
                        .busy(busy)
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            if event.click_count() >= 2 || matches!(event, ClickEvent::Keyboard(_)) {
                                this.play_archive(position, cx);
                            }
                        })),
                )
            }
            card @ (Row::OfficialArtist(_)
            | Row::OfficialAlbum(_)
            | Row::OfficialPlaylist(_)
            | Row::ServerArtist(_)
            | Row::ServerAlbum(_)) => self.frame.row(self.card_row(card, cx)?),
            Row::Track(position) => {
                let position = *position;
                let track = self.results()?.tracks.get(position)?;
                let text = self.tracks.get(position)?;
                let data = TrackData {
                    index: u32::try_from(position + 1).unwrap_or(u32::MAX),
                    title: text.title.clone(),
                    artist: text.artist.clone(),
                    album: SharedString::default(),
                    duration: text.duration.clone(),
                    liked: self.liked.contains(&track.id),
                    download: download_state(&self.downloads, track),
                    available: true,
                    cover_url: text.cover_url.clone(),
                };
                let liked_track = track.clone();
                let download_track = track.clone();
                let menu_track = track.clone();
                let mut row = TrackRow::new(("search-row", position), data)
                    .show_album(false)
                    .on_more(cx.listener(move |_, event: &ClickEvent, _, cx| {
                        cx.emit(PageEvent::TrackMenu(Box::new(TrackMenu {
                            track: menu_track.clone(),
                            at: event.position(),
                            from_playlist: None,
                        })));
                    }))
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        // A click does nothing; double click or Enter plays.
                        if event.click_count() >= 2 || matches!(event, ClickEvent::Keyboard(_)) {
                            this.play(position, cx);
                        }
                    }))
                    .on_like(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.core.read(cx).library(LibraryCmd::ToggleLike(liked_track.clone()));
                    }))
                    .on_download(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.core.read(cx).downloads(download_command(&this.downloads, &download_track));
                    }));
                row = with_links(row, track, cx);
                if let Some((id, state)) = self.now
                    && id == track.id
                {
                    row = row.playing_at(state, &self.spot);
                }
                self.frame.row(row)
            }
        })
    }
}

impl SearchPage {
    /// An artist, album or playlist row: a click opens its page.
    fn card_row(&self, row: &Row, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ix = row.card_key();
        let card = &self.cards.iter().find(|(known, _)| known == row)?.1;
        let round = matches!(row, Row::OfficialArtist(_) | Row::ServerArtist(_));
        let art =
            if round { RowArt::Round(card.cover_url.clone()) } else { RowArt::Cover(card.cover_url.clone()) };
        let event = match *row {
            Row::OfficialArtist(position) => {
                // An official artist must first be brought to the server: the core does it and `Shell`
                // opens the page when it is there (`CatalogEvent::MetaArtistOpened`).
                let id = self.catalog.search.artists.get(position)?.id;
                let core = self.core.clone();
                return Some(
                    ListRow::new(("search-card", ix), art, card.title.clone())
                        .inset(SPACE_3)
                        .detail(card.detail.clone())
                        .on_click(move |_, _, cx| core.read(cx).catalog(CatalogCmd::OpenMetaArtist(id)))
                        .into_any_element(),
                );
            }
            Row::OfficialAlbum(position) => {
                PageEvent::OpenSource(Box::new(self.official_albums.get(position)?.clone()))
            }
            Row::OfficialPlaylist(position) => {
                PageEvent::OpenSource(Box::new(self.official_playlists.get(position)?.clone()))
            }
            Row::ServerArtist(position) => {
                PageEvent::Open(Page::Artist(self.results()?.artists.get(position)?.id))
            }
            Row::ServerAlbum(position) => {
                PageEvent::Open(Page::Album(self.results()?.albums.get(position)?.id))
            }
            _ => return None,
        };
        Some(
            ListRow::new(("search-card", ix), art, card.title.clone())
                .inset(SPACE_3)
                .detail(card.detail.clone())
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(event.clone())))
                .into_any_element(),
        )
    }
}

/// The two views have the same results (the same `Arc`s): a change elsewhere in the catalogue does not
/// redraw the page.
fn same_search(a: &SearchView, b: &SearchView) -> bool {
    same_load(&a.results, &b.results)
        && same_load(&a.sources, &b.sources)
        && same_load(&a.discover, &b.discover)
        && same_load(&a.playlists, &b.playlists)
        && Arc::ptr_eq(&a.artists, &b.artists)
        && a.submitted == b.submitted
}

fn same_load<T: ?Sized>(a: &Load<Arc<T>>, b: &Load<Arc<T>>) -> bool {
    match (a, b) {
        (Load::Ready(x), Load::Ready(y)) => Arc::ptr_eq(x, y),
        (Load::Idle, Load::Idle) | (Load::Loading, Load::Loading) => true,
        (Load::Failed(x), Load::Failed(y)) => x == y,
        _ => false,
    }
}

impl Render for SearchPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The spot is marked again in this frame's paint, if the playing row is visible.
        self.spot.clear();
        self.frame.begin(window, None);
        let content: AnyElement = match &self.screen {
            Screen::Idle => centered(EmptyState::crate_(
                "Cerca nel catalogo",
                "Scrivi nella barra in alto: i risultati compaiono mentre digiti.",
            ))
            .into_any_element(),
            Screen::Loading => centered(LoadingIndicator::new().caption("Cerco…")).into_any_element(),
            Screen::Failed(error) => {
                let core = self.core.clone();
                let query = self.catalog.search.submitted.clone();
                centered(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(dp(SPACE_5))
                        .child(div().type_role(TYPE_BODY).text_color(col(TEXT_ERROR)).child(error.clone()))
                        .child(Button::new("search-retry", "Riprova").secondary().on_click(
                            move |_, _, cx| {
                                core.read(cx).catalog(CatalogCmd::Search(query.clone()));
                            },
                        )),
                )
                .into_any_element()
            }
            Screen::Empty => centered(EmptyState::crate_(
                "Nessun risultato",
                "Prova con un altro titolo o con il nome di un artista.",
            ))
            .into_any_element(),
            Screen::List => {
                let list = uniform_list(
                    "search-results",
                    self.rows.len(),
                    cx.processor(|this: &mut Self, range: std::ops::Range<usize>, window, cx| {
                        range.filter_map(|ix| this.row(ix, window, cx)).collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll)
                .size_full();
                div().size_full().pt(dp(SPACE_3)).child(list).into_any_element()
            }
        };
        div()
            .relative()
            .size_full()
            .bg(col(SURFACE_CANVAS))
            .child(self.frame.measure(cx.entity().downgrade(), Self::set_frame))
            .child(content)
    }
}

/// Content in the middle, a third from the top, like the guidelines' empty states.
fn centered(content: impl IntoElement) -> gpui::Div {
    div().size_full().flex().justify_center().pt(dp(96.0)).child(content)
}

#[cfg(test)]
mod tests {
    use psytify_core::error::AppError;

    use super::*;

    fn headings(rows: &[Row]) -> Vec<Section> {
        rows.iter()
            .filter_map(|row| match row {
                Row::Heading(section, _) => Some(*section),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn nothing_searched_is_the_idle_page_and_the_server_loading_hides_the_rest() {
        let idle = plan(Counted::Idle, Counted::Ready(3), Counted::Idle);
        assert_eq!((idle.screen, idle.rows), (Screen::Idle, Vec::new()));
        let loading = plan(Counted::Loading, Counted::Ready(3), Counted::Ready(2));
        assert_eq!((loading.screen, loading.rows), (Screen::Loading, Vec::new()));
    }

    #[test]
    fn the_official_catalogue_comes_first_then_the_server_then_archive() {
        let plan = plan(Counted::Ready(2), Counted::Ready(3), Counted::Ready(1));
        assert_eq!(plan.screen, Screen::List);
        assert_eq!(
            plan.rows,
            vec![
                Row::Heading(Section::Official, 3),
                Row::Source(0),
                Row::Source(1),
                Row::Source(2),
                Row::Heading(Section::Server, 2),
                Row::Track(0),
                Row::Track(1),
                Row::Heading(Section::Archive, 1),
                Row::Archive(0),
            ]
        );
    }

    #[test]
    fn artists_albums_and_playlists_sit_with_their_catalogue() {
        let extra = Cards {
            official_artists: 1,
            official_albums: 1,
            official_playlists: 1,
            server_artists: 1,
            server_albums: 1,
        };
        let plan = plan_all(Counted::Ready(1), Counted::Ready(1), Counted::Idle, extra);
        assert_eq!(
            headings(&plan.rows),
            [
                Section::OfficialArtists,
                Section::Official,
                Section::OfficialAlbums,
                Section::OfficialPlaylists,
                Section::Server,
                Section::ServerArtists,
                Section::ServerAlbums,
            ]
        );
        // While the official tracks arrive, its artists wait.
        let waiting = plan_all(Counted::Ready(0), Counted::Loading, Counted::Idle, extra);
        assert!(!headings(&waiting.rows).contains(&Section::OfficialArtists));
        assert_eq!(Section::ServerArtists.count(1), "1 artista");
        assert_eq!(Section::OfficialAlbums.count(3), "3 album");
    }

    #[test]
    fn cards_flow_into_lines_and_tracks_keep_one_column() {
        let rows = vec![
            Row::Heading(Section::OfficialArtists, 3),
            Row::OfficialArtist(0),
            Row::OfficialArtist(1),
            Row::OfficialArtist(2),
            Row::Heading(Section::Official, 1),
            Row::Source(0),
        ];
        let flowed = flow_rows(rows.clone(), Flow { columns: 2 });
        assert_eq!(
            flowed,
            vec![
                Row::Heading(Section::OfficialArtists, 3),
                Row::Line(vec![Row::OfficialArtist(0), Row::OfficialArtist(2)]),
                Row::Line(vec![Row::OfficialArtist(1)]),
                Row::Heading(Section::Official, 1),
                Row::Source(0),
            ]
        );
        assert_eq!(flow_rows(rows.clone(), Flow { columns: 1 }), rows);
    }

    #[test]
    fn only_twelve_official_tracks_are_listed() {
        let plan = plan(Counted::Ready(0), Counted::Ready(40), Counted::Idle);
        assert_eq!(plan.rows.len(), 1 + OFFICIAL_TRACKS);
        assert_eq!(plan.rows[0], Row::Heading(Section::Official, OFFICIAL_TRACKS));
        assert_eq!(plan.rows.last(), Some(&Row::Source(OFFICIAL_TRACKS - 1)));
    }

    #[test]
    fn official_results_show_even_when_the_server_has_no_tracks() {
        let plan = plan(Counted::Ready(0), Counted::Ready(2), Counted::Ready(0));
        assert_eq!(plan.screen, Screen::List);
        assert_eq!(headings(&plan.rows), [Section::Official]);
    }

    #[test]
    fn a_section_still_searching_says_so_and_a_failed_one_says_why() {
        let plan = plan(Counted::Ready(1), Counted::Loading, Counted::Failed(&AppError::Network));
        assert_eq!(plan.rows[0], Row::Heading(Section::Official, 0));
        assert_eq!(plan.rows[1], Row::Waiting(Section::Official));
        assert_eq!(plan.rows.last(), Some(&Row::Problem(Section::Archive, AppError::Network.to_string())));
    }

    #[test]
    fn nothing_anywhere_is_empty_unless_something_is_still_coming() {
        let nothing = plan(Counted::Ready(0), Counted::Ready(0), Counted::Idle);
        assert_eq!((nothing.screen, nothing.rows), (Screen::Empty, Vec::new()));
        let coming = plan(Counted::Ready(0), Counted::Loading, Counted::Ready(0));
        assert_eq!(coming.screen, Screen::List, "il catalogo ufficiale sta ancora cercando");
    }

    #[test]
    fn a_failed_server_search_shows_the_other_sections_or_the_error() {
        let alone = plan(Counted::Failed(&AppError::Timeout), Counted::Idle, Counted::Idle);
        assert_eq!(alone.screen, Screen::Failed(AppError::Timeout.to_string()));
        let with_official = plan(Counted::Failed(&AppError::Timeout), Counted::Ready(1), Counted::Idle);
        assert_eq!(with_official.screen, Screen::List);
    }

    #[test]
    fn other_sources_are_named_in_the_detail_and_the_official_one_is_not() {
        let item = |source| SourceItem {
            source,
            id: "1".into(),
            title: "Sera".into(),
            artist: "Marea".into(),
            duration_ms: Some(61_000),
            cover_url: None,
        };
        assert_eq!(source_detail(&item(SourceKind::Deezer)), "Marea");
        assert_eq!(source_detail(&item(SourceKind::YouTube)), "YouTube · Marea");
        assert_eq!(source_data(0, &item(SourceKind::Deezer)).duration, SharedString::from("1:01"));
        let no_length = SourceItem { duration_ms: None, ..item(SourceKind::Bandcamp) };
        assert_eq!(source_data(4, &no_length).duration, SharedString::default());
        assert_eq!(source_data(4, &no_length).index, 5);
    }

    #[test]
    fn archive_rows_say_who_the_licence_and_the_quality() {
        let item = DiscoverItem {
            identifier: "marea-live".into(),
            title: "Live".into(),
            creator: Some("Marea".into()),
            licence: "cc-by".into(),
            year: Some("2019".into()),
            cover_url: None,
            lossless: true,
        };
        assert_eq!(archive_data(0, &item).detail, SharedString::from("Marea · CC BY · 2019 · Lossless"));
        let bare = DiscoverItem { creator: None, year: None, lossless: false, ..item };
        assert_eq!(archive_data(0, &bare).detail, SharedString::from("CC BY"));
    }
}
