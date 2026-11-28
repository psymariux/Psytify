//! The sidebar (desktop v2 §4). Three widths, one toggle (the title bar button and Ctrl+B), remembered in
//! `SaveUiPrefs` (`sidebar`: `"expanded"` or `"rail"`, `sidebar_width` in dp):
//!
//! - expanded (default `SIDEBAR_DEFAULT_W`, the right edge drags from `SIDEBAR_MIN_W` to `SIDEBAR_MAX_W`): the
//!   nav, the *Jam* card while a jam is live, then *La tua libreria* with its tabs, a filter and 48 dp rows;
//! - rail (`SIDEBAR_RAIL_W`): nav icons and the library as 40 dp covers with tooltips;
//! - under `BP_RAIL` of window it is a rail whatever the setting, and the toggle lays the expanded sidebar over
//!   the canvas with a hard shadow.
//!
//! The width eases in `MOTION_PANEL`. The sidebar is drawn by `Shell` in its cached `Chrome` view; what it
//! shows is in `ChromeKey::Sidebar` (with `rev`, bumped whenever the library rows or the jam card change).

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, Entity, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, Styled, UniformListScrollHandle, Window, div, uniform_list,
};
use psytify_core::snapshot::{JamPhase, JamView, LibraryView};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    LIME, PURE_WHITE, SURFACE_CHROME, SURFACE_RAISED, SURFACE_SELECTED, TEXT_PRIMARY, TEXT_SECONDARY,
    TEXT_TERTIARY,
};
use psytify_design::tokens::motion::{MOTION_FAST, STEP_FRAMES};
use psytify_design::tokens::shape::MARKER_W;
use psytify_design::tokens::size::cover::ART_ROW;
use psytify_design::tokens::size::{
    BP_RAIL, JAM_CARD_H, SIDEBAR_DEFAULT_W, SIDEBAR_MAX_W, SIDEBAR_MIN_W, SIDEBAR_RAIL_W, TRACK_ROW_H,
};
use psytify_design::tokens::space::{SPACE_2, SPACE_3};
use psytify_design::tokens::type_::{TYPE_ARTIST, TYPE_META, TYPE_TRACK_TITLE};
use serde_json::{Map, Value};

use super::Shell;
use super::nav::{Nav, Page};
use crate::components::badge::{Badge, BadgeKind};
use crate::components::control::{OnClick, Preview, wire, with_click};
use crate::components::icon_button::IconButton;
use crate::components::list_row::{RowArt, row_art};
use crate::components::navigation::{NavigationItem, Tab as TabItem, Tabs};
use crate::components::search_field::SearchField;
use crate::components::separator::Separator;
use crate::components::tile::Cover;
use crate::components::tooltip::with_tooltip;
use crate::format;
use crate::motion::{follow, steps};
use crate::pixel::shape::inset_ring;
use crate::theme::{Ap, TypeStyle, col, dp, fade, to_dp};

pub const MODE_KEY: &str = "sidebar";
pub const WIDTH_KEY: &str = "sidebar_width";

/// Avatars shown on the Jam card; the rest is a count.
const JAM_AVATARS: usize = 4;
const JAM_AVATAR: f32 = 24.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Expanded,
    Rail,
}

/// The saved choice: the mode and the expanded width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prefs {
    pub mode: Mode,
    pub width: f32,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { mode: Mode::Expanded, width: SIDEBAR_DEFAULT_W }
    }
}

impl Prefs {
    /// The saved preference; unknown values fall back to the defaults.
    pub fn read(ui: &Map<String, Value>) -> Self {
        let mode = match ui.get(MODE_KEY).and_then(Value::as_str) {
            Some("rail") => Mode::Rail,
            _ => Mode::Expanded,
        };
        // A width in dp: a few hundred, `f32` is exact.
        #[allow(clippy::cast_possible_truncation)]
        let width =
            ui.get(WIDTH_KEY).and_then(Value::as_f64).map_or(SIDEBAR_DEFAULT_W, |w| clamp_width(w as f32));
        Self { mode, width }
    }

    pub fn prefs(self) -> Map<String, Value> {
        let mut prefs = Map::new();
        let mode = match self.mode {
            Mode::Expanded => "expanded",
            Mode::Rail => "rail",
        };
        prefs.insert(MODE_KEY.to_owned(), Value::String(mode.to_owned()));
        if let Some(width) = serde_json::Number::from_f64(f64::from(self.width.round())) {
            prefs.insert(WIDTH_KEY.to_owned(), Value::Number(width));
        }
        prefs
    }
}

