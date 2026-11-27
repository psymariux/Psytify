//! La lista che si riordina: pressione lunga sulla maniglia a destra di una riga, poi si trascina.
//! La riga segue il dito, le altre si spostano per far posto, vicino ai bordi la lista scorre da
//! sé, e al rilascio parte `on_move(da, a)`.
//!
//! Le righe hanno tutte la stessa altezza (`row_height`): il posto di ognuna è un calcolo, non una
//! misura. La lista vive dentro uno `scrollable`: la posizione del dito si legge dal cursore, che lo
//! `scrollable` traduce già nello spazio del contenuto, e lo scorrimento automatico si chiede con
//! `on_scroll(dp)` (l'app lo gira allo `scrollable`). Una pressione lunga a mezzo secondo, fuori
//! dalla maniglia, resta al menu contestuale della riga.
//!
//! La pressione lunga scatta a 450 ms, prima di quella della patch di `iced_winit` (500 ms): il clic
//! destro che quella manda mentre la riga è in mano viene mangiato qui, così non si apre il menu.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay, renderer};
use iced::event::{self, Event};
use iced::{Border, Color, Element, Length, Point, Rectangle, Shadow, Size, Vector, mouse, window};

use super::recognizer::Output;
use super::tracker::Tracker;

/// Larghezza della maniglia a destra di ogni riga, dove si afferra.
pub const GRIP_WIDTH: f32 = 56.0;
/// Fascia ai bordi della parte visibile dove la lista comincia a scorrere (dp).
const EDGE: f32 = 56.0;
/// Velocità massima dello scorrimento automatico (dp/s), raggiunta oltre il bordo.
const MAX_SCROLL: f32 = 720.0;

type OnMove<'a, Message> = Box<dyn Fn(usize, usize) -> Message + 'a>;
type OnScroll<'a, Message> = Box<dyn Fn(f32) -> Message + 'a>;

pub struct Reorder<'a, Message, Theme, Renderer> {
    rows: Vec<Element<'a, Message, Theme, Renderer>>,
    row_height: f32,
    raised: Color,
    on_move: OnMove<'a, Message>,
    on_scroll: OnScroll<'a, Message>,
}

/// Le righe di una lista riordinabile, tutte alte `row_height`.
pub fn reorder<'a, Message: 'a, Theme, Renderer>(
    rows: Vec<Element<'a, Message, Theme, Renderer>>,
    row_height: f32,
    raised: Color,
    on_move: impl Fn(usize, usize) -> Message + 'a,
    on_scroll: impl Fn(f32) -> Message + 'a,
) -> Reorder<'a, Message, Theme, Renderer> {
    Reorder { rows, row_height, raised, on_move: Box::new(on_move), on_scroll: Box::new(on_scroll) }
}

#[derive(Debug, Clone, Copy)]
struct Held {
    index: usize,
    /// Dove sta il dito rispetto alla cima della riga che tiene.
    grab: f32,
    /// La y del dito nello spazio del contenuto.
    y: f32,
}

#[derive(Debug, Default)]
struct State {
    tracker: Tracker,
    armed: bool,
    held: Option<Held>,
    /// L'ultimo passo dello scorrimento automatico.
    last_scroll: Option<Instant>,
}

/// La riga su cui sta il dito (la riga sotto `y` dalla cima della lista).
pub fn row_at(y: f32, row_height: f32, count: usize) -> usize {
    if count == 0 || row_height <= 0.0 {
        return 0;
    }
    ((y / row_height).floor().max(0.0) as usize).min(count - 1)
}

/// Dove andrebbe la riga `held` se il suo centro sta a `center` dalla cima della lista.
pub fn target_for(center: f32, row_height: f32, count: usize) -> usize {
    row_at(center, row_height, count)
}

/// Di quanto va spostata la riga `index` (a riposo) mentre la riga `from` va verso `to`.
pub fn shift_of(index: usize, from: usize, to: usize, row_height: f32) -> f32 {
    if index == from {
        0.0
    } else if from < to && index > from && index <= to {
        -row_height
    } else if from > to && index >= to && index < from {
        row_height
    } else {
        0.0
    }
}

