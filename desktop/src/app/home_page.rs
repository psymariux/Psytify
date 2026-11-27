//! La Home: le sezioni del client Iced (`client/src/ui/pages/home.rs`), nello stesso ordine, disegnate come
//! da `PSYTIFY_ART_DIRECTION.md` (Home). In cima il titolo in tipo pixel e una riga di saluto; poi *Riprendi*
//! (brani che ti piacciono, le prime playlist e gli ultimi ascolti in una griglia di due colonne), *Aggiunti
//! di recente* come registro datato delle uscite (righe raggruppate per giorno, al posto dello scaffale),
//! *Novità dai tuoi artisti* (le uscite non ancora viste portano «Nuovo»), gli artisti, *Mix per te*, *Più
//! ascoltati da te*, *Ascoltati di recente*, gli *Album* del catalogo, *Il tuo ascolto* e *Dalla A alla Z*.
//! Each shelf is one full row of a fluid tile grid (`components::frame::TileGrid`, tiles stretch to the
//! width): the arrows move it, «Mostra tutto» opens it as a grid where it is. *Riprendi* is a 2 to 4 column
//! grid of 56 dp shortcuts and *Aggiunti di recente* flows into one to three columns (`frame::Flow`) under
//! date headers that span the row. The page fills the canvas with the frame margins and every section
//! enters with the page-enter move (`motion::Enter`).
//!
//! I dati: le tre sezioni della Home del catalogo (`CatalogView::home`, caricate da `CatalogCmd::Ensure`,
//! che sulla Home già pronta fa rifare il feed se gli ascolti sono cambiati) e il feed che il dominio della
//! libreria ne ricava (`LibraryView::feed`: uscite, novità, artisti, mix, classifica). La pagina li copia nei
//! suoi campi quando il core pubblica e prepara una volta i testi; `render` non li legge.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, ListState, ParentElement, Render,
    SharedString, Styled, Subscription, Window, canvas, div, list,
};
use psytify_core::api::{AlbumSummary, Track};
use psytify_core::catalog::Load;
use psytify_core::command::{CatalogCmd, LibraryCmd, PlaybackCmd, Resource};
use psytify_core::config::taste;
use psytify_core::feed::{self, Release};
use psytify_core::snapshot::{CatalogView, DownloadsView, LibraryView, PlaybackView};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    SURFACE_CANVAS, TEXT_ERROR, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
};
use psytify_design::tokens::size::cover::ART_ROW;
use psytify_design::tokens::size::{TILE_GAP, TRACK_ROW_DENSE_H, TRACK_ROW_H};
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{TYPE_BODY, TYPE_HEADING, TYPE_LABEL, TYPE_META};
use uuid::Uuid;

use super::page_list::{new_list, remeasure_header, set_rows};
use super::track_list::{RowText, download_command, download_state, now_playing, plays, with_links};
use super::track_menu::TrackMenu;
use super::{Page, PageEvent};
use crate::components::badge::BadgeKind;
use crate::components::button::Button;
use crate::components::card::Card;
use crate::components::frame::{Flow, Frame, TileGrid};
use crate::components::icon_button::IconButton;
use crate::components::indicators::{PlayState, Skeleton};
use crate::components::list_row::{ListRow, RowArt};
use crate::components::page_header::mosaic_urls;
use crate::components::playing_spot::PlayingSpot;
use crate::components::section::{PageTitle, note_at, section_heading_at};
use crate::components::shelf::{Shelf, ShelfPage, Wheel};
use crate::components::shortcut::Shortcut;
use crate::components::tile::AlbumTile;
use crate::components::track_row::{TrackData, TrackRow};
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::motion::{self, Enter};
use crate::theme::{Ap, TypeStyle, cased, col, dp, to_dp};

/// Le voci di *Riprendi* al massimo (due colonne).
const SHORTCUTS: usize = 8;
/// Le playlist in *Riprendi*, dopo i brani che ti piacciono.
const SHORTCUT_PLAYLISTS: usize = 3;
/// Le uscite del registro prima di «Mostra tutte».
const LOG: usize = 8;
/// Le novità dei tuoi artisti nello scaffale (come il client Iced; le altre sono in «Novità»).
const NEWS: usize = 12;
/// I brani della classifica (come il client Iced).
const TOP: usize = 10;
/// Righe dello scheletro del registro mentre la Home arriva.
const LOG_SKELETON: usize = 6;

/// Gli scaffali della pagina.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    News,
    Artists,
    Mixes,
    Listened,
    Albums,
    Alphabetical,
}

impl Kind {
    const COUNT: usize = 6;

    fn ix(self) -> usize {
        self as usize
    }
}

/// Lo stato di un caricamento, per il piano delle righe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Status {
    #[default]
    Loading,
    Failed,
    Ready,
}

impl Status {
    fn of<T>(load: &Load<T>) -> Self {
        match load {
            Load::Idle | Load::Loading => Self::Loading,
            Load::Failed(_) => Self::Failed,
            Load::Ready(_) => Self::Ready,
        }
    }
}

/// Le intestazioni che non sono uno scaffale intero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Head {
    Recent,
    Top,
    /// Lo scaffale aperto in griglia, o in caricamento, fallito, vuoto.
    Kind(Kind),
}

/// Una riga di stato sotto un'intestazione.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Note {
    RecentFailed,
    RecentEmpty,
    AlbumsFailed,
    AlbumsEmpty,
    AlphabeticalFailed,
    AlphabeticalEmpty,
}

/// Una riga dopo il titolo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    /// A row of *Riprendi*, from this item.
    Shortcuts(usize),
    Heading(Head),
    /// Una voce del registro (una data o un'uscita).
    Log(usize),
    LogSkeleton,
    Note(Note),
    Shelf(Kind),
    SkeletonShelf(Kind),
    /// Una riga della griglia di uno scaffale aperto: le copertine da questa.
    Grid(Kind, usize),
    Top(usize),
    Listening,
}

/// Che cosa c'è da mostrare, già contato. Puro, perché `plan` si possa provare.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Shape {
    shortcuts: usize,
    shortcut_columns: usize,
    recent: Status,
    /// Le voci del registro (date e uscite), già tagliate.
    log: usize,
    news: usize,
    artists: usize,
    mixes: usize,
    top: usize,
    listened: usize,
    albums: Status,
    album_count: usize,
    alphabetical: Status,
    alphabetical_count: usize,
    listening: bool,
    columns: usize,
    open: [bool; Kind::COUNT],
}

impl Shape {
    fn count(&self, kind: Kind) -> usize {
        match kind {
            Kind::News => self.news,
            Kind::Artists => self.artists,
            Kind::Mixes => self.mixes,
            Kind::Listened => self.listened,
            Kind::Albums => self.album_count,
            Kind::Alphabetical => self.alphabetical_count,
        }
    }
}

