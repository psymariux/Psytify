//! The pages the app opens: Cerca, Libreria, album, artist, an external album or playlist, the
//! liked tracks and a playlist. Each page is a model here, built from the core's snapshot with the
//! same rules as the GPUI pages (`desktop/src/app/*_page.rs`): which rows, in which order, with
//! which texts, and what a tap or a button does. The app only draws the rows.
//!
//! The app opens a page with `PsytifyCore::open_page` and gets a [`PageHandle`]; while the handle
//! lives, the bridge rebuilds the page when a domain it reads changes and tells the page's
//! [`PageListener`] only when what it shows changed. Long lists cross a page of rows at a time
//! ([`PageHandle::rows`]); the view carries the first [`FIRST_ROWS`]. Taps and buttons come back with
//! the view's version, so a tap on rows that have changed since does nothing.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use psytify_core::Command;
use psytify_core::Snapshot;
use psytify_core::api::Track;
use psytify_core::catalog::Load;
use psytify_core::command::{CatalogCmd, ExternalCmd, PlaybackCmd, Resource, SocialCmd};
use psytify_core::player::PlaybackState;
use psytify_core::snapshot::{ExternalView, ImportLinks, Importing, ImportsView};
use uuid::Uuid;

use crate::Inner;
use crate::dialog::{DialogView, Request};
use crate::downloads::{DownloadMark, Marks};
use crate::menu::MenuView;
use crate::playback::count;

mod album;
mod artist;
mod collection;
mod downloads;
pub(crate) mod friends;
pub(crate) mod jam;
mod library;
mod profile;
pub(crate) mod route;
mod search;
mod source;
#[cfg(test)]
mod tests;

pub use route::{Route, Source, SourceRef};

/// Rows carried by the view itself; the rest are read with [`PageHandle::rows`].
pub const FIRST_ROWS: usize = 60;
/// At most this many rows in one read.
pub const MAX_ROWS: u32 = 100;

/// The picture of a row or a header.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Art {
    /// A square cover; the drawn placeholder without an address.
    Cover { url: Option<String> },
    /// A portrait in the pixel circle (artists).
    Round { url: Option<String> },
    /// The pixel heart of «Brani che ti piacciono».
    Liked,
    /// The pixel icon of «Scaricati».
    Downloads,
    /// A past search.
    Search,
}

