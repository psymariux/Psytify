//! La pagina «Libreria»: ciò che è tuo, in una lista sola, come la Libreria del telefono nel client Iced
//! (`client/src/ui/phone/library.rs`; sul desktop Iced la stessa lista sta nella barra laterale). In cima
//! il titolo con «Nuova playlist»; poi «Brani che ti piacciono» e le playlist (un clic le apre; Rinomina ed
//! Elimina al passaggio del mouse, con il dialogo di `Shell`), gli album salvati e gli artisti seguiti.
//!
//! Un clic su un album o su un artista apre la sua pagina (`album_page`, `artist_page`).
//! Tutto viene da `LibraryView`; la pagina non manda comandi da sé, chiede a `Shell` di aprire una pagina o
//! un dialogo ([`PageEvent`]). I testi delle righe si rifanno solo quando una delle quattro liste cambia
//! (un ascolto cambia la libreria ma non queste).

use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, UniformListScrollHandle, Window, div, uniform_list,
};
use psytify_core::snapshot::LibraryView;
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_SECONDARY};
use psytify_design::tokens::space::SPACE_3;
use psytify_design::tokens::type_::TYPE_BODY;

use super::playlist_dialog::DialogRequest;
use super::{Page, PageEvent};
use crate::components::button::Button;
use crate::components::icon_button::IconButton;
use crate::components::list_row::{ListRow, RowArt};
use crate::components::section::{PageTitle, note, section_heading};
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{TypeStyle, col, dp};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Playlists,
    Albums,
    Artists,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Self::Playlists => "Playlist",
            Self::Albums => "Album salvati",
            Self::Artists => "Artisti seguiti",
        }
    }
}

/// Una riga della lista, nell'ordine in cui si disegna. Gli indici sono nelle liste del core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Heading(Section),
    Liked,
    Playlist(usize),
    /// Nessuna playlist: una frase che dice come crearne una.
    NoPlaylists,
    Album(usize),
    Artist(usize),
}

/// Quali righe mostrare. «Brani che ti piacciono» c'è sempre; album e artisti solo se ce ne sono. Pura.
fn plan(playlists: usize, albums: usize, artists: usize) -> Vec<Row> {
    let mut rows = Vec::with_capacity(playlists + albums + artists + 5);
    rows.push(Row::Heading(Section::Playlists));
    rows.push(Row::Liked);
    rows.extend((0..playlists).map(Row::Playlist));
    if playlists == 0 {
        rows.push(Row::NoPlaylists);
    }
    if albums > 0 {
        rows.push(Row::Heading(Section::Albums));
        rows.extend((0..albums).map(Row::Album));
    }
    if artists > 0 {
        rows.push(Row::Heading(Section::Artists));
        rows.extend((0..artists).map(Row::Artist));
    }
    rows
}

/// «3 playlist · 300 brani piaciuti · 2 album · 3 artisti»: le parti a zero restano fuori, le playlist no.
fn summary(playlists: usize, liked: usize, albums: usize, artists: usize) -> String {
    let mut parts = vec![format!("{playlists} playlist")];
    if liked > 0 {
        parts.push(if liked == 1 {
            "1 brano piaciuto".to_owned()
        } else {
            format!("{liked} brani piaciuti")
        });
    }
    if albums > 0 {
        parts.push(format!("{albums} album"));
    }
    if artists > 0 {
        parts.push(if artists == 1 { "1 artista".to_owned() } else { format!("{artists} artisti") });
    }
    parts.join(" · ")
}

/// I testi di una voce, pronti.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    title: SharedString,
    detail: SharedString,
    cover_url: Option<SharedString>,
}

pub struct LibraryPage {
    view: Arc<LibraryView>,
    rows: Vec<Row>,
    summary: SharedString,
    liked: SharedString,
    playlists: Vec<Entry>,
    albums: Vec<Entry>,
    artists: Vec<Entry>,
    scroll: UniformListScrollHandle,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for LibraryPage {}

impl LibraryPage {
    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let changes = cx.subscribe(&core, |this, core, changed: &Changed, cx| {
            if changed.library {
                let view = Arc::clone(&core.read(cx).snapshot().library);
                if this.take(view) {
                    cx.notify();
                }
            }
        });
        let mut page = Self {
            view: Arc::default(),
            rows: Vec::new(),
            summary: SharedString::default(),
            liked: SharedString::default(),
            playlists: Vec::new(),
            albums: Vec::new(),
            artists: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            _changes: changes,
        };
        page.rebuild(Arc::clone(&core.read(cx).snapshot().library));
        page
    }

    /// Prende la vista nuova; `true` se una delle liste mostrate è cambiata (e i testi sono rifatti).
    fn take(&mut self, view: Arc<LibraryView>) -> bool {
        let old = &self.view;
        let same = view.playlists.same(&old.playlists)
            && view.liked.same(&old.liked)
            && view.saved_albums.same(&old.saved_albums)
            && view.followed.same(&old.followed);
        if same {
            self.view = view;
            return false;
        }
        self.rebuild(view);
        true
    }

