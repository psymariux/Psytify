//! The page of an album of the server catalogue (`PSYTIFY_ART_DIRECTION.md`, Album): the head with the cover, «ALBUM · 2024», the title, the artist (opens),
//! tracks, duration and `LOSSLESS` when every track is; the 56 dp play button, shuffle, save to the library
//! (the heart) and «Scarica tutto»; then the tracks (`TrackRow`, one column, title : album 3 : 2) and the
//! credits of the licences that ask for attribution. The page fills the frame (desktop v2 §2) and enters
//! with the page-enter move.
//!
//! The core's catalogue keeps the album (`CatalogView::album`, LRU of 24 pages, quietly reread after 90 s):
//! opening the page sends `CatalogCmd::Ensure`, and the page shows what the core publishes for its id: the
//! skeleton while it loads, the error with «Riprova» (`CatalogCmd::Retry`), the album. Row texts are rebuilt
//! only when the album changes (another `Arc`).

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, ListState, ParentElement, Render,
    SharedString, Styled, Subscription, Window, div, list,
};
use psytify_core::api::{Album, Track};
use psytify_core::catalog::Load;
use psytify_core::command::{CatalogCmd, DownloadsCmd, LibraryCmd, PlaybackCmd, Resource};
use psytify_core::config::SavedAlbum;
use psytify_core::snapshot::{CatalogView, DownloadsView, LibraryView};
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_ERROR, TEXT_PRIMARY, TEXT_SECONDARY};
use psytify_design::tokens::size::cover::ART_ROW;
use psytify_design::tokens::size::{TRACK_INDEX_W, TRACK_ROW_H};
use psytify_design::tokens::space::{SPACE_1, SPACE_3, SPACE_5, SPACE_8};
use psytify_design::tokens::type_::{TYPE_BODY, TYPE_META, TYPE_NUMERIC};
use uuid::Uuid;

use super::page_frame::PageFrame;
use super::page_list::{new_list, remeasure_header, set_rows, to_top};
use super::track_list::{
    RowText, batch_label, download_all_label, download_command, download_state, now_playing, plays,
    with_links,
};
use super::track_menu::TrackMenu;
use super::{Page, PageEvent};
use crate::components::button::Button;
use crate::components::frame::{Frame, measure};
use crate::components::icon_button::{IconButton, OnStyle};
use crate::components::indicators::{PlayState, Skeleton};
use crate::components::page_header::{HeaderArt, PageHeader};
use crate::components::playing_spot::PlayingSpot;
use crate::components::section::{note_at, section_heading_at};
use crate::components::track_row::{TrackData, TrackRow};
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{TypeStyle, col, dp};

/// Skeleton rows while the album arrives.
const SKELETON_ROWS: usize = 8;

/// A row after the head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Track(usize),
    /// No track published.
    Empty,
    CreditsHeading,
    Credit(usize),
}

/// The rows: the tracks, then the credits (if a licence asks for them). Pure.
fn plan(tracks: usize, credits: usize) -> Vec<Row> {
    let mut rows: Vec<Row> = (0..tracks).map(Row::Track).collect();
    if tracks == 0 {
        rows.push(Row::Empty);
    }
    if credits > 0 {
        rows.push(Row::CreditsHeading);
        rows.extend((0..credits).map(Row::Credit));
    }
    rows
}

/// «Album · 2024» (with the year when known).
fn kicker(album: &Album) -> String {
    match album.released_on.as_deref().and_then(|date| date.get(..4)) {
        Some(year) => format!("Album · {year}"),
        None => "Album".to_owned(),
    }
}

/// `Lossless` when every track is in a lossless format (FLAC, WAV): a quality, not a state.
fn quality(tracks: &[Track]) -> Option<&'static str> {
    let lossless = |t: &Track| matches!(t.audio_format.as_str(), "flac" | "wav" | "alac" | "aiff");
    (!tracks.is_empty() && tracks.iter().all(lossless)).then_some("Lossless")
}

/// The credits of the licences that ask for attribution (CC BY and the like): «Title — CC BY · Holder».
pub fn credits(tracks: &[Track]) -> Vec<SharedString> {
    tracks
        .iter()
        .filter(|t| t.licence.requires_attribution())
        .map(|t| format!("{} — {} · {}", t.title, t.licence.label(), t.licence.attribution).into())
        .collect()
}