/// How a row of a track looks right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RowState {
    Idle,
    /// The current track, playing (the lime equaliser).
    Playing,
    /// The current track, paused.
    Paused,
    /// Loading, or being imported by the server («Apro…»): the row does not answer.
    Busy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Tone {
    Plain,
    /// Something is on its way: the loader next to the text.
    Waiting,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Chip {
    pub label: String,
    pub on: bool,
}

/// A row of a page. `key` is stable within the page: the app keys its list items with it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Row {
    /// A section title, with a count or a note on the right.
    Heading {
        key: String,
        title: String,
        detail: String,
    },
    Note {
        key: String,
        text: String,
        tone: Tone,
    },
    /// A track, playable: a tap plays it (or pauses it, if it is the current one).
    Track {
        key: String,
        /// The track number (album pages); `None` shows the cover instead.
        number: Option<u32>,
        art: Option<Art>,
        title: String,
        subtitle: String,
        duration: String,
        state: RowState,
        /// The row has a track for the «Altro» sheet and the swipe to the queue (an official row
        /// only once it has been imported).
        more: bool,
        /// Where the track is in the downloads.
        download: DownloadMark,
    },
    /// A download of the queue («Scaricati»): `PageHandle::row_action` cancels, retries or drops it.
    Download {
        key: String,
        art: Art,
        title: String,
        /// «In coda», «45%», «Non riuscito. …».
        status: String,
        /// Failed: the status in the error tint, Riprova and «Togli dall'elenco».
        failed: bool,
        /// Steps of the ten-step bar that are lit, while the download lives.
        steps: Option<u8>,
    },
    /// A track on the device («Scaricati»): a tap plays the saved tracks from it.
    Saved {
        key: String,
        art: Art,
        title: String,
        /// «Artista · FLAC · 12,3 MB», or what happened to its file.
        detail: String,
        /// The file is missing or changed: the detail in the error tint, «Scarica di nuovo».
        lost: bool,
        state: RowState,
    },
    /// An album, artist, playlist or past search: a tap opens it.
    Item {
        key: String,
        art: Art,
        title: String,
        detail: String,
        state: RowState,
    },
    /// Filters; `PageHandle::chip` picks one.
    Chips {
        key: String,
        chips: Vec<Chip>,
    },
    /// A text button on its own row («Mostra altri»).
    Action {
        key: String,
        label: String,
    },
    /// A person: the avatar in the pixel circle, the name, a status line and the row's buttons. A
    /// tap opens the profile (`PageHandle::tap`); a button goes to `PageHandle::row_action`.
    Person {
        key: String,
        avatar_url: Option<String>,
        name: String,
        /// «@luca», «Sta ascoltando Titolo · Artista», «Tu».
        status: String,
        /// Listening right now: the status has the lime live dot.
        live: bool,
        buttons: Vec<RowButton>,
    },
    /// The card «Ascolta insieme» of the Jam page with no jam: the name and code fields are the
    /// app's (`PsytifyCore::jam_create`, `jam_join`); the error that closed the last jam, if any.
    JamStart {
        key: String,
        failure: Option<String>,
        /// «Riprova» makes sense (a wrong code is corrected, not retried).
        retry: bool,
    },
    /// The card of the room: the code (none while connecting), `LIVE` once open, who is in it.
    /// `RowAction::Copy` copies the code, `RowAction::Leave` leaves.
    JamRoom {
        key: String,
        code: Option<String>,
        members: Vec<String>,
    },
}

/// How a row's button looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ButtonKind {
    Secondary,
    /// Text only (Rifiuta, Ritira, Ignora).
    Ghost,
}

/// A button of a row.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RowButton {
    pub action: RowAction,
    pub label: String,
    pub kind: ButtonKind,
    /// Off while a request for this row is on its way, or for a state shown as a button («Amici»).
    pub enabled: bool,
    /// A choice that is on («Scelto»).
    pub on: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PageAction {
    Play,
    Shuffle,
    /// Save or remove the album or the external playlist.
    Save,
    Follow,
    Radio,
    /// «Riprova» after a failed load.
    Retry,
    /// The header's subtitle: the album's artist.
    OpenArtist,
    /// Libreria: a new, empty playlist (the dialog).
    NewPlaylist,
    /// A playlist page: rename or delete it (the dialogs).
    Rename,
    Delete,
    /// One's own profile: sign out.
    SignOut,
    /// «Scarica tutto» on an album or a playlist.
    DownloadAll,
    /// «Scaricati»: «Controlla i file».
    Verify,
    /// Someone else's profile: the relation button (add, accept, withdraw, or remove a friend after
    /// a confirmation).
    Relation,
}

/// A button of a row («Scaricati»).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RowAction {
    /// «Annulla» a download that waits or runs.
    Cancel,
    /// «Riprova» a failed download.
    Retry,
    /// «Togli dall'elenco» a failed download.
    Dismiss,
    /// «Rimuovi dal dispositivo».
    Remove,
    /// «Scarica di nuovo» a track whose file is lost.
    DownloadAgain,
    /// A person found or asking: «Aggiungi», «Accetta».
    Add,
    /// «Rifiuta» a request, «Ritira» one sent.
    Refuse,
    /// A jam invite: «Entra».
    Join,
    /// A jam invite: «Ignora».
    Ignore,
    /// A friend to invite into the jam: «Scegli» / «Scelto».
    Choose,
    /// The jam's code to the clipboard («Copia»).
    Copy,
    /// «Esci dalla jam».
    Leave,
}

/// A button under the header.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HeaderButton {
    pub action: PageAction,
    pub label: String,
    /// The primary button (the first, «Riproduci»).
    pub primary: bool,
    /// A switch that is on («Segui già», «Salvato»).
    pub on: bool,
    /// Off while a request it started is on its way.
    pub enabled: bool,
}

