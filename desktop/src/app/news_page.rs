//! «Novità»: le uscite degli artisti che segui, come la pagina del client Iced (`client/src/ui/pages/news.rs`).
//! Il titolo e una riga che dice quante uscite aspettano; gli artisti seguiti in uno scaffale; poi *Da
//! ascoltare* (le uscite arrivate dopo l'ultima visita, con «Nuovo») e *Uscite precedenti*, righe con la
//! copertina, il tipo, l'artista e la data. Senza artisti seguiti la pagina lo spiega e propone quelli del
//! catalogo; con artisti ma senza uscite lo dice, con «Vai alla Home».
//!
//! Il catalogo non sa chi segue chi: l'elenco è il feed del dominio della libreria (`LibraryView::feed.news`),
//! ricavato dai brani recenti della Home. Uscendo dalla pagina `Shell` manda `LibraryCmd::MarkNewsSeen`,
//! come il client Iced: ciò che c'era si considera visto.

use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, ListState, ParentElement, Render,
    SharedString, Styled, Subscription, Window, canvas, div, list,
};
use psytify_core::api::Track;
use psytify_core::catalog::Load;
use psytify_core::command::{CatalogCmd, PlaybackCmd, Resource};
use psytify_core::feed::{self, ArtistCard, Release};
use psytify_core::snapshot::{LibraryView, PlaybackView};
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_ERROR};
use psytify_design::tokens::size::GRID_GAP;
use psytify_design::tokens::space::SPACE_5;
use psytify_design::tokens::type_::TYPE_BODY;
use uuid::Uuid;

use super::home_page::TILE;
use super::page_list::{new_list, remeasure_header, set_rows, to_top};
use super::track_list::now_playing;
use super::{Page, PageEvent};
use crate::components::badge::BadgeKind;
use crate::components::button::Button;
use crate::components::icon_button::IconButton;
use crate::components::indicators::{PlayState, Skeleton};
use crate::components::list_row::{ListRow, RowArt};
use crate::components::overlay::EmptyState;
use crate::components::section::{PageTitle, note, section_heading};
use crate::components::shelf::{Shelf, ShelfPage, Wheel, fit};
use crate::components::tile::AlbumTile;
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{TypeStyle, col, dp};

/// Righe dello scheletro mentre la Home arriva.
const SKELETON: usize = 6;

/// Che cosa mostra la pagina, oltre al titolo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Skeleton,
    Failed,
    /// Nessun artista seguito: lo si spiega.
    NoFollow,
    /// Lo scaffale degli artisti (seguiti, o quelli da cui iniziare).
    Artists,
    /// Artisti seguiti, ma nessuna uscita.
    NoNews,
    Heading(Group),
    Release(Group, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Fresh,
    Earlier,
}

/// Le righe della pagina. Pura: `loaded` è lo stato dei brani recenti della Home.
fn plan(
    loaded: Option<bool>,
    following: usize,
    suggestions: usize,
    fresh: usize,
    earlier: usize,
) -> Vec<Row> {
    match loaded {
        None => return (0..SKELETON).map(|_| Row::Skeleton).collect(),
        Some(false) => return vec![Row::Failed],
        Some(true) => {}
    }
    if following == 0 {
        let mut rows = vec![Row::NoFollow];
        if suggestions > 0 {
            rows.push(Row::Artists);
        }
        return rows;
    }
    let mut rows = vec![Row::Artists];
    if fresh + earlier == 0 {
        rows.push(Row::NoNews);
        return rows;
    }
    for (group, count) in [(Group::Fresh, fresh), (Group::Earlier, earlier)] {
        if count > 0 {
            rows.push(Row::Heading(group));
            rows.extend((0..count).map(|ix| Row::Release(group, ix)));
        }
    }
    rows
}

/// «1 uscita da ascoltare», come la riga del client Iced.
fn subline(following: usize, unread: usize) -> String {
    match (following, unread) {
        (0, _) => "Segui gli artisti per ritrovare qui le loro uscite".to_owned(),
        (_, 0) => "Sei in pari con le uscite degli artisti che segui".to_owned(),
        (_, 1) => "1 uscita da ascoltare".to_owned(),
        (_, n) => format!("{n} uscite da ascoltare"),
    }
}