/// A dragged width, on whole dp between the limits.
pub fn clamp_width(width: f32) -> f32 {
    if width.is_finite() { width.round().clamp(SIDEBAR_MIN_W, SIDEBAR_MAX_W) } else { SIDEBAR_DEFAULT_W }
}

/// How the sidebar sits in a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The width it takes in the row, beside the canvas.
    pub flow: f32,
    /// It shows the rail (icons and covers).
    pub rail: bool,
    /// The expanded sidebar lies over the canvas (under `BP_RAIL`), `width` wide.
    pub overlay: Option<f32>,
    /// The right edge can be dragged.
    pub resizable: bool,
}

/// The layout for a window `window_w` dp wide.
pub fn layout(window_w: f32, prefs: Prefs, overlay: bool) -> Layout {
    if window_w < BP_RAIL {
        return Layout {
            flow: SIDEBAR_RAIL_W,
            rail: !overlay,
            overlay: overlay.then_some(prefs.width),
            resizable: false,
        };
    }
    match prefs.mode {
        Mode::Expanded => Layout { flow: prefs.width, rail: false, overlay: None, resizable: true },
        Mode::Rail => Layout { flow: SIDEBAR_RAIL_W, rail: true, overlay: None, resizable: false },
    }
}

/// A tab of *La tua libreria*.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Playlists,
    Albums,
    Artists,
}

impl Tab {
    pub const ALL: [Self; 3] = [Self::Playlists, Self::Albums, Self::Artists];

    pub fn label(self) -> &'static str {
        match self {
            Self::Playlists => "Playlist",
            Self::Albums => "Album",
            Self::Artists => "Artisti",
        }
    }

    fn ix(self) -> usize {
        match self {
            Self::Playlists => 0,
            Self::Albums => 1,
            Self::Artists => 2,
        }
    }
}

/// A library row, ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub title: SharedString,
    pub meta: SharedString,
    pub art: Art,
    pub page: Page,
    /// Title and meta in lower case, for the filter.
    key: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Art {
    Cover(Option<SharedString>),
    Round(Option<SharedString>),
    Liked,
}

impl Art {
    fn row(&self) -> RowArt {
        match self {
            Self::Cover(url) => RowArt::Cover(url.clone()),
            Self::Round(url) => RowArt::Round(url.clone()),
            Self::Liked => RowArt::Sprite(sprite::ICON_HEART_FILL),
        }
    }
}

fn entry(title: impl Into<SharedString>, meta: String, art: Art, page: Page) -> Entry {
    let title = title.into();
    let key = format!("{title} {meta}").to_lowercase();
    Entry { title, meta: meta.into(), art, page, key }
}

/// The rows of each tab, in order; "Brani che ti piacciono" is pinned first among the playlists. Pure.
pub fn entries(view: &LibraryView) -> [Vec<Entry>; 3] {
    let mut playlists = Vec::with_capacity(view.playlists.len() + 1);
    playlists.push(entry(
        "Brani che ti piacciono",
        format!("Playlist · {}", format::tracks(view.liked.len())),
        Art::Liked,
        Page::Liked,
    ));
    playlists.extend(view.playlists.iter().map(|playlist| {
        entry(
            playlist.name.clone(),
            format!("Playlist · {}", format::tracks(playlist.tracks.len())),
            Art::Cover(playlist.tracks.first().and_then(|t| t.cover_url.clone()).map(SharedString::from)),
            Page::Playlist(playlist.id),
        )
    }));
    let albums = view
        .saved_albums
        .iter()
        .map(|album| {
            entry(
                album.title.clone(),
                format!("Album · {}", album.artist.name),
                Art::Cover(album.cover_url.clone().map(SharedString::from)),
                Page::Album(album.id),
            )
        })
        .collect();
    let artists = view
        .followed
        .iter()
        .map(|artist| {
            entry(
                artist.name.clone(),
                "Artista".to_owned(),
                Art::Round(artist.cover_url.clone().map(SharedString::from)),
                Page::Artist(artist.id),
            )
        })
        .collect();
    [playlists, albums, artists]
}

/// The positions of `entries` that match `filter` (case-insensitive, on the title and the meta line).
pub fn filtered(entries: &[Entry], filter: &str) -> Vec<usize> {
    let needle = filter.trim().to_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| needle.is_empty() || entry.key.contains(&needle))
        .map(|(ix, _)| ix)
        .collect()
}

