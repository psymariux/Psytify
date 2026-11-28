//! An album of the official catalogue, a Bandcamp album or a YouTube playlist
//! The head with the cover, the kind («ALBUM», «EP», «ALBUM ·
//! BANDCAMP»), the title, the author, tracks and duration; the 56 dp play button, «Salva nella libreria» and
//! «Torna alla ricerca», how many tracks are on their way to the queue, a note on how the audio arrives
//! (within the running-text measure); then the tracks (`SourceRow`). A track enters the server catalogue
//! when it is played (`ExternalCmd::PlayPlaylist`: the core imports and plays); saving imports them all,
//! one at a time. The page fills the frame and enters with the page-enter move.
//!
//! The core's external domain keeps the open playlist (`ExternalView::open`): opening the page sends
//! `ExternalCmd::OpenPlaylist` when another one is open, and the page shows what the core publishes.

use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, ListState, ParentElement, Render,
    SharedString, Styled, Subscription, Window, div, list,
};
use psytify_core::api::{SourceItem, SourceKind, SourcePlaylist};
use psytify_core::catalog::Load;
use psytify_core::command::ExternalCmd;
use psytify_core::snapshot::ExternalView;
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_ERROR, TEXT_SECONDARY, TEXT_TERTIARY};
use psytify_design::tokens::space::{SPACE_5, SPACE_6};
use psytify_design::tokens::type_::{TYPE_BODY, TYPE_META};

use super::album_page::skeleton_page;
use super::page_frame::PageFrame;
use super::page_list::{new_list, remeasure_header, set_rows, to_top};
use super::search_page::source_data;
use super::track_list::plays;
use super::{Page, PageEvent};
use crate::components::button::Button;
use crate::components::frame::{Frame, measure};
use crate::components::page_header::{HeaderArt, PageHeader};
use crate::components::section::note_at;
use crate::components::source_row::{SourceData, SourceRow};
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{TypeStyle, col, dp};

/// The kind above the title.
fn kicker(playlist: &SourcePlaylist) -> &'static str {
    match playlist.source {
        SourceKind::Deezer => playlist.kind_label(),
        SourceKind::Bandcamp => "Album · Bandcamp",
        SourceKind::YouTube | SourceKind::SoundCloud => "Playlist · YouTube",
    }
}

/// How the audio arrives.
fn explanation(source: SourceKind) -> &'static str {
    if source == SourceKind::Deezer {
        "Il server trova l'audio quando premi play. Salvando, i brani entrano nella libreria uno alla volta, \
         anche se chiudi l'app."
    } else {
        "L'audio passa dal server: ogni brano entra nel catalogo quando lo ascolti, o tutti quando salvi la \
         playlist. Quelli senza licenza libera verificata si ascoltano ma non si scaricano."
    }
}

/// «Author · 12 brani · 48 min» (the duration when the sources give it).
fn meta(playlist: &SourcePlaylist, items: Option<&[SourceItem]>) -> String {
    let Some(items) = items else { return playlist.owner.clone() };
    let total: u64 = items.iter().filter_map(|item| item.duration_ms).sum();
    let mut parts = vec![playlist.owner.clone(), format::tracks(items.len())];
    if total > 0 {
        parts.push(format::total(total));
    }
    parts.retain(|part| !part.is_empty());
    parts.join(" · ")
}

/// What the list shows after the head.
#[derive(Clone, Debug, PartialEq)]
enum Body {
    Loading,
    Failed(SharedString),
    Empty,
    Items,
}