/// Lo scorrimento automatico (dp/s, + verso il basso) per un dito a `y` in una parte visibile
/// `[top, bottom]`: zero lontano dai bordi, cresce avvicinandosi e oltre.
pub fn autoscroll_speed(y: f32, top: f32, bottom: f32) -> f32 {
    if y < top + EDGE {
        -MAX_SCROLL * ((top + EDGE - y) / EDGE).clamp(0.0, 1.0)
    } else if y > bottom - EDGE {
        MAX_SCROLL * ((y - (bottom - EDGE)) / EDGE).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Reorder<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        self.rows.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.rows);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let width = limits.max().width;
        let each = layout::Limits::new(Size::ZERO, Size::new(width, self.row_height))
            .width(Length::Fill)
            .height(Length::Fixed(self.row_height));
        let nodes = self
            .rows
            .iter()
            .zip(tree.children.iter_mut())
            .enumerate()
            .map(|(i, (row, child))| {
                row.as_widget()
                    .layout(child, renderer, &each)
                    .move_to(Point::new(0.0, i as f32 * self.row_height))
            })
            .collect();
        layout::Node::with_children(Size::new(width, self.row_height * self.rows.len() as f32), nodes)
    }

    fn operate(&self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, op: &mut dyn Operation) {
        for ((row, child), layout) in self.rows.iter().zip(tree.children.iter_mut()).zip(layout.children()) {
            row.as_widget().operate(child, layout, renderer, op);
        }
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) -> event::Status {
        let bounds = layout.bounds();
        let count = self.rows.len();
        let state = tree.state.downcast_mut::<State>();

        // Il gesto vale solo se comincia sulla maniglia.
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(button))
                if button == super::tracker::TOUCH_DOWN || button == mouse::Button::Left
        ) {
            state.armed = cursor.position_in(bounds).is_some_and(|p| p.x >= bounds.width - GRIP_WIDTH);
        }
        let finger_y = |cursor: mouse::Cursor| cursor.position().map(|p| p.y - bounds.y);

        if state.armed
            && let Some(out) = state.tracker.feed(&event, bounds, cursor)
        {
            match out {
                Output::LongPress { .. } => {
                    if let Some(y) = finger_y(cursor) {
                        let index = row_at(y, self.row_height, count);
                        state.held = Some(Held { index, grab: y - index as f32 * self.row_height, y });
                        state.last_scroll = None;
                        shell.request_redraw(window::RedrawRequest::NextFrame);
                    }
                }
                Output::HeldMove { delta, .. } => {
                    if let Some(held) = state.held.as_mut() {
                        held.y = finger_y(cursor).unwrap_or(held.y + delta.y);
                        shell.request_redraw(window::RedrawRequest::NextFrame);
                    }
                }
                Output::HeldEnd { .. } => {
                    if let Some(held) = state.held.take() {
                        let to =
                            target_for(held.y - held.grab + self.row_height / 2.0, self.row_height, count);
                        if to != held.index {
                            shell.publish((self.on_move)(held.index, to));
                        }
                        shell.invalidate_layout();
                        shell.request_redraw(window::RedrawRequest::NextFrame);
                    }
                    state.armed = false;
                }
                Output::Cancel => {
                    if state.held.take().is_some() {
                        shell.request_redraw(window::RedrawRequest::NextFrame);
                    }
                    state.armed = false;
                }
                // Un trascinamento prima della pressione lunga è della pagina che scorre.
                Output::PanStart { .. } => {
                    state.tracker.cancel();
                    state.armed = false;
                }
                _ => {}
            }
        }
        if let Some(at) = state.tracker.deadline() {
            shell.request_redraw(window::RedrawRequest::At(at));
        }

        if let Some(held) = state.held.as_mut() {
            // Lo scorrimento automatico, a ogni frame mentre il dito sta vicino al bordo.
            if matches!(event, Event::Window(window::Event::RedrawRequested(_))) {
                let now = Instant::now();
                let top = viewport.y - bounds.y;
                let bottom = top + viewport.height;
                let seen = finger_y(cursor);
                if let Some(y) = seen {
                    held.y = y;
                }
                let speed = autoscroll_speed(held.y, top, bottom);
                if speed != 0.0 {
                    let dt = state.last_scroll.map_or(Duration::from_millis(16), |at| now - at);
                    state.last_scroll = Some(now);
                    let step = speed * dt.as_secs_f32().min(0.05);
                    // Col dito fuori dalla lista il cursore non si vede: la riga scorre con la lista.
                    if seen.is_none() {
                        held.y += step;
                    }
                    shell.publish((self.on_scroll)(step));
                    shell.request_redraw(window::RedrawRequest::NextFrame);
                } else {
                    state.last_scroll = None;
                }
            }
            // Mentre una riga è in mano le altre non ricevono niente (e il clic destro della patch
            // di `iced_winit` per il menu è mangiato).
            return event::Status::Captured;
        }
        // Una pressione lunga che sta per scattare sulla maniglia: il clic destro non apre il menu.
        if state.armed
            && state.tracker.is_tracking()
            && matches!(
                event,
                Event::Mouse(
                    mouse::Event::ButtonPressed(mouse::Button::Right)
                        | mouse::Event::ButtonReleased(mouse::Button::Right)
                )
            )
        {
            return event::Status::Captured;
        }

        let mut status = event::Status::Ignored;
        for ((row, child), layout) in
            self.rows.iter_mut().zip(tree.children.iter_mut()).zip(layout.children())
        {
            let result = row.as_widget_mut().on_event(
                child,
                event.clone(),
                layout,
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
            status = status.merge(result);
        }
        status
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let count = self.rows.len();
        let drawn = |i: usize, shift: f32, renderer: &mut Renderer, layout: Layout<'_>, cursor| {
            renderer.with_translation(Vector::new(0.0, shift), |renderer| {
                self.rows[i].as_widget().draw(
                    &tree.children[i],
                    renderer,
                    theme,
                    style,
                    layout,
                    cursor,
                    viewport,
                );
            });
        };
        let Some(held) = state.held else {
            for (i, layout) in layout.children().enumerate() {
                if layout.bounds().intersects(viewport) {
                    self.rows[i].as_widget().draw(
                        &tree.children[i],
                        renderer,
                        theme,
                        style,
                        layout,
                        cursor,
                        viewport,
                    );
                }
            }
            return;
        };
        let to = target_for(held.y - held.grab + self.row_height / 2.0, self.row_height, count);
        let mut held_layout = None;
        for (i, layout) in layout.children().enumerate() {
            if i == held.index {
                held_layout = Some(layout);
                continue;
            }
            drawn(
                i,
                shift_of(i, held.index, to, self.row_height),
                renderer,
                layout,
                mouse::Cursor::Unavailable,
            );
        }
        if let Some(layout) = held_layout {
            let top = (held.y - held.grab).clamp(0.0, (self.row_height * (count as f32 - 1.0)).max(0.0));
            let shift = top - held.index as f32 * self.row_height;
            let area = Rectangle { y: bounds.y + top, ..layout.bounds() };
            // La riga in mano sta sopra le altre, su uno sfondo sollevato con l'ombra dura.
            renderer.with_layer(
                Rectangle { y: area.y - 4.0, height: area.height + 8.0, ..area },
                |renderer| {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: area,
                            border: Border::default(),
                            shadow: Shadow {
                                color: Color { a: 0.6, ..Color::BLACK },
                                offset: Vector::new(0.0, 2.0),
                                blur_radius: 0.0,
                            },
                        },
                        self.raised,
                    );
                    drawn(held.index, shift, renderer, layout, mouse::Cursor::Unavailable);
                },
            );
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        if state.held.is_some() {
            return mouse::Interaction::Grabbing;
        }
        self.rows
            .iter()
            .zip(tree.children.iter())
            .zip(layout.children())
            .map(|((row, child), layout)| {
                row.as_widget().mouse_interaction(child, layout, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default()
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let children: Vec<_> = self
            .rows
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(layout.children())
            .filter_map(|((row, child), layout)| {
                row.as_widget_mut().overlay(child, layout, renderer, translation)
            })
            .collect();
        (!children.is_empty()).then(|| overlay::Group::with_children(children).overlay())
    }
}

impl<'a, Message, Theme, Renderer> From<Reorder<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(list: Reorder<'a, Message, Theme, Renderer>) -> Self {
        Element::new(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_found_by_height_and_clamped() {
        assert_eq!(row_at(0.0, 56.0, 5), 0);
        assert_eq!(row_at(55.9, 56.0, 5), 0);
        assert_eq!(row_at(56.0, 56.0, 5), 1);
        assert_eq!(row_at(10_000.0, 56.0, 5), 4);
        assert_eq!(row_at(-30.0, 56.0, 5), 0);
        assert_eq!(row_at(10.0, 56.0, 0), 0);
    }

    #[test]
    fn dragging_down_pushes_the_rows_in_between_up() {
        // La riga 1 va verso la 3: le righe 2 e 3 salgono.
        let shifts: Vec<f32> = (0..5).map(|i| shift_of(i, 1, 3, 56.0)).collect();
        assert_eq!(shifts, vec![0.0, 0.0, -56.0, -56.0, 0.0]);
    }

    #[test]
    fn dragging_up_pushes_the_rows_in_between_down() {
        let shifts: Vec<f32> = (0..5).map(|i| shift_of(i, 3, 1, 56.0)).collect();
        assert_eq!(shifts, vec![0.0, 56.0, 56.0, 0.0, 0.0]);
    }

    #[test]
    fn a_row_dropped_on_itself_moves_nothing() {
        assert!((0..5).all(|i| shift_of(i, 2, 2, 56.0) == 0.0));
    }

    #[test]
    fn the_target_follows_the_center_of_the_held_row() {
        // Riga 1 afferrata a metà (grab 28): il dito a y=200 mette il centro a 200-28+28=200 → riga 3.
        assert_eq!(target_for(200.0, 56.0, 6), 3);
        assert_eq!(target_for(0.0, 56.0, 6), 0);
        assert_eq!(target_for(9999.0, 56.0, 6), 5);
    }

    #[test]
    fn the_list_scrolls_near_the_edges_and_not_in_the_middle() {
        assert_eq!(autoscroll_speed(300.0, 0.0, 600.0), 0.0);
        assert!(autoscroll_speed(10.0, 0.0, 600.0) < 0.0);
        assert!(autoscroll_speed(590.0, 0.0, 600.0) > 0.0);
        // Più vicino al bordo, più veloce; oltre il bordo, il massimo.
        assert!(autoscroll_speed(5.0, 0.0, 600.0) < autoscroll_speed(40.0, 0.0, 600.0));
        assert_eq!(autoscroll_speed(-100.0, 0.0, 600.0), -MAX_SCROLL);
        assert_eq!(autoscroll_speed(900.0, 0.0, 600.0), MAX_SCROLL);
    }

    use crate::ui::gesture::harness::{Rig, blank};
    use iced::{Point, Size};

    #[derive(Debug, Clone, PartialEq)]
    enum Msg {
        Move(usize, usize),
        Scroll(bool),
    }

    fn rig<'a>() -> Rig<'a, Msg> {
        let rows = (0..5).map(|_| blank::<Msg>(400.0, 56.0)).collect();
        let list = reorder(rows, 56.0, iced::Color::BLACK, Msg::Move, |dy| Msg::Scroll(dy > 0.0));
        Rig::new(list, Size::new(400.0, 280.0))
    }

    const GRIP_X: f32 = 380.0;

    #[test]
    fn long_press_on_the_grip_then_drag_moves_the_row_on_release() {
        let mut r = rig();
        r.press(GRIP_X, 20.0);
        r.wait(470);
        r.frame(Some(Point::new(GRIP_X, 20.0)));
        r.moved(GRIP_X, 90.0);
        r.moved(GRIP_X, 150.0);
        r.release(GRIP_X, 150.0);
        // Vicino al bordo alto la lista può chiedere di scorrere: conta l'ultimo messaggio.
        assert_eq!(r.messages.last(), Some(&Msg::Move(0, 2)), "{:?}", r.messages);
    }

    #[test]
    fn dropping_where_it_started_moves_nothing() {
        let mut r = rig();
        r.press(GRIP_X, 70.0);
        r.wait(470);
        r.frame(Some(Point::new(GRIP_X, 70.0)));
        r.moved(GRIP_X, 75.0);
        r.release(GRIP_X, 75.0);
        assert!(r.messages.is_empty());
    }

    #[test]
    fn a_press_outside_the_grip_is_not_a_reorder() {
        let mut r = rig();
        r.press(100.0, 20.0);
        r.wait(470);
        r.frame(Some(Point::new(100.0, 20.0)));
        r.moved(100.0, 200.0);
        r.release(100.0, 200.0);
        assert!(r.messages.is_empty());
    }

    #[test]
    fn moving_before_the_long_press_is_a_scroll_not_a_reorder() {
        let mut r = rig();
        r.press(GRIP_X, 20.0);
        r.moved(GRIP_X, 50.0);
        r.moved(GRIP_X, 120.0);
        r.wait(470);
        r.frame(Some(Point::new(GRIP_X, 120.0)));
        r.release(GRIP_X, 120.0);
        assert!(r.messages.is_empty());
    }

    #[test]
    fn near_the_bottom_edge_the_list_asks_to_scroll_down() {
        let mut r = rig();
        r.press(GRIP_X, 20.0);
        r.wait(470);
        r.frame(Some(Point::new(GRIP_X, 20.0)));
        r.moved(GRIP_X, 270.0);
        r.frame(Some(Point::new(GRIP_X, 270.0)));
        assert!(r.messages.contains(&Msg::Scroll(true)), "{:?}", r.messages);
    }

    #[test]
    fn the_context_menu_click_is_swallowed_while_the_row_is_held() {
        use iced::event::{Event, Status};
        use iced::mouse;
        let mut r = rig();
        r.press(GRIP_X, 20.0);
        r.wait(470);
        r.frame(Some(Point::new(GRIP_X, 20.0)));
        let status = r.send(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
            Some(Point::new(GRIP_X, 20.0)),
        );
        assert_eq!(status, Status::Captured);
    }
}
