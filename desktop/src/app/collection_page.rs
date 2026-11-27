//! Le pagine «Brani che ti piacciono» e di una playlist locale: una vista sola, che cambia raccolta quando
//! `Shell` apre l'altra (`CollectionPage::show`). Come nel client Iced (`client/src/ui/pages/liked.rs` e
//! `playlist.rs`): il titolo con il conteggio e la durata, Riproduci, Casuale e «Scarica tutto» (per la
//! playlist anche Rinomina ed Elimina, con il dialogo di `Shell`); poi i brani, `TrackRow` da 48 dp. La
//! testa è il `PageHeader` delle pagine d'oggetto (il cuore su campo `GRAPHITE` per i piaciuti, il mosaico
//! delle prime copertine per una playlist) e scorre con i brani (`page_list`).
//!
//! Doppio clic o Invio su un brano: la coda diventa la raccolta e parte da lì (`PlaybackCmd::Play`), o
//! pausa se è il brano in corso. Il cuore (bianco, mai lime) manda `LibraryCmd::ToggleLike`: nei brani
//! piaciuti toglierlo fa sparire la riga. Il download manda `DownloadsCmd::Download`, «Scarica tutto»
//! `DownloadAll` (mentre il lotto scarica, al suo posto c'è il conteggio). Al passaggio
//! del mouse «Altro» apre il menu del brano (di `Shell`: coda, playlist e, in una playlist, «Togli da
//! questa playlist»). Niente qui ricalcola la logica del core.
//!
//! La stessa vista mostra la playlist pubblica di un amico (`Collection::Public`, dalla pagina del suo profilo):
//! «Playlist pubblica» con il nome del proprietario che apre il suo profilo, Riproduci, Casuale e «Scarica
//! tutto», senza Rinomina ed Elimina; i brani vengono da `SocialView::playlist`.
//!
//! I testi delle righe si rifanno solo quando la raccolta cambia (la sua lista in `LibraryView`, che il
//! core cambia solo se la tocca); un ascolto o un cuore altrove notificano la pagina senza rifarli.

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
use psytify_design::tokens::size::BP_RAIL;
use psytify_design::tokens::space::SPACE_5;
use psytify_design::tokens::type_::{TYPE_BODY, TYPE_NUMERIC};
use uuid::Uuid;

use super::page_list::{new_list, remeasure_header, set_rows, to_top};
use super::playlist_dialog::DialogRequest;
use super::track_list::{
    RowText, batch_label, download_all_label, download_command, download_state, now_playing, plays,
    with_links,
};
use super::track_menu::TrackMenu;
use super::{Page, PageEvent};
use crate::components::button::Button;
use crate::components::icon_button::IconButton;
use crate::components::indicators::{DownloadState, PlayState};
use crate::components::overlay::EmptyState;
use crate::components::page_header::{HeaderArt, PageHeader, mosaic_urls};
use crate::components::playing_spot::PlayingSpot;
use crate::components::section::note;
use crate::components::track_row::{TrackData, TrackRow};
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{TypeStyle, col, dp};

/// Quale raccolta mostra la pagina.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collection {
    Liked,
    Playlist(Uuid),
    /// La playlist pubblica di un amico.
    Public(Uuid),
}

/// La raccolta letta dalla libreria: i brani e ciò che il titolo dice. `None` se la playlist non c'è più.
#[derive(Clone, Debug)]
struct Found {
    name: String,
    tracks: Arc<[Track]>,
    /// Brani di una playlist esterna ancora da importare, e da dove arrivano.
    pending: Option<(usize, &'static str)>,
    /// Di chi è una playlist pubblica: nome utente e nome.
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

/// La playlist pubblica pubblicata, per sapere se è cambiata (la stessa `Arc` o lo stesso stato).
fn public_key(social: &SocialView) -> (u8, usize) {
    match &social.playlist {
        None => (0, 0),
        Some(Load::Idle) => (1, 0),
        Some(Load::Loading) => (2, 0),
        Some(Load::Failed(_)) => (3, 0),
        Some(Load::Ready(playlist)) => (4, Arc::as_ptr(playlist) as usize),
    }
}

/// La raccolta è cambiata fra due viste della libreria: la sua lista non è più la stessa `Arc`.
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
    /// La vista sociale (la playlist pubblica aperta) e il nome utente del suo proprietario.
    social: Arc<SocialView>,
    public_handle: String,
    downloads: Arc<DownloadsView>,
    liked: Arc<HashSet<Uuid>>,
    found: Option<Found>,
    texts: Vec<RowText>,
    meta: SharedString,
    pending_note: Option<SharedString>,
    download_label: Option<&'static str>,
    /// Le copertine del mosaico di una playlist.
    mosaic: Vec<SharedString>,
    /// Il conteggio di «Scarica tutto» mentre il lotto scarica («3/10»).
    batch: Option<SharedString>,
    now: Option<(Uuid, PlayState)>,
    /// La testa (voce 0) e le righe.
    list: ListState,
    /// Il posto dell'equalizzatore del brano in corso: lo dipinge `Shell` (BACKLOG 23).
    spot: PlayingSpot,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for CollectionPage {}

impl CollectionPage {
    /// Il posto dell'equalizzatore di questa vista, per lo strato di `Shell`.
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
            core,
            spot: PlayingSpot::new(),
            _changes: changes,
        };
        page.rebuild();
        page
    }