/// Un'uscita pronta da mostrare.
struct Item {
    release: Release,
    title: SharedString,
    detail: SharedString,
    cover: Option<SharedString>,
}

impl Item {
    fn of(release: &Release) -> Self {
        let when = feed::date_label(&release.created_at).unwrap_or_default();
        Self {
            title: release.title.clone().into(),
            detail: format!(
                "{} · {} · {when} · {}",
                release.kind_label(),
                release.artist.name,
                format::tracks(release.tracks.len().max(1))
            )
            .into(),
            cover: release.cover_url.clone().map(SharedString::from),
            release: release.clone(),
        }
    }
}

pub struct NewsPage {
    core: Entity<CoreModel>,
    loaded: Option<bool>,
    library: Arc<LibraryView>,
    subline: SharedString,
    artists: Vec<ArtistCard>,
    fresh: Vec<Item>,
    earlier: Vec<Item>,
    now: Option<(Uuid, PlayState)>,
    now_album: Option<Uuid>,
    start: usize,
    wheel: Wheel,
    columns: usize,
    rows: Vec<Row>,
    list: ListState,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for NewsPage {}

fn loaded<T>(load: &Load<T>) -> Option<bool> {
    match load {
        Load::Ready(_) => Some(true),
        Load::Failed(_) => Some(false),
        Load::Idle | Load::Loading => None,
    }
}

fn now_album(view: &PlaybackView) -> Option<Uuid> {
    view.queue.current().and_then(|track| track.album.as_ref().map(|album| album.id))
}

impl NewsPage {
    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let changes =
            cx.subscribe(&core, |this, core, changed: &Changed, cx| this.refresh(&core, *changed, cx));
        let snapshot = core.read(cx).snapshot();
        let mut page = Self {
            loaded: loaded(&snapshot.catalog.home.recent),
            library: Arc::clone(&snapshot.library),
            subline: SharedString::default(),
            artists: Vec::new(),
            fresh: Vec::new(),
            earlier: Vec::new(),
            now: now_playing(&snapshot.playback),
            now_album: now_album(&snapshot.playback),
            start: 0,
            wheel: Wheel::default(),
            columns: 4,
            rows: Vec::new(),
            list: new_list(0),
            core,
            _changes: changes,
        };
        page.rebuild();
        page
    }

    /// La pagina è in vista: le uscite si ricavano dalla Home, che il core carica se serve. Ciò che è nuovo
    /// si misura da qui (rifatto adesso, con la data dell'ultima visita).
    pub fn show(&mut self, cx: &mut Context<Self>) {
        self.core.read(cx).catalog(CatalogCmd::Ensure(Resource::Home));
        self.rebuild();
        to_top(&self.list);
        cx.notify();
    }