/// The head of an album, artist or playlist page.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Header {
    /// «Album · 2024», «Artista», «Playlist».
    pub kicker: String,
    pub title: String,
    /// The artist or the owner; it opens the artist when `PageAction::OpenArtist` is among the
    /// links.
    pub subtitle: Option<String>,
    pub subtitle_opens: bool,
    /// «12 brani · 48 min · Lossless».
    pub meta: String,
    pub art: Art,
    pub buttons: Vec<HeaderButton>,
    /// A line under the buttons (how the audio of an external album arrives).
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Enum)]
pub enum PageStatus {
    #[default]
    Loading,
    Ready,
    /// Nothing to show, and why (with a hint).
    Empty {
        message: String,
    },
    /// The load failed: `PageAction::Retry` tries again.
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PageView {
    /// Grows with every change of the page; taps, buttons and row reads take it back.
    pub version: u64,
    pub title: String,
    pub header: Option<Header>,
    pub status: PageStatus,
    /// Rows of the whole page.
    pub total: u32,
    /// Rows `0..rows.len()` (at most [`FIRST_ROWS`]).
    pub rows: Vec<Row>,
}

/// Rows `offset..offset + rows.len()` of the page at `version`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RowPage {
    pub version: u64,
    pub offset: u32,
    pub rows: Vec<Row>,
}

/// What a tap asks of the app, besides what it already sent to the core.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Tap {
    Nothing,
    Open {
        route: Route,
    },
    /// A past search was picked: the field shows it (the core already searches it).
    Search {
        query: String,
    },
    /// Put this text on the clipboard.
    Copy {
        text: String,
    },
    /// Share this text with another app.
    Share {
        text: String,
    },
    /// Show this dialog; its answer goes to `PsytifyCore::confirm_dialog`.
    Dialog {
        dialog: DialogView,
    },
    /// The page at this route is gone (a deleted playlist): take it off the back stacks.
    Leave {
        route: Route,
    },
}

/// Receives the page's changes, on a thread of the bridge (or of the caller of `open_page`).
#[uniffi::export(with_foreign)]
pub trait PageListener: Send + Sync {
    fn page_changed(&self, page: PageView);
}

/// The page as the bridge knows it, parsed from its route.
#[derive(Debug, Clone)]
pub(crate) enum Page {
    Search,
    Library,
    Album(Uuid),
    Artist(Uuid),
    Source(psytify_core::api::SourcePlaylist),
    Liked,
    Playlist(Uuid),
    Profile(String),
    Downloads,
    Friends,
    Jam,
    /// A friend's public playlist.
    Public {
        handle: String,
        id: Uuid,
    },
    /// A route whose identifier does not parse.
    Invalid,
}

impl Page {
    fn of(route: &Route) -> Self {
        let id = |text: &str| Uuid::parse_str(text).ok();
        match route {
            Route::Search => Self::Search,
            Route::Library => Self::Library,
            Route::Liked => Self::Liked,
            Route::Album { id: text } => id(text).map_or(Self::Invalid, Self::Album),
            Route::Artist { id: text } => id(text).map_or(Self::Invalid, Self::Artist),
            Route::Playlist { id: text } => id(text).map_or(Self::Invalid, Self::Playlist),
            Route::Source { source } => Self::Source(source.playlist()),
            Route::Profile { handle } => Self::Profile(handle.clone()),
            Route::Downloads => Self::Downloads,
            Route::Friends => Self::Friends,
            Route::Jam => Self::Jam,
            Route::FriendPlaylist { handle, id: text } => {
                id(text).map_or(Self::Invalid, |id| Self::Public { handle: handle.clone(), id })
            }
        }
    }