    /// Mostra la playlist pubblica `id` di `handle` e la chiede al core.
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

    /// Mostra un'altra raccolta (la lista torna in cima). Niente se è già quella.
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
            // I cuori cambiano solo se cambia l'insieme dei piaciuti.
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

    /// Doppio clic o Invio: la coda diventa la raccolta, da `start`; sul brano in corso, play o pausa.
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

    /// Le righe della lista: la nota delle importazioni in arrivo (se c'è), poi i brani.
    fn offset(&self) -> usize {
        usize::from(self.pending_note.is_some())
    }

    /// La voce `ix` della lista: la testa, poi le righe.
    fn item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let element = match ix.checked_sub(1) {
            None => self.found.clone().map(|found| self.header(&found, cx).into_any_element()),
            Some(row) => {
                // La colonna dell'album c'è solo con la barra laterale intera (linee guida: da 1100 dp).
                let wide = f32::from(window.viewport_size().width) >= BP_RAIL;
                self.row(row, wide, cx)
            }
        };
        element.unwrap_or_else(|| div().into_any_element())
    }

    fn row(&self, ix: usize, wide: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(text) = self.pending_note.as_ref().filter(|_| ix == 0) {
            return Some(
                note(div().type_role(TYPE_BODY).text_color(col(TEXT_SECONDARY)).child(text.clone()))
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
                // Un clic non fa niente; doppio clic o Invio suonano.
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
        Some(row.into_any_element())
    }

    fn header(&self, found: &Found, cx: &mut Context<Self>) -> PageHeader {
        let art = match self.collection {
            Collection::Liked => HeaderArt::Sprite(sprite::ICON_HEART_FILL),
            Collection::Playlist(_) | Collection::Public(_) => HeaderArt::Mosaic(self.mosaic.clone()),
        };
        let kicker = if found.owner.is_some() { "Playlist pubblica" } else { "Playlist" };
        let mut title = PageHeader::new(art, kicker, found.name.clone()).meta(self.meta.clone());
        if let Some((handle, name)) = found.owner.clone() {
            title = title.link(
                "col-owner",
                name,
                cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::OpenProfile(handle.clone()))),
            );
        }
        if !found.tracks.is_empty() {
            title = title
                .action(Button::new("col-play", "Riproduci").primary().icon(sprite::ICON_PLAY).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(tracks) = this.tracks() {
                            this.core
                                .read(cx)
                                .playback(PlaybackCmd::Play { tracks: Arc::clone(tracks), start: 0 });
                        }
                    }),
                ))
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
                // Mentre il lotto scarica, al posto dell'icona c'è il conteggio.
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Il posto si segna di nuovo nel paint di questo fotogramma, se la riga in corso è visibile.
        self.spot.clear();
        let page = div().size_full().flex().flex_col().bg(col(SURFACE_CANVAS));
        let Some(found) = self.found.as_ref() else {
            if let Collection::Public(id) = self.collection {
                // La playlist di un amico arriva dalla rete: lo scheletro, o l'errore con «Riprova».
                return match &self.social.playlist {
                    Some(Load::Failed(error)) => page.child(super::album_page::failure(
                        "col-public-retry",
                        error.to_string().into(),
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let handle = this.public_handle.clone();
                            this.core.read(cx).social(SocialCmd::OpenFriendPlaylist { handle, id });
                        }),
                    )),
                    _ => page.child(PageHeader::loading(false)),
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

/// Un contenuto al centro, a un terzo dall'alto, come gli stati vuoti delle linee guida.
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
        // Un ascolto: liked e playlist sono le stesse `Arc`.
        let listened = LibraryView { recent: vec![fixtures::track(9)].into(), ..old.clone() };
        assert!(!moved(&old, &listened, Collection::Liked));
        assert!(!moved(&old, &listened, Collection::Playlist(Uuid::nil())));
        let hearted = LibraryView { liked: vec![fixtures::track(2)].into(), ..old.clone() };
        assert!(moved(&old, &hearted, Collection::Liked));
        assert!(!moved(&old, &hearted, Collection::Playlist(Uuid::nil())));
    }
}