    fn refresh(&mut self, core: &Entity<CoreModel>, changed: Changed, cx: &mut Context<Self>) {
        let snapshot = core.read(cx).snapshot();
        let mut rebuild = false;
        let mut dirty = false;
        if changed.catalog {
            let state = loaded(&snapshot.catalog.home.recent);
            if state != self.loaded {
                self.loaded = state;
                rebuild = true;
            }
        }
        if changed.library && !Arc::ptr_eq(&snapshot.library, &self.library) {
            let before = &self.library;
            let library = &snapshot.library;
            // Il segno di «visto» si sposta uscendo: le righe restano com'erano finché si è sulla pagina.
            rebuild |= !Arc::ptr_eq(&library.feed, &before.feed)
                || !library.followed.same(&before.followed)
                || library.unread_news != before.unread_news;
            self.library = Arc::clone(library);
        }
        if changed.playback {
            let now = now_playing(&snapshot.playback);
            let album = now_album(&snapshot.playback);
            if now != self.now || album != self.now_album {
                self.now = now;
                self.now_album = album;
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

    fn rebuild(&mut self) {
        let library = Arc::clone(&self.library);
        self.subline = subline(library.followed.len(), library.unread_news).into();
        self.artists = if library.followed.is_empty() {
            library.feed.artists.clone()
        } else {
            library
                .followed
                .iter()
                .map(|a| ArtistCard {
                    artist: psytify_core::api::models::ArtistRef { id: a.id, name: a.name.clone() },
                    cover_url: a.cover_url.clone(),
                    tracks: 0,
                })
                .collect()
        };
        let seen = library.news_seen.as_str();
        let (fresh, earlier): (Vec<&Release>, Vec<&Release>) =
            library.feed.news.iter().partition(|r| r.created_at.as_str() > seen);
        self.fresh = fresh.into_iter().map(Item::of).collect();
        self.earlier = earlier.into_iter().map(Item::of).collect();
        self.rows = plan(
            self.loaded,
            library.followed.len(),
            self.artists.len(),
            self.fresh.len(),
            self.earlier.len(),
        );
        set_rows(&self.list, self.rows.len());
        remeasure_header(&self.list);
    }

    fn set_columns(&mut self, columns: usize, cx: &mut Context<Self>) {
        if columns != self.columns {
            self.columns = columns;
            self.wheel.reset();
            cx.notify();
        }
    }

    fn page(&self) -> ShelfPage {
        ShelfPage::new(self.start, self.columns, self.artists.len())
    }

    fn move_to(&mut self, start: usize, cx: &mut Context<Self>) {
        if start != self.start {
            self.start = start;
            cx.notify();
        }
    }

    fn release_playing(&self, release: &Release) -> Option<PlayState> {
        let (id, state) = self.now?;
        let inside = if release.is_album {
            self.now_album == Some(release.id)
        } else {
            release.tracks.iter().any(|t| t.id == id)
        };
        inside.then_some(state)
    }

    fn play(&self, release: &Release, cx: &gpui::App) {
        let core = self.core.read(cx);
        if self.release_playing(release).is_some() {
            core.playback(PlaybackCmd::Toggle);
        } else if let (false, Some(single)) = (release.is_album, release.tracks.first()) {
            let tracks: Arc<[Track]> = Arc::from([single.clone()]);
            core.playback(PlaybackCmd::Play { tracks, start: 0 });
        } else {
            core.playback(PlaybackCmd::PlayAlbum(release.id));
        }
    }

    fn item(&mut self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let element = match ix.checked_sub(1) {
            None => Some(PageTitle::screen("Novità").meta(self.subline.clone()).into_any_element()),
            Some(row) => self.row(row, cx),
        };
        element.unwrap_or_else(|| div().into_any_element())
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(match *self.rows.get(ix)? {
            Row::Skeleton => note(
                div()
                    .flex()
                    .items_center()
                    .gap(dp(SPACE_5))
                    .child(Skeleton::new(32.0, 32.0))
                    .child(Skeleton::new(240.0, 12.0)),
            )
            .into_any_element(),
            Row::Failed => note(
                div()
                    .flex()
                    .items_center()
                    .gap(dp(SPACE_5))
                    .child(
                        div()
                            .type_role(TYPE_BODY)
                            .text_color(col(TEXT_ERROR))
                            .child("Le novità non sono arrivate."),
                    )
                    .child(Button::new("news-retry", "Riprova").secondary().on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.core.read(cx).catalog(CatalogCmd::Retry(Resource::Home));
                        },
                    ))),
            )
            .into_any_element(),
            Row::NoFollow => div()
                .flex()
                .justify_center()
                .py(dp(48.0))
                .child(EmptyState::new(
                    sprite::NASTRO_IDLE,
                    "Non segui ancora nessuno",
                    "Quando segui un artista, le sue nuove uscite arrivano qui e la Home le mette in primo piano.",
                ))
                .into_any_element(),
            Row::NoNews => div()
                .flex()
                .justify_center()
                .py(dp(48.0))
                .child(
                    EmptyState::crate_(
                        "Ancora nessuna uscita",
                        "Gli artisti che segui non hanno brani recenti nel catalogo. Appena ne esce uno, lo trovi qui.",
                    )
                    .action(Button::new("news-home", "Vai alla Home").secondary().on_click(cx.listener(
                        |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Home)),
                    ))),
                )
                .into_any_element(),
            Row::Artists => self.artists_shelf(cx),
            Row::Heading(group) => section_heading(
                match group {
                    Group::Fresh => "Da ascoltare",
                    Group::Earlier => "Uscite precedenti",
                }
                .into(),
                SharedString::default(),
            )
            .into_any_element(),
            Row::Release(group, position) => self.release_row(group, position, cx)?,
        })
    }

    fn artists_shelf(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = self.page();
        let tiles =
            page.range().filter_map(|position| {
                let card = self.artists.get(position)?;
                let id = card.artist.id;
                Some(
                    AlbumTile::new(("news-artist", position), card.artist.name.clone(), "Artista")
                        .side(TILE)
                        .round()
                        .cover_url(card.cover_url.clone().map(SharedString::from))
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(PageEvent::Open(Page::Artist(id)))
                        }))
                        .into_any_element(),
                )
            });
        let title = if self.library.followed.is_empty() { "Da cui iniziare" } else { "Artisti che segui" };
        let (paged, wheeled) = (cx.entity().downgrade(), cx.entity().downgrade());
        Shelf::new("news-artists", title, page)
            .tiles(tiles.collect::<Vec<_>>())
            .on_page(move |start, _, cx| {
                paged.update(cx, |this, cx| this.move_to(start, cx)).ok();
            })
            .on_wheel(move |dx, _, cx| {
                wheeled
                    .update(cx, |this, cx| {
                        let steps = this.wheel.step(dx, TILE + GRID_GAP);
                        if steps != 0 {
                            let start = this.page().moved(steps);
                            this.move_to(start, cx);
                        }
                    })
                    .ok();
            })
            .into_any_element()
    }

    fn release_row(&self, group: Group, position: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let items = match group {
            Group::Fresh => &self.fresh,
            Group::Earlier => &self.earlier,
        };
        let item = items.get(position)?;
        let (open, play) = (item.release.clone(), item.release.clone());
        let mut row = ListRow::new(
            (SharedString::from(format!("news-{group:?}")), position),
            RowArt::Cover(item.cover.clone()),
            item.title.clone(),
        )
        .detail(item.detail.clone())
        .hover_action(
            IconButton::new(
                (SharedString::from(format!("news-play-{group:?}")), position),
                sprite::ICON_PLAY,
            )
            .tooltip("Riproduci", None)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.play(&play, cx))),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            if open.is_album {
                cx.emit(PageEvent::Open(Page::Album(open.id)));
            } else {
                this.play(&open, cx);
            }
        }));
        if group == Group::Fresh {
            row = row.badge("Nuovo", BadgeKind::Count);
        }
        Some(row.into_any_element())
    }
}

