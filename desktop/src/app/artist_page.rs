//! The page of an artist (`PSYTIFY_ART_DIRECTION.md`, Artist), with its sections in the
//! same order: the head with the 200 dp photo in the pixel circle, the name,
//! albums, singles and fans; the 56 dp play button, Segui / Segui già and Radio. Then, from the official
//! catalogue («Altro di …», `ArtistExtras`): *Popolari* (5, or two columns of five above 1500 dp; «Mostra
//! altri» shows them all) and *Discografia* (tabs Tutto, Album, Singoli when both kinds exist; a cover opens
//! the album) on the fluid `TileGrid`; then the tracks already in the server catalogue, *Altri artisti nel
//! catalogo* (one row of photos), *Informazioni* and the *Crediti*. The page fills the frame (desktop v2 §2)
//! and enters with the page-enter move.
//!
//! The core's catalogue keeps the page (`CatalogView::artist`, LRU of 40, quietly reread after 90 s) and the
//! external domain «Altro di …» (`ExternalView::artists`). Everything is one virtual list: the discography
//! grid is rows of tiles, as many as the width takes, and only the visible ones are built.

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, ListState, ParentElement, Render,
    SharedString, Styled, Subscription, Window, div, list,
};
use psytify_core::api::{ArtistPage as Artist, SourceItem, SourcePlaylist, Track};
use psytify_core::catalog::{Load, portrait_url};
use psytify_core::command::{CatalogCmd, DownloadsCmd, ExternalCmd, LibraryCmd, PlaybackCmd, Resource};
use psytify_core::config::FollowedArtist;
use psytify_core::feed::{self, ArtistCard};
use psytify_core::snapshot::{ArtistExtras, CatalogView, DownloadsView, ExternalView, LibraryView};
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_ERROR, TEXT_PRIMARY, TEXT_SECONDARY};
use psytify_design::tokens::size::{BP_FLOW_3, TILE_GAP};
use psytify_design::tokens::space::{SPACE_3, SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{TYPE_BODY, TYPE_META, TYPE_NUMERIC};
use uuid::Uuid;

use super::album_page::{credit_line, credits, failure, skeleton_page, skeleton_row};
use super::page_frame::PageFrame;
use super::page_list::{new_list, remeasure_header, set_rows, to_top};
use super::search_page::source_data;
use super::track_list::{
    RowText, batch_label, download_all_label, download_command, download_state, now_playing, plays,
    with_links,
};
use super::track_menu::TrackMenu;
use super::{Page, PageEvent};
use crate::components::button::Button;
use crate::components::frame::{Flow, Frame, TileGrid, measure};
use crate::components::icon_button::IconButton;
use crate::components::indicators::PlayState;
use crate::components::navigation::{Tab, Tabs};
use crate::components::page_header::{HeaderArt, PageHeader};
use crate::components::playing_spot::PlayingSpot;
use crate::components::section::{note_at, section_heading_at};
use crate::components::source_row::{SourceData, SourceRow};
use crate::components::tile::AlbumTile;
use crate::components::track_row::{TrackData, TrackRow};
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{TypeStyle, col, dp};

/// Tracks per column of *Popolari*; «Mostra altri» shows them all.
const POPULAR: usize = 5;
/// Other artists offered at the bottom (at most, and only those that fit one row).
const RELATED: usize = 12;
/// Skeleton rows while «Altro di …» arrives.
const EXTRAS_SKELETON: usize = 6;

/// Columns of *Popolari* for an inner width: two above `BP_FLOW_3`.
fn popular_columns(inner: f32) -> usize {
    if inner >= BP_FLOW_3 { 2 } else { 1 }
}

/// How the discography is filtered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Discography {
    #[default]
    All,
    Albums,
    Singles,
}

impl Discography {
    const ALL: [Self; 3] = [Self::All, Self::Albums, Self::Singles];

    fn label(self) -> &'static str {
        match self {
            Self::All => "Tutto",
            Self::Albums => "Album",
            Self::Singles => "Singoli",
        }
    }

    fn keeps(self, release: &SourcePlaylist) -> bool {
        match self {
            Self::All => true,
            Self::Albums => release.kind == "album",
            Self::Singles => release.kind != "album",
        }
    }
}

/// The page's sections, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Popular,
    Discography,
    /// The server's tracks: «Già nel catalogo del server» next to the official catalogue, «Brani» alone.
    Server,
    Related,
    About,
    Credits,
}

