//! «Scaricati»: the running downloads and the tracks on the device, which play without network. Everything
//! comes from `DownloadsView` and every gesture is a `DownloadsCmd` (or a `PlaybackCmd` to play): the page
//! touches no disk and computes nothing while drawing, like the rest of the page.
//!
//! At the top the title with the count and the space used, the folder and the quality, and the buttons
//! Riproduci, Casuale, «Apri cartella» and «Controlla i file» (the deep check, `DownloadsCmd::Verify`). Below,
//! one `uniform_list` of 48 dp rows at the frame margins: «In corso» (state, ten-step bar, cancel; retry and
//! «Togli dall'elenco» for a failed one) and «Sul dispositivo» (format and size, or «File mancante» / «File
//! cambiato» in the error tint with «Scarica di nuovo»; «Rimuovi dal dispositivo» and «Altro» on hover).
//! Double click or Enter plays a track, with the device tracks as the queue. Empty, «Scopri il catalogo» opens
//! the Home. The notices (saved, removed, check done) are core events that `Shell` shows. The page enters
//! with the page-enter move.
//!
//! Row texts are rebuilt when the downloads view changes: the device tracks only when the index or the lost
//! files change (there may be hundreds), the queue on every publish (a few rows).

use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, UniformListScrollHandle, Window, div, uniform_list,
};
use psytify_core::api::Track;
use psytify_core::command::{DownloadsCmd, PlaybackCmd};
use psytify_core::snapshot::{DownloadsView, Item, ItemState, LostReason, SavedTrack};
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_SECONDARY};
use psytify_design::tokens::size::TRACK_ROW_H;
use psytify_design::tokens::space::SPACE_3;
use psytify_design::tokens::type_::TYPE_BODY;
use uuid::Uuid;

use super::page_frame::PageFrame;
use super::track_menu::TrackMenu;
use super::{Page, PageEvent, play_state};
use crate::components::button::Button;
use crate::components::frame::Frame;
use crate::components::icon_button::IconButton;
use crate::components::indicators::PlayState;
use crate::components::list_row::{ListRow, RowArt};
use crate::components::overlay::EmptyState;
use crate::components::playing_spot::PlayingSpot;
use crate::components::section::{PageTitle, note_at, section_heading_at};
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{TypeStyle, col, dp};

/// The two sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    InProgress,
    OnDevice,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Self::InProgress => "In corso",
            Self::OnDevice => "Sul dispositivo",
        }
    }
}

/// A row of the list, in the order it is drawn. The indices are into the core's lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Heading(Section),
    /// A download of the queue (`DownloadsView::items`).
    Item(usize),
    /// Nothing on the device, but something in progress: a sentence in place of the section.
    NothingSaved,
    /// A track on the device (`DownloadsView::saved`).
    Saved(usize),
}

/// Which rows to show: first the downloads in progress, then the tracks on the device. Empty if there is neither
/// of the two (the page then shows its empty state). Pure.
fn plan(items: usize, saved: usize) -> Vec<Row> {
    let mut rows = Vec::with_capacity(items + saved + 2);
    if items > 0 {
        rows.push(Row::Heading(Section::InProgress));
        rows.extend((0..items).map(Row::Item));
    }
    if saved > 0 {
        rows.push(Row::Heading(Section::OnDevice));
        rows.extend((0..saved).map(Row::Saved));
    } else if items > 0 {
        rows.push(Row::NothingSaved);
    }
    rows
}

/// The texts of a queued download.
#[derive(Clone, Debug, PartialEq)]
struct ItemText {
    id: Uuid,
    title: SharedString,
    status: SharedString,
    failed: bool,
    /// The ten-step bar, as long as the download is alive.
    steps: Option<f32>,
    cover_url: Option<SharedString>,
}