/// Le righe della Home, nell'ordine del client Iced. Pura.
fn plan(shape: Shape) -> Vec<Row> {
    let mut rows: Vec<Row> =
        (0..shape.shortcuts).step_by(shape.shortcut_columns.max(1)).map(Row::Shortcuts).collect();
    rows.push(Row::Heading(Head::Recent));
    match shape.recent {
        Status::Loading => rows.extend((0..LOG_SKELETON).map(|_| Row::LogSkeleton)),
        Status::Failed => rows.push(Row::Note(Note::RecentFailed)),
        Status::Ready if shape.log == 0 => rows.push(Row::Note(Note::RecentEmpty)),
        Status::Ready => rows.extend((0..shape.log).map(Row::Log)),
    }
    let shelf = |rows: &mut Vec<Row>, kind: Kind| {
        let count = shape.count(kind);
        let columns = shape.columns.max(1);
        if count == 0 {
            return;
        }
        if shape.open[kind.ix()] && count > columns {
            rows.push(Row::Heading(Head::Kind(kind)));
            rows.extend((0..count).step_by(columns).map(|start| Row::Grid(kind, start)));
        } else {
            rows.push(Row::Shelf(kind));
        }
    };
    shelf(&mut rows, Kind::News);
    shelf(&mut rows, Kind::Artists);
    shelf(&mut rows, Kind::Mixes);
    if shape.top > 0 {
        rows.push(Row::Heading(Head::Top));
        rows.extend((0..shape.top).map(Row::Top));
    }
    shelf(&mut rows, Kind::Listened);
    let catalogue = |rows: &mut Vec<Row>, kind: Kind, status: Status, failed: Note, empty: Note| match status
    {
        Status::Loading => rows.push(Row::SkeletonShelf(kind)),
        Status::Failed => rows.extend([Row::Heading(Head::Kind(kind)), Row::Note(failed)]),
        Status::Ready if shape.count(kind) == 0 => {
            rows.extend([Row::Heading(Head::Kind(kind)), Row::Note(empty)])
        }
        Status::Ready => shelf(rows, kind),
    };
    catalogue(&mut rows, Kind::Albums, shape.albums, Note::AlbumsFailed, Note::AlbumsEmpty);
    if shape.listening {
        rows.push(Row::Listening);
    }
    catalogue(
        &mut rows,
        Kind::Alphabetical,
        shape.alphabetical,
        Note::AlphabeticalFailed,
        Note::AlphabeticalEmpty,
    );
    rows
}

/// A line of the release log.
#[derive(Clone, Debug, PartialEq, Eq)]
enum LogItem {
    /// «21 nov 2026»: the day of the releases that follow, across the row.
    Date(SharedString),
    /// A row of releases (indices in `feed.releases`), one per flowing column.
    Releases(Vec<usize>),
}

/// The log: the first `limit` releases (newest first), a date before each new day, and each day's releases
/// flowed into `flow` columns.
fn release_log(releases: &[Release], limit: usize, flow: Flow) -> Vec<LogItem> {
    let mut log = Vec::new();
    let mut day: Vec<usize> = Vec::new();
    let mut label: Option<String> = None;
    let close = |log: &mut Vec<LogItem>, day: &mut Vec<usize>| {
        for row in 0..flow.rows(day.len()) {
            log.push(LogItem::Releases(flow.cells(row, day.len()).map(|ix| day[ix]).collect()));
        }
        day.clear();
    };
    for (ix, release) in releases.iter().enumerate().take(limit) {
        let date = feed::date_label(&release.created_at).unwrap_or_else(|| "Senza data".to_owned());
        if label.as_deref() != Some(date.as_str()) {
            close(&mut log, &mut day);
            log.push(LogItem::Date(date.clone().into()));
            label = Some(date);
        }
        day.push(ix);
    }
    close(&mut log, &mut day);
    log
}

/// La riga di saluto sotto il titolo. Finché le uscite non sono arrivate (`ready` falso) non si sa se ce ne
/// sono di nuove: si resta sulla frase neutra.
fn greeting(ready: bool, unread: usize, following: usize) -> &'static str {
    match (ready, unread, following) {
        (false, ..) | (_, 0, 0) => "Le ultime uscite del catalogo e ciò che ascolti",
        (_, 0, _) => "Sei in pari con gli artisti che segui",
        _ => "Ci sono uscite nuove dai tuoi artisti",
    }
}

/// «12.4 h» o «35 min»: il tempo ascoltato, con lo stesso formato del client Iced.
fn listened_label(ms: u64) -> String {
    // Millisecondi in ore: la precisione di un `f64` basta e avanza.
    #[allow(clippy::cast_precision_loss)]
    let hours = ms as f64 / 3_600_000.0;
    if hours >= 1.0 {
        format!("{hours:.1} h")
    } else {
        // Meno di un'ora: un numero di minuti piccolo e positivo.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let minutes = (hours * 60.0).round() as u64;
        format!("{minutes} min")
    }
}

/// Che cosa fa una voce di *Riprendi*.
#[derive(Clone, Debug)]
enum ShortcutTarget {
    Liked,
    Playlist(Uuid),
    Track(Box<Track>),
}

struct ShortcutItem {
    art: RowArt,
    title: SharedString,
    target: ShortcutTarget,
}

/// I testi di un'uscita, pronti.
struct ReleaseText {
    title: SharedString,
    /// «Album · Artista» sotto la copertina.
    subtitle: SharedString,
    /// «Artista · 12 brani» nel registro.
    detail: SharedString,
    kind: SharedString,
    cover: Option<SharedString>,
    new: bool,
}

impl ReleaseText {
    fn of(release: &Release, new: bool) -> Self {
        let detail = if release.is_album {
            format!("{} · {}", release.artist.name, format::tracks(release.tracks.len().max(1)))
        } else {
            release.artist.name.clone()
        };
        Self {
            title: release.title.clone().into(),
            subtitle: format!("{} · {}", release.kind_label(), release.artist.name).into(),
            detail: detail.into(),
            kind: release.kind_label().into(),
            cover: release.cover_url.clone().map(SharedString::from),
            new,
        }
    }
}

/// Un mix, pronto: il mosaico, il nome e chi c'è dentro.
struct MixText {
    mosaic: Vec<SharedString>,
    title: SharedString,
    subtitle: SharedString,
}

/// Uno scaffale: dove parte la riga, la rotella, se è aperto in griglia.
#[derive(Clone, Copy, Debug, Default)]
struct ShelfState {
    start: usize,
    wheel: Wheel,
    open: bool,
}

/// Le tre cifre di *Il tuo ascolto*.
struct Listening {
    time: SharedString,
    plays: SharedString,
    plays_label: &'static str,
    artist: SharedString,
}