/// The Jam card while a jam is live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JamCard {
    pub members: Vec<SharedString>,
    pub meta: SharedString,
}

/// The card for a jam view: only while the room is open.
pub fn jam_card(jam: &JamView) -> Option<JamCard> {
    if jam.phase != JamPhase::Open {
        return None;
    }
    let members: Vec<SharedString> = jam.members.iter().map(|m| SharedString::from(m.name.clone())).collect();
    let meta = match members.len() {
        1 => "Solo tu, per ora".to_owned(),
        n => format!("{n} persone"),
    };
    Some(JamCard { members, meta: meta.into() })
}

/// The sidebar state `Shell` keeps.
pub struct Sidebar {
    pub prefs: Prefs,
    pub tab: Tab,
    /// The expanded sidebar lies over the canvas (window under `BP_RAIL`).
    pub overlay: bool,
    pub filter: Entity<SearchField>,
    filter_text: String,
    entries: [Arc<[Entry]>; 3],
    shown: Arc<[usize]>,
    jam: Option<JamCard>,
    library: Arc<LibraryView>,
    /// A drag of the right edge: the pointer x and the width when it started, in dp.
    pub drag: Option<(f32, f32)>,
    /// The width drawn now, easing to the layout's.
    width: Option<crate::motion::Tween>,
    scroll: UniformListScrollHandle,
    /// Bumped whenever the rows or the jam card change (for `ChromeKey::Sidebar`).
    pub rev: u64,
}

impl Sidebar {
    pub fn new(prefs: Prefs, filter: Entity<SearchField>, library: Arc<LibraryView>, jam: &JamView) -> Self {
        let mut sidebar = Self {
            prefs,
            tab: Tab::default(),
            overlay: false,
            filter,
            filter_text: String::new(),
            entries: Default::default(),
            shown: Arc::from([]),
            jam: jam_card(jam),
            library: Arc::default(),
            drag: None,
            width: None,
            scroll: UniformListScrollHandle::new(),
            rev: 0,
        };
        sidebar.rebuild(library);
        sidebar
    }

    fn rebuild(&mut self, view: Arc<LibraryView>) {
        self.entries = entries(&view).map(Arc::from);
        self.library = view;
        self.refilter();
    }

    fn refilter(&mut self) {
        self.shown = filtered(&self.entries[self.tab.ix()], &self.filter_text).into();
        self.rev += 1;
    }

    /// A new library view; `true` if the rows changed.
    pub fn take_library(&mut self, view: Arc<LibraryView>) -> bool {
        let old = &self.library;
        let same = view.playlists.same(&old.playlists)
            && view.liked.same(&old.liked)
            && view.saved_albums.same(&old.saved_albums)
            && view.followed.same(&old.followed);
        if same {
            self.library = view;
            return false;
        }
        self.rebuild(view);
        true
    }

    /// A new jam view; `true` if the card changed.
    pub fn take_jam(&mut self, jam: &JamView) -> bool {
        let card = jam_card(jam);
        if card == self.jam {
            return false;
        }
        self.jam = card;
        self.rev += 1;
        true
    }

    pub fn set_tab(&mut self, tab: Tab) -> bool {
        if tab == self.tab {
            return false;
        }
        self.tab = tab;
        self.refilter();
        true
    }

    pub fn set_filter(&mut self, text: &str) -> bool {
        if text == self.filter_text {
            return false;
        }
        text.clone_into(&mut self.filter_text);
        self.refilter();
        true
    }

    pub fn layout(&self, window_w: f32) -> Layout {
        layout(window_w, self.prefs, self.overlay)
    }

    /// The width drawn now for a layout: it eases in `ms` to `flow`, at once while dragging. Second value:
    /// still moving.
    pub fn width(&mut self, flow: f32, ms: u32) -> (f32, bool) {
        let now = std::time::Instant::now();
        let ms = if self.drag.is_some() { 0 } else { ms };
        let tween = self.width.get_or_insert_with(|| crate::motion::Tween::at(flow));
        tween.set(flow, ms, now);
        (tween.value(now), tween.running(now))
    }
}

/// A library row in the sidebar: 48 dp, 40 dp art, name over the meta line; on the rail the art alone with a
/// tooltip. The hover tone eases in; the lime marker of the open page grows from the centre like the nav.
#[derive(IntoElement)]
pub struct LibraryItem {
    id: ElementId,
    art: RowArt,
    title: SharedString,
    meta: SharedString,
    selected: bool,
    rail: bool,
    preview: Preview,
    on_click: Option<OnClick>,
}