    /// What the core must load or open for this page, each time it opens (and when the core starts
    /// again under it).
    fn ensure(&self, snapshot: &Snapshot) -> Vec<Command> {
        match self {
            Self::Album(id) => vec![Command::Catalog(CatalogCmd::Ensure(Resource::Album(*id)))],
            Self::Artist(id) => vec![Command::Catalog(CatalogCmd::Ensure(Resource::Artist(*id)))],
            Self::Source(playlist) => {
                let open = snapshot.external.open.as_ref().map(|open| &open.playlist);
                if open.is_some_and(|open| open.source == playlist.source && open.id == playlist.id) {
                    Vec::new()
                } else {
                    vec![Command::External(ExternalCmd::OpenPlaylist(playlist.clone()))]
                }
            }
            // The core does nothing if this profile is already open and loaded.
            Self::Profile(handle) => vec![Command::Social(SocialCmd::OpenProfile(handle.clone()))],
            Self::Friends if snapshot.social.account.profile().is_some() => {
                vec![Command::Social(SocialCmd::RefreshFriends)]
            }
            Self::Public { handle, id } => {
                let open = matches!(&snapshot.social.playlist, Some(Load::Ready(open)) if open.id == *id);
                if open {
                    Vec::new()
                } else {
                    vec![Command::Social(SocialCmd::OpenFriendPlaylist { handle: handle.clone(), id: *id })]
                }
            }
            _ => Vec::new(),
        }
    }
}

/// What a row or a button does.
#[derive(Debug, Clone)]
pub(crate) enum Target {
    None,
    Open(Route),
    /// Play `list` from `start`, or play and pause if that track is the current one.
    Play(Arc<[Track]>, usize),
    Send(Command),
    /// Send and tell the person (saving, following).
    Notify(Command, String),
    /// Search this past query again.
    Search(String),
    /// Show every popular track of the artist page.
    MorePopular,
    /// Save the open external playlist as a new playlist of the library.
    SaveSource,
    /// Open a playlist dialog.
    Dialog(Request),
    /// Choose a friend to invite into the jam, or drop the choice.
    Choose(String),
    /// Invite the chosen friends.
    Invite,
    /// Enter the jam of this invite (the invite goes).
    JoinInvite(String),
    /// Put this text on the clipboard.
    Copy(String),
}

/// What a row watches to show its state: a track of the queue or an import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mark {
    None,
    Track(Uuid),
    Import(Importing),
}

/// The page's choices that are not in the core: the chip, «Mostra altri», the friends chosen for
/// a jam invite, and the jam event already left behind (`Inner::jam_form`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Ui {
    pub(crate) chip: usize,
    pub(crate) more: bool,
    pub(crate) chosen: Vec<String>,
    pub(crate) jam_dismissed: u64,
}

/// A page as built from a snapshot, before the row states.
#[derive(Debug, Default)]
pub(crate) struct Built {
    pub(crate) title: String,
    pub(crate) header: Option<Header>,
    pub(crate) status: PageStatus,
    pub(crate) rows: Vec<Row>,
    pub(crate) marks: Vec<Mark>,
    pub(crate) targets: Vec<Target>,
    /// Header buttons, by action.
    pub(crate) actions: Vec<(PageAction, Target)>,
    /// Buttons of rows: the row, the button, what it does.
    pub(crate) row_actions: Vec<(usize, RowAction, Target)>,
}

impl Built {
    pub(crate) fn titled(title: &str) -> Self {
        Self { title: title.to_owned(), ..Self::default() }
    }

    pub(crate) fn push(&mut self, row: Row, mark: Mark, target: Target) {
        self.rows.push(row);
        self.marks.push(mark);
        self.targets.push(target);
    }

    pub(crate) fn heading(&mut self, key: &str, title: &str, detail: String) {
        let row = Row::Heading { key: format!("h:{key}"), title: title.to_owned(), detail };
        self.push(row, Mark::None, Target::None);
    }

    pub(crate) fn note(&mut self, key: &str, text: String, tone: Tone) {
        self.push(Row::Note { key: format!("n:{key}"), text, tone }, Mark::None, Target::None);
    }

    pub(crate) fn action(&mut self, action: PageAction, target: Target) {
        self.actions.push((action, target));
    }