/// A credit line, dense, at the frame margin and within the running-text measure;
/// the last leaves room at the bottom of the page.
pub fn credit_line(text: SharedString, last: bool, margin: f32) -> gpui::Div {
    div().px(dp(margin)).pt(dp(SPACE_1)).pb(dp(if last { SPACE_8 } else { SPACE_1 })).child(
        div()
            .max_w(measure(TYPE_META))
            .min_w_0()
            .truncate()
            .type_role(TYPE_META)
            .text_color(col(TEXT_SECONDARY))
            .child(text),
    )
}

/// What the page shows.
#[derive(Clone, Debug, PartialEq)]
enum Shown {
    Loading,
    Failed(SharedString),
    Ready,
}

pub struct AlbumPage {
    core: Entity<CoreModel>,
    /// The album the page shows (`Shell` picks it).
    id: Option<Uuid>,
    shown: Shown,
    album: Option<Arc<Album>>,
    tracks: Arc<[Track]>,
    texts: Vec<RowText>,
    rows: Vec<Row>,
    kicker: SharedString,
    meta: SharedString,
    quality: Option<&'static str>,
    credits: Vec<SharedString>,
    download_label: Option<&'static str>,
    batch: Option<SharedString>,
    saved: bool,
    liked: Arc<HashSet<Uuid>>,
    library: Arc<LibraryView>,
    downloads: Arc<DownloadsView>,
    now: Option<(Uuid, PlayState)>,
    list: ListState,
    frame: PageFrame,
    /// Where the playing track's equalizer goes: `Shell` paints it (BACKLOG 23).
    spot: PlayingSpot,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for AlbumPage {}

impl AlbumPage {
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
            album: None,
            tracks: Arc::from(Vec::new()),
            texts: Vec::new(),
            rows: Vec::new(),
            kicker: SharedString::default(),
            meta: SharedString::default(),
            quality: None,
            credits: Vec::new(),
            download_label: None,
            batch: batch_label(&snapshot.downloads),
            saved: false,
            liked: core.read(cx).liked(),
            library: Arc::clone(&snapshot.library),
            downloads: Arc::clone(&snapshot.downloads),
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

    /// A new width: the margins may move.
    fn set_frame(&mut self, frame: Frame, cx: &mut Context<Self>) {
        if frame != self.frame.frame {
            self.frame.frame = frame;
            cx.notify();
        }
    }

    /// Shows album `id`: the core serves it from its cache or loads it.
    pub fn show(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.core.read(cx).catalog(CatalogCmd::Ensure(Resource::Album(id)));
        if self.id == Some(id) {
            return;
        }
        self.id = Some(id);
        self.album = None;
        let catalog = Arc::clone(&self.core.read(cx).snapshot().catalog);
        self.take(&catalog);
        to_top(&self.list);
        cx.notify();
    }