pub struct HomePage {
    core: Entity<CoreModel>,
    recent: Load<Arc<[Track]>>,
    alphabetical: Load<Arc<[Track]>>,
    albums: Load<Arc<[AlbumSummary]>>,
    library: Arc<LibraryView>,
    downloads: Arc<DownloadsView>,
    liked: Arc<HashSet<Uuid>>,
    now: Option<(Uuid, PlayState)>,
    /// L'album del brano in corso: la sua copertina mostra l'equalizzatore.
    now_album: Option<Uuid>,
    greeting: SharedString,
    shortcuts: Vec<ShortcutItem>,
    log: Vec<LogItem>,
    releases: Vec<ReleaseText>,
    news: Vec<ReleaseText>,
    mixes: Vec<MixText>,
    top: Arc<[Track]>,
    top_texts: Vec<RowText>,
    listened: Arc<[Track]>,
    alphabetical_tracks: Arc<[Track]>,
    album_list: Arc<[AlbumSummary]>,
    listening: Option<Listening>,
    shelves: [ShelfState; Kind::COUNT],
    log_open: bool,
    /// The layout for the page width (margins, tile grid, flowing columns).
    frame: Frame,
    /// The zoom of the last draw: a new one makes the list measure its rows again.
    zoom: f32,
    rows: Vec<Row>,
    /// The section of each row (for the staggered enter): 0 is the title.
    sections: Vec<usize>,
    /// The page enter in progress, if any.
    enter: Option<Enter>,
    list: ListState,
    /// I posti dell'equalizzatore del brano in corso: li dipinge `Shell` (BACKLOG 23).
    spot: PlayingSpot,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for HomePage {}

/// Lo stesso dato: i due `Arc` sono la stessa allocazione (o lo stato non ne ha, ed è uguale).
fn same<T: ?Sized>(a: &Load<Arc<T>>, b: &Load<Arc<T>>) -> bool {
    match (a, b) {
        (Load::Ready(a), Load::Ready(b)) => Arc::ptr_eq(a, b),
        (Load::Idle, Load::Idle) | (Load::Loading, Load::Loading) => true,
        (Load::Failed(a), Load::Failed(b)) => a == b,
        _ => false,
    }
}

fn now_album(view: &PlaybackView) -> Option<Uuid> {
    view.queue.current().and_then(|track| track.album.as_ref().map(|album| album.id))
}

impl HomePage {
    /// I posti dell'equalizzatore di questa vista, per lo strato di `Shell`.
    pub fn spot(&self) -> PlayingSpot {
        self.spot.clone()
    }

    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let changes =
            cx.subscribe(&core, |this, core, changed: &Changed, cx| this.refresh(&core, *changed, cx));
        let snapshot = core.read(cx).snapshot();
        let empty: Arc<[Track]> = Arc::from(Vec::new());
        let mut page = Self {
            recent: snapshot.catalog.home.recent.clone(),
            alphabetical: snapshot.catalog.home.alphabetical.clone(),
            albums: snapshot.catalog.home.albums.clone(),
            library: Arc::clone(&snapshot.library),
            downloads: Arc::clone(&snapshot.downloads),
            liked: core.read(cx).liked(),
            now: now_playing(&snapshot.playback),
            now_album: now_album(&snapshot.playback),
            greeting: SharedString::default(),
            shortcuts: Vec::new(),
            log: Vec::new(),
            releases: Vec::new(),
            news: Vec::new(),
            mixes: Vec::new(),
            top: Arc::clone(&empty),
            top_texts: Vec::new(),
            listened: Arc::clone(&empty),
            alphabetical_tracks: empty,
            album_list: Arc::from(Vec::new()),
            listening: None,
            shelves: [ShelfState::default(); Kind::COUNT],
            log_open: false,
            frame: Frame::default(),
            zoom: crate::theme::zoom(),
            rows: Vec::new(),
            sections: Vec::new(),
            enter: None,
            list: new_list(0),
            core,
            spot: PlayingSpot::new(),
            _changes: changes,
        };
        page.rebuild();
        page
    }

    /// La Home è in vista: il core la carica se serve, e se è già pronta rifà il feed quando gli ascolti
    /// sono cambiati (come il client Iced a ogni rientro).
    pub fn show(&mut self, cx: &mut Context<Self>) {
        self.core.read(cx).catalog(CatalogCmd::Ensure(Resource::Home));
    }

    /// The page comes into view: its sections enter, rising unless it came from Back or Forward.
    pub fn enter(&mut self, rise: bool, cx: &mut Context<Self>) {
        self.enter = Enter::begin(rise, cx);
        cx.notify();
    }

    fn refresh(&mut self, core: &Entity<CoreModel>, changed: Changed, cx: &mut Context<Self>) {
        let snapshot = core.read(cx).snapshot();
        let mut rebuild = false;
        let mut dirty = false;
        if changed.catalog {
            rebuild |= self.take(&snapshot.catalog);
        }
        if changed.library && !Arc::ptr_eq(&snapshot.library, &self.library) {
            self.library = Arc::clone(&snapshot.library);
            self.liked = core.read(cx).liked();
            rebuild = true;
        }
        if changed.downloads && !Arc::ptr_eq(&snapshot.downloads, &self.downloads) {
            self.downloads = Arc::clone(&snapshot.downloads);
            // Solo la classifica mostra lo stato dei download.
            dirty |= !self.top.is_empty();
        }
        if changed.playback {
            let now = now_playing(&snapshot.playback);
            let album = now_album(&snapshot.playback);
            if now != self.now || album != self.now_album {
                self.now = now;
                self.now_album = album;
                dirty = true;
            }
        }
        if rebuild {
            self.rebuild();
        }
        if rebuild || dirty {
            cx.notify();
        }
    }

    /// Prende dal catalogo le tre sezioni della Home; `true` se una è cambiata.
    fn take(&mut self, catalog: &CatalogView) -> bool {
        let home = &catalog.home;
        let changed = !same(&home.recent, &self.recent)
            || !same(&home.alphabetical, &self.alphabetical)
            || !same(&home.albums, &self.albums);
        if changed {
            self.recent = home.recent.clone();
            self.alphabetical = home.alphabetical.clone();
            self.albums = home.albums.clone();
        }
        changed
    }