    pub(crate) fn row_action(&mut self, row: usize, action: RowAction, target: Target) {
        self.row_actions.push((row, action, target));
    }

    /// The same page on screen: nothing to tell the app.
    fn same(&self, other: &Self) -> bool {
        self.title == other.title
            && self.header == other.header
            && self.status == other.status
            && self.rows == other.rows
            && self.marks == other.marks
    }
}

fn build(page: &Page, snapshot: &Snapshot, ui: &Ui, memo: &mut downloads::Memo) -> Built {
    match page {
        Page::Search => search::build(snapshot),
        Page::Library => library::build(snapshot, ui),
        Page::Album(id) => album::build(*id, snapshot),
        Page::Artist(id) => artist::build(*id, snapshot, ui),
        Page::Friends => friends::build(snapshot),
        Page::Jam => jam::build(snapshot, ui),
        Page::Public { handle, id } => collection::public(handle, *id, snapshot),
        Page::Source(playlist) => source::build(playlist, snapshot),
        Page::Liked => collection::liked(snapshot),
        Page::Playlist(id) => collection::playlist(*id, snapshot),
        Page::Profile(handle) => profile::build(handle, snapshot),
        Page::Downloads => downloads::build(snapshot, memo),
        Page::Invalid => Built {
            status: PageStatus::Failed { message: "Questa pagina non esiste.".into() },
            ..Built::default()
        },
    }
}

/// The current entry and how its rows look.
fn now_of(snapshot: &Snapshot) -> Option<(Uuid, RowState)> {
    let playback = &snapshot.playback;
    let state = match playback.state {
        PlaybackState::Playing => RowState::Playing,
        PlaybackState::Paused => RowState::Paused,
        PlaybackState::Loading { .. } => RowState::Busy,
        PlaybackState::Stopped | PlaybackState::Error(_) => return None,
    };
    playback.queue.current().map(|track| (track.id, state))
}

/// Whether the core is importing this right now.
fn importing(external: &ExternalView, import: &Importing) -> bool {
    match import {
        Importing::Source(source, id) => external.importing(*source, id),
        Importing::Archive(identifier) => external.importing_archive(identifier),
    }
}

/// A page's model, behind its handle.
pub(crate) struct Model {
    page: Page,
    ui: Ui,
    version: u64,
    snapshot: Arc<Snapshot>,
    built: Built,
    now: Option<(Uuid, RowState)>,
    imports: ImportsView,
    links: Arc<ImportLinks>,
    /// The download mark of each row (`DownloadMark::None` for rows that are not tracks).
    downloads: Vec<DownloadMark>,
    memo: downloads::Memo,
}

/// What changed in a snapshot, for the open pages.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Changes {
    /// A domain the pages read, or the shape of the downloads.
    pub(crate) pages: bool,
    /// The downloads moved at all (a percent): only «Scaricati» shows that.
    pub(crate) progress: bool,
}

/// The marks of `built`'s track rows.
fn download_marks(built: &Built, snapshot: &Snapshot, links: &ImportLinks) -> Vec<DownloadMark> {
    let view = &snapshot.downloads;
    if view.items.is_empty() && view.saved_ids.is_empty() {
        return vec![DownloadMark::None; built.rows.len()];
    }
    let marks = Marks::of(view);
    built
        .rows
        .iter()
        .zip(&built.marks)
        .map(|(row, mark)| match (row, mark) {
            (Row::Track { .. }, Mark::Track(id)) => marks.mark(*id),
            (Row::Track { .. }, Mark::Import(import)) => {
                links.track(import).map_or(DownloadMark::None, |track| marks.mark(track.id))
            }
            _ => DownloadMark::None,
        })
        .collect()
}

impl Model {
    pub(crate) fn new(page: Page) -> Self {
        Self {
            page,
            ui: Ui::default(),
            version: 0,
            snapshot: Arc::default(),
            built: Built::default(),
            now: None,
            imports: ImportsView::default(),
            links: Arc::default(),
            downloads: Vec::new(),
            memo: downloads::Memo::default(),
        }
    }