impl Render for NewsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let current = self.columns;
        let measure = canvas(
            move |bounds, _, cx| {
                let columns = fit(f32::from(bounds.size.width), TILE, GRID_GAP);
                if columns != current {
                    cx.defer(move |cx| {
                        entity.update(cx, |this, cx| this.set_columns(columns, cx)).ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        div().relative().size_full().bg(col(SURFACE_CANVAS)).child(measure).child(
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

    #[test]
    fn the_page_follows_the_iced_states() {
        assert_eq!(plan(None, 1, 1, 1, 1).len(), SKELETON);
        assert_eq!(plan(Some(false), 1, 1, 1, 1), vec![Row::Failed]);
        assert_eq!(plan(Some(true), 0, 3, 0, 0), vec![Row::NoFollow, Row::Artists]);
        assert_eq!(plan(Some(true), 0, 0, 0, 0), vec![Row::NoFollow]);
        assert_eq!(plan(Some(true), 2, 2, 0, 0), vec![Row::Artists, Row::NoNews]);
        assert_eq!(
            plan(Some(true), 2, 2, 1, 2),
            vec![
                Row::Artists,
                Row::Heading(Group::Fresh),
                Row::Release(Group::Fresh, 0),
                Row::Heading(Group::Earlier),
                Row::Release(Group::Earlier, 0),
                Row::Release(Group::Earlier, 1),
            ]
        );
        assert_eq!(plan(Some(true), 2, 2, 0, 1)[1], Row::Heading(Group::Earlier));
    }

    #[test]
    fn the_subline_counts_what_is_waiting() {
        assert_eq!(subline(0, 3), "Segui gli artisti per ritrovare qui le loro uscite");
        assert_eq!(subline(2, 0), "Sei in pari con le uscite degli artisti che segui");
        assert_eq!(subline(2, 1), "1 uscita da ascoltare");
        assert_eq!(subline(2, 4), "4 uscite da ascoltare");
    }
}