    /// Rifà testi e righe: quando cambiano i dati, le colonne o uno scaffale si apre.
    fn rebuild(&mut self) {
        let library = Arc::clone(&self.library);
        let feed = Arc::clone(&library.feed);
        let ready = matches!(self.recent, Load::Ready(_));
        self.greeting = greeting(ready, library.unread_news, library.followed.len()).into();
        self.shortcuts = self.shortcut_items(&library);
        let seen = library.news_seen.as_str();
        self.releases = feed.releases.iter().map(|release| ReleaseText::of(release, false)).collect();
        let limit = if self.log_open { feed.releases.len() } else { LOG };
        self.log = release_log(&feed.releases, limit, self.frame.flow);
        self.news = feed
            .news
            .iter()
            .take(NEWS)
            .map(|release| ReleaseText::of(release, release.created_at.as_str() > seen))
            .collect();
        self.mixes = feed
            .mixes
            .iter()
            .map(|mix| MixText {
                mosaic: mosaic_urls(mix.tracks.iter().map(|t| t.cover_url.as_deref())),
                title: mix.title.clone().into(),
                subtitle: mix.subtitle.clone().into(),
            })
            .collect();
        self.top = feed.top.iter().take(TOP).map(|stat| stat.track.clone()).collect();
        self.top_texts = self.top.iter().map(RowText::of).collect();
        self.listened = library.recent.iter().cloned().collect();
        self.alphabetical_tracks =
            self.alphabetical.ready().map_or_else(|| Arc::from(Vec::new()), Arc::clone);
        self.album_list = self.albums.ready().map_or_else(|| Arc::from(Vec::new()), Arc::clone);
        self.listening = (!library.stats.is_empty() || library.listened_ms > 0).then(|| {
            let plays = taste::total_plays(&library.stats);
            Listening {
                time: listened_label(library.listened_ms).into(),
                plays: plays.to_string().into(),
                plays_label: if plays == 1 { "brano riprodotto" } else { "brani riprodotti" },
                artist: taste::top_artists(&library.stats)
                    .first()
                    .map_or_else(|| "—".to_owned(), |a| a.artist.name.clone())
                    .into(),
            }
        });
        let mut open = [false; Kind::COUNT];
        for (ix, state) in self.shelves.iter().enumerate() {
            open[ix] = state.open;
        }
        let log = if Status::of(&self.recent) == Status::Ready { self.log.len() } else { 0 };
        self.rows = plan(Shape {
            shortcuts: self.shortcuts.len(),
            shortcut_columns: self.frame.shortcuts,
            recent: Status::of(&self.recent),
            log,
            news: self.news.len(),
            artists: feed.artists.len(),
            mixes: self.mixes.len(),
            top: self.top.len(),
            listened: self.listened.len(),
            albums: Status::of(&self.albums),
            album_count: self.album_list.len(),
            alphabetical: Status::of(&self.alphabetical),
            alphabetical_count: self.alphabetical_tracks.len(),
            listening: self.listening.is_some(),
            columns: self.frame.grid.columns,
            open,
        });
        self.sections = sections(&self.rows);
        set_rows(&self.list, self.rows.len());
        remeasure_header(&self.list);
    }

    /// *Riprendi*: i brani che ti piacciono, le prime playlist, poi gli ultimi ascolti (al più 8).
    fn shortcut_items(&self, library: &LibraryView) -> Vec<ShortcutItem> {
        let mut items = Vec::new();
        if !library.liked.is_empty() {
            items.push(ShortcutItem {
                art: RowArt::Sprite(sprite::ICON_HEART_FILL),
                title: "Brani che ti piacciono".into(),
                target: ShortcutTarget::Liked,
            });
        }
        for playlist in library.playlists.iter().take(SHORTCUT_PLAYLISTS) {
            items.push(ShortcutItem {
                art: RowArt::Cover(
                    playlist.tracks.first().and_then(|t| t.cover_url.clone()).map(SharedString::from),
                ),
                title: playlist.name.clone().into(),
                target: ShortcutTarget::Playlist(playlist.id),
            });
        }
        for track in library.recent.iter() {
            if items.len() >= SHORTCUTS {
                break;
            }
            items.push(ShortcutItem {
                art: RowArt::Cover(track.cover_url.clone().map(SharedString::from)),
                title: track.title.clone().into(),
                target: ShortcutTarget::Track(Box::new(track.clone())),
            });
        }
        items
    }

    /// The play button of a *Riprendi* item: the liked songs, a playlist or the track.
    fn play_shortcut(&self, target: &ShortcutTarget, cx: &gpui::App) {
        let tracks: Arc<[Track]> = match target {
            ShortcutTarget::Liked => self.library.liked.iter().cloned().collect(),
            ShortcutTarget::Playlist(id) => match self.library.playlists.iter().find(|p| p.id == *id) {
                Some(playlist) => playlist.tracks.iter().cloned().collect(),
                None => return,
            },
            ShortcutTarget::Track(track) => Arc::from([(**track).clone()]),
        };
        if !tracks.is_empty() {
            self.play_tracks(&tracks, 0, cx);
        }
    }

    fn kind_count(&self, kind: Kind) -> usize {
        match kind {
            Kind::News => self.news.len(),
            Kind::Artists => self.library.feed.artists.len(),
            Kind::Mixes => self.mixes.len(),
            Kind::Listened => self.listened.len(),
            Kind::Albums => self.album_list.len(),
            Kind::Alphabetical => self.alphabetical_tracks.len(),
        }
    }

    fn shelf_page(&self, kind: Kind) -> ShelfPage {
        ShelfPage::new(self.shelves[kind.ix()].start, self.frame.grid.columns, self.kind_count(kind))
    }

    /// The page width changed: new column counts rebuild the rows, a new tile size only redraws.
    fn set_frame(&mut self, frame: Frame, cx: &mut Context<Self>) {
        if frame == self.frame {
            return;
        }
        let rebuild = frame.columns() != self.frame.columns();
        self.frame = frame;
        if rebuild {
            for state in &mut self.shelves {
                state.wheel.reset();
            }
            self.rebuild();
        }
        cx.notify();
    }

    fn move_shelf(&mut self, kind: Kind, start: usize, cx: &mut Context<Self>) {
        let state = &mut self.shelves[kind.ix()];
        if state.start != start {
            state.start = start;
            cx.notify();
        }
    }

    fn wheel_shelf(&mut self, kind: Kind, dx: f32, cx: &mut Context<Self>) {
        let page = self.shelf_page(kind);
        let steps = self.shelves[kind.ix()].wheel.step(dx, self.frame.grid.tile + TILE_GAP);
        if steps != 0 {
            self.move_shelf(kind, page.moved(steps), cx);
        }
    }

    fn toggle_shelf(&mut self, kind: Kind, cx: &mut Context<Self>) {
        let state = &mut self.shelves[kind.ix()];
        state.open = !state.open;
        self.rebuild();
        cx.notify();
    }

    fn toggle_log(&mut self, cx: &mut Context<Self>) {
        self.log_open = !self.log_open;
        self.rebuild();
        cx.notify();
    }

    /// Suona dei brani da `start`, o mette in pausa se è il brano in corso.
    fn play_tracks(&self, tracks: &Arc<[Track]>, start: usize, cx: &gpui::App) {
        let core = self.core.read(cx);
        match (tracks.get(start), self.now) {
            (Some(track), Some((id, _))) if track.id == id => core.playback(PlaybackCmd::Toggle),
            _ => core.playback(PlaybackCmd::Play { tracks: Arc::clone(tracks), start }),
        }
    }

