//! La pagina «Scaricati»: i download in corso e i brani sul dispositivo, che suonano anche senza rete.
//! Tutto viene da `DownloadsView` e ogni gesto è un `DownloadsCmd` (o un `PlaybackCmd` per suonare): la
//! pagina non tocca il disco e non calcola niente nel disegno, come la pagina Iced
//! (`client/src/ui/pages/downloads.rs`).
//!
//! In cima il titolo con il conteggio e lo spazio occupato, la cartella e la qualità, e i pulsanti
//! Riproduci, Casuale, «Apri cartella» e «Controlla i file» (il controllo a fondo, `DownloadsCmd::Verify`).
//! Sotto, una sola `uniform_list` di righe da 48 dp: «In corso» (stato, barra a dieci passi, Annulla;
//! Riprova e «Togli dall'elenco» per un download fallito) e «Sul dispositivo» (formato e peso, o «File
//! mancante»/«File cambiato» in tinta d'errore con «Scarica di nuovo»; «Rimuovi dal dispositivo» al
//! passaggio del mouse, con il menu «Altro» del brano). Doppio clic o Invio su un brano lo suona, con la
//! coda dei brani sul dispositivo. Vuota, «Scopri il catalogo» apre la Home (il catalogo dalla A alla Z).
//! Gli avvisi (salvato, rimosso, controllo finito) sono eventi del core e li mostra `Shell`.
//!
//! I testi delle righe si rifanno quando cambia la vista dei download: quelli dei brani sul dispositivo
//! solo se cambia l'indice o l'elenco dei file persi (possono essere centinaia), quelli della coda a ogni
//! pubblicazione (pochi, e l'avanzamento cambia di un punto alla volta).

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
use psytify_design::tokens::space::SPACE_3;
use psytify_design::tokens::type_::TYPE_BODY;
use uuid::Uuid;

use super::track_menu::TrackMenu;
use super::{Page, PageEvent, play_state};
use crate::components::button::Button;
use crate::components::icon_button::IconButton;
use crate::components::indicators::PlayState;
use crate::components::list_row::{ListRow, RowArt};
use crate::components::overlay::EmptyState;
use crate::components::playing_spot::PlayingSpot;
use crate::components::section::{PageTitle, note, section_heading};
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{TypeStyle, col, dp};

/// Le due sezioni.
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

/// Una riga della lista, nell'ordine in cui si disegna. Gli indici sono nelle liste del core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Heading(Section),
    /// Un download della coda (`DownloadsView::items`).
    Item(usize),
    /// Niente sul dispositivo, ma qualcosa in corso: una frase al posto della sezione.
    NothingSaved,
    /// Un brano sul dispositivo (`DownloadsView::saved`).
    Saved(usize),
}

/// Quali righe mostrare: prima i download in corso, poi i brani sul dispositivo. Vuoto se non c'è né
/// l'una né l'altra cosa (la pagina mostra allora il suo stato vuoto). Pura.
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

/// I testi di un download della coda.
#[derive(Clone, Debug, PartialEq)]
struct ItemText {
    id: Uuid,
    title: SharedString,
    status: SharedString,
    failed: bool,
    /// La barra a dieci passi, finché il download è vivo.
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

/// I testi di un brano sul dispositivo: «Artista · FLAC · 12,3 MB», o il problema del suo file.
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

/// La riga sotto il titolo: quanti brani e quanto pesano, o che il controllo è in corso.
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
    /// I brani sul dispositivo come coda da suonare: si rifà solo quando l'indice cambia.
    tracks: Arc<[Track]>,
    summary: SharedString,
    folder: SharedString,
    /// Il brano in corso e come mostrarlo.
    now: Option<(Uuid, PlayState)>,
    scroll: UniformListScrollHandle,
    /// Il posto dell'equalizzatore del brano in corso: lo dipinge `Shell` (BACKLOG 23).
    spot: PlayingSpot,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for DownloadsPage {}

impl DownloadsPage {
    /// Il posto dell'equalizzatore di questa vista, per lo strato di `Shell`.
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

    /// Prende la vista nuova e rifà i testi: quelli dei brani sul dispositivo solo se l'indice o i file
    /// persi sono cambiati (o la prima volta).
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

    /// Doppio clic o Invio su un brano sul dispositivo: suona (o mette in pausa se è già lui).
    fn play(&self, position: usize, cx: &gpui::App) {
        let Some(text) = self.saved.get(position) else { return };
        let core = self.core.read(cx);
        if self.now.is_some_and(|(id, _)| id == text.id) {
            core.playback(PlaybackCmd::Toggle);
        } else {
            core.playback(PlaybackCmd::Play { tracks: Arc::clone(&self.tracks), start: position });
        }
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(match *self.rows.get(ix)? {
            Row::Heading(section) => {
                let count = match section {
                    Section::InProgress => self.items.len(),
                    Section::OnDevice => self.saved.len(),
                };
                section_heading(section.title().into(), format::tracks(count).into()).into_any_element()
            }
            Row::NothingSaved => note(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_SECONDARY))
                    .child("Niente sul dispositivo, per ora: i brani arrivano qui appena scaricati."),
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
                    // Come le righe di brano: un clic non fa niente; doppio clic o Invio suonano.
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
                // Il menu «Altro» del brano, come il tasto destro sulla riga del client Iced.
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
        let mut title = PageTitle::screen("Scaricati").meta(self.summary.clone()).detail(self.folder.clone());
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Il posto si segna di nuovo nel paint di questo fotogramma, se la riga in corso è visibile.
        self.spot.clear();
        let title = self.title(cx);
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
                    // Come nel client Iced; il catalogo dalla A alla Z del desktop sta nella Home.
                    .action(Button::new("dl-discover", "Scopri il catalogo").on_click(cx.listener(
                        |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Home)),
                    ))),
                )
                .into_any_element()
        } else {
            let list = uniform_list(
                "downloads-list",
                self.rows.len(),
                cx.processor(|this: &mut Self, range: std::ops::Range<usize>, _window, cx| {
                    range.filter_map(|ix| this.row(ix, cx)).collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .size_full();
            div().flex_1().min_h_0().pt(dp(SPACE_3)).child(list).into_any_element()
        };
        div().size_full().flex().flex_col().bg(col(SURFACE_CANVAS)).child(title).child(body)
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