/// A row after the head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Heading(Section),
    /// The discography tabs (only with albums and singles together).
    Tabs,
    /// A line of *Popolari*: one cell per column.
    Popular(usize),
    /// A grid row: the releases from `start`, at most `columns`.
    Releases(usize),
    /// «Altro di …» is on its way: empty rows at their real size.
    ExtrasLoading,
    ExtrasFailed,
    /// Neither the official catalogue nor server tracks.
    Empty,
    Track(usize),
    Related,
    About,
    Credit(usize),
}

/// What there is to show: the counts, already computed. Pure, so `plan` can be tested.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Shape {
    /// `None` until «Altro di …» is ready, or when it failed.
    extras: Option<ExtrasShape>,
    extras_failed: bool,
    tracks: usize,
    related: usize,
    credits: usize,
    columns: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ExtrasShape {
    /// The lines of *Popolari* to show (the tracks over the columns).
    popular: usize,
    /// The releases after the filter.
    releases: usize,
    /// The tabs are needed (there are albums and singles).
    tabs: bool,
}

/// The page's rows. Pure.
fn plan(shape: Shape) -> Vec<Row> {
    let mut rows = Vec::new();
    match shape.extras {
        Some(extras) => {
            if extras.popular > 0 {
                rows.push(Row::Heading(Section::Popular));
                rows.extend((0..extras.popular).map(Row::Popular));
            }
            if extras.releases > 0 || extras.tabs {
                rows.push(Row::Heading(Section::Discography));
                if extras.tabs {
                    rows.push(Row::Tabs);
                }
                let columns = shape.columns.max(1);
                rows.extend((0..extras.releases).step_by(columns).map(Row::Releases));
            }
            if extras.popular == 0 && extras.releases == 0 && !extras.tabs && shape.tracks == 0 {
                rows.push(Row::Empty);
            }
        }
        None if shape.extras_failed => {
            if shape.tracks == 0 {
                rows.push(Row::ExtrasFailed);
            }
        }
        None => rows.extend((0..EXTRAS_SKELETON).map(|_| Row::ExtrasLoading)),
    }
    if shape.tracks > 0 {
        rows.push(Row::Heading(Section::Server));
        rows.extend((0..shape.tracks).map(Row::Track));
    }
    if shape.related > 0 {
        rows.push(Row::Heading(Section::Related));
        rows.push(Row::Related);
    }
    if shape.tracks > 0 {
        rows.push(Row::Heading(Section::About));
        rows.push(Row::About);
        if shape.credits > 0 {
            rows.push(Row::Heading(Section::Credits));
            rows.extend((0..shape.credits).map(Row::Credit));
        }
    }
    rows
}

/// «4 album · 6 singoli ed EP · 12.400 fan» from the official catalogue; «N brani» until it arrives.
fn meta(extras: Option<&ArtistExtras>, tracks: usize) -> String {
    let Some(extras) = extras else { return format::tracks(tracks) };
    let mut parts = Vec::new();
    let albums = extras.albums.iter().filter(|a| a.kind == "album").count();
    if albums > 0 {
        parts.push(format!("{albums} album"));
    }
    let others = extras.albums.len() - albums;
    if others > 0 {
        parts.push(format!("{others} singoli ed EP"));
    }
    if extras.artist.fans > 0 {
        parts.push(format::fans(extras.artist.fans));
    }
    if parts.is_empty() { format::tracks(tracks) } else { parts.join(" · ") }
}

/// «2024 · Album» under a discography cover.
fn release_subtitle(release: &SourcePlaylist) -> SharedString {
    match release.released_on.as_deref().and_then(|date| date.get(..4)) {
        Some(year) => format!("{year} · {}", release.kind_label()).into(),
        None => release.kind_label().into(),
    }
}

/// What the page shows.
#[derive(Clone, Debug, PartialEq)]
enum Shown {
    Loading,
    Failed(SharedString),
    Ready,
}

/// A discography cover, with its texts ready.
struct Release {
    playlist: SourcePlaylist,
    subtitle: SharedString,
}