    /// Suona un'uscita: l'album intero (lo chiede il core al server) o il singolo. Se suona già, pausa.
    fn play_release(&self, release: &Release, cx: &gpui::App) {
        let core = self.core.read(cx);
        if self.release_playing(release).is_some() {
            core.playback(PlaybackCmd::Toggle);
        } else if let (false, Some(single)) = (release.is_album, release.tracks.first()) {
            core.playback(PlaybackCmd::Play { tracks: Arc::from([single.clone()]), start: 0 });
        } else {
            core.playback(PlaybackCmd::PlayAlbum(release.id));
        }
    }

    fn play_album(&self, id: Uuid, cx: &gpui::App) {
        let core = self.core.read(cx);
        if self.now.is_some() && self.now_album == Some(id) {
            core.playback(PlaybackCmd::Toggle);
        } else {
            core.playback(PlaybackCmd::PlayAlbum(id));
        }
    }

    fn track_playing(&self, track: &Track) -> Option<PlayState> {
        self.now.filter(|(id, _)| *id == track.id).map(|(_, state)| state)
    }

    fn release_playing(&self, release: &Release) -> Option<PlayState> {
        let (id, state) = self.now?;
        let inside = if release.is_album {
            self.now_album == Some(release.id)
        } else {
            release.tracks.iter().any(|t| t.id == id)
        };
        inside.then_some(state)
    }

    fn album_playing(&self, id: Uuid) -> Option<PlayState> {
        self.now.filter(|_| self.now_album == Some(id)).map(|(_, state)| state)
    }

