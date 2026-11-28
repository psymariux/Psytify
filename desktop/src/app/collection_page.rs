//! The liked songs and a local playlist: one view that switches collection when `Shell` opens the other
//! (`CollectionPage::show`). The head
//! with the count and duration, the 56 dp play button, shuffle and «Scarica tutto» (a playlist also has
//! rename and delete, through `Shell`'s dialog); then the tracks, 48 dp `TrackRow`s in one column (title :
//! album 3 : 2). The head is the object pages' `PageHeader` (the heart on `GRAPHITE` for the liked songs, the
//! mosaic of the first covers for a playlist) and scrolls with the tracks (`page_list`). The page fills the
//! frame (desktop v2 §2) and enters with the page-enter move.
//!
//! Double click or Enter on a track: the queue becomes the collection from there (`PlaybackCmd::Play`), or
//! pauses on the playing track. The heart (white, never lime) sends `LibraryCmd::ToggleLike`: in the liked
//! songs, removing it removes the row. The download sends `DownloadsCmd::Download`, «Scarica tutto»
//! `DownloadAll` (while the batch downloads its count takes the place). On hover «Altro» opens `Shell`'s
//! track menu. Nothing here recomputes the core's logic.
//!
//! The same view shows a friend's public playlist (`Collection::Public`, from their profile): «Playlist
//! pubblica» with the owner's name linking to the profile, without rename and delete; the tracks come from
//! `SocialView::playlist`.
//!
//! Row texts are rebuilt only when the collection changes (its list in `LibraryView`, which the core only
//! replaces when it touches it).

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, ListState, ParentElement, Render,
    SharedString, Styled, Subscription, Window, div, list,
};
use psytify_core::api::Track;
use psytify_core::catalog::Load;
use psytify_core::command::SocialCmd;
use psytify_core::command::{DownloadsCmd, LibraryCmd, PlaybackCmd};
use psytify_core::snapshot::{DownloadsView, LibraryView, SocialView};
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_PRIMARY, TEXT_SECONDARY};
use psytify_design::tokens::space::SPACE_5;
use psytify_design::tokens::type_::{TYPE_BODY, TYPE_NUMERIC};
use uuid::Uuid;

use super::page_frame::PageFrame;
use super::page_list::{new_list, remeasure_header, set_rows, to_top};
use super::playlist_dialog::DialogRequest;
use super::track_list::{
    RowText, batch_label, download_all_label, download_command, download_state, now_playing, plays,
    with_links,
};
use super::track_menu::TrackMenu;
use super::{Page, PageEvent};
use crate::components::button::Button;
use crate::components::frame::Frame;
use crate::components::icon_button::IconButton;
use crate::components::indicators::{DownloadState, PlayState};
use crate::components::overlay::EmptyState;
use crate::components::page_header::{HeaderArt, PageHeader, mosaic_urls};
use crate::components::playing_spot::PlayingSpot;
use crate::components::section::note_at;
use crate::components::track_row::{TrackData, TrackRow};
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{TypeStyle, col, dp};

/// The album column shows from this inner width (dp) up.
const ALBUM_COLUMN_MIN: f32 = 640.0;

/// Which collection the page shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collection {
    Liked,
    Playlist(Uuid),
    /// A friend's public playlist.
    Public(Uuid),
}