pub struct ArtistPage {
    core: Entity<CoreModel>,
    id: Option<Uuid>,
    shown: Shown,
    artist: Option<Arc<Artist>>,
    extras: Option<ArtistExtras>,
    extras_failed: bool,
    tracks: Arc<[Track]>,
    texts: Vec<RowText>,
    popular: Vec<SourceData>,
    releases: Vec<Release>,
    related: Vec<ArtistCard>,
    credits: Vec<SharedString>,
    about: Vec<SharedString>,
    filter: Discography,
    more_popular: bool,
    /// The columns the rows were planned for: tile grid and *Popolari*.
    columns: (usize, usize),
    rows: Vec<Row>,
    meta: SharedString,
    portrait: Option<SharedString>,
    following: bool,
    radio: Option<Track>,
    /// «Scarica tutto» of the server's tracks, only when the official catalogue gives no *Popolari* (as in
    /// the page, which then plays the server's tracks); and the running batch's count.
    download_label: Option<&'static str>,
    batch: Option<SharedString>,
    liked: Arc<HashSet<Uuid>>,
    library: Arc<LibraryView>,
    downloads: Arc<DownloadsView>,
    external: Arc<ExternalView>,
    now: Option<(Uuid, PlayState)>,
    list: ListState,
    frame: PageFrame,
    /// Where the playing track's equalizer goes: `Shell` paints it (BACKLOG 23).
    spot: PlayingSpot,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for ArtistPage {}

impl ArtistPage {
    /// This view's equalizer spot, for `Shell`'s layer.
    pub fn spot(&self) -> PlayingSpot {
        self.spot.clone()
    }

    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let changes =
            cx.subscribe(&core, |this, core, changed: &Changed, cx| this.refresh(&core, *changed, cx));
        let snapshot = core.read(cx).snapshot();
        Self {
            id: None,
            shown: Shown::Loading,
            artist: None,
            extras: None,
            extras_failed: false,
            tracks: Arc::from(Vec::new()),
            texts: Vec::new(),
            popular: Vec::new(),
            releases: Vec::new(),
            related: Vec::new(),
            credits: Vec::new(),
            about: Vec::new(),
            filter: Discography::All,
            more_popular: false,
            columns: (0, 0),
            rows: Vec::new(),
            meta: SharedString::default(),
            portrait: None,
            following: false,
            radio: None,
            download_label: None,
            batch: batch_label(&snapshot.downloads),
            liked: core.read(cx).liked(),
            library: Arc::clone(&snapshot.library),
            downloads: Arc::clone(&snapshot.downloads),
            external: Arc::clone(&snapshot.external),
            now: now_playing(&snapshot.playback),
            list: new_list(0),
            frame: PageFrame::default(),
            core,
            spot: PlayingSpot::new(),
            _changes: changes,
        }
    }

    /// The page comes into view (`Shell::page_shown`).
    pub fn enter(&mut self, rise: bool, cx: &mut Context<Self>) {
        self.frame.enter(rise, cx);
        cx.notify();
    }

    /// The grid and *Popolari* columns for the frame now.
    fn plan_columns(&self) -> (usize, usize) {
        (self.frame.frame.grid.columns, popular_columns(self.frame.frame.inner))
    }

    /// A new width: new column counts rebuild the rows, a new tile size only redraws.
    fn set_frame(&mut self, frame: Frame, cx: &mut Context<Self>) {
        if frame == self.frame.frame {
            return;
        }
        self.frame.frame = frame;
        if self.plan_columns() != self.columns {
            self.rebuild();
        }
        cx.notify();
    }

    /// Shows artist `id`: the core serves it from its cache or loads it. Another artist resets the filter
    /// and «Mostra altri».
    pub fn show(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.core.read(cx).catalog(CatalogCmd::Ensure(Resource::Artist(id)));
        if self.id == Some(id) {
            return;
        }
        self.id = Some(id);
        self.artist = None;
        self.extras = None;
        self.extras_failed = false;
        self.filter = Discography::All;
        self.more_popular = false;
        let snapshot = self.core.read(cx).snapshot();
        self.take(&snapshot.catalog);
        self.take_extras(&snapshot.external);
        self.rebuild();
        to_top(&self.list);
        cx.notify();
    }