    fn item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let element = match ix.checked_sub(1) {
            None => Some(
                PageTitle::screen("Home")
                    .meta(self.greeting.clone())
                    .margin(self.frame.margin)
                    .into_any_element(),
            ),
            Some(row) => self.row(row, cx),
        };
        let element = element.unwrap_or_else(|| div().into_any_element());
        match self.enter {
            Some(enter) => {
                let section = ix.checked_sub(1).and_then(|row| self.sections.get(row)).copied().unwrap_or(0);
                enter.wrap(element, section, Ap::of(window), Instant::now())
            }
            None => element,
        }
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(match *self.rows.get(ix)? {
            Row::Shortcuts(start) => self.shortcut_row(start, cx),
            Row::Heading(head) => self.heading(head, cx),
            Row::Log(position) => self.log_row(position, cx)?,
            Row::LogSkeleton => log_skeleton(self.frame.margin),
            Row::Note(which) => self.note(which, cx),
            Row::Shelf(kind) => self.shelf(kind, cx),
            Row::SkeletonShelf(kind) => self.skeleton_shelf(kind),
            Row::Grid(kind, start) => {
                let end = (start + self.frame.grid.columns.max(1)).min(self.kind_count(kind));
                TileGrid::row(
                    self.frame.margin,
                    (start..end).filter_map(|position| self.tile(kind, position, cx)),
                )
                .pb(dp(SPACE_6))
                .into_any_element()
            }
            Row::Top(position) => div()
                .px(dp(self.frame.margin - SPACE_3))
                .child(self.top_row(position, cx)?)
                .into_any_element(),
            Row::Listening => self.listening_card()?,
        })
    }

    fn shortcut_row(&self, start: usize, cx: &mut Context<Self>) -> AnyElement {
        let columns = self.frame.shortcuts.max(1);
        let last = start + columns >= self.shortcuts.len();
        let cells = (start..(start + columns).min(self.shortcuts.len())).filter_map(|position| {
            let item = self.shortcuts.get(position)?;
            let target = item.target.clone();
            let mut shortcut =
                Shortcut::new(("home-shortcut", position), item.art.clone(), item.title.clone())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| match &target {
                        ShortcutTarget::Liked => cx.emit(PageEvent::Open(Page::Liked)),
                        ShortcutTarget::Playlist(id) => cx.emit(PageEvent::Open(Page::Playlist(*id))),
                        ShortcutTarget::Track(track) => {
                            this.play_tracks(&Arc::from([(**track).clone()]), 0, cx)
                        }
                    }))
                    .on_play({
                        let target = item.target.clone();
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.play_shortcut(&target, cx))
                    });
            if let ShortcutTarget::Track(track) = &item.target
                && let Some(state) = self.track_playing(track)
            {
                shortcut = shortcut.playing_at(state, &self.spot);
            }
            Some(shortcut)
        });
        // Equal columns across the width: without `w_full` the grid shrinks to its content.
        let mut cells: Vec<AnyElement> = cells.map(IntoElement::into_any_element).collect();
        while cells.len() < columns {
            cells.push(div().into_any_element());
        }
        div()
            .w_full()
            .grid()
            .grid_cols(u16::try_from(columns).unwrap_or(2))
            .gap(dp(SPACE_3))
            .px(dp(self.frame.margin))
            .pb(dp(if last { SPACE_6 } else { SPACE_3 }))
            .children(cells)
            .into_any_element()
    }

    fn head_title(&self, head: Head) -> SharedString {
        match head {
            Head::Recent => "Aggiunti di recente".into(),
            Head::Top => "Più ascoltati da te".into(),
            Head::Kind(kind) => self.kind_title(kind),
        }
    }

    fn kind_title(&self, kind: Kind) -> SharedString {
        match kind {
            Kind::News if self.library.unread_news > 0 => "Novità dai tuoi artisti · da ascoltare".into(),
            Kind::News => "Novità dai tuoi artisti".into(),
            Kind::Artists if self.library.feed.artists_are_yours => "I tuoi artisti".into(),
            Kind::Artists => "Artisti nel catalogo".into(),
            Kind::Mixes => "Mix per te".into(),
            Kind::Listened => "Ascoltati di recente".into(),
            Kind::Albums => "Album".into(),
            Kind::Alphabetical => "Dalla A alla Z".into(),
        }
    }

    fn heading(&self, head: Head, cx: &mut Context<Self>) -> AnyElement {
        let heading = section_heading_at(self.head_title(head), SharedString::default(), self.frame.margin);
        match head {
            Head::Recent if self.library.feed.releases.len() > LOG => {
                let label = if self.log_open { "Mostra meno" } else { "Mostra tutte" };
                heading
                    .child(div().flex_1())
                    .child(
                        Button::new("home-log-more", label)
                            .ghost()
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_log(cx))),
                    )
                    .into_any_element()
            }
            Head::Kind(kind)
                if self.shelves[kind.ix()].open && self.kind_count(kind) > self.frame.grid.columns =>
            {
                heading
                    .child(div().flex_1())
                    .child(Button::new(("home-less", kind.ix()), "Mostra meno").ghost().on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_shelf(kind, cx)),
                    ))
                    .into_any_element()
            }
            _ => heading.into_any_element(),
        }
    }

    /// A line of the log: the day's date, or a row of releases in flowing columns.
    fn log_row(&self, position: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let cells = match self.log.get(position)? {
            LogItem::Date(label) => return Some(date_row(label.clone(), self.frame.margin)),
            LogItem::Releases(cells) => cells,
        };
        let rows = cells.iter().filter_map(|ix| self.release_row(*ix, cx));
        Some(self.frame.flow.row(self.frame.margin - SPACE_3, rows).into_any_element())
    }

    /// A release of the log (an album opens, a single plays).
    fn release_row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let release = self.library.feed.releases.get(ix)?;
        let text = self.releases.get(ix)?;
        let (open, play) = (release.clone(), release.clone());
        let row = ListRow::new(("home-log", ix), RowArt::Cover(text.cover.clone()), text.title.clone())
            .inset(SPACE_3)
            .badge(text.kind.clone(), BadgeKind::Neutral)
            .detail(text.detail.clone())
            .hover_action(
                IconButton::new(("home-log-play", ix), sprite::ICON_PLAY)
                    .tooltip("Riproduci", None)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.play_release(&play, cx))),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if open.is_album {
                    cx.emit(PageEvent::Open(Page::Album(open.id)));
                } else {
                    this.play_release(&open, cx);
                }
            }));
        Some(row.into_any_element())
    }

    fn note(&self, which: Note, cx: &mut Context<Self>) -> AnyElement {
        let (text, error) = match which {
            Note::RecentFailed => {
                let message = match &self.recent {
                    Load::Failed(error) => format!("Le novità non sono arrivate. {error}"),
                    _ => "Le novità non sono arrivate.".to_owned(),
                };
                return note_at(
                    div()
                        .flex()
                        .items_center()
                        .gap(dp(SPACE_5))
                        .child(div().type_role(TYPE_BODY).text_color(col(TEXT_ERROR)).child(message))
                        .child(Button::new("home-retry", "Riprova").secondary().on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.core.read(cx).catalog(CatalogCmd::Retry(Resource::Home));
                            },
                        ))),
                    self.frame.margin,
                )
                .into_any_element();
            }
            Note::RecentEmpty => ("Il catalogo non ha ancora brani pubblicati.", false),
            Note::AlbumsFailed => ("Gli album non sono arrivati: riprova più tardi.", true),
            Note::AlbumsEmpty => ("Nessun album nel catalogo: per ora solo singoli.", false),
            Note::AlphabeticalFailed => ("Il catalogo non è arrivato: riprova più tardi.", true),
            Note::AlphabeticalEmpty => ("Nessun brano pubblicato.", false),
        };
        note_at(
            div()
                .type_role(TYPE_BODY)
                .text_color(col(if error { TEXT_ERROR } else { TEXT_SECONDARY }))
                .child(text),
            self.frame.margin,
        )
        .into_any_element()
    }

    fn shelf(&self, kind: Kind, cx: &mut Context<Self>) -> AnyElement {
        let page = self.shelf_page(kind);
        let tiles: Vec<AnyElement> =
            page.range().filter_map(|position| self.tile(kind, position, cx)).collect();
        let (paged, wheeled) = (cx.entity().downgrade(), cx.entity().downgrade());
        let mut shelf = Shelf::new(("home-shelf", kind.ix()), self.kind_title(kind), page)
            .frame(self.frame.margin, TILE_GAP)
            .tiles(tiles)
            .on_page(move |start, _, cx| {
                paged.update(cx, |this, cx| this.move_shelf(kind, start, cx)).ok();
            })
            .on_wheel(move |dx, _, cx| {
                wheeled.update(cx, |this, cx| this.wheel_shelf(kind, dx, cx)).ok();
            });
        if kind == Kind::News {
            shelf = shelf.action(
                Button::new("home-news", "Apri Novità")
                    .ghost()
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::News)))),
            );
        } else {
            shelf = shelf.on_show_all(
                "Mostra tutto",
                cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_shelf(kind, cx)),
            );
        }
        shelf.into_any_element()
    }

    /// La copertina `position` di uno scaffale (o della sua griglia).
    fn tile(&self, kind: Kind, position: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = (SharedString::from(format!("home-{kind:?}")), position);
        let tile = match kind {
            Kind::News => {
                let release = self.library.feed.news.get(position)?;
                let text = self.news.get(position)?;
                let (open, play) = (release.clone(), release.clone());
                let mut tile = AlbumTile::new(id, text.title.clone(), text.subtitle.clone())
                    .cover_url(text.cover.clone())
                    .on_play(cx.listener(move |this, _: &ClickEvent, _, cx| this.play_release(&play, cx)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if open.is_album {
                            cx.emit(PageEvent::Open(Page::Album(open.id)));
                        } else {
                            this.play_release(&open, cx);
                        }
                    }));
                if text.new {
                    tile = tile.badge("Nuovo");
                }
                match self.release_playing(release) {
                    Some(state) => tile.playing_at(state, &self.spot),
                    None => tile,
                }
            }
            Kind::Artists => {
                let card = self.library.feed.artists.get(position)?;
                let artist = card.artist.id;
                let following = self.library.followed.iter().any(|known| known.id == artist);
                AlbumTile::new(id, card.artist.name.clone(), if following { "Seguito" } else { "Artista" })
                    .round()
                    .cover_url(card.cover_url.clone().map(SharedString::from))
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.emit(PageEvent::Open(Page::Artist(artist)));
                    }))
            }
            Kind::Mixes => {
                let mix = self.mixes.get(position)?;
                let play = move |this: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                    this.core.read(cx).playback(PlaybackCmd::PlayMix(position));
                };
                AlbumTile::new(id, mix.title.clone(), mix.subtitle.clone())
                    .mosaic(mix.mosaic.clone())
                    .on_play(cx.listener(play))
                    .on_click(cx.listener(play))
            }
            Kind::Listened | Kind::Alphabetical => {
                let tracks = if kind == Kind::Listened { &self.listened } else { &self.alphabetical_tracks };
                let track = tracks.get(position)?;
                let list = Arc::clone(tracks);
                let play = move |this: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                    this.play_tracks(&list, position, cx);
                };
                let tile = AlbumTile::new(id, track.title.clone(), track.artist.name.clone())
                    .cover_url(track.cover_url.clone().map(SharedString::from))
                    .on_play(cx.listener(play.clone()))
                    .on_click(cx.listener(play));
                match self.track_playing(track) {
                    Some(state) => tile.playing_at(state, &self.spot),
                    None => tile,
                }
            }
            Kind::Albums => {
                let album = self.album_list.get(position)?;
                let album_id = album.id;
                let tile = AlbumTile::new(id, album.title.clone(), album.artist.name.clone())
                    .cover_url(album.cover_url.clone().map(SharedString::from))
                    .on_play(cx.listener(move |this, _: &ClickEvent, _, cx| this.play_album(album_id, cx)))
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.emit(PageEvent::Open(Page::Album(album_id)));
                    }));
                match self.album_playing(album_id) {
                    Some(state) => tile.playing_at(state, &self.spot),
                    None => tile,
                }
            }
        };
        Some(tile.side(self.frame.grid.tile).into_any_element())
    }

    /// Uno scaffale che aspetta: l'intestazione e le copertine vuote alla misura vera.
    fn skeleton_shelf(&self, kind: Kind) -> AnyElement {
        let tile = self.frame.grid.tile;
        let tiles = (0..self.frame.grid.columns.max(1)).map(|_| {
            div()
                .flex()
                .flex_col()
                .gap(dp(SPACE_3))
                .child(Skeleton::new(tile, tile))
                .child(Skeleton::new(tile * 0.7, 12.0))
                .child(Skeleton::new(tile * 0.45, 10.0))
                .into_any_element()
        });
        div()
            .flex()
            .flex_col()
            .child(section_heading_at(self.kind_title(kind), SharedString::default(), self.frame.margin))
            .child(TileGrid::row(self.frame.margin, tiles).pb(dp(SPACE_6)))
            .into_any_element()
    }

    fn top_row(&self, position: usize, cx: &mut Context<Self>) -> Option<TrackRow> {
        let track = self.top.get(position)?;
        let text = self.top_texts.get(position)?;
        let data = TrackData {
            index: u32::try_from(position + 1).unwrap_or(u32::MAX),
            title: text.title.clone(),
            artist: text.artist.clone(),
            album: text.album.clone(),
            duration: text.duration.clone(),
            liked: self.liked.contains(&track.id),
            download: download_state(&self.downloads, track),
            available: true,
            cover_url: text.cover_url.clone(),
        };
        let (liked, download, menu) = (track.clone(), track.clone(), track.clone());
        let mut row = TrackRow::new(("home-top", position), data)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if plays(event) {
                    let top = Arc::clone(&this.top);
                    this.play_tracks(&top, position, cx);
                }
            }))
            .on_like(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.core.read(cx).library(LibraryCmd::ToggleLike(liked.clone()));
            }))
            .on_download(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.core.read(cx).downloads(download_command(&this.downloads, &download));
            }))
            .on_more(cx.listener(move |_, event: &ClickEvent, _, cx| {
                cx.emit(PageEvent::TrackMenu(Box::new(TrackMenu {
                    track: menu.clone(),
                    at: event.position(),
                    from_playlist: None,
                })));
            }));
        row = with_links(row, track, cx);
        if let Some(state) = self.track_playing(track) {
            row = row.playing_at(state, &self.spot);
        }
        // L'ultima riga della classifica lascia lo spazio prima della sezione dopo.
        Some(row)
    }

    /// *Il tuo ascolto*: tempo, brani e l'artista più ascoltato, in una scheda.
    fn listening_card(&self) -> Option<AnyElement> {
        let listening = self.listening.as_ref()?;
        let stat = |value: SharedString, label: &'static str, pixel: bool| {
            let value = if pixel {
                div().text_color(col(TEXT_PRIMARY)).truncate().child(value).into_any_element()
            } else {
                div()
                    .type_role(TYPE_HEADING)
                    .text_color(col(TEXT_PRIMARY))
                    .truncate()
                    .child(value)
                    .into_any_element()
            };
            (value, label)
        };
        let stats = [
            stat(listening.time.clone(), "ascoltati", true),
            stat(listening.plays.clone(), listening.plays_label, true),
            stat(listening.artist.clone(), "l'artista che ascolti di più", false),
        ];
        Some(
            div()
                .px(dp(self.frame.margin))
                .pt(dp(SPACE_6))
                .pb(dp(SPACE_6))
                .child(ListeningCard { stats })
                .into_any_element(),
        )
    }
}