    fn row(&self, index: usize) -> Option<Row> {
        let mut row = self.built.rows.get(index)?.clone();
        let state_of = |id: Uuid| match self.now {
            Some((current, state)) if current == id => state,
            _ => RowState::Idle,
        };
        let state = match self.built.marks.get(index)? {
            Mark::None => return Some(row),
            Mark::Import(import) if importing(&self.snapshot.external, import) => RowState::Busy,
            // Once imported, the row is its server track: the equaliser while that plays.
            Mark::Import(import) => {
                self.links.track(import).map_or(RowState::Idle, |track| state_of(track.id))
            }
            Mark::Track(id) => state_of(*id),
        };
        let has_track = self.track_at(index).is_some();
        match &mut row {
            Row::Track { state: shown, more, download, .. } => {
                *shown = state;
                *more = has_track;
                *download = self.downloads.get(index).copied().unwrap_or_default();
            }
            Row::Item { state: shown, .. } | Row::Saved { state: shown, .. } => *shown = state,
            _ => {}
        }
        Some(row)
    }

    /// The track of row `index` for its menu, and the playlist position it has there.
    fn track_at(&self, index: usize) -> Option<(Track, Option<(Uuid, usize)>)> {
        match (self.built.targets.get(index)?, self.built.marks.get(index)?) {
            (Target::Play(list, at), _) => {
                let track = list.get(*at)?.clone();
                let from = match self.page {
                    Page::Playlist(id) => Some((id, *at)),
                    _ => None,
                };
                Some((track, from))
            }
            (_, Mark::Import(import)) => self.links.track(import).map(|track| (track.clone(), None)),
            _ => None,
        }
    }

    fn view(&self) -> PageView {
        let total = self.built.rows.len();
        PageView {
            version: self.version,
            title: self.built.title.clone(),
            header: self.built.header.clone(),
            status: self.built.status.clone(),
            total: count(total),
            rows: (0..total.min(FIRST_ROWS)).filter_map(|index| self.row(index)).collect(),
        }
    }

    /// Takes `snapshot`: `true` if what the page shows changed.
    fn take(&mut self, snapshot: &Arc<Snapshot>) -> bool {
        self.snapshot = Arc::clone(snapshot);
        let built = build(&self.page, snapshot, &self.ui, &mut self.memo);
        let now = now_of(snapshot);
        let imports = &snapshot.external.imports;
        let links = &snapshot.external.links;
        let downloads = download_marks(&built, snapshot, links);
        if built.same(&self.built)
            && now == self.now
            && *imports == self.imports
            && Arc::ptr_eq(links, &self.links)
            && downloads == self.downloads
        {
            return false;
        }
        self.built = built;
        self.now = now;
        self.imports.clone_from(imports);
        self.links = Arc::clone(links);
        self.downloads = downloads;
        true
    }

    /// The UI choices changed: rebuild from the same snapshot.
    fn rebuild(&mut self) {
        self.built = build(&self.page, &self.snapshot, &self.ui, &mut self.memo);
        self.downloads = download_marks(&self.built, &self.snapshot, &self.links);
    }
}

/// A page the app has open.
pub(crate) struct PageState {
    model: Mutex<Model>,
    listener: Arc<dyn PageListener>,
}

impl PageState {
    fn model(&self) -> MutexGuard<'_, Model> {
        self.model.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tells the listener the page as it is now, with a new version.
    fn publish(&self, inner: &Inner, mut model: MutexGuard<'_, Model>) {
        model.version = inner.next_page_version();
        let view = model.view();
        drop(model);
        self.listener.page_changed(view);
    }

    /// The core published `snapshot`. When `first` (a core that just started), the page asks again
    /// for what it needs.
    pub(crate) fn refresh(
        &self,
        inner: &Arc<Inner>,
        snapshot: &Arc<Snapshot>,
        first: bool,
        changes: Changes,
    ) {
        let mut model = self.model();
        let reads_progress = matches!(model.page, Page::Downloads);
        if !first && !changes.pages && !(reads_progress && changes.progress) {
            return;
        }
        if first {
            let commands = model.page.ensure(snapshot);
            for command in commands {
                inner.send(command);
            }
        }
        model.ui.jam_dismissed = inner.jam_dismissed();
        if model.take(snapshot) {
            self.publish(inner, model);
        }
    }
}