impl ItemText {
    fn of(item: &Item) -> Self {
        let status = match &item.state {
            ItemState::Queued => "In coda".to_owned(),
            ItemState::Preparing => "Preparazione…".to_owned(),
            ItemState::Downloading { .. } => {
                item.percent().map_or_else(|| "Scaricamento…".to_owned(), |percent| format!("{percent}%"))
            }
            ItemState::Failed(error) => format!("Non riuscito. {error}"),
        };
        Self {
            id: item.track.id,
            title: item.track.title.clone().into(),
            status: status.into(),
            failed: !item.is_busy(),
            steps: item.is_busy().then(|| item.percent().map_or(0.0, |p| f32::from(p) / 100.0)),
            cover_url: item.track.cover_url.clone().map(SharedString::from),
        }
    }
}

/// The texts of a track on the device: «Artista · FLAC · 12,3 MB», or the problem with its file.
#[derive(Clone, Debug, PartialEq)]
struct SavedText {
    id: Uuid,
    title: SharedString,
    detail: SharedString,
    lost: bool,
    cover_url: Option<SharedString>,
}

impl SavedText {
    fn of(entry: &SavedTrack, lost: Option<LostReason>) -> Self {
        let track = &entry.track;
        let detail = match lost {
            Some(LostReason::Missing) => format!("{} · File mancante", track.artist.name),
            Some(LostReason::Changed) => format!("{} · File cambiato", track.artist.name),
            None => {
                format!(
                    "{} · {} · {}",
                    track.artist.name,
                    entry.format.to_uppercase(),
                    format::size(entry.size)
                )
            }
        };
        Self {
            id: track.id,
            title: track.title.clone().into(),
            detail: detail.into(),
            lost: lost.is_some(),
            cover_url: track.cover_url.clone().map(SharedString::from),
        }
    }
}

/// The line under the title: how many tracks and how much they weigh, or that the check is in progress.
fn summary(view: &DownloadsView) -> String {
    if view.verifying {
        "Controllo dei file…".to_owned()
    } else if view.saved.is_empty() {
        "Nessun brano sul dispositivo".to_owned()
    } else {
        format!("{} · {}", format::tracks(view.saved.len()), format::size(view.used_bytes))
    }
}

pub struct DownloadsPage {
    core: Entity<CoreModel>,
    view: Arc<DownloadsView>,
    rows: Vec<Row>,
    items: Vec<ItemText>,
    saved: Vec<SavedText>,
    /// The device tracks as a queue to play: rebuilt only when the index changes.
    tracks: Arc<[Track]>,
    summary: SharedString,
    folder: SharedString,
    /// The playing track and how to show it.
    now: Option<(Uuid, PlayState)>,
    scroll: UniformListScrollHandle,
    frame: PageFrame,
    /// The equaliser spot of the playing track: `Shell` paints it (BACKLOG 23).
    spot: PlayingSpot,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for DownloadsPage {}

impl DownloadsPage {
    /// The equaliser spot of this view, for `Shell`'s layer.
    pub fn spot(&self) -> PlayingSpot {
        self.spot.clone()
    }

    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let changes =
            cx.subscribe(&core, |this, core, changed: &Changed, cx| this.refresh(&core, *changed, cx));
        let snapshot = core.read(cx).snapshot();
        let mut page = Self {
            view: Arc::default(),
            rows: Vec::new(),
            items: Vec::new(),
            saved: Vec::new(),
            tracks: Arc::from([]),
            summary: SharedString::default(),
            folder: SharedString::default(),
            now: now_playing(&snapshot.playback),
            scroll: UniformListScrollHandle::new(),
            frame: PageFrame::default(),
            core,
            spot: PlayingSpot::new(),
            _changes: changes,
        };
        page.take(&snapshot.downloads, true);
        page
    }