/// The collection read from the library: the tracks and what the head says. `None` if the playlist is gone.
#[derive(Clone, Debug)]
struct Found {
    name: String,
    tracks: Arc<[Track]>,
    /// Tracks of an external playlist still to import, and where they come from.
    pending: Option<(usize, &'static str)>,
    /// Whose public playlist it is: handle and name.
    owner: Option<(String, SharedString)>,
}

fn find(view: &LibraryView, social: &SocialView, collection: Collection) -> Option<Found> {
    match collection {
        Collection::Liked => Some(Found {
            name: "Brani che ti piacciono".to_owned(),
            tracks: Arc::from(view.liked.as_slice()),
            pending: None,
            owner: None,
        }),
        Collection::Playlist(id) => view.playlists.iter().find(|p| p.id == id).map(|playlist| Found {
            name: playlist.name.clone(),
            tracks: Arc::from(playlist.tracks.as_slice()),
            pending: playlist.pending.first().map(|first| (playlist.pending.len(), first.source.label())),
            owner: None,
        }),
        Collection::Public(id) => match &social.playlist {
            Some(Load::Ready(playlist)) if playlist.id == id => Some(Found {
                name: playlist.name.clone(),
                tracks: Arc::clone(&playlist.tracks),
                pending: None,
                owner: Some((playlist.owner.handle.clone(), playlist.owner.display_name.clone().into())),
            }),
            _ => None,
        },
    }
}

/// The published public playlist, to tell whether it changed (the same `Arc` or the same state).
fn public_key(social: &SocialView) -> (u8, usize) {
    match &social.playlist {
        None => (0, 0),
        Some(Load::Idle) => (1, 0),
        Some(Load::Loading) => (2, 0),
        Some(Load::Failed(_)) => (3, 0),
        Some(Load::Ready(playlist)) => (4, Arc::as_ptr(playlist) as usize),
    }
}

/// The collection changed between two library views: its list is no longer the same `Arc`.
fn moved(old: &LibraryView, new: &LibraryView, collection: Collection) -> bool {
    match collection {
        Collection::Liked => !new.liked.same(&old.liked),
        Collection::Playlist(_) => !new.playlists.same(&old.playlists),
        Collection::Public(_) => false,
    }
}

pub struct CollectionPage {
    core: Entity<CoreModel>,
    collection: Collection,
    library: Arc<LibraryView>,
    /// The social view (the open public playlist) and its owner's handle.
    social: Arc<SocialView>,
    public_handle: String,
    downloads: Arc<DownloadsView>,
    liked: Arc<HashSet<Uuid>>,
    found: Option<Found>,
    texts: Vec<RowText>,
    meta: SharedString,
    pending_note: Option<SharedString>,
    download_label: Option<&'static str>,
    /// The covers of a playlist's mosaic.
    mosaic: Vec<SharedString>,
    /// The «Scarica tutto» count while the batch downloads («3/10»).
    batch: Option<SharedString>,
    now: Option<(Uuid, PlayState)>,
    /// The head (item 0) and the rows.
    list: ListState,
    frame: PageFrame,
    /// Where the playing track's equalizer goes: `Shell` paints it (BACKLOG 23).
    spot: PlayingSpot,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for CollectionPage {}

impl CollectionPage {
    /// This view's equalizer spot, for `Shell`'s layer.
    pub fn spot(&self) -> PlayingSpot {
        self.spot.clone()
    }

    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let changes =
            cx.subscribe(&core, |this, core, changed: &Changed, cx| this.refresh(&core, *changed, cx));
        let snapshot = core.read(cx).snapshot();
        let mut page = Self {
            library: Arc::clone(&snapshot.library),
            social: core.read(cx).social_view(),
            public_handle: String::new(),
            downloads: Arc::clone(&snapshot.downloads),
            liked: core.read(cx).liked(),
            collection: Collection::Liked,
            found: None,
            texts: Vec::new(),
            meta: SharedString::default(),
            pending_note: None,
            download_label: None,
            mosaic: Vec::new(),
            batch: batch_label(&snapshot.downloads),
            now: now_playing(&snapshot.playback),
            list: new_list(0),
            frame: PageFrame::default(),
            core,
            spot: PlayingSpot::new(),
            _changes: changes,
        };
        page.rebuild();
        page
    }

    /// The page comes into view (`Shell::page_shown`).
    pub fn enter(&mut self, rise: bool, cx: &mut Context<Self>) {
        self.frame.enter(rise, cx);
        cx.notify();
    }

    /// A new width: the margins and the album column may change.
    fn set_frame(&mut self, frame: Frame, cx: &mut Context<Self>) {
        if frame != self.frame.frame {
            self.frame.frame = frame;
            cx.notify();
        }
    }

    /// Shows `handle`'s public playlist `id` and asks the core for it.
    pub fn show_public(&mut self, handle: &str, id: Uuid, cx: &mut Context<Self>) {
        self.core.read(cx).social(SocialCmd::OpenFriendPlaylist { handle: handle.to_owned(), id });
        self.public_handle = handle.to_owned();
        self.social = self.core.read(cx).social_view();
        if self.collection == Collection::Public(id) {
            self.rebuild();
            cx.notify();
        }
        self.show(Collection::Public(id), cx);
    }

    /// Shows another collection (the list goes back to the top). Nothing if it is the same.
    pub fn show(&mut self, collection: Collection, cx: &mut Context<Self>) {
        if collection == self.collection {
            return;
        }
        self.collection = collection;
        self.rebuild();
        to_top(&self.list);
        cx.notify();
    }