pub struct SourcePage {
    core: Entity<CoreModel>,
    /// The playlist the page shows (`Shell` picks it).
    playlist: Option<SourcePlaylist>,
    items: Option<Arc<[SourceItem]>>,
    rows: Vec<SourceData>,
    body: Body,
    meta: SharedString,
    queued: usize,
    external: Arc<ExternalView>,
    list: ListState,
    frame: PageFrame,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for SourcePage {}

impl SourcePage {
    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let changes =
            cx.subscribe(&core, |this, core, changed: &Changed, cx| this.refresh(&core, *changed, cx));
        let external = Arc::clone(&core.read(cx).snapshot().external);
        Self {
            playlist: None,
            items: None,
            rows: Vec::new(),
            body: Body::Loading,
            meta: SharedString::default(),
            queued: external.queued(),
            external,
            list: new_list(0),
            frame: PageFrame::default(),
            core,
            _changes: changes,
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

    /// Shows `playlist`: if the core has another one open, opens this one.
    pub fn show(&mut self, playlist: SourcePlaylist, cx: &mut Context<Self>) {
        let open = self.external.open.as_ref().map(|open| &open.playlist);
        if !open.is_some_and(|open| open.source == playlist.source && open.id == playlist.id) {
            self.core.read(cx).external(ExternalCmd::OpenPlaylist(playlist.clone()));
        }
        if self.playlist.as_ref() == Some(&playlist) {
            return;
        }
        self.playlist = Some(playlist);
        self.items = None;
        self.body = Body::Loading;
        let external = Arc::clone(&self.external);
        self.take(&external);
        self.rebuild();
        to_top(&self.list);
        cx.notify();
    }

    fn refresh(&mut self, core: &Entity<CoreModel>, changed: Changed, cx: &mut Context<Self>) {
        if !changed.external {
            return;
        }
        let external = Arc::clone(&core.read(cx).snapshot().external);
        if Arc::ptr_eq(&external, &self.external) {
            return;
        }
        let imports_changed = external.imports != self.external.imports;
        self.external = Arc::clone(&external);
        let rebuild = self.take(&external);
        let queued = external.queued();
        if queued != self.queued {
            self.queued = queued;
            remeasure_header(&self.list);
        }
        if rebuild {
            self.rebuild();
        }
        if rebuild || imports_changed {
            cx.notify();
        }
    }

    /// Takes this page's tracks; `true` if what is shown changed.
    fn take(&mut self, external: &ExternalView) -> bool {
        let Some(playlist) = &self.playlist else { return false };
        let open = external
            .open
            .as_ref()
            .filter(|open| open.playlist.source == playlist.source && open.playlist.id == playlist.id);
        let (body, items) = match open.map(|open| &open.items) {
            Some(Load::Ready(items)) if items.is_empty() => (Body::Empty, Some(Arc::clone(items))),
            Some(Load::Ready(items)) => (Body::Items, Some(Arc::clone(items))),
            Some(Load::Failed(error)) => (Body::Failed(error.to_string().into()), None),
            _ => (Body::Loading, None),
        };
        let same = match (&items, &self.items) {
            (Some(new), Some(old)) => Arc::ptr_eq(new, old),
            (None, None) => true,
            _ => false,
        };
        if same && body == self.body {
            return false;
        }
        self.body = body;
        self.items = items;
        true
    }

    fn rebuild(&mut self) {
        let Some(playlist) = &self.playlist else { return };
        self.meta = meta(playlist, self.items.as_deref()).into();
        self.rows = self
            .items
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(position, item)| source_data(position, item))
            .collect();
        // The note, then the tracks or a state row.
        let rows = 1 + if self.body == Body::Items { self.rows.len() } else { 1 };
        set_rows(&self.list, rows);
        remeasure_header(&self.list);
    }

    fn item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let element = match ix {
            0 => self.playlist.clone().map(|playlist| self.header(&playlist, cx).into_any_element()),
            1 => self.playlist.as_ref().map(|playlist| {
                self.frame
                    .inset(
                        div()
                            .max_w(measure(TYPE_META))
                            .type_role(TYPE_META)
                            .text_color(col(TEXT_TERTIARY))
                            .child(explanation(playlist.source)),
                    )
                    .pb(dp(SPACE_5))
                    .into_any_element()
            }),
            _ => self.row(ix - 2, cx),
        };
        let element = element.unwrap_or_else(|| div().into_any_element());
        self.frame.wrap(element, ix.min(2), window)
    }