    fn refresh(&mut self, core: &Entity<CoreModel>, changed: Changed, cx: &mut Context<Self>) {
        let snapshot = core.read(cx).snapshot();
        let mut dirty = false;
        if changed.downloads && !Arc::ptr_eq(&snapshot.downloads, &self.view) {
            self.take(&snapshot.downloads, false);
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

    /// Takes the new view and rebuilds the texts: those of the device tracks only if the index or the lost
    /// files changed (or the first time).
    fn take(&mut self, view: &Arc<DownloadsView>, first: bool) {
        let saved_changed = first || !view.saved.same(&self.view.saved) || !view.lost.same(&self.view.lost);
        if saved_changed {
            self.saved = view
                .saved
                .iter()
                .map(|entry| SavedText::of(entry, view.lost_reason(entry.track.id)))
                .collect();
        }
        if first || !view.saved.same(&self.view.saved) {
            self.tracks = view.saved.iter().map(|entry| entry.track.clone()).collect();
        }
        self.items = view.items.iter().map(ItemText::of).collect();
        self.rows = plan(view.items.len(), view.saved.len());
        self.summary = summary(view).into();
        self.folder = format!(
            "In {} · qualità {}, si cambia in Impostazioni",
            view.folder.display(),
            view.quality.label()
        )
        .into();
        self.view = Arc::clone(view);
    }

    fn send(&self, command: DownloadsCmd, cx: &gpui::App) {
        self.core.read(cx).downloads(command);
    }

    /// Double click or Enter on a device track: plays it (or pauses if it is already the one).
    fn play(&self, position: usize, cx: &gpui::App) {
        let Some(text) = self.saved.get(position) else { return };
        let core = self.core.read(cx);
        if self.now.is_some_and(|(id, _)| id == text.id) {
            core.playback(PlaybackCmd::Toggle);
        } else {
            core.playback(PlaybackCmd::Play { tracks: Arc::clone(&self.tracks), start: position });
        }
    }

    /// The page comes into view (`Shell::page_shown`).
    pub fn enter(&mut self, rise: bool, cx: &mut Context<Self>) {
        self.frame.enter(rise, cx);
        cx.notify();
    }

    fn set_frame(&mut self, frame: Frame, cx: &mut Context<Self>) {
        if frame != self.frame.frame {
            self.frame.frame = frame;
            cx.notify();
        }
    }

    /// List row `ix` at the frame margins, in its enter move (one step per section).
    fn line(&self, ix: usize, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let element = match self.rows.get(ix)? {
            Row::Heading(_) | Row::NothingSaved => self.row(ix, cx)?,
            _ => self.frame.row(self.row(ix, cx)?),
        };
        let section = 1 + self.rows.iter().take(ix + 1).filter(|row| matches!(row, Row::Heading(_))).count();
        let element = div().h(dp(TRACK_ROW_H)).w_full().child(element).into_any_element();
        Some(self.frame.wrap(element, section, window))
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let margin = self.frame.margin();
        Some(match *self.rows.get(ix)? {
            Row::Heading(section) => {
                let count = match section {
                    Section::InProgress => self.items.len(),
                    Section::OnDevice => self.saved.len(),
                };
                section_heading_at(section.title().into(), format::tracks(count).into(), margin)
                    .into_any_element()
            }
            Row::NothingSaved => note_at(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_SECONDARY))
                    .child("Niente sul dispositivo, per ora: i brani arrivano qui appena scaricati."),
                margin,
            )
            .into_any_element(),
            Row::Item(position) => {
                let text = self.items.get(position)?;
                let id = text.id;
                let mut row = ListRow::new(
                    ("dl-item", position),
                    RowArt::Cover(text.cover_url.clone()),
                    text.title.clone(),
                )
                .inset(SPACE_3)
                .detail(text.status.clone())
                .detail_error(text.failed);
                if let Some(progress) = text.steps {
                    row = row.steps(progress).action(
                        IconButton::new(("dl-cancel", position), sprite::ICON_CLOSE)
                            .tooltip("Annulla", None)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.send(DownloadsCmd::Cancel(id), cx)
                            })),
                    );
                } else {
                    row = row
                        .action(
                            IconButton::new(("dl-retry", position), sprite::ICON_REPEAT)
                                .tooltip("Riprova", None)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.send(DownloadsCmd::Retry(id), cx)
                                })),
                        )
                        .action(
                            IconButton::new(("dl-drop", position), sprite::ICON_CLOSE)
                                .tooltip("Togli dall'elenco", None)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.send(DownloadsCmd::Cancel(id), cx)
                                })),
                        );
                }
                row.into_any_element()
            }
            Row::Saved(position) => {
                let text = self.saved.get(position)?;
                let id = text.id;
                let mut row = ListRow::new(
                    ("dl-saved", position),
                    RowArt::Cover(text.cover_url.clone()),
                    text.title.clone(),
                )
                .index(u32::try_from(position + 1).unwrap_or(u32::MAX))
                .detail(text.detail.clone())
                .detail_error(text.lost)
                .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                    // As track rows: a click does nothing; double click or Enter plays.
                    if event.click_count() >= 2 || matches!(event, ClickEvent::Keyboard(_)) {
                        this.play(position, cx);
                    }
                }));
                if let Some((playing, state)) = self.now
                    && playing == id
                {
                    row = row.playing_at(state, &self.spot);
                }
                if text.lost {
                    let track = self.tracks.get(position).cloned();
                    row = row.action(
                        IconButton::new(("dl-again", position), sprite::ICON_DOWNLOAD)
                            .tooltip("Scarica di nuovo", None)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                if let Some(track) = &track {
                                    this.send(DownloadsCmd::Download(Box::new(track.clone())), cx);
                                }
                            })),
                    );
                }
                // The track's «Altro» menu, like a right click on a row.
                let menu_track = self.tracks.get(position).cloned();
                row.hover_action(
                    IconButton::new(("dl-more", position), sprite::ICON_MORE)
                        .tooltip("Altre azioni", None)
                        .on_click(cx.listener(move |_, event: &ClickEvent, _, cx| {
                            if let Some(track) = &menu_track {
                                cx.emit(PageEvent::TrackMenu(Box::new(TrackMenu {
                                    track: track.clone(),
                                    at: event.position(),
                                    from_playlist: None,
                                })));
                            }
                        })),
                )
                .hover_action(
                    IconButton::new(("dl-remove", position), sprite::ICON_TRASH)
                        .tooltip("Rimuovi dal dispositivo", None)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.send(DownloadsCmd::Remove(id), cx)
                        })),
                )
                .into_any_element()
            }
        })
    }

    fn title(&self, cx: &mut Context<Self>) -> PageTitle {
        let has_saved = !self.tracks.is_empty();
        let mut title = PageTitle::screen("Scaricati")
            .margin(self.frame.margin())
            .meta(self.summary.clone())
            .detail(self.folder.clone());
        if has_saved {
            let tracks = Arc::clone(&self.tracks);
            let shuffled = Arc::clone(&self.tracks);
            title = title
                .action(Button::new("dl-play", "Riproduci").primary().icon(sprite::ICON_PLAY).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.core
                            .read(cx)
                            .playback(PlaybackCmd::Play { tracks: Arc::clone(&tracks), start: 0 })
                    }),
                ))
                .action(
                    IconButton::new("dl-shuffle", sprite::ICON_SHUFFLE)
                        .tooltip("Riproduci in ordine casuale", None)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.core.read(cx).playback(PlaybackCmd::ShufflePlay(Arc::clone(&shuffled)))
                        })),
                );
        }
        title
            .action(
                Button::new("dl-folder", "Apri cartella").secondary().on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.send(DownloadsCmd::OpenFolder, cx)),
                ),
            )
            .action(
                Button::new("dl-verify", "Controlla i file")
                    .secondary()
                    .loading(self.view.verifying)
                    .disabled(self.view.verifying || !has_saved)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.send(DownloadsCmd::Verify, cx))),
            )
    }
}