    fn refresh(&mut self, core: &Entity<CoreModel>, changed: Changed, cx: &mut Context<Self>) {
        let snapshot = core.read(cx).snapshot();
        let mut rebuild = false;
        let mut dirty = false;
        if changed.catalog {
            rebuild |= self.take(&snapshot.catalog);
        }
        if changed.external && !Arc::ptr_eq(&snapshot.external, &self.external) {
            self.external = Arc::clone(&snapshot.external);
            rebuild |= self.take_extras(&snapshot.external);
            // «Apro…» on the *Popolari* rows while the core imports.
            dirty = true;
        }
        if changed.library && !Arc::ptr_eq(&snapshot.library, &self.library) {
            let feed_changed = !Arc::ptr_eq(&snapshot.library.feed, &self.library.feed)
                || snapshot.library.stats.len() != self.library.stats.len();
            self.library = Arc::clone(&snapshot.library);
            self.liked = core.read(cx).liked();
            rebuild |= feed_changed;
            let following = self.is_following();
            if following != self.following {
                self.following = following;
                remeasure_header(&self.list);
            }
            dirty = true;
        }
        if changed.downloads && !Arc::ptr_eq(&snapshot.downloads, &self.downloads) {
            self.downloads = Arc::clone(&snapshot.downloads);
            let batch = batch_label(&self.downloads);
            if batch != self.batch {
                self.batch = batch;
                remeasure_header(&self.list);
            }
            dirty = true;
        }
        if changed.playback {
            let now = now_playing(&snapshot.playback);
            if now != self.now {
                self.now = now;
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

    fn is_following(&self) -> bool {
        self.id.is_some_and(|id| self.library.followed.iter().any(|artist| artist.id == id))
    }

    /// Takes this artist's page from the catalogue; `true` if it changed.
    fn take(&mut self, catalog: &CatalogView) -> bool {
        let Some(id) = self.id else { return false };
        let (shown, artist) = match &catalog.artist {
            Some((open, Load::Ready(page))) if *open == id => (Shown::Ready, Some(Arc::clone(page))),
            Some((open, Load::Failed(error))) if *open == id => {
                (Shown::Failed(error.to_string().into()), None)
            }
            _ => (Shown::Loading, None),
        };
        let same = match (&artist, &self.artist) {
            (Some(new), Some(old)) => Arc::ptr_eq(new, old),
            (None, None) => true,
            _ => false,
        };
        if same && shown == self.shown {
            return false;
        }
        self.shown = shown;
        self.artist = artist;
        true
    }

    /// Takes this artist's «Altro di …»; `true` if it changed.
    fn take_extras(&mut self, external: &ExternalView) -> bool {
        let Some(id) = self.id else { return false };
        let (extras, failed) = match external.artist(id) {
            Some(Load::Ready(extras)) => (Some(extras), false),
            Some(Load::Failed(_)) => (None, true),
            _ => (None, false),
        };
        let same = match (extras, &self.extras) {
            (Some(new), Some(old)) => {
                Arc::ptr_eq(&new.top, &old.top) && Arc::ptr_eq(&new.albums, &old.albums)
            }
            (None, None) => true,
            _ => false,
        };
        if same && failed == self.extras_failed {
            return false;
        }
        self.extras = extras.cloned();
        self.extras_failed = failed;
        true
    }

    /// Rebuilds texts and rows: when the page, «Altro di …», the filter, the columns or the feed change.
    fn rebuild(&mut self) {
        self.columns = self.plan_columns();
        let (grid_columns, popular_cols) = self.columns;
        let Some(artist) = self.artist.clone() else {
            self.rows.clear();
            set_rows(&self.list, 0);
            return;
        };
        self.tracks = Arc::from(artist.tracks.as_slice());
        self.texts = self.tracks.iter().map(RowText::of).collect();
        self.portrait = portrait_url(&artist)
            .or_else(|| self.extras.as_ref().and_then(|e| e.artist.image_url.as_deref()))
            .map(|url| SharedString::from(url.to_owned()));
        self.meta = meta(self.extras.as_ref(), self.tracks.len()).into();
        self.following = self.is_following();
        // The radio starts from this artist's most played track already in the server catalogue.
        self.radio = feed::popular(&self.tracks, self.library.stats.as_slice(), 1).into_iter().next();
        let has_top = self.extras.as_ref().is_some_and(|extras| !extras.top.is_empty());
        self.download_label = if has_top { None } else { download_all_label(&self.tracks) };
        let extras_shape = self.extras.as_ref().map(|extras| {
            self.popular = extras
                .top
                .iter()
                .take(if self.more_popular { extras.top.len() } else { POPULAR * popular_cols })
                .enumerate()
                .map(|(position, item)| source_data(position, item))
                .collect();
            let albums = extras.albums.iter().filter(|a| a.kind == "album").count();
            let singles = extras.albums.len() - albums;
            let tabs = albums > 0 && singles > 0;
            // With one kind of release the tabs are not needed and the filter does not count.
            let filter = if tabs { self.filter } else { Discography::All };
            self.releases = extras
                .albums
                .iter()
                .filter(|release| filter.keeps(release))
                .map(|release| Release { playlist: release.clone(), subtitle: release_subtitle(release) })
                .collect();
            let lines = Flow { columns: popular_cols }.rows(self.popular.len());
            ExtrasShape { popular: lines, releases: self.releases.len(), tabs }
        });
        if extras_shape.is_none() {
            self.popular.clear();
            self.releases.clear();
        }
        self.related = self
            .library
            .feed
            .artists
            .iter()
            .filter(|card| card.artist.id != artist.id)
            .take(RELATED.min(grid_columns))
            .cloned()
            .collect();
        self.credits = credits(&self.tracks);
        self.about = self.about_lines(&artist);
        self.rows = plan(Shape {
            extras: extras_shape,
            extras_failed: self.extras_failed,
            tracks: self.tracks.len(),
            related: self.related.len(),
            credits: self.credits.len(),
            columns: grid_columns,
        });
        set_rows(&self.list, self.rows.len());
        remeasure_header(&self.list);
    }

    /// The lines of *Informazioni*: tracks and duration, your plays, the licences.
    fn about_lines(&self, artist: &Artist) -> Vec<SharedString> {
        let total: u64 = self.tracks.iter().map(|t| t.duration_ms).sum();
        let mut lines = vec![SharedString::from(format!(
            "{} · {}",
            format::tracks(self.tracks.len()),
            format::total(total)
        ))];
        let plays: u32 = self
            .library
            .stats
            .iter()
            .filter(|stat| stat.track.artist.id == artist.id)
            .map(|stat| stat.plays)
            .sum();
        if plays > 0 {
            lines.push(format!("Li hai ascoltati {plays} volte").into());
        }
        let licences = format::licence_summary(self.tracks.iter().map(|t| t.licence.label()));
        lines.push(format!("Licenze: {licences}").into());
        lines
    }

    fn set_filter(&mut self, filter: Discography, cx: &mut Context<Self>) {
        if filter != self.filter {
            self.filter = filter;
            self.rebuild();
            cx.notify();
        }
    }

    /// Play: the most played tracks of the official catalogue (the server imports them one at a time);
    /// until they arrive, the server's tracks.
    fn play(&self, start: usize, cx: &gpui::App) {
        let Some(id) = self.id else { return };
        let core = self.core.read(cx);
        if self.extras.as_ref().is_some_and(|extras| !extras.top.is_empty()) {
            core.external(ExternalCmd::PlayArtistTop { artist: id, start, video: false });
        } else if !self.tracks.is_empty() {
            core.playback(PlaybackCmd::Play { tracks: Arc::clone(&self.tracks), start: 0 });
        }
    }

    fn play_track(&self, start: usize, cx: &gpui::App) {
        let core = self.core.read(cx);
        match (self.tracks.get(start), self.now) {
            (Some(track), Some((id, _))) if track.id == id => core.playback(PlaybackCmd::Toggle),
            _ => core.playback(PlaybackCmd::Play { tracks: Arc::clone(&self.tracks), start }),
        }
    }

    fn item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let element = match ix.checked_sub(1) {
            None => self.artist.clone().map(|artist| self.header(&artist, cx).into_any_element()),
            Some(row) => self.row(row, cx),
        };
        let element = element.unwrap_or_else(|| div().into_any_element());
        self.frame.wrap(element, self.section(ix), window)
    }

    /// The enter step of list item `ix`: the head, then one per section heading.
    fn section(&self, ix: usize) -> usize {
        let Some(row) = ix.checked_sub(1) else { return 0 };
        1 + self.rows.iter().take(row + 1).filter(|row| matches!(row, Row::Heading(_))).count()
    }

    fn header(&self, artist: &Artist, cx: &mut Context<Self>) -> PageHeader {
        let mut header =
            PageHeader::new(HeaderArt::Round(self.portrait.clone()), "Artista", artist.name.clone())
                .margin(self.frame.margin())
                .meta(self.meta.clone());
        let can_play =
            self.extras.as_ref().is_some_and(|extras| !extras.top.is_empty()) || !self.tracks.is_empty();
        if can_play {
            header = header.play("artist-play", cx.listener(|this, _: &ClickEvent, _, cx| this.play(0, cx)));
        }
        let followed = FollowedArtist {
            id: artist.id,
            name: artist.name.clone(),
            cover_url: self.portrait.as_ref().map(ToString::to_string),
        };
        let mut follow = Button::new("artist-follow", if self.following { "Segui già" } else { "Segui" })
            .secondary()
            .selected(self.following);
        if !self.following {
            follow = follow.icon(sprite::ICON_USER_PLUS);
        }
        header = header.action(follow.on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
            cx.emit(PageEvent::ToggleFollow(Box::new(followed.clone())));
        })));
        if let Some(track) = self.radio.clone() {
            header = header.action(
                Button::new("artist-radio", "Radio").secondary().icon(sprite::ICON_RADIO).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.core.read(cx).playback(PlaybackCmd::StartRadio(Box::new(track.clone())));
                    }),
                ),
            );
        }
        match (&self.batch, self.download_label) {
            // While the batch downloads, the count replaces the icon.
            (Some(count), Some(_)) => {
                header = header
                    .action(div().type_role(TYPE_NUMERIC).text_color(col(TEXT_PRIMARY)).child(count.clone()))
            }
            (None, Some(label)) => {
                header = header.action(
                    IconButton::new("artist-download", sprite::ICON_DOWNLOAD).tooltip(label, None).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.core.read(cx).downloads(DownloadsCmd::DownloadAll(Arc::clone(&this.tracks)));
                        }),
                    ),
                )
            }
            _ => {}
        }
        header
    }

    fn heading(&self, section: Section, cx: &mut Context<Self>) -> AnyElement {
        let title: SharedString = match section {
            Section::Popular => "Popolari".into(),
            Section::Discography => "Discografia".into(),
            Section::Server if self.extras.is_some() => "Già nel catalogo del server".into(),
            Section::Server => "Brani".into(),
            Section::Related => "Altri artisti nel catalogo".into(),
            Section::About => "Informazioni".into(),
            Section::Credits => "Crediti".into(),
        };
        let heading = section_heading_at(title, SharedString::default(), self.frame.margin());
        let more = self.extras.as_ref().is_some_and(|extras| extras.top.len() > POPULAR);
        if section == Section::Popular && more {
            let label = if self.more_popular { "Mostra meno" } else { "Mostra altri" };
            return heading
                .child(div().flex_1())
                .child(Button::new("artist-more", label).ghost().on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.more_popular = !this.more_popular;
                        this.rebuild();
                        cx.notify();
                    },
                )))
                .into_any_element();
        }
        heading.into_any_element()
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let margin = self.frame.margin();
        Some(match *self.rows.get(ix)? {
            Row::Heading(section) => self.heading(section, cx),
            Row::Tabs => {
                let selected = Discography::ALL.iter().position(|f| *f == self.filter).unwrap_or(0);
                let tabs = Discography::ALL
                    .iter()
                    .map(|filter| {
                        Tab::new(SharedString::from(format!("disc-{}", filter.label())), filter.label())
                    })
                    .collect();
                let entity = cx.entity().downgrade();
                div()
                    .px(dp(margin))
                    .pb(dp(SPACE_5))
                    .child(Tabs::new(tabs, selected).on_select(move |ix, _, cx| {
                        let filter = Discography::ALL.get(ix).copied().unwrap_or_default();
                        entity.update(cx, |this, cx| this.set_filter(filter, cx)).ok();
                    }))
                    .into_any_element()
            }
            Row::Popular(line) => self.popular_line(line, cx),
            Row::Releases(start) => self.releases_row(start, cx),
            Row::ExtrasLoading => skeleton_row(margin).into_any_element(),
            Row::ExtrasFailed => note_at(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_ERROR))
                    .child("Il catalogo ufficiale non risponde: riprova più tardi."),
                margin,
            )
            .into_any_element(),
            Row::Empty => note_at(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_SECONDARY))
                    .child("Il catalogo non ha brani di questo artista."),
                margin,
            )
            .into_any_element(),
            Row::Track(position) => self.frame.row(self.track_row(position, cx)?),
            Row::Related => self.related_row(cx),
            Row::About => self.about_card(cx),
            Row::Credit(position) => {
                credit_line(self.credits.get(position)?.clone(), position + 1 == self.credits.len(), margin)
                    .into_any_element()
            }
        })
    }

    /// A line of *Popolari*: one track per column, the columns read top to bottom.
    fn popular_line(&self, line: usize, cx: &mut Context<Self>) -> AnyElement {
        let flow = Flow { columns: self.columns.1 };
        let extras = self.extras.as_ref();
        let cells = flow.cells(line, self.popular.len()).filter_map(|position| {
            let item = extras?.top.get(position)?;
            Some(
                SourceRow::new(("artist-popular", position), self.popular.get(position)?.clone())
                    .busy(self.importing(item))
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        if plays(event) {
                            this.play(position, cx);
                        }
                    }))
                    .into_any_element(),
            )
        });
        flow.row(self.frame.margin() - SPACE_3, cells).into_any_element()
    }

    fn importing(&self, item: &SourceItem) -> bool {
        self.external.importing(item.source, &item.id)
    }

    /// A row of discography tiles: a click opens the album (the source page).
    fn releases_row(&self, start: usize, cx: &mut Context<Self>) -> AnyElement {
        let tile = self.frame.frame.grid.tile;
        let end = (start + self.columns.0.max(1)).min(self.releases.len());
        let tiles = (start..end).filter_map(|position| {
            let release = self.releases.get(position)?;
            let playlist = release.playlist.clone();
            Some(
                AlbumTile::new(
                    ("artist-release", position),
                    release.playlist.title.clone(),
                    release.subtitle.clone(),
                )
                .side(tile)
                .cover_url(release.playlist.cover_url.clone().map(SharedString::from))
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.emit(PageEvent::OpenSource(Box::new(playlist.clone())));
                }))
                .into_any_element(),
            )
        });
        TileGrid::row(self.frame.margin(), tiles).pb(dp(TILE_GAP)).into_any_element()
    }

    /// A row of other artists of the catalogue: a click opens their page.
    fn related_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let tile = self.frame.frame.grid.tile;
        let tiles = self.related.iter().enumerate().map(|(position, card)| {
            let id = card.artist.id;
            AlbumTile::new(("artist-related", position), card.artist.name.clone(), "Artista")
                .side(tile)
                .round()
                .cover_url(card.cover_url.clone().map(SharedString::from))
                .on_click(
                    cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Artist(id)))),
                )
                .into_any_element()
        });
        TileGrid::row(self.frame.margin(), tiles).pb(dp(TILE_GAP)).into_any_element()
    }

    /// *Informazioni*: the data and «Cerca altro di …», on the canvas (no cards, per the art direction).
    fn about_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let name = self.artist.as_ref().map(|artist| artist.name.clone()).unwrap_or_default();
        let lines = self.about.iter().enumerate().map(|(ix, line)| {
            div()
                .type_role(TYPE_META)
                .text_color(col(if ix == 0 { TEXT_PRIMARY } else { TEXT_SECONDARY }))
                .child(line.clone())
        });
        div()
            .px(dp(self.frame.margin()))
            .pb(dp(SPACE_6))
            .child(
                div().max_w(measure(TYPE_META)).flex().flex_col().gap(dp(SPACE_3)).children(lines).child(
                    div().pt(dp(SPACE_3)).flex().child(
                        Button::new("artist-search", format!("Cerca altro di {name}"))
                            .secondary()
                            .icon(sprite::ICON_SEARCH)
                            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                cx.emit(PageEvent::Search(name.clone().into()));
                            })),
                    ),
                ),
            )
            .into_any_element()
    }

    fn track_row(&self, position: usize, cx: &mut Context<Self>) -> Option<TrackRow> {
        let track = self.tracks.get(position)?;
        let text = self.texts.get(position)?;
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
        let liked_track = track.clone();
        let download_track = track.clone();
        let menu_track = track.clone();
        let mut row = TrackRow::new(("artist-row", position), data)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if plays(event) {
                    this.play_track(position, cx);
                }
            }))
            .on_like(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.core.read(cx).library(LibraryCmd::ToggleLike(liked_track.clone()));
            }))
            .on_download(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.core.read(cx).downloads(download_command(&this.downloads, &download_track));
            }))
            .on_more(cx.listener(move |_, event: &ClickEvent, _, cx| {
                cx.emit(PageEvent::TrackMenu(Box::new(TrackMenu {
                    track: menu_track.clone(),
                    at: event.position(),
                    from_playlist: None,
                })));
            }));
        row = with_links(row, track, cx);
        if let Some((id, state)) = self.now
            && id == track.id
        {
            row = row.playing_at(state, &self.spot);
        }
        Some(row)
    }

    fn retry(&self, cx: &gpui::App) {
        if let Some(id) = self.id {
            self.core.read(cx).catalog(CatalogCmd::Retry(Resource::Artist(id)));
        }
    }
}