    fn refresh(&mut self, core: &Entity<CoreModel>, changed: Changed, cx: &mut Context<Self>) {
        let snapshot = core.read(cx).snapshot();
        let mut dirty = false;
        if changed.library && !Arc::ptr_eq(&snapshot.library, &self.library) {
            let rebuild = moved(&self.library, &snapshot.library, self.collection);
            let liked = core.read(cx).liked();
            // The hearts change only when the liked set does.
            dirty |= !Arc::ptr_eq(&liked, &self.liked);
            self.liked = liked;
            self.library = Arc::clone(&snapshot.library);
            if rebuild {
                self.rebuild();
                dirty = true;
            }
        }
        if changed.social {
            let social = core.read(cx).social_view();
            let moved = public_key(&social) != public_key(&self.social);
            self.social = social;
            if moved && matches!(self.collection, Collection::Public(_)) {
                self.rebuild();
                dirty = true;
            }
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

    fn rebuild(&mut self) {
        self.found = find(&self.library, &self.social, self.collection);
        self.fill();
        let rows = self.found.as_ref().map_or(0, |found| found.tracks.len()) + self.offset();
        set_rows(&self.list, rows);
        remeasure_header(&self.list);
    }

    fn fill(&mut self) {
        let Some(found) = &self.found else {
            self.texts.clear();
            self.meta = SharedString::default();
            self.pending_note = None;
            self.download_label = None;
            self.mosaic.clear();
            return;
        };
        self.mosaic = mosaic_urls(found.tracks.iter().map(|t| t.cover_url.as_deref()));
        self.texts = found.tracks.iter().map(RowText::of).collect();
        let total: u64 = found.tracks.iter().map(|t| t.duration_ms).sum();
        self.meta = format!("{} · {}", format::tracks(found.tracks.len()), format::total(total)).into();
        self.pending_note = found.pending.map(|(count, source)| {
            format!(
                "Altri {count} brani in arrivo da {source}: entrano uno alla volta, anche se chiudi l'app."
            )
            .into()
        });
        self.download_label = download_all_label(&found.tracks);
    }

    fn tracks(&self) -> Option<&Arc<[Track]>> {
        self.found.as_ref().map(|found| &found.tracks)
    }

    /// Double click or Enter: the queue becomes the collection from `start`; on the playing track, play or
    /// pause.
    fn play(&self, start: usize, cx: &gpui::App) {
        let Some(tracks) = self.tracks() else { return };
        let core = self.core.read(cx);
        match (tracks.get(start), self.now) {
            (Some(track), Some((id, _))) if track.id == id => core.playback(PlaybackCmd::Toggle),
            _ => core.playback(PlaybackCmd::Play { tracks: Arc::clone(tracks), start }),
        }
    }

    fn download_state(&self, track: &Track) -> DownloadState {
        download_state(&self.downloads, track)
    }

    /// The list rows: the note on imports on their way (if any), then the tracks.
    fn offset(&self) -> usize {
        usize::from(self.pending_note.is_some())
    }

    /// List item `ix`: the head, then the rows; the head and the rows enter one step apart.
    fn item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let element = match ix.checked_sub(1) {
            None => self.found.clone().map(|found| self.header(&found, cx).into_any_element()),
            Some(row) => self.row(row, self.frame.frame.inner >= ALBUM_COLUMN_MIN, cx),
        };
        let element = element.unwrap_or_else(|| div().into_any_element());
        self.frame.wrap(element, usize::from(ix > 0), window)
    }

    fn row(&self, ix: usize, wide: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(text) = self.pending_note.as_ref().filter(|_| ix == 0) {
            return Some(
                note_at(
                    div().type_role(TYPE_BODY).text_color(col(TEXT_SECONDARY)).child(text.clone()),
                    self.frame.margin(),
                )
                .into_any_element(),
            );
        }
        let position = ix - self.offset();
        let track = self.tracks()?.get(position)?;
        let text = self.texts.get(position)?;
        let data = TrackData {
            index: u32::try_from(position + 1).unwrap_or(u32::MAX),
            title: text.title.clone(),
            artist: text.artist.clone(),
            album: text.album.clone(),
            duration: text.duration.clone(),
            liked: self.liked.contains(&track.id),
            download: self.download_state(track),
            available: true,
            cover_url: text.cover_url.clone(),
        };
        let liked_track = track.clone();
        let download_track = track.clone();
        let mut row = TrackRow::new(("collection-row", position), data)
            .show_album(wide)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                // A click selects nothing; double click or Enter plays.
                if plays(event) {
                    this.play(position, cx);
                }
            }))
            .on_like(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.core.read(cx).library(LibraryCmd::ToggleLike(liked_track.clone()));
            }))
            .on_download(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.core.read(cx).downloads(download_command(&this.downloads, &download_track));
            }));
        let from_playlist = match self.collection {
            Collection::Playlist(id) => Some((id, position)),
            Collection::Liked | Collection::Public(_) => None,
        };
        let menu_track = track.clone();
        row = row.on_more(cx.listener(move |_, event: &ClickEvent, _, cx| {
            cx.emit(PageEvent::TrackMenu(Box::new(TrackMenu {
                track: menu_track.clone(),
                at: event.position(),
                from_playlist,
            })));
        }));
        row = with_links(row, track, cx);
        if let Some((id, state)) = self.now
            && id == track.id
        {
            row = row.playing_at(state, &self.spot);
        }
        Some(self.frame.row(row))
    }

    fn header(&self, found: &Found, cx: &mut Context<Self>) -> PageHeader {
        let art = match self.collection {
            Collection::Liked => HeaderArt::Sprite(sprite::ICON_HEART_FILL),
            Collection::Playlist(_) | Collection::Public(_) => HeaderArt::Mosaic(self.mosaic.clone()),
        };
        let kicker = if found.owner.is_some() { "Playlist pubblica" } else { "Playlist" };
        let mut title = PageHeader::new(art, kicker, found.name.clone())
            .margin(self.frame.margin())
            .meta(self.meta.clone());
        if let Some((handle, name)) = found.owner.clone() {
            title = title.link(
                "col-owner",
                name,
                cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::OpenProfile(handle.clone()))),
            );
        }
        if !found.tracks.is_empty() {
            title = title
                .play(
                    "col-play",
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(tracks) = this.tracks() {
                            this.core
                                .read(cx)
                                .playback(PlaybackCmd::Play { tracks: Arc::clone(tracks), start: 0 });
                        }
                    }),
                )
                .action(
                    IconButton::new("col-shuffle", sprite::ICON_SHUFFLE)
                        .tooltip("Riproduci in ordine casuale", None)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            if let Some(tracks) = this.tracks() {
                                this.core.read(cx).playback(PlaybackCmd::ShufflePlay(Arc::clone(tracks)));
                            }
                        })),
                );
            match (&self.batch, self.download_label) {
                // While the batch downloads, the count replaces the icon.
                (Some(count), _) => {
                    title = title.action(
                        div().type_role(TYPE_NUMERIC).text_color(col(TEXT_PRIMARY)).child(count.clone()),
                    )
                }
                (None, Some(label)) => {
                    title = title.action(
                        IconButton::new("col-download", sprite::ICON_DOWNLOAD).tooltip(label, None).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                if let Some(tracks) = this.tracks() {
                                    this.core
                                        .read(cx)
                                        .downloads(DownloadsCmd::DownloadAll(Arc::clone(tracks)));
                                }
                            }),
                        ),
                    )
                }
                (None, None) => {}
            }
        }
        if let Collection::Playlist(id) = self.collection {
            let rename = found.name.clone();
            let delete = found.name.clone();
            title = title
                .action(IconButton::new("col-rename", sprite::ICON_EDIT).tooltip("Rinomina", None).on_click(
                    cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.emit(PageEvent::Dialog(DialogRequest::Rename { id, name: rename.clone() }))
                    }),
                ))
                .action(
                    IconButton::new("col-delete", sprite::ICON_TRASH)
                        .tooltip("Elimina la playlist", None)
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(PageEvent::Dialog(DialogRequest::Delete { id, name: delete.clone() }))
                        })),
                );
        }
        title
    }
}