    fn refresh(&mut self, core: &Entity<CoreModel>, changed: Changed, cx: &mut Context<Self>) {
        let snapshot = core.read(cx).snapshot();
        let mut dirty = false;
        if changed.catalog {
            dirty |= self.take(&snapshot.catalog);
        }
        if changed.library && !Arc::ptr_eq(&snapshot.library, &self.library) {
            self.library = Arc::clone(&snapshot.library);
            self.liked = core.read(cx).liked();
            let saved = self.is_saved();
            if saved != self.saved {
                self.saved = saved;
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
        if dirty {
            cx.notify();
        }
    }

    fn is_saved(&self) -> bool {
        self.id.is_some_and(|id| self.library.saved_albums.iter().any(|album| album.id == id))
    }

    /// Takes this page's album from the catalogue; `true` if what is shown changed.
    fn take(&mut self, catalog: &CatalogView) -> bool {
        let Some(id) = self.id else { return false };
        let (shown, album) = match &catalog.album {
            Some((open, Load::Ready(album))) if *open == id => (Shown::Ready, Some(Arc::clone(album))),
            Some((open, Load::Failed(error))) if *open == id => {
                (Shown::Failed(error.to_string().into()), None)
            }
            _ => (Shown::Loading, None),
        };
        let same_album = match (&album, &self.album) {
            (Some(new), Some(old)) => Arc::ptr_eq(new, old),
            (None, None) => true,
            _ => false,
        };
        if same_album && shown == self.shown {
            return false;
        }
        self.shown = shown;
        self.album = album;
        self.rebuild();
        true
    }

    fn rebuild(&mut self) {
        let Some(album) = self.album.clone() else {
            self.tracks = Arc::from(Vec::new());
            self.texts.clear();
            self.rows.clear();
            self.credits.clear();
            set_rows(&self.list, 0);
            return;
        };
        self.tracks = Arc::from(album.tracks.as_slice());
        self.texts = self.tracks.iter().map(RowText::of).collect();
        self.kicker = kicker(&album).into();
        let total: u64 = self.tracks.iter().map(|t| t.duration_ms).sum();
        self.meta = if total > 0 {
            format!("{} · {}", format::tracks(self.tracks.len()), format::total(total)).into()
        } else {
            format::tracks(self.tracks.len()).into()
        };
        self.quality = quality(&self.tracks);
        self.credits = credits(&self.tracks);
        self.download_label = download_all_label(&self.tracks);
        self.saved = self.is_saved();
        self.rows = plan(self.tracks.len(), self.credits.len());
        set_rows(&self.list, self.rows.len());
        remeasure_header(&self.list);
    }

    /// Double click or Enter: the queue becomes the album from `start`; on the playing track, play or pause.
    fn play(&self, start: usize, cx: &gpui::App) {
        let core = self.core.read(cx);
        match (self.tracks.get(start), self.now) {
            (Some(track), Some((id, _))) if track.id == id => core.playback(PlaybackCmd::Toggle),
            _ => core.playback(PlaybackCmd::Play { tracks: Arc::clone(&self.tracks), start }),
        }
    }

    fn item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let element = match ix.checked_sub(1) {
            None => self.album.clone().map(|album| self.header(&album, cx).into_any_element()),
            Some(row) => self.row(row, cx),
        };
        let element = element.unwrap_or_else(|| div().into_any_element());
        // The head, the tracks and the credits enter one step apart.
        let section = match ix.checked_sub(1).and_then(|row| self.rows.get(row)) {
            None => 0,
            Some(Row::CreditsHeading | Row::Credit(_)) => 2,
            Some(_) => 1,
        };
        self.frame.wrap(element, section, window)
    }

    fn header(&self, album: &Album, cx: &mut Context<Self>) -> PageHeader {
        let artist = album.artist.id;
        let mut header = PageHeader::new(
            HeaderArt::Cover(album.cover_url.clone().map(SharedString::from)),
            self.kicker.clone(),
            album.title.clone(),
        )
        .margin(self.frame.margin())
        .link(
            "album-artist",
            album.artist.name.clone(),
            cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Artist(artist)))),
        )
        .meta(self.meta.clone());
        if let Some(quality) = self.quality {
            header = header.quality(quality);
        }
        if !self.tracks.is_empty() {
            header = header
                .play(
                    "album-play",
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.core
                            .read(cx)
                            .playback(PlaybackCmd::Play { tracks: Arc::clone(&this.tracks), start: 0 });
                    }),
                )
                .action(
                    IconButton::new("album-shuffle", sprite::ICON_SHUFFLE)
                        .tooltip("Riproduci in ordine casuale", None)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.core.read(cx).playback(PlaybackCmd::ShufflePlay(Arc::clone(&this.tracks)));
                        })),
                );
        }
        let saved = SavedAlbum {
            id: album.id,
            title: album.title.clone(),
            artist: album.artist.clone(),
            cover_url: album.cover_url.clone(),
        };
        header = header.action(
            IconButton::new("album-save", sprite::ICON_HEART)
                .icon_on(sprite::ICON_HEART_FILL)
                .on_style(OnStyle::Active)
                .selected(self.saved)
                .tooltip(if self.saved { "Togli dalla libreria" } else { "Salva nella libreria" }, None)
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.emit(PageEvent::ToggleSaveAlbum(Box::new(saved.clone())))
                })),
        );
        match (&self.batch, self.download_label) {
            // While the batch downloads, the count replaces the icon.
            (Some(count), _) => {
                header = header
                    .action(div().type_role(TYPE_NUMERIC).text_color(col(TEXT_PRIMARY)).child(count.clone()))
            }
            (None, Some(label)) => {
                header = header.action(
                    IconButton::new("album-download", sprite::ICON_DOWNLOAD).tooltip(label, None).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.core.read(cx).downloads(DownloadsCmd::DownloadAll(Arc::clone(&this.tracks)));
                        }),
                    ),
                )
            }
            (None, None) => {}
        }
        header
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let margin = self.frame.margin();
        Some(match *self.rows.get(ix)? {
            Row::Empty => note_at(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_SECONDARY))
                    .child("Questo album non ha ancora brani pubblicati."),
                margin,
            )
            .into_any_element(),
            Row::CreditsHeading => {
                section_heading_at("Crediti".into(), SharedString::default(), margin).into_any_element()
            }
            Row::Credit(position) => {
                credit_line(self.credits.get(position)?.clone(), position + 1 == self.credits.len(), margin)
                    .into_any_element()
            }
            Row::Track(position) => self.frame.row(self.track_row(position, cx)?),
        })
    }

    fn track_row(&self, position: usize, cx: &mut Context<Self>) -> Option<TrackRow> {
        let track = self.tracks.get(position)?;
        let text = self.texts.get(position)?;
        let data = TrackData {
            index: track.track_no.unwrap_or(u32::try_from(position + 1).unwrap_or(u32::MAX)),
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
        let mut row = TrackRow::new(("album-row", position), data)
            .show_album(false)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if plays(event) {
                    this.play(position, cx);
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
            self.core.read(cx).catalog(CatalogCmd::Retry(Resource::Album(id)));
        }
    }
}