    fn rebuild(&mut self, view: Arc<LibraryView>) {
        self.rows = plan(view.playlists.len(), view.saved_albums.len(), view.followed.len());
        self.summary =
            summary(view.playlists.len(), view.liked.len(), view.saved_albums.len(), view.followed.len())
                .into();
        self.liked = format!("Playlist · {}", format::tracks(view.liked.len())).into();
        self.playlists = view
            .playlists
            .iter()
            .map(|playlist| Entry {
                title: playlist.name.clone().into(),
                detail: format!("Playlist · {}", format::tracks(playlist.tracks.len())).into(),
                // La copertina del primo brano, come la pagina della playlist.
                cover_url: playlist.tracks.first().and_then(|t| t.cover_url.clone()).map(SharedString::from),
            })
            .collect();
        self.albums = view
            .saved_albums
            .iter()
            .map(|album| Entry {
                title: album.title.clone().into(),
                detail: format!("Album · {}", album.artist.name).into(),
                cover_url: album.cover_url.clone().map(SharedString::from),
            })
            .collect();
        self.artists = view
            .followed
            .iter()
            .map(|artist| Entry {
                title: artist.name.clone().into(),
                detail: "Artista".into(),
                cover_url: artist.cover_url.clone().map(SharedString::from),
            })
            .collect();
        self.view = view;
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(match *self.rows.get(ix)? {
            Row::Heading(section) => {
                let count = match section {
                    // Come il titolo: «Brani che ti piacciono» non è una playlist che si conta.
                    Section::Playlists => self.playlists.len(),
                    Section::Albums => self.albums.len(),
                    Section::Artists => self.artists.len(),
                };
                section_heading(section.title().into(), count.to_string().into()).into_any_element()
            }
            Row::Liked => {
                ListRow::new("lib-liked", RowArt::Sprite(sprite::ICON_HEART_FILL), "Brani che ti piacciono")
                    .detail(self.liked.clone())
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Liked))))
                    .into_any_element()
            }
            Row::NoPlaylists => note(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_SECONDARY))
                    .child("Nessuna playlist, per ora: creane una con «Nuova playlist»."),
            )
            .into_any_element(),
            Row::Playlist(position) => {
                let entry = self.playlists.get(position)?;
                let playlist = self.view.playlists.get(position)?;
                let (id, name) = (playlist.id, playlist.name.clone());
                let rename = name.clone();
                ListRow::new(
                    ("lib-playlist", position),
                    RowArt::Cover(entry.cover_url.clone()),
                    entry.title.clone(),
                )
                .detail(entry.detail.clone())
                .hover_action(
                    IconButton::new(("lib-rename", position), sprite::ICON_EDIT)
                        .tooltip("Rinomina", None)
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(PageEvent::Dialog(DialogRequest::Rename { id, name: rename.clone() }))
                        })),
                )
                .hover_action(
                    IconButton::new(("lib-delete", position), sprite::ICON_TRASH)
                        .tooltip("Elimina la playlist", None)
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(PageEvent::Dialog(DialogRequest::Delete { id, name: name.clone() }))
                        })),
                )
                .on_click(
                    cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Playlist(id)))),
                )
                .into_any_element()
            }
            Row::Album(position) => {
                let entry = self.albums.get(position)?;
                let id = self.view.saved_albums.get(position)?.id;
                ListRow::new(
                    ("lib-album", position),
                    RowArt::Cover(entry.cover_url.clone()),
                    entry.title.clone(),
                )
                .detail(entry.detail.clone())
                .on_click(
                    cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Album(id)))),
                )
                .into_any_element()
            }
            Row::Artist(position) => {
                let entry = self.artists.get(position)?;
                let id = self.view.followed.get(position)?.id;
                ListRow::new(
                    ("lib-artist", position),
                    RowArt::Round(entry.cover_url.clone()),
                    entry.title.clone(),
                )
                .detail(entry.detail.clone())
                .on_click(
                    cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Artist(id)))),
                )
                .into_any_element()
            }
        })
    }
}

impl Render for LibraryPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = PageTitle::screen("Libreria").meta(self.summary.clone()).action(
            Button::new("lib-new", "Nuova playlist").secondary().icon(sprite::ICON_PLUS).on_click(
                cx.listener(|_, _: &ClickEvent, _, cx| {
                    cx.emit(PageEvent::Dialog(DialogRequest::NewPlaylist { seed: Vec::new() }))
                }),
            ),
        );
        let list = uniform_list(
            "library-list",
            self.rows.len(),
            cx.processor(|this: &mut Self, range: std::ops::Range<usize>, _window, cx| {
                range.filter_map(|ix| this.row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(col(SURFACE_CANVAS))
            .child(title)
            .child(div().flex_1().min_h_0().pt(dp(SPACE_3)).child(list))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liked_songs_and_playlists_come_first_then_albums_and_artists() {
        assert_eq!(
            plan(2, 1, 1),
            vec![
                Row::Heading(Section::Playlists),
                Row::Liked,
                Row::Playlist(0),
                Row::Playlist(1),
                Row::Heading(Section::Albums),
                Row::Album(0),
                Row::Heading(Section::Artists),
                Row::Artist(0),
            ]
        );
    }

    #[test]
    fn an_empty_library_still_has_the_liked_songs_and_says_how_to_make_a_playlist() {
        assert_eq!(plan(0, 0, 0), vec![Row::Heading(Section::Playlists), Row::Liked, Row::NoPlaylists]);
    }

    #[test]
    fn the_summary_leaves_out_what_is_empty() {
        assert_eq!(summary(0, 0, 0, 0), "0 playlist");
        assert_eq!(summary(3, 300, 2, 3), "3 playlist · 300 brani piaciuti · 2 album · 3 artisti");
        assert_eq!(summary(1, 1, 0, 1), "1 playlist · 1 brano piaciuto · 1 artista");
    }
}