impl Render for CollectionPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The spot is marked again in this frame's paint, if the playing row is visible.
        self.spot.clear();
        self.frame.begin(window, Some(&self.list));
        let page = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(col(SURFACE_CANVAS))
            .child(self.frame.measure(cx.entity().downgrade(), Self::set_frame));
        let Some(found) = self.found.as_ref() else {
            if let Collection::Public(id) = self.collection {
                // A friend's playlist comes from the network: the skeleton, or the error with «Riprova».
                return match &self.social.playlist {
                    Some(Load::Failed(error)) => page.child(super::album_page::failure(
                        "col-public-retry",
                        error.to_string().into(),
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let handle = this.public_handle.clone();
                            this.core.read(cx).social(SocialCmd::OpenFriendPlaylist { handle, id });
                        }),
                    )),
                    _ => page.child(PageHeader::loading(false, self.frame.margin())),
                };
            }
            return page.child(centered(
                EmptyState::crate_("Questa playlist non esiste più", "Forse è stata eliminata.").action(
                    Button::new("col-back", "Vai alla libreria").on_click(
                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Library))),
                    ),
                ),
            ));
        };
        if found.tracks.is_empty() && found.pending.is_none() {
            let title = self.header(found, cx);
            let empty = match self.collection {
                Collection::Liked => EmptyState::crate_(
                    "Ancora nessun cuore",
                    "Premi il cuore accanto a un brano e lo ritrovi qui, su questo computer.",
                ),
                Collection::Playlist(_) => EmptyState::crate_(
                    "Questa playlist è vuota",
                    "Aggiungi i brani dal menu «Altro» di un brano: lo trovi passando sulla riga.",
                ),
                Collection::Public(_) => EmptyState::crate_(
                    "Nessun brano da ascoltare",
                    "I brani di questa playlist non sono più nel catalogo.",
                ),
            }
            .action(
                Button::new("col-search", "Cerca brani")
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Search)))),
            );
            return page.child(title).child(centered(empty));
        }
        let list = list(
            self.list.clone(),
            cx.processor(|this: &mut Self, ix: usize, window, cx| this.item(ix, window, cx)),
        )
        .size_full();
        page.child(div().flex_1().min_h_0().child(list))
    }
}