fn now_playing(view: &psytify_core::snapshot::PlaybackView) -> Option<(Uuid, PlayState)> {
    play_state(view).and_then(|state| view.queue.current().map(|track| (track.id, state)))
}

impl Render for DownloadsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The spot is marked again in this frame's paint, if the playing row is visible.
        self.spot.clear();
        self.frame.begin(window, None);
        let title = self.frame.wrap(self.title(cx).into_any_element(), 0, window);
        let body: AnyElement = if self.rows.is_empty() {
            div()
                .flex_1()
                .flex()
                .justify_center()
                .pt(dp(64.0))
                .child(
                    EmptyState::crate_(
                        "Niente sul dispositivo",
                        "I brani che scarichi stanno qui e suonano anche senza rete. Il tasto Scarica sta accanto ai brani che la licenza lascia tenere.",
                    )
                    // The desktop's A to Z catalogue is on the Home.
                    .action(Button::new("dl-discover", "Scopri il catalogo").on_click(cx.listener(
                        |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Home)),
                    ))),
                )
                .into_any_element()
        } else {
            let list = uniform_list(
                "downloads-list",
                self.rows.len(),
                cx.processor(|this: &mut Self, range: std::ops::Range<usize>, window, cx| {
                    range.filter_map(|ix| this.line(ix, window, cx)).collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .size_full();
            div().flex_1().min_h_0().pt(dp(SPACE_3)).child(list).into_any_element()
        };
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(col(SURFACE_CANVAS))
            .child(self.frame.measure(cx.entity().downgrade(), Self::set_frame))
            .child(title)
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use psytify_core::api::models::fixtures;
    use psytify_core::error::AppError;

    use super::*;

    #[test]
    fn in_progress_comes_first_then_the_tracks_on_the_device() {
        assert_eq!(
            plan(2, 2),
            vec![
                Row::Heading(Section::InProgress),
                Row::Item(0),
                Row::Item(1),
                Row::Heading(Section::OnDevice),
                Row::Saved(0),
                Row::Saved(1),
            ]
        );
        assert_eq!(plan(0, 1), vec![Row::Heading(Section::OnDevice), Row::Saved(0)]);
    }

    #[test]
    fn nothing_at_all_is_the_empty_page_and_nothing_saved_is_one_sentence() {
        assert!(plan(0, 0).is_empty());
        assert_eq!(plan(1, 0), vec![Row::Heading(Section::InProgress), Row::Item(0), Row::NothingSaved]);
    }

    fn item(state: ItemState) -> Item {
        Item {
            track: Arc::new(fixtures::track(1)),
            quality: psytify_core::api::DownloadQuality::High,
            state,
            batched: false,
        }
    }

    #[test]
    fn a_download_says_where_it_is_and_only_a_live_one_has_the_bar() {
        let queued = ItemText::of(&item(ItemState::Queued));
        assert_eq!((queued.status.as_ref(), queued.steps), ("In coda", Some(0.0)));
        let half = ItemText::of(&item(ItemState::Downloading { received: 50, total: Some(100) }));
        assert_eq!((half.status.as_ref(), half.steps), ("50%", Some(0.5)));
        let unknown = ItemText::of(&item(ItemState::Downloading { received: 50, total: None }));
        assert_eq!(unknown.status.as_ref(), "Scaricamento…");
        let failed = ItemText::of(&item(ItemState::Failed(AppError::Network)));
        assert!(failed.failed && failed.steps.is_none());
        assert!(failed.status.starts_with("Non riuscito."));
    }

    fn saved() -> SavedTrack {
        let mut track = fixtures::track(2);
        track.artist.name = "Marea".into();
        SavedTrack {
            track,
            path: "/x/marea.flac".into(),
            format: "flac".into(),
            quality: None,
            size: 12_300_000,
            sha256: String::new(),
        }
    }

    #[test]
    fn a_track_on_the_device_says_format_and_weight_or_what_happened_to_its_file() {
        let fine = SavedText::of(&saved(), None);
        assert_eq!(fine.detail.as_ref(), "Marea · FLAC · 12,3 MB");
        assert!(!fine.lost);
        let missing = SavedText::of(&saved(), Some(LostReason::Missing));
        assert_eq!(missing.detail.as_ref(), "Marea · File mancante");
        assert!(missing.lost);
        assert_eq!(
            SavedText::of(&saved(), Some(LostReason::Changed)).detail.as_ref(),
            "Marea · File cambiato"
        );
    }

    #[test]
    fn the_summary_counts_and_weighs_or_says_the_check_is_running() {
        let mut view = DownloadsView::default();
        assert_eq!(summary(&view), "Nessun brano sul dispositivo");
        view.saved = vec![saved()].into();
        view.used_bytes = 12_300_000;
        assert_eq!(summary(&view), "1 brano · 12,3 MB");
        view.verifying = true;
        assert_eq!(summary(&view), "Controllo dei file…");
    }
}
