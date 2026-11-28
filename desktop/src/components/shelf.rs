//! Shelf: a section of covers in a single row (Home, «Novità»), as in `PSYTIFY_ART_DIRECTION.md`
//! (Home): the header with the title, the arrows and «Mostra tutto», then as many whole covers as fit
//! in the width. No carousel that scrolls sideways: the arrows (and the horizontal wheel,
//! or with Shift) move the row of whole covers, and «Mostra tutto» opens it in a grid (what
//! decides whoever uses the shelf). Only the visible covers are built: whoever uses the shelf asks
//! [`ShelfPage::range`] which ones and passes only those.
//!
//! The counting part is pure ([`ShelfPage`], [`Wheel`], [`fit`]), so it can be tested without a window.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, ScrollEvent,
    SharedString, Styled, Window, div, px,
};
use psytify_design::sprite;
use psytify_design::tokens::size::{GRID_GAP, TRACK_ROW_H};
use psytify_design::tokens::space::{SPACE_1, SPACE_6};

use super::button::Button;
use super::control::OnClick;
use super::icon_button::IconButton;
use super::section::section_heading_at;
use crate::theme::dp;

/// How many whole tiles `side` dp wide fit a row `width` dp wide, with the page margins (`SPACE_6` each side)
/// and `gap` between them. At least one. (Fluid grids use `components::frame::TileGrid` instead.)
pub fn fit(width: f32, side: f32, gap: f32) -> usize {
    let inner = (width - 2.0 * SPACE_6).max(side);
    // The count is on small, positive numbers: the truncation is intended.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let columns = ((inner + gap) / (side + gap)).floor() as usize;
    columns.max(1)
}

/// The shelf's window: where it starts from, how many fit, how many there are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShelfPage {
    pub start: usize,
    pub columns: usize,
    pub count: usize,
}

impl ShelfPage {
    /// The window with `start` brought back inside: the last page is always full (if there are enough
    /// covers), so widening the window leaves no gap on the right.
    pub fn new(start: usize, columns: usize, count: usize) -> Self {
        let columns = columns.max(1);
        Self { start: start.min(count.saturating_sub(columns)), columns, count }
    }

    /// Le copertine da costruire.
    pub fn range(&self) -> Range<usize> {
        self.start..(self.start + self.columns).min(self.count)
    }

    /// There are more covers than fit: the arrows and «Mostra tutto» are needed.
    pub fn overflows(&self) -> bool {
        self.count > self.columns
    }

    pub fn can_back(&self) -> bool {
        self.start > 0
    }

    pub fn can_forward(&self) -> bool {
        self.start + self.columns < self.count
    }

    /// Where the row starts after `steps` covers (negative: back), within the edges.
    pub fn moved(&self, steps: isize) -> usize {
        let last = self.count.saturating_sub(self.columns);
        self.start.saturating_add_signed(steps).min(last)
    }

    /// Una pagina indietro (la freccia sinistra).
    pub fn back(&self) -> usize {
        self.moved(-isize::try_from(self.columns).unwrap_or(isize::MAX))
    }

    /// Una pagina avanti (la freccia destra).
    pub fn forward(&self) -> usize {
        self.moved(isize::try_from(self.columns).unwrap_or(isize::MAX))
    }
}

/// La rotella in orizzontale: somma i pixel e dice quante copertine intere spostare. Un touchpad manda
/// many small moves, a few large ones for the wheel: one cover every `unit` pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wheel(f32);

impl Wheel {
    /// `dx` as GPUI gives it (positive: to the left, that is back); returns the steps to take (positive:
    /// forward) and keeps the rest for the next event.
    pub fn step(&mut self, dx: f32, unit: f32) -> isize {
        self.0 -= dx;
        let unit = unit.max(1.0);
        let steps = (self.0 / unit).trunc();
        self.0 -= steps * unit;
        // `steps` is a small whole number (few steps per event): the conversion is exact.
        #[allow(clippy::cast_possible_truncation)]
        let steps = steps as isize;
        steps
    }

    /// The shelf has changed (or the direction): the rest no longer counts.
    pub fn reset(&mut self) {
        self.0 = 0.0;
    }
}

/// A wheel movement as the shelf sees it: only the horizontal part (or the vertical one
/// with Shift, as in browsers). `None` if the event is vertical: the page scrolls as usual.
fn horizontal(event: &ScrollEvent) -> Option<f32> {
    let delta = event.delta.pixel_delta(px(TRACK_ROW_H));
    let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
    if event.modifiers.shift && dx == 0.0 {
        return (dy != 0.0).then_some(dy);
    }
    (dx.abs() > dy.abs()).then_some(dx)
}

type OnPage = Rc<dyn Fn(usize, &mut Window, &mut App)>;
type OnWheel = Rc<dyn Fn(f32, &mut Window, &mut App)>;

/// A shelf: header and a row of covers (already built, only those of [`ShelfPage::range`]).
#[derive(IntoElement)]
pub struct Shelf {
    id: ElementId,
    title: SharedString,
    meta: SharedString,
    page: ShelfPage,
    tiles: Vec<AnyElement>,
    gap: f32,
    margin: f32,
    actions: Vec<AnyElement>,
    more: Option<(SharedString, OnClick)>,
    on_page: Option<OnPage>,
    on_wheel: Option<OnWheel>,
}

