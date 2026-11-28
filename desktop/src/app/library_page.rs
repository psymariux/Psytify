//! «Libreria»: what is yours, in one list, like the phone Library. At the top the title with «Nuova playlist»; then «Brani che ti
//! piacciono» and the playlists (a click opens them; rename and delete on hover, through `Shell`'s dialog),
//! the saved albums and the followed artists. Each section's rows flow into columns (desktop v2 §2) and the
//! page enters with the page-enter move.
//!
//! Everything comes from `LibraryView`; the page sends no command itself, it asks `Shell` to open a page or a
//! dialog ([`PageEvent`]). Row texts are rebuilt only when one of the four lists changes.

use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, UniformListScrollHandle, Window, div, uniform_list,
};
use psytify_core::snapshot::LibraryView;
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_SECONDARY};
use psytify_design::tokens::size::TRACK_ROW_H;
use psytify_design::tokens::space::SPACE_3;
use psytify_design::tokens::type_::TYPE_BODY;

use super::page_frame::PageFrame;
use super::playlist_dialog::DialogRequest;
use super::{Page, PageEvent};
use crate::components::button::Button;
use crate::components::frame::{Frame, Line, flow_lines};
use crate::components::icon_button::IconButton;
use crate::components::list_row::{ListRow, RowArt};
use crate::components::section::{PageTitle, note_at, section_heading_at};
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

/// A list row, in drawing order. Indices are into the core's lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Heading(Section),
    Liked,
    Playlist(usize),
    /// No playlist: a line on how to make one.
    NoPlaylists,
    Album(usize),
    Artist(usize),
}

/// Which rows to show. The liked songs are always there; albums and artists only if any. Pure.
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

/// «3 playlist · 300 brani piaciuti · 2 album · 3 artisti»: zero parts stay out, playlists do not.
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

/// The section a row's run flows in (headings and notes stay across the width).
fn run(row: &Row) -> Option<Section> {
    match row {
        Row::Liked | Row::Playlist(_) => Some(Section::Playlists),
        Row::Album(_) => Some(Section::Albums),
        Row::Artist(_) => Some(Section::Artists),
        Row::Heading(_) | Row::NoPlaylists => None,
    }
}
/// The texts of an entry, ready.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    title: SharedString,
    detail: SharedString,
    cover_url: Option<SharedString>,
}

pub struct LibraryPage {
    view: Arc<LibraryView>,
    rows: Vec<Row>,
    /// `rows` flowed into the page's columns.
    lines: Vec<Line<Row>>,
    summary: SharedString,
    liked: SharedString,
    playlists: Vec<Entry>,
    albums: Vec<Entry>,
    artists: Vec<Entry>,
    scroll: UniformListScrollHandle,
    frame: PageFrame,
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
            lines: Vec::new(),
            summary: SharedString::default(),
            liked: SharedString::default(),
            playlists: Vec::new(),
            albums: Vec::new(),
            artists: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            frame: PageFrame::default(),
            _changes: changes,
        };
        page.rebuild(Arc::clone(&core.read(cx).snapshot().library));
        page
    }

    /// The page comes into view (`Shell::page_shown`).
    pub fn enter(&mut self, rise: bool, cx: &mut Context<Self>) {
        self.frame.enter(rise, cx);
        cx.notify();
    }

    fn set_frame(&mut self, frame: Frame, cx: &mut Context<Self>) {
        if frame == self.frame.frame {
            return;
        }
        let reflow = frame.flow != self.frame.frame.flow;
        self.frame.frame = frame;
        if reflow {
            self.flow();
        }
        cx.notify();
    }

    fn flow(&mut self) {
        self.lines = flow_lines(self.rows.iter().copied(), self.frame.frame.flow, run);
    }

    /// Takes the new view; `true` if a shown list changed (and the texts are rebuilt).
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
        self.flow();
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
                // The first track's cover, like the playlist page.
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

    /// List line `ix`, in its enter move: one step per section.
    fn line(&self, ix: usize, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let margin = self.frame.margin();
        let element = match self.lines.get(ix)? {
            Line::One(row) => self.row(*row, cx)?,
            Line::Cells(cells) => {
                let flow = self.frame.frame.flow;
                flow.row(margin - SPACE_3, cells.iter().filter_map(|row| self.row(*row, cx)))
                    .into_any_element()
            }
        };
        let section =
            self.lines.iter().take(ix + 1).filter(|line| matches!(line, Line::One(Row::Heading(_)))).count();
        let element = div().h(dp(TRACK_ROW_H)).w_full().child(element).into_any_element();
        Some(self.frame.wrap(element, section, window))
    }

    fn row(&self, row: Row, cx: &mut Context<Self>) -> Option<AnyElement> {
        let margin = self.frame.margin();
        Some(match row {
            Row::Heading(section) => {
                let count = match section {
                    // As the title: the liked songs are not a playlist that is counted.
                    Section::Playlists => self.playlists.len(),
                    Section::Albums => self.albums.len(),
                    Section::Artists => self.artists.len(),
                };
                section_heading_at(section.title().into(), count.to_string().into(), margin)
                    .into_any_element()
            }
            Row::Liked => {
                ListRow::new("lib-liked", RowArt::Sprite(sprite::ICON_HEART_FILL), "Brani che ti piacciono")
                    .inset(SPACE_3)
                    .detail(self.liked.clone())
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Liked))))
                    .into_any_element()
            }
            Row::NoPlaylists => note_at(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_SECONDARY))
                    .child("Nessuna playlist, per ora: creane una con «Nuova playlist»."),
                margin,
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
                .inset(SPACE_3)
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
                .inset(SPACE_3)
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
                .inset(SPACE_3)
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame.begin(window, None);
        let title = PageTitle::screen("Libreria")
            .margin(self.frame.margin())
            .meta(self.summary.clone())
            .action(Button::new("lib-new", "Nuova playlist").secondary().icon(sprite::ICON_PLUS).on_click(
                cx.listener(|_, _: &ClickEvent, _, cx| {
                    cx.emit(PageEvent::Dialog(DialogRequest::NewPlaylist { seed: Vec::new() }))
                }),
            ));
        let list = uniform_list(
            "library-list",
            self.lines.len(),
            cx.processor(|this: &mut Self, range: std::ops::Range<usize>, window, cx| {
                range.filter_map(|ix| this.line(ix, window, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full();
        let title = self.frame.wrap(title.into_any_element(), 0, window);
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(col(SURFACE_CANVAS))
            .child(self.frame.measure(cx.entity().downgrade(), Self::set_frame))
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