    fn header(&self, playlist: &SourcePlaylist, cx: &mut Context<Self>) -> PageHeader {
        let mut header = PageHeader::new(
            HeaderArt::Cover(playlist.cover_url.clone().map(SharedString::from)),
            kicker(playlist),
            playlist.title.clone(),
        )
        .margin(self.frame.margin())
        .meta(self.meta.clone());
        if self.body == Body::Items {
            header = header
                .play("source-play", cx.listener(|this, _: &ClickEvent, _, cx| this.play(0, cx)))
                .action(
                    Button::new("source-save", "Salva nella libreria")
                        .secondary()
                        .icon(sprite::ICON_PLUS)
                        .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PageEvent::SaveSource))),
                );
        }
        header = header.action(
            Button::new("source-back", "Torna alla ricerca")
                .ghost()
                .icon(sprite::ICON_SEARCH)
                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Search)))),
        );
        if self.queued > 0 {
            header = header.action(
                div()
                    .type_role(TYPE_META)
                    .text_color(col(TEXT_SECONDARY))
                    .child(format!("{} in arrivo nella coda", format::tracks(self.queued))),
            );
        }
        header
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let margin = self.frame.margin();
        Some(match &self.body {
            Body::Loading => div().into_any_element(),
            Body::Failed(error) => note_at(
                div()
                    .flex()
                    .items_center()
                    .gap(dp(SPACE_6))
                    .child(div().type_role(TYPE_BODY).text_color(col(TEXT_ERROR)).child(error.clone()))
                    .child(Button::new("source-retry", "Riprova").secondary().on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| this.core.read(cx).external(ExternalCmd::RetryPlaylist),
                    ))),
                margin,
            )
            .into_any_element(),
            Body::Empty => note_at(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_SECONDARY))
                    .child("Questa playlist non ha brani che si possano ascoltare."),
                margin,
            )
            .into_any_element(),
            Body::Items => {
                let item = self.items.as_ref()?.get(ix)?;
                self.frame.row(
                    SourceRow::new(("source-page-row", ix), self.rows.get(ix)?.clone())
                        .busy(self.external.importing(item.source, &item.id))
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            if plays(event) {
                                this.play(ix, cx);
                            }
                        })),
                )
            }
        })
    }

    /// Plays the playlist from `start`: the core imports the first track and plays it, the others follow.
    fn play(&self, start: usize, cx: &gpui::App) {
        self.core.read(cx).external(ExternalCmd::PlayPlaylist { start, video: false });
    }
}

impl Render for SourcePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame.begin(window, Some(&self.list));
        let page = div()
            .relative()
            .size_full()
            .bg(col(SURFACE_CANVAS))
            .child(self.frame.measure(cx.entity().downgrade(), Self::set_frame));
        if self.playlist.is_none() || self.body == Body::Loading {
            return page.child(skeleton_page(false, self.frame.margin()));
        }
        page.child(
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
    use super::*;

    fn playlist(source: SourceKind, kind: &str) -> SourcePlaylist {
        SourcePlaylist {
            source,
            id: "1".into(),
            title: "Sera".into(),
            owner: "Marea".into(),
            cover_url: None,
            kind: kind.into(),
            released_on: None,
        }
    }

    #[test]
    fn the_kicker_says_what_the_source_is() {
        assert_eq!(kicker(&playlist(SourceKind::Deezer, "ep")), "EP");
        assert_eq!(kicker(&playlist(SourceKind::Bandcamp, "album")), "Album · Bandcamp");
        assert_eq!(kicker(&playlist(SourceKind::YouTube, "playlist")), "Playlist · YouTube");
    }

    #[test]
    fn the_meta_line_grows_when_the_tracks_arrive() {
        let list = playlist(SourceKind::Deezer, "album");
        assert_eq!(meta(&list, None), "Marea");
        let item = |ms| SourceItem {
            source: SourceKind::Deezer,
            id: "x".into(),
            title: "T".into(),
            artist: "A".into(),
            duration_ms: ms,
            cover_url: None,
        };
        assert_eq!(meta(&list, Some(&[item(Some(120_000)), item(None)])), "Marea · 2 brani · 2 min");
        assert_eq!(meta(&list, Some(&[item(None)])), "Marea · 1 brano");
    }
}