impl Shelf {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>, page: ShelfPage) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            meta: SharedString::default(),
            page,
            tiles: Vec::new(),
            gap: GRID_GAP,
            margin: SPACE_6,
            actions: Vec::new(),
            more: None,
            on_page: None,
            on_wheel: None,
        }
    }

    /// The page's canvas margin and the gap of a fluid grid (`components::frame`).
    pub fn frame(mut self, margin: f32, gap: f32) -> Self {
        self.margin = margin;
        self.gap = gap;
        self
    }

    /// The count next to the title, already ready.
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = meta.into();
        self
    }

    /// Le copertine visibili, nell'ordine.
    pub fn tiles(mut self, tiles: impl IntoIterator<Item = AnyElement>) -> Self {
        self.tiles.extend(tiles);
        self
    }

    /// One more button in the header, before the arrows («Apri Novità»).
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    /// «Mostra tutto»: appears only if the covers do not all fit in one row.
    pub fn on_show_all(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.more = Some((label.into(), Rc::new(handler)));
        self
    }

    /// The arrows: the row's new starting point.
    pub fn on_page(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_page = Some(Rc::new(handler));
        self
    }

    /// La rotella in orizzontale sopra le copertine: i pixel, da passare a [`Wheel::step`].
    pub fn on_wheel(mut self, handler: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_wheel = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Shelf {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let page = self.page;
        let mut heading = section_heading_at(self.title, self.meta, self.margin)
            .child(div().flex_1())
            .children(self.actions);
        if page.overflows() {
            if let Some(on_page) = self.on_page.clone() {
                let arrow = |suffix: &str, icon: &'static str, label: &str, enabled: bool, to: usize| {
                    let on_page = Rc::clone(&on_page);
                    IconButton::new(SharedString::from(format!("{:?}-{suffix}", self.id)), icon)
                        .tooltip(label.to_owned(), None)
                        .disabled(!enabled)
                        .on_click(move |_, window, cx| on_page(to, window, cx))
                };
                heading = heading.child(
                    div()
                        .flex()
                        .flex_none()
                        .gap(dp(SPACE_1))
                        .child(arrow(
                            "back",
                            sprite::ICON_CHEVRON_LEFT,
                            "Indietro",
                            page.can_back(),
                            page.back(),
                        ))
                        .child(arrow(
                            "forward",
                            sprite::ICON_CHEVRON_RIGHT,
                            "Avanti",
                            page.can_forward(),
                            page.forward(),
                        )),
                );
            }
            if let Some((label, handler)) = self.more {
                heading = heading.child(
                    Button::new(SharedString::from(format!("{:?}-more", self.id)), label)
                        .ghost()
                        .on_click(move |event, window, cx| handler(event, window, cx)),
                );
            }
        }
        let mut row = div().flex().gap(dp(self.gap)).px(dp(self.margin)).pb(dp(SPACE_6)).children(self.tiles);
        if let Some(on_wheel) = self.on_wheel.filter(|_| page.overflows()) {
            row = row.on_scroll(move |event, window, cx| {
                if let Some(dx) = horizontal(event) {
                    // The movement belongs to the shelf: the page does not scroll vertically for this.
                    cx.stop_propagation();
                    on_wheel(dx, window, cx);
                }
            });
        }
        div().w_full().flex().flex_col().child(heading).child(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_whole_tiles_fit() {
        let width = |n: f32| 2.0 * SPACE_6 + n * 160.0 + (n - 1.0) * GRID_GAP;
        assert_eq!(fit(width(4.0), 160.0, GRID_GAP), 4);
        assert_eq!(fit(width(4.0) - 1.0, 160.0, GRID_GAP), 3);
        assert_eq!(fit(10.0, 160.0, GRID_GAP), 1);
    }

    #[test]
    fn the_shelf_shows_a_window_of_whole_pages() {
        let page = ShelfPage::new(0, 4, 10);
        assert_eq!(page.range(), 0..4);
        assert!(page.overflows() && !page.can_back() && page.can_forward());
        assert_eq!(page.forward(), 4);
        let page = ShelfPage::new(page.forward(), 4, 10);
        assert_eq!(page.forward(), 6, "the last page is full, not two tiles and a hole");
        let last = ShelfPage::new(6, 4, 10);
        assert_eq!(last.range(), 6..10);
        assert!(!last.can_forward());
        assert_eq!(last.back(), 2);
        assert_eq!(ShelfPage::new(2, 4, 10).back(), 0);
    }

    #[test]
    fn a_wider_window_pulls_the_start_back_in() {
        assert_eq!(ShelfPage::new(6, 8, 10).start, 2);
        assert_eq!(ShelfPage::new(6, 12, 10).start, 0);
        let few = ShelfPage::new(0, 6, 3);
        assert_eq!(few.range(), 0..3);
        assert!(!few.overflows());
    }

    #[test]
    fn the_wheel_moves_one_tile_per_unit_and_keeps_the_rest() {
        let mut wheel = Wheel::default();
        // Towards the right (forward): GPUI gives a negative dx.
        assert_eq!(wheel.step(-60.0, 176.0), 0);
        assert_eq!(wheel.step(-60.0, 176.0), 0);
        assert_eq!(wheel.step(-60.0, 176.0), 1, "180 px make one tile, 4 px stay");
        assert_eq!(wheel.step(-400.0, 176.0), 2);
        wheel.reset();
        assert_eq!(wheel.step(200.0, 176.0), -1);
    }

    #[test]
    fn steps_stay_inside_the_edges() {
        let page = ShelfPage::new(0, 4, 10);
        assert_eq!(page.moved(-3), 0);
        assert_eq!(page.moved(100), 6);
        assert_eq!(ShelfPage::new(5, 4, 10).moved(-2), 3);
    }
}
