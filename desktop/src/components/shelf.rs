//! Shelf: una sezione di copertine in una riga sola (Home, «Novità»), come da `PSYTIFY_ART_DIRECTION.md`
//! (Home): l'intestazione con il titolo, le frecce e «Mostra tutto», poi tante copertine quante ne stanno
//! intere nella larghezza. Niente carosello che scorre di lato: le frecce (e la rotella in orizzontale,
//! o con Maiuscolo) spostano la riga di copertine intere, e «Mostra tutto» la apre in una griglia (lo
//! decide chi usa lo scaffale). Si costruiscono solo le copertine visibili: chi usa lo scaffale chiede a
//! [`ShelfPage::range`] quali e passa solo quelle.
//!
//! La parte di conti è pura ([`ShelfPage`], [`Wheel`], [`fit`]), perché si possa provare senza finestra.

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
use super::section::section_heading;
use crate::theme::dp;

/// Quante copertine di lato `side` stanno intere in una riga larga `width`, con i margini della pagina
/// (`SPACE_6` per lato) e `gap` fra l'una e l'altra. Almeno una.
pub fn fit(width: f32, side: f32, gap: f32) -> usize {
    let inner = (width - 2.0 * SPACE_6).max(side);
    // Il conto è su numeri piccoli e positivi: il troncamento è voluto.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let columns = ((inner + gap) / (side + gap)).floor() as usize;
    columns.max(1)
}

/// La finestra dello scaffale: da quale copertina parte, quante ne stanno, quante sono.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShelfPage {
    pub start: usize,
    pub columns: usize,
    pub count: usize,
}

impl ShelfPage {
    /// La finestra con `start` riportato dentro: l'ultima pagina è sempre piena (se ci sono abbastanza
    /// copertine), così allargando la finestra non resta un buco a destra.
    pub fn new(start: usize, columns: usize, count: usize) -> Self {
        let columns = columns.max(1);
        Self { start: start.min(count.saturating_sub(columns)), columns, count }
    }

    /// Le copertine da costruire.
    pub fn range(&self) -> Range<usize> {
        self.start..(self.start + self.columns).min(self.count)
    }

    /// Ci sono più copertine di quante ne stanno: servono le frecce e «Mostra tutto».
    pub fn overflows(&self) -> bool {
        self.count > self.columns
    }

    pub fn can_back(&self) -> bool {
        self.start > 0
    }

    pub fn can_forward(&self) -> bool {
        self.start + self.columns < self.count
    }

    /// Dove parte la riga dopo `steps` copertine (negativo: indietro), dentro i bordi.
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
/// tanti spostamenti piccoli, una rotella pochi e grandi: una copertina ogni `unit` pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wheel(f32);

impl Wheel {
    /// `dx` come lo dà GPUI (positivo: verso sinistra, cioè indietro); torna i passi da fare (positivo:
    /// avanti) e tiene il resto per il prossimo evento.
    pub fn step(&mut self, dx: f32, unit: f32) -> isize {
        self.0 -= dx;
        let unit = unit.max(1.0);
        let steps = (self.0 / unit).trunc();
        self.0 -= steps * unit;
        // `steps` è un numero intero piccolo (pochi passi per evento): la conversione è esatta.
        #[allow(clippy::cast_possible_truncation)]
        let steps = steps as isize;
        steps
    }

    /// Lo scaffale è cambiato (o la direzione): il resto non vale più.
    pub fn reset(&mut self) {
        self.0 = 0.0;
    }
}

/// Uno spostamento della rotella come lo vede lo scaffale: solo la parte orizzontale (o quella verticale
/// con Maiuscolo, come nei browser). `None` se l'evento è verticale: la pagina scorre come sempre.
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

/// Uno scaffale: intestazione e una riga di copertine (già costruite, solo quelle di [`ShelfPage::range`]).
#[derive(IntoElement)]
pub struct Shelf {
    id: ElementId,
    title: SharedString,
    meta: SharedString,
    page: ShelfPage,
    tiles: Vec<AnyElement>,
    gap: f32,
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
            actions: Vec::new(),
            more: None,
            on_page: None,
            on_wheel: None,
        }
    }

    /// Il conteggio accanto al titolo, già pronto.
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = meta.into();
        self
    }

    /// Le copertine visibili, nell'ordine.
    pub fn tiles(mut self, tiles: impl IntoIterator<Item = AnyElement>) -> Self {
        self.tiles.extend(tiles);
        self
    }

    /// Un pulsante in più nell'intestazione, prima delle frecce («Apri Novità»).
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    /// «Mostra tutto»: compare solo se le copertine non stanno tutte in una riga.
    pub fn on_show_all(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.more = Some((label.into(), Rc::new(handler)));
        self
    }

    /// Le frecce: il nuovo punto di partenza della riga.
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
        let mut heading = section_heading(self.title, self.meta).child(div().flex_1()).children(self.actions);
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
        let mut row = div().flex().gap(dp(self.gap)).px(dp(SPACE_6)).pb(dp(SPACE_6)).children(self.tiles);
        if let Some(on_wheel) = self.on_wheel.filter(|_| page.overflows()) {
            row = row.on_scroll(move |event, window, cx| {
                if let Some(dx) = horizontal(event) {
                    // Lo spostamento è dello scaffale: la pagina non scorre in verticale per questo.
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
        // Verso destra (avanti): GPUI dà dx negativo.
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