/// The pages open, held weakly: a page closes when the app drops its handle.
#[derive(Default)]
pub(crate) struct Pages(Vec<Weak<PageState>>);

impl Pages {
    /// The pages still open; the closed ones go.
    pub(crate) fn live(&mut self) -> Vec<Arc<PageState>> {
        self.0.retain(|page| page.strong_count() > 0);
        self.0.iter().filter_map(Weak::upgrade).collect()
    }

    fn add(&mut self, page: &Arc<PageState>) {
        self.0.retain(|page| page.strong_count() > 0);
        self.0.push(Arc::downgrade(page));
    }
}

/// An open page. Dropping it (`close()` in Kotlin) closes the page.
#[derive(uniffi::Object)]
pub struct PageHandle {
    inner: Arc<Inner>,
    state: Arc<PageState>,
}

impl PageHandle {
    pub(crate) fn open(inner: &Arc<Inner>, route: &Route, listener: Arc<dyn PageListener>) -> Arc<Self> {
        let snapshot = inner.snapshot().unwrap_or_default();
        let page = Page::of(route);
        let ensure = page.ensure(&snapshot);
        let sent = !ensure.is_empty();
        for command in ensure {
            inner.send(command);
        }
        // What was just sent may already show (a fixture answers at once).
        let snapshot = if sent { inner.snapshot().unwrap_or(snapshot) } else { snapshot };
        let mut model = Model::new(page);
        model.ui.jam_dismissed = inner.jam_dismissed();
        model.take(&snapshot);
        let state = Arc::new(PageState { model: Mutex::new(model), listener });
        inner.pages().add(&state);
        state.publish(inner, state.model());
        Arc::new(Self { inner: Arc::clone(inner), state })
    }

    /// Plays, pauses or sends what `target` says; what the app must do next.
    fn run(&self, target: Target) -> Tap {
        match target {
            Target::None => Tap::Nothing,
            Target::Open(route) => Tap::Open { route },
            Target::Play(list, start) => {
                let Some(chosen) = list.get(start) else { return Tap::Nothing };
                let current = self.inner.snapshot().and_then(|snapshot| {
                    let playback = &snapshot.playback;
                    let active = !matches!(playback.state, PlaybackState::Stopped);
                    playback.queue.current().filter(|_| active).map(|track| track.id)
                });
                let command = if current == Some(chosen.id) {
                    PlaybackCmd::Toggle
                } else {
                    PlaybackCmd::Play { tracks: list, start }
                };
                self.inner.send(Command::Playback(command));
                Tap::Nothing
            }
            Target::Send(command) => {
                self.inner.send(command);
                Tap::Nothing
            }
            Target::Notify(command, notice) => {
                self.inner.send(command);
                self.inner.listener().notice(notice, false);
                Tap::Nothing
            }
            Target::Search(query) => {
                self.inner.send(Command::Catalog(CatalogCmd::Search(query.clone())));
                Tap::Search { query }
            }
            Target::SaveSource => {
                let id = Uuid::new_v4();
                self.inner.send(Command::External(ExternalCmd::SavePlaylist { id }));
                Tap::Nothing
            }
            Target::Dialog(request) => Tap::Dialog { dialog: self.inner.open_dialog(request) },
            Target::MorePopular => {
                let mut model = self.state.model();
                model.ui.more = true;
                model.rebuild();
                self.state.publish(&self.inner, model);
                Tap::Nothing
            }
            Target::Choose(handle) => {
                let mut model = self.state.model();
                jam::toggle(&mut model.ui.chosen, &handle);
                model.rebuild();
                self.state.publish(&self.inner, model);
                Tap::Nothing
            }
            Target::Invite => {
                let mut model = self.state.model();
                let chosen = std::mem::take(&mut model.ui.chosen);
                if chosen.is_empty() {
                    return Tap::Nothing;
                }
                model.rebuild();
                self.state.publish(&self.inner, model);
                self.inner.send(Command::Social(SocialCmd::InviteToJam(chosen)));
                Tap::Nothing
            }
            Target::JoinInvite(code) => {
                self.inner.join_invite(&code);
                Tap::Nothing
            }
            Target::Copy(text) => Tap::Copy { text },
        }
    }
}