/// Content in the middle, a third from the top, like the guidelines' empty states.
fn centered(content: impl IntoElement) -> gpui::Div {
    div().flex_1().flex().justify_center().pt(dp(64.0 + SPACE_5)).child(content)
}

#[cfg(test)]
mod tests {
    use psytify_core::api::models::fixtures;
    use psytify_core::config::Playlist;

    use super::*;

    fn library(liked: Vec<Track>, playlists: Vec<Playlist>) -> LibraryView {
        LibraryView { liked: liked.into(), playlists: playlists.into(), ..LibraryView::default() }
    }

    #[test]
    fn the_liked_songs_and_a_playlist_are_found_and_a_deleted_one_is_not() {
        let mut list = Playlist::new("Sera", vec![fixtures::track(1), fixtures::track(2)]);
        list.pending = Vec::new();
        let id = list.id;
        let view = library(vec![fixtures::track(3)], vec![list]);
        let social = SocialView::default();
        let liked = find(&view, &social, Collection::Liked).map(|found| found.tracks.len());
        assert_eq!(liked, Some(1));
        let playlist = find(&view, &social, Collection::Playlist(id));
        assert_eq!(playlist.as_ref().map(|found| found.name.as_str()), Some("Sera"));
        assert_eq!(playlist.map(|found| found.tracks.len()), Some(2));
        assert!(find(&view, &social, Collection::Playlist(Uuid::from_u128(99))).is_none());
    }

    #[test]
    fn a_friend_playlist_is_found_only_once_it_arrives() {
        use psytify_core::api::social::{Profile, PublicPlaylist};
        let id = Uuid::from_u128(7);
        let view = library(Vec::new(), Vec::new());
        let mut social = SocialView { playlist: Some(Load::Loading), ..SocialView::default() };
        assert!(find(&view, &social, Collection::Public(id)).is_none());
        let loading = public_key(&social);
        social.playlist = Some(Load::Ready(Arc::new(PublicPlaylist {
            id,
            name: "Sera al porto".to_owned(),
            owner: Profile { handle: "luca".to_owned(), display_name: "Luca".to_owned(), avatar_url: None },
            tracks: Arc::from(vec![fixtures::track(1)]),
        })));
        assert_ne!(public_key(&social), loading);
        let found = find(&view, &social, Collection::Public(id));
        assert_eq!(found.as_ref().map(|f| f.tracks.len()), Some(1));
        assert_eq!(found.and_then(|f| f.owner).map(|(handle, _)| handle).as_deref(), Some("luca"));
        assert!(find(&view, &social, Collection::Public(Uuid::nil())).is_none(), "un'altra playlist");
    }

    #[test]
    fn only_the_shown_collection_moving_rebuilds_the_rows() {
        let old = library(vec![fixtures::track(1)], vec![Playlist::new("Sera", Vec::new())]);
        // One listen: liked and playlist are the same `Arc`s.
        let listened = LibraryView { recent: vec![fixtures::track(9)].into(), ..old.clone() };
        assert!(!moved(&old, &listened, Collection::Liked));
        assert!(!moved(&old, &listened, Collection::Playlist(Uuid::nil())));
        let hearted = LibraryView { liked: vec![fixtures::track(2)].into(), ..old.clone() };
        assert!(moved(&old, &hearted, Collection::Liked));
        assert!(!moved(&old, &hearted, Collection::Playlist(Uuid::nil())));
    }
}