impl Render for ArtistPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The spot is marked again in this frame's paint, if the playing row is visible.
        self.spot.clear();
        self.frame.begin(window, Some(&self.list));
        let page = div()
            .relative()
            .size_full()
            .bg(col(SURFACE_CANVAS))
            .child(self.frame.measure(cx.entity().downgrade(), Self::set_frame));
        match &self.shown {
            Shown::Loading => page.child(skeleton_page(true, self.frame.margin())),
            Shown::Failed(error) => page.child(failure(
                "artist-retry",
                error.clone(),
                cx.listener(|this, _: &ClickEvent, _, cx| this.retry(cx)),
            )),
            Shown::Ready => page.child(
                list(
                    self.list.clone(),
                    cx.processor(|this: &mut Self, ix: usize, window, cx| this.item(ix, window, cx)),
                )
                .size_full(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> Shape {
        Shape { columns: 4, ..Shape::default() }
    }

    #[test]
    fn the_sections_follow_the_iced_order() {
        let rows = plan(Shape {
            extras: Some(ExtrasShape { popular: 2, releases: 5, tabs: true }),
            tracks: 1,
            related: 3,
            credits: 1,
            ..shape()
        });
        assert_eq!(
            rows,
            vec![
                Row::Heading(Section::Popular),
                Row::Popular(0),
                Row::Popular(1),
                Row::Heading(Section::Discography),
                Row::Tabs,
                Row::Releases(0),
                Row::Releases(4),
                Row::Heading(Section::Server),
                Row::Track(0),
                Row::Heading(Section::Related),
                Row::Related,
                Row::Heading(Section::About),
                Row::About,
                Row::Heading(Section::Credits),
                Row::Credit(0),
            ]
        );
    }

    #[test]
    fn while_the_official_catalogue_loads_there_are_placeholder_rows() {
        let rows = plan(Shape { tracks: 1, ..shape() });
        assert_eq!(rows.iter().filter(|row| **row == Row::ExtrasLoading).count(), EXTRAS_SKELETON);
        assert!(rows.contains(&Row::Track(0)));
    }

    #[test]
    fn a_failure_shows_only_when_there_is_nothing_else() {
        assert_eq!(plan(Shape { extras_failed: true, ..shape() }), vec![Row::ExtrasFailed]);
        let rows = plan(Shape { extras_failed: true, tracks: 1, ..shape() });
        assert!(!rows.contains(&Row::ExtrasFailed));
    }

    #[test]
    fn an_artist_with_nothing_says_so() {
        let rows = plan(Shape { extras: Some(ExtrasShape::default()), ..shape() });
        assert_eq!(rows, vec![Row::Empty]);
    }

    #[test]
    fn popular_takes_two_columns_above_1500_dp() {
        assert_eq!(popular_columns(1499.0), 1);
        assert_eq!(popular_columns(1500.0), 2);
        // Ten tracks in two columns: five lines, the first column 0..5, the second 5..10.
        let flow = Flow { columns: 2 };
        assert_eq!(flow.rows(10), 5);
        assert_eq!(flow.cells(0, 10).collect::<Vec<_>>(), vec![0, 5]);
    }

    #[test]
    fn the_filter_splits_albums_from_singles_and_eps() {
        let release = |kind: &str| SourcePlaylist {
            source: psytify_core::api::SourceKind::Deezer,
            id: "1".into(),
            title: "X".into(),
            owner: "Y".into(),
            cover_url: None,
            kind: kind.into(),
            released_on: Some("2021-03-04".into()),
        };
        assert!(Discography::Albums.keeps(&release("album")));
        assert!(!Discography::Albums.keeps(&release("ep")));
        assert!(Discography::Singles.keeps(&release("single")));
        assert!(Discography::All.keeps(&release("ep")));
        assert_eq!(release_subtitle(&release("album")), SharedString::from("2021 · Album"));
    }
}