/// The skeleton of an object page: the head and a few rows, at their real size.
pub fn skeleton_page(round: bool, margin: f32) -> gpui::Div {
    let rows = (0..SKELETON_ROWS).map(|_| skeleton_row(margin));
    div().size_full().flex().flex_col().child(PageHeader::loading(round, margin)).children(rows)
}

/// A waiting track row on the `TrackRow` grid: margin, index column, cover, two lines.
pub fn skeleton_row(margin: f32) -> gpui::Div {
    div()
        .h(dp(TRACK_ROW_H))
        .flex()
        .items_center()
        .gap(dp(SPACE_3))
        .px(dp(margin))
        .child(div().w(dp(TRACK_INDEX_W)).flex_none())
        .child(Skeleton::new(ART_ROW, ART_ROW))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(dp(SPACE_3 / 2.0))
                .child(Skeleton::new(220.0, 12.0))
                .child(Skeleton::new(140.0, 10.0)),
        )
}

/// A failure in the middle, with «Riprova».
pub fn failure(
    id: &'static str,
    text: SharedString,
    retry: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Div {
    div().size_full().flex().justify_center().pt(dp(96.0)).child(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(dp(SPACE_5))
            .child(div().type_role(TYPE_BODY).text_color(col(TEXT_ERROR)).child(text))
            .child(Button::new(id, "Riprova").secondary().on_click(retry)),
    )
}

impl Render for AlbumPage {
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
            Shown::Loading => page.child(skeleton_page(false, self.frame.margin())),
            Shown::Failed(error) => page.child(failure(
                "album-retry",
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
    use psytify_core::api::models::fixtures;

    use super::*;

    #[test]
    fn tracks_come_first_then_the_credits() {
        assert_eq!(plan(2, 1), vec![Row::Track(0), Row::Track(1), Row::CreditsHeading, Row::Credit(0)]);
        assert_eq!(plan(0, 0), vec![Row::Empty]);
        assert_eq!(plan(1, 0), vec![Row::Track(0)]);
    }

    #[test]
    fn the_kicker_has_the_year_when_the_server_knows_it() {
        let album = |released: Option<&str>| Album {
            id: Uuid::nil(),
            title: "Live".into(),
            released_on: released.map(str::to_owned),
            artist: fixtures::track(1).artist,
            cover_url: None,
            tracks: Vec::new(),
        };
        assert_eq!(kicker(&album(Some("2024-05-01"))), "Album · 2024");
        assert_eq!(kicker(&album(None)), "Album");
    }

    #[test]
    fn lossless_only_when_every_track_is() {
        let mut flac = fixtures::track(1);
        flac.audio_format = "flac".into();
        let mut m4a = fixtures::track(2);
        m4a.audio_format = "m4a".into();
        assert_eq!(quality(&[flac.clone(), flac.clone()]), Some("Lossless"));
        assert_eq!(quality(&[flac, m4a]), None);
        assert_eq!(quality(&[]), None);
    }

    #[test]
    fn credits_list_only_the_licences_that_ask_for_them() {
        let mut by = fixtures::track(1);
        by.licence.kind = "cc-by".into();
        by.licence.attribution = "Marea".into();
        let mut zero = fixtures::track(2);
        zero.licence.kind = "cc0".into();
        let lines: Vec<String> = credits(&[by, zero]).iter().map(ToString::to_string).collect();
        assert_eq!(lines, ["Brano 1 — CC BY · Marea"]);
    }
}