#[uniffi::export]
impl PageHandle {
    /// Up to `limit` rows (at most [`MAX_ROWS`]) from `offset`, as the page is now: if `version`
    /// is not the page's any more, the rows are those of the newer version it carries.
    pub fn rows(&self, offset: u32, limit: u32) -> RowPage {
        let model = self.state.model();
        let start = offset as usize;
        let end = start.saturating_add(limit.min(MAX_ROWS) as usize).min(model.built.rows.len());
        RowPage {
            version: model.version,
            offset,
            rows: (start..end).filter_map(|index| model.row(index)).collect(),
        }
    }

    /// A tap on row `row` of the page at `version`.
    pub fn tap(&self, version: u64, row: u32) -> Tap {
        let target = {
            let model = self.state.model();
            if model.version != version {
                return Tap::Nothing;
            }
            match model.built.targets.get(row as usize) {
                Some(target) => target.clone(),
                None => return Tap::Nothing,
            }
        };
        self.run(target)
    }

    /// The «Altro» sheet of row `row` (long-press), if the row has a track. It stays open in the
    /// bridge until a choice or `PsytifyCore::close_menu`.
    pub fn menu(&self, version: u64, row: u32) -> Option<MenuView> {
        let (track, from_playlist) = {
            let model = self.state.model();
            if model.version != version {
                return None;
            }
            model.track_at(row as usize)?
        };
        Some(self.inner.open_menu(track, from_playlist))
    }

    /// Row `row` swiped to the right: its track goes to the end of the queue. `false` if the row
    /// has no track (or the page changed).
    pub fn enqueue(&self, version: u64, row: u32) -> bool {
        let track = {
            let model = self.state.model();
            if model.version != version {
                return false;
            }
            match model.track_at(row as usize) {
                Some((track, _)) => track,
                None => return false,
            }
        };
        let notice = format!("«{}» aggiunto alla coda", track.title);
        self.inner.send(Command::Playback(PlaybackCmd::Enqueue(Box::new(track))));
        self.inner.listener().notice(notice, false);
        true
    }

    /// Chip `chip` of the chips on row `row`.
    pub fn chip(&self, version: u64, row: u32, chip: u32) {
        let mut model = self.state.model();
        let valid = matches!(
            model.built.rows.get(row as usize),
            Some(Row::Chips { chips, .. }) if (chip as usize) < chips.len()
        );
        if model.version != version || !valid || model.ui.chip == chip as usize {
            return;
        }
        model.ui.chip = chip as usize;
        model.rebuild();
        self.state.publish(&self.inner, model);
    }

    /// Button `action` of row `row` («Scaricati»).
    pub fn row_action(&self, version: u64, row: u32, action: RowAction) -> Tap {
        let target = {
            let model = self.state.model();
            if model.version != version {
                return Tap::Nothing;
            }
            let found =
                model.built.row_actions.iter().find(|(at, known, _)| *at == row as usize && *known == action);
            match found {
                Some((_, _, target)) => target.clone(),
                None => return Tap::Nothing,
            }
        };
        self.run(target)
    }

    /// A header button, or «Riprova».
    pub fn act(&self, version: u64, action: PageAction) -> Tap {
        let target = {
            let model = self.state.model();
            if model.version != version {
                return Tap::Nothing;
            }
            let found = model.built.actions.iter().find(|(known, _)| *known == action);
            match found {
                Some((_, target)) => target.clone(),
                None => return Tap::Nothing,
            }
        };
        self.run(target)
    }
}