impl LibraryItem {
    pub fn new(id: impl Into<ElementId>, art: RowArt, title: SharedString, meta: SharedString) -> Self {
        Self {
            id: id.into(),
            art,
            title,
            meta,
            selected: false,
            rail: false,
            preview: Preview::Live,
            on_click: None,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn rail(mut self, rail: bool) -> Self {
        self.rail = rail;
        self
    }

    pub fn preview(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }
}

impl RenderOnce for LibraryItem {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let wired = wire(&self.id, false, self.preview, window, cx);
        let v = wired.visual;
        let fill = if v.pressed {
            Some(col(SURFACE_SELECTED))
        } else if v.hover_t > 0.0 {
            Some(fade(SURFACE_RAISED, v.hover_t))
        } else {
            None
        };
        let marker_id = ElementId::NamedChild(Arc::new(self.id.clone()), "marker".into());
        let grow = follow(marker_id, if self.selected { 1.0 } else { 0.0 }, MOTION_FAST, window, cx);
        let marker_frames = steps(grow, STEP_FRAMES);
        let mut row = wired
            .element
            .relative()
            .h(dp(TRACK_ROW_H))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .gap(dp(SPACE_3))
            .px(dp(SPACE_3))
            .child(row_art(self.art, ART_ROW));
        if self.rail {
            row = row.justify_center();
        } else {
            row = row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .type_role(TYPE_TRACK_TITLE)
                            .text_color(col(TEXT_PRIMARY))
                            .truncate()
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .type_role(TYPE_ARTIST)
                            .text_color(col(TEXT_SECONDARY))
                            .truncate()
                            .child(self.meta.clone()),
                    ),
            );
        }
        if let Some(color) = fill {
            row = row.bg(color);
        }
        if marker_frames > 0 {
            let full = (TRACK_ROW_H - 2.0 * to_dp(ap.one())).max(0.0);
            let h = full * marker_frames as f32 / STEP_FRAMES as f32;
            row = row.child(
                div()
                    .absolute()
                    .left(dp(0.0))
                    .top(dp((TRACK_ROW_H - h) / 2.0))
                    .h(dp(h))
                    .w(ap.n(MARKER_W))
                    .bg(col(LIME)),
            );
        }
        row = row.child(inset_ring(col(PURE_WHITE), wired.focus.clone(), v.force_ring));
        let row = with_click(row, self.on_click);
        if self.rail { with_tooltip(row, format!("{} · {}", self.title, self.meta), None) } else { row }
    }
}

/// The 56 dp Jam card: the `LIVE` badge, who is in, and the members' avatars. Opens the Jam.
pub fn jam_card_element(card: &JamCard, on_click: OnClick, window: &mut Window, cx: &mut App) -> AnyElement {
    let wired = wire(&ElementId::from("sidebar-jam"), false, Preview::Live, window, cx);
    let v = wired.visual;
    let fill = if v.pressed {
        col(SURFACE_SELECTED)
    } else {
        crate::theme::mix(SURFACE_RAISED, SURFACE_SELECTED, v.hover_t)
    };
    let extra = card.members.len().saturating_sub(JAM_AVATARS);
    let avatars = card.members.iter().take(JAM_AVATARS).enumerate().map(|(ix, name)| {
        with_tooltip(
            div().id(("sidebar-jam-avatar", ix)).flex_none().child(Cover::new(JAM_AVATAR).circle()),
            name.clone(),
            None,
        )
        .into_any_element()
    });
    let row = wired
        .element
        .relative()
        .h(dp(JAM_CARD_H))
        .flex_none()
        .flex()
        .items_center()
        .gap(dp(SPACE_3))
        .px(dp(SPACE_3))
        .bg(fill)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(dp(2.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(dp(SPACE_2))
                        .child(div().type_role(TYPE_TRACK_TITLE).text_color(col(TEXT_PRIMARY)).child("Jam"))
                        .child(Badge::new("LIVE", BadgeKind::Live)),
                )
                .child(
                    div()
                        .type_role(TYPE_META)
                        .text_color(col(TEXT_SECONDARY))
                        .truncate()
                        .child(card.meta.clone()),
                ),
        )
        .child(div().flex().flex_none().gap(dp(2.0)).children(avatars).when_some_extra(extra))
        .child(inset_ring(col(PURE_WHITE), wired.focus.clone(), v.force_ring));
    with_click(row, Some(on_click)).into_any_element()
}