/// The section of each row, for the staggered enter: a heading, a shelf, a skeleton shelf, the listening
/// card and the first shortcut row each start one (0 is the page title).
fn sections(rows: &[Row]) -> Vec<usize> {
    let mut section = 0;
    let mut previous: Option<Row> = None;
    rows.iter()
        .map(|row| {
            let starts = match row {
                Row::Shortcuts(_) => !matches!(previous, Some(Row::Shortcuts(_))),
                Row::Heading(_) | Row::Shelf(_) | Row::SkeletonShelf(_) | Row::Listening => true,
                _ => false,
            };
            if starts {
                section += 1;
            }
            previous = Some(*row);
            section
        })
        .collect()
}

/// La scheda di *Il tuo ascolto*: le cifre in tipo pixel (l'artista, che viene dal catalogo, in Inter).
#[derive(IntoElement)]
struct ListeningCard {
    stats: [(AnyElement, &'static str); 3],
}

impl gpui::RenderOnce for ListeningCard {
    fn render(self, _window: &mut Window, _cx: &mut gpui::App) -> impl IntoElement {
        let cells = self.stats.into_iter().enumerate().map(|(ix, (value, label))| {
            let value = if ix < 2 { div().pixel_type(true).child(value) } else { div().child(value) };
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(dp(SPACE_2))
                .child(value)
                .child(div().type_role(TYPE_META).text_color(col(TEXT_TERTIARY)).child(label))
        });
        Card::new().child(
            div()
                .flex()
                .flex_col()
                .gap(dp(SPACE_5))
                .child(
                    div()
                        .type_role(TYPE_LABEL)
                        .text_color(col(TEXT_SECONDARY))
                        .child(cased("Il tuo ascolto", TYPE_LABEL)),
                )
                .child(div().flex().gap(dp(SPACE_5)).children(cells))
                .child(
                    div()
                        .type_role(TYPE_META)
                        .text_color(col(TEXT_TERTIARY))
                        .child("Resta su questo computer: il catalogo non sa chi ascolta cosa."),
                ),
        )
    }
}

/// The date of a day in the log, in `TYPE_LABEL`, across the row.
fn date_row(label: SharedString, margin: f32) -> AnyElement {
    div()
        .h(dp(TRACK_ROW_DENSE_H))
        .flex()
        .items_end()
        .px(dp(margin))
        .pb(dp(SPACE_2))
        .type_role(TYPE_LABEL)
        .text_color(col(TEXT_TERTIARY))
        .child(cased(&label, TYPE_LABEL))
        .into_any_element()
}

/// A waiting log row, on the `ListRow` grid.
fn log_skeleton(margin: f32) -> AnyElement {
    div()
        .h(dp(TRACK_ROW_H))
        .flex()
        .items_center()
        .gap(dp(SPACE_3))
        .px(dp(margin))
        .child(Skeleton::new(ART_ROW, ART_ROW))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(dp(SPACE_3 / 2.0))
                .child(Skeleton::new(220.0, 12.0))
                .child(Skeleton::new(140.0, 10.0)),
        )
        .into_any_element()
}

impl Render for HomePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // I posti si segnano di nuovo nel paint di questo fotogramma, se ciò che suona è visibile.
        self.spot.clear();
        // The layout comes from the real width, read at every draw: if it changes, the rows follow on the next
        // frame (one frame).
        let now = Instant::now();
        if self.enter.is_some_and(|enter| !enter.running(now)) {
            self.enter = None;
        }
        motion::keep_running(window, self.enter.is_some());
        let zoom = crate::theme::zoom();
        if (zoom - self.zoom).abs() > f32::EPSILON {
            self.zoom = zoom;
            self.list.remeasure();
        }
        let entity = cx.entity().downgrade();
        let current = self.frame;
        let measure = canvas(
            move |bounds, window, cx| {
                let frame = Frame::of(to_dp(bounds.size.width), to_dp(window.viewport_size().width));
                if frame != current {
                    cx.defer(move |cx| {
                        entity.update(cx, |this, cx| this.set_frame(frame, cx)).ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        div().relative().size_full().bg(col(SURFACE_CANVAS)).child(measure).child(
            list(
                self.list.clone(),
                cx.processor(|this: &mut Self, ix: usize, window, cx| this.item(ix, window, cx)),
            )
            .size_full(),
        )
    }
}

#[cfg(test)]
mod tests {
    use psytify_core::api::models::fixtures;

    use super::*;

    fn ready() -> Shape {
        Shape {
            recent: Status::Ready,
            albums: Status::Ready,
            alphabetical: Status::Ready,
            columns: 4,
            shortcut_columns: 2,
            ..Shape::default()
        }
    }

    #[test]
    fn the_sections_follow_the_iced_order() {
        let rows = plan(Shape {
            shortcuts: 3,
            log: 2,
            news: 1,
            artists: 2,
            mixes: 1,
            top: 2,
            listened: 1,
            album_count: 3,
            listening: true,
            alphabetical_count: 2,
            ..ready()
        });
        assert_eq!(
            rows,
            vec![
                Row::Shortcuts(0),
                Row::Shortcuts(2),
                Row::Heading(Head::Recent),
                Row::Log(0),
                Row::Log(1),
                Row::Shelf(Kind::News),
                Row::Shelf(Kind::Artists),
                Row::Shelf(Kind::Mixes),
                Row::Heading(Head::Top),
                Row::Top(0),
                Row::Top(1),
                Row::Shelf(Kind::Listened),
                Row::Shelf(Kind::Albums),
                Row::Listening,
                Row::Shelf(Kind::Alphabetical),
            ]
        );
    }

    #[test]
    fn while_the_home_loads_there_are_skeletons_and_local_sections() {
        let rows = plan(Shape { top: 1, listened: 2, columns: 4, ..Shape::default() });
        assert_eq!(rows.iter().filter(|row| **row == Row::LogSkeleton).count(), LOG_SKELETON);
        assert!(rows.contains(&Row::SkeletonShelf(Kind::Albums)));
        assert!(rows.contains(&Row::SkeletonShelf(Kind::Alphabetical)));
        assert!(rows.contains(&Row::Shelf(Kind::Listened)), "what is on this computer shows at once");
        assert!(rows.contains(&Row::Top(0)));
    }

    #[test]
    fn failures_and_empty_sections_say_so() {
        let rows = plan(Shape {
            recent: Status::Failed,
            albums: Status::Failed,
            alphabetical: Status::Ready,
            columns: 4,
            ..Shape::default()
        });
        assert!(rows.contains(&Row::Note(Note::RecentFailed)));
        assert!(rows.contains(&Row::Note(Note::AlbumsFailed)));
        assert!(rows.contains(&Row::Note(Note::AlphabeticalEmpty)));
        let rows = plan(ready());
        assert!(rows.contains(&Row::Note(Note::RecentEmpty)));
        assert!(rows.contains(&Row::Note(Note::AlbumsEmpty)));
    }

    #[test]
    fn an_open_shelf_becomes_rows_of_whole_tiles() {
        let mut open = [false; Kind::COUNT];
        open[Kind::Albums.ix()] = true;
        let rows = plan(Shape { album_count: 9, open, ..ready() });
        let grid: Vec<Row> = rows.iter().copied().filter(|row| matches!(row, Row::Grid(..))).collect();
        assert_eq!(
            grid,
            vec![Row::Grid(Kind::Albums, 0), Row::Grid(Kind::Albums, 4), Row::Grid(Kind::Albums, 8)]
        );
        assert!(rows.contains(&Row::Heading(Head::Kind(Kind::Albums))));
        // Se le copertine stanno in una riga, aperto o no è uno scaffale.
        let rows = plan(Shape { album_count: 3, open, ..ready() });
        assert!(rows.contains(&Row::Shelf(Kind::Albums)));
    }

    fn release(n: u128, day: &str) -> Release {
        let mut track = fixtures::track(n);
        track.created_at = format!("2026-11-{day}T10:00:00Z");
        feed::releases(&[track]).remove(0)
    }

    #[test]
    fn the_log_puts_a_date_before_each_new_day() {
        let releases = [release(1, "21"), release(2, "21"), release(3, "20")];
        let one = Flow { columns: 1 };
        let log = release_log(&releases, 10, one);
        assert_eq!(
            log,
            vec![
                LogItem::Date("21 nov 2026".into()),
                LogItem::Releases(vec![0]),
                LogItem::Releases(vec![1]),
                LogItem::Date("20 nov 2026".into()),
                LogItem::Releases(vec![2]),
            ]
        );
        assert_eq!(
            release_log(&releases, 1, one),
            vec![LogItem::Date("21 nov 2026".into()), LogItem::Releases(vec![0])]
        );
        // In two columns a day's releases share a row; the date still spans it.
        assert_eq!(
            release_log(&releases, 10, Flow { columns: 2 }),
            vec![
                LogItem::Date("21 nov 2026".into()),
                LogItem::Releases(vec![0, 1]),
                LogItem::Date("20 nov 2026".into()),
                LogItem::Releases(vec![2]),
            ]
        );
    }

    #[test]
    fn each_section_enters_on_its_own_step() {
        let rows = [
            Row::Shortcuts(0),
            Row::Shortcuts(2),
            Row::Heading(Head::Recent),
            Row::Log(0),
            Row::Shelf(Kind::News),
            Row::Listening,
        ];
        assert_eq!(sections(&rows), vec![1, 1, 2, 2, 3, 4]);
    }

    #[test]
    fn texts_for_the_greeting_and_the_time_listened() {
        assert_eq!(greeting(true, 2, 1), "Ci sono uscite nuove dai tuoi artisti");
        assert_eq!(greeting(true, 0, 1), "Sei in pari con gli artisti che segui");
        assert_eq!(greeting(false, 0, 1), "Le ultime uscite del catalogo e ciò che ascolti");
        assert_eq!(listened_label(35 * 60_000), "35 min");
        assert_eq!(listened_label(3 * 3_600_000 + 1_440_000), "3.4 h");
    }
}