/// "+N" after the avatars when more people are in.
trait Extra {
    fn when_some_extra(self, extra: usize) -> Self;
}

impl Extra for gpui::Div {
    fn when_some_extra(self, extra: usize) -> Self {
        if extra == 0 {
            return self;
        }
        self.child(
            div()
                .h(dp(JAM_AVATAR))
                .flex()
                .items_center()
                .type_role(TYPE_META)
                .text_color(col(TEXT_TERTIARY))
                .child(format!("+{extra}")),
        )
    }
}

impl Shell {
    /// The sidebar for its layout (`rail`: icons and covers only).
    pub(super) fn sidebar(&self, rail: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let current = self.history.current();
        let nav_current = current.nav();
        let items = Nav::ALL.iter().copied().enumerate().map(|(ix, nav)| {
            let (idle, filled) = nav.icons();
            // Animated icons say "something is going on" without a badge: unread news, a running download, a
            // jam invite.
            let (idle, filled) = match nav {
                Nav::News if self.unread_news > 0 => (sprite::ICON_BELL_RING, sprite::ICON_BELL_RING),
                Nav::Downloads if self.busy_downloads > 0 => {
                    (sprite::ICON_DOWNLOAD_BUSY, sprite::ICON_DOWNLOAD_BUSY)
                }
                Nav::Jam if self.invites > 0 => (sprite::ICON_PEOPLE_LIVE, sprite::ICON_PEOPLE_LIVE),
                _ => (idle, filled),
            };
            let mut item = NavigationItem::new(("nav", ix), nav.label(), idle).icon_selected(filled);
            let count = match nav {
                Nav::Downloads => self.busy_downloads,
                Nav::News => self.unread_news,
                Nav::Friends => self.incoming,
                Nav::Jam => self.invites,
                _ => 0,
            };
            if count > 0 {
                item = item.count(u32::try_from(count).unwrap_or(u32::MAX));
            }
            item.selected(nav_current == nav).rail(rail).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    if nav == Nav::Search {
                        this.focus_search(window, cx);
                    } else {
                        this.show(nav.page(), cx);
                    }
                },
            ))
        });
        let side = &self.sidebar;
        let mut column = div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(col(SURFACE_CHROME))
            .child(div().flex().flex_col().flex_none().gap(dp(2.0)).py(dp(SPACE_3)).children(items))
            .child(Separator::horizontal());
        if let Some(card) = side.jam.as_ref().filter(|_| !rail) {
            let open: OnClick =
                std::rc::Rc::new(cx.listener(|this, _: &ClickEvent, _, cx| this.show(Page::Jam, cx)));
            column = column
                .child(div().px(dp(SPACE_3)).pt(dp(SPACE_3)).child(jam_card_element(card, open, window, cx)));
        }
        if !rail {
            let tabs =
                Tab::ALL.iter().map(|tab| TabItem::new(("sidebar-tab", tab.ix()), tab.label())).collect();
            column = column
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .h(dp(TRACK_ROW_H))
                        .flex_none()
                        .pl(dp(SPACE_3 + SPACE_3))
                        .pr(dp(SPACE_3))
                        .child(
                            div()
                                .type_role(TYPE_META)
                                .text_color(col(TEXT_SECONDARY))
                                .child("La tua libreria"),
                        )
                        .child(
                            IconButton::new("sidebar-new-playlist", sprite::ICON_PLUS)
                                .tooltip("Nuova playlist", None)
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.open_dialog(
                                        super::playlist_dialog::DialogRequest::NewPlaylist {
                                            seed: Vec::new(),
                                        },
                                        window,
                                        cx,
                                    );
                                })),
                        ),
                )
                .child(div().px(dp(SPACE_3)).flex_none().child(Tabs::new(tabs, side.tab.ix()).on_select({
                    let shell = cx.entity().downgrade();
                    move |ix: usize, _: &mut Window, cx: &mut App| {
                        shell
                            .update(cx, |this, cx| {
                                if let Some(tab) = Tab::ALL.get(ix).copied()
                                    && this.sidebar.set_tab(tab)
                                {
                                    cx.notify();
                                }
                            })
                            .ok();
                    }
                })))
                .child(div().px(dp(SPACE_3)).py(dp(SPACE_3)).flex_none().child(side.filter.clone()));
        } else {
            column = column.child(div().h(dp(SPACE_3)).flex_none());
        }
        let entries = Arc::clone(&side.entries[side.tab.ix()]);
        let shown: Arc<[usize]> = if rail {
            // The rail shows the whole tab, unfiltered.
            (0..entries.len()).collect()
        } else {
            Arc::clone(&side.shown)
        };
        let shell = cx.entity().downgrade();
        let tab = side.tab.ix();
        let list = uniform_list(
            "sidebar-library",
            shown.len(),
            move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                range
                    .filter_map(|position| {
                        let ix = *shown.get(position)?;
                        let entry = entries.get(ix)?;
                        let page = entry.page;
                        let shell = shell.clone();
                        Some(
                            LibraryItem::new(
                                ("sidebar-lib", tab * 1_000_000 + ix),
                                entry.art.row(),
                                entry.title.clone(),
                                entry.meta.clone(),
                            )
                            .selected(page == current)
                            .rail(rail)
                            .on_click(move |_, _, cx| {
                                shell.update(cx, |shell, cx| shell.show(page, cx)).ok();
                            })
                            .into_any_element(),
                        )
                    })
                    .collect()
            },
        )
        .track_scroll(&side.scroll)
        .flex_1()
        .min_h_0();
        column.child(list).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use psytify_core::api::models::fixtures;

    use super::*;

    #[test]
    fn the_preference_round_trips_and_widths_stay_in_range() {
        let prefs = Prefs { mode: Mode::Rail, width: 300.0 };
        assert_eq!(Prefs::read(&prefs.prefs()), prefs);
        assert_eq!(Prefs::read(&Map::new()), Prefs::default());
        let mut ui = Map::new();
        ui.insert(WIDTH_KEY.into(), Value::from(1000.0));
        ui.insert(MODE_KEY.into(), Value::from("wide"));
        assert_eq!(Prefs::read(&ui), Prefs { mode: Mode::Expanded, width: SIDEBAR_MAX_W });
        assert_eq!(clamp_width(100.0), SIDEBAR_MIN_W);
        assert_eq!(clamp_width(f32::NAN), SIDEBAR_DEFAULT_W);
    }

    #[test]
    fn a_narrow_window_makes_a_rail_and_the_toggle_lays_the_sidebar_over_it() {
        let prefs = Prefs { mode: Mode::Expanded, width: 300.0 };
        assert_eq!(
            layout(1600.0, prefs, false),
            Layout { flow: 300.0, rail: false, overlay: None, resizable: true }
        );
        let rail = Prefs { mode: Mode::Rail, ..prefs };
        assert_eq!(layout(1600.0, rail, false).flow, SIDEBAR_RAIL_W);
        let narrow = layout(900.0, prefs, false);
        assert!(narrow.rail && narrow.overlay.is_none() && narrow.flow == SIDEBAR_RAIL_W);
        let over = layout(900.0, prefs, true);
        assert_eq!(over.overlay, Some(300.0));
        assert!(!over.rail, "the overlay shows the expanded sidebar");
        assert_eq!(over.flow, SIDEBAR_RAIL_W, "the canvas does not move under the overlay");
    }

    #[test]
    fn liked_songs_come_first_and_the_filter_reads_title_and_meta() {
        let view = LibraryView { liked: vec![fixtures::track(1)].into(), ..LibraryView::default() };
        let [playlists, albums, artists] = entries(&view);
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].title, "Brani che ti piacciono");
        assert_eq!(playlists[0].meta, "Playlist · 1 brano");
        assert_eq!(playlists[0].page, Page::Liked);
        assert!(albums.is_empty() && artists.is_empty());
        assert_eq!(filtered(&playlists, "  PIACCIONO "), vec![0]);
        assert_eq!(filtered(&playlists, "playlist"), vec![0], "the meta line counts");
        assert!(filtered(&playlists, "jazz").is_empty());
        assert_eq!(filtered(&playlists, ""), vec![0]);
    }

    #[test]
    fn the_jam_card_shows_only_in_an_open_room() {
        let mut jam = JamView::default();
        assert_eq!(jam_card(&jam), None);
        jam.phase = JamPhase::Connecting;
        assert_eq!(jam_card(&jam), None);
        jam.phase = JamPhase::Open;
        jam.members = Arc::from([psytify_core::jam::Member { id: uuid::Uuid::nil(), name: "ada".into() }]);
        let card = jam_card(&jam);
        assert_eq!(card.map(|c| c.meta), Some("Solo tu, per ora".into()));
    }
}
