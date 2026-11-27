//! La riga che si trascina di lato per un'azione: a destra «aggiungi alla coda», a sinistra «mi
//! piace». Il dito sposta la riga, sotto compare un fondo con l'icona; oltre la soglia il fondo si
//! accende e, al rilascio, parte l'azione. Poi la riga torna a posto con un ease-out.
//!
//! Non scatta mai mentre la lista scorre: il riconoscitore blocca l'asse quando il dito supera la
//! soglia e la diagonale è verticale (`gesture::recognizer::lock_axis`), la stessa regola della
//! patch di `iced_winit` che fa scorrere la pagina. Un trascinamento verticale annulla il gesto
//! della riga e la lista scorre da sé.
//!
//! L'animazione di ritorno chiede il frame successivo solo mentre dura (`RedrawRequest`), senza
//! timer né messaggi: uno stato fermo non costa niente.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay, renderer};
use iced::event::{self, Event};
use iced::{Border, Color, Element, Length, Point, Rectangle, Size, Vector, mouse, window};

use super::recognizer::{Axis, Output};
use super::tracker::Tracker;
use crate::ui::motion::ease_out;

/// Quanto va trascinata la riga perché l'azione scatti al rilascio (dp).
pub const THRESHOLD: f32 = 88.0;
/// Un lancio veloce nella direzione dell'azione basta, se la riga si è già mossa di almeno così.
const FLING_MIN_OFFSET: f32 = 28.0;
/// Velocità (dp/s) di un lancio che vale come soglia raggiunta.
const FLING_SPEED: f32 = 700.0;
/// Oltre la soglia la riga segue il dito con questa frazione: si sente che è il limite.
const RESISTANCE: f32 = 0.3;
/// Il ritorno della riga a posto.
const RETURN: Duration = Duration::from_millis(220);
/// Larghezza della zona dell'icona sul fondo.
const ZONE: f32 = 72.0;

/// Cosa mostra e fa uno dei due lati.
pub struct Reveal<'a, Message, Theme, Renderer> {
    /// L'icona a riposo (prima della soglia).
    pub idle: Element<'a, Message, Theme, Renderer>,
    /// L'icona sul fondo acceso (oltre la soglia).
    pub armed: Element<'a, Message, Theme, Renderer>,
    /// Il messaggio che parte al rilascio oltre la soglia.
    pub message: Message,
    /// Colori del fondo a riposo e acceso.
    pub background: Color,
    pub armed_background: Color,
}

pub struct SwipeRow<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    /// Trascinando a destra (il fondo compare a sinistra).
    right: Option<Reveal<'a, Message, Theme, Renderer>>,
    /// Trascinando a sinistra (il fondo compare a destra).
    left: Option<Reveal<'a, Message, Theme, Renderer>>,
}

/// Una riga con le azioni a scorrimento. Senza nessun lato è la riga com'è.
pub fn swipe_row<'a, Message: Clone + 'a, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> SwipeRow<'a, Message, Theme, Renderer> {
    SwipeRow { content: content.into(), right: None, left: None }
}

impl<'a, Message, Theme, Renderer> SwipeRow<'a, Message, Theme, Renderer> {
    pub fn on_right(mut self, reveal: Reveal<'a, Message, Theme, Renderer>) -> Self {
        self.right = Some(reveal);
        self
    }

    pub fn on_left(mut self, reveal: Reveal<'a, Message, Theme, Renderer>) -> Self {
        self.left = Some(reveal);
        self
    }

    /// Gli elementi nell'ordine dei figli: contenuto, poi (se ci sono) destra idle/armed, sinistra.
    fn parts(&self) -> Vec<&Element<'a, Message, Theme, Renderer>> {
        let mut parts = vec![&self.content];
        for side in [&self.right, &self.left].into_iter().flatten() {
            parts.push(&side.idle);
            parts.push(&side.armed);
        }
        parts
    }
}

#[derive(Debug, Default)]
struct State {
    tracker: Tracker,
    /// Spostamento orizzontale della riga adesso (dp, + a destra).
    offset: f32,
    /// Il dito sta trascinando la riga.
    dragging: bool,
    /// Il ritorno a posto: da dove è partito e quando.
    returning: Option<(f32, Instant)>,
}

impl State {
    /// Lo spostamento mostrato a `now`, tenendo conto del ritorno.
    fn shown(&self, now: Instant) -> f32 {
        match self.returning {
            Some((from, since)) if !self.dragging => {
                let t = now.saturating_duration_since(since).as_secs_f32() / RETURN.as_secs_f32();
                from * (1.0 - ease_out(t))
            }
            _ => self.offset,
        }
    }
}

/// Dallo spostamento grezzo del dito a quello della riga: 1:1 fino alla soglia, poi con resistenza.
pub fn resisted(raw: f32) -> f32 {
    let magnitude = raw.abs();
    let shown =
        if magnitude <= THRESHOLD { magnitude } else { THRESHOLD + (magnitude - THRESHOLD) * RESISTANCE };
    shown.copysign(raw)
}

/// Il rilascio fa scattare l'azione: la riga ha passato la soglia, o è stata lanciata nella direzione
/// dell'azione dopo essersi mossa un po'.
pub fn should_fire(raw_total: f32, fling_velocity: Option<f32>) -> bool {
    if raw_total.abs() >= THRESHOLD {
        return true;
    }
    fling_velocity.is_some_and(|v| {
        v.abs() >= FLING_SPEED && v.signum() == raw_total.signum() && raw_total.abs() >= FLING_MIN_OFFSET
    })
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer> for SwipeRow<'a, Message, Theme, Renderer>
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
        self.parts().into_iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        let parts: Vec<&Element<'a, Message, Theme, Renderer>> = self.parts();
        tree.diff_children_custom(&parts, |child, part| child.diff(*part), |part| Tree::new(*part));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(&self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let content = self.content.as_widget().layout(&mut tree.children[0], renderer, limits);
        let size = content.size();
        let mut nodes = vec![content];
        let zone = layout::Limits::new(Size::ZERO, Size::new(ZONE, size.height));
        let mut index = 1;
        // Destra (fondo a sinistra): la zona sta al bordo sinistro; sinistra: al bordo destro.
        for (side, at_right) in [(&self.right, false), (&self.left, true)] {
            if side.is_none() {
                continue;
            }
            for _ in 0..2 {
                let x = if at_right { size.width - ZONE } else { 0.0 };
                let node = self.parts()[index]
                    .as_widget()
                    .layout(&mut tree.children[index], renderer, &zone)
                    .move_to(Point::new(x.max(0.0), 0.0));
                nodes.push(node);
                index += 1;
            }
        }
        layout::Node::with_children(size, nodes)
    }

    fn operate(&self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, op: &mut dyn Operation) {
        if let Some(child) = layout.children().next() {
            self.content.as_widget().operate(&mut tree.children[0], child, renderer, op);
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
        let (can_right, can_left) = (self.right.is_some(), self.left.is_some());
        let state = tree.state.downcast_mut::<State>();
        let was_dragging = state.dragging;

        if let Some(out) = state.tracker.feed(&event, bounds, cursor) {
            match out {
                Output::PanStart { axis: Axis::Horizontal, total, .. } => {
                    // Un lato senza azione non si muove: il gesto non è della riga.
                    if (total.x > 0.0 && can_right) || (total.x < 0.0 && can_left) {
                        state.dragging = true;
                        state.returning = None;
                        state.offset = resisted(total.x);
                        shell.request_redraw(window::RedrawRequest::NextFrame);
                    } else {
                        state.tracker.cancel();
                    }
                }
                // Verticale: scorre la lista, la riga non c'entra.
                Output::PanStart { axis: Axis::Vertical, .. } => state.tracker.cancel(),
                Output::PanMove { axis: Axis::Horizontal, total, .. } if state.dragging => {
                    let allowed = (total.x > 0.0 && can_right) || (total.x < 0.0 && can_left);
                    state.offset = if allowed { resisted(total.x) } else { 0.0 };
                    shell.request_redraw(window::RedrawRequest::NextFrame);
                }
                Output::PanEnd { total, .. } if state.dragging => {
                    release(state, shell, total.x, None, &self.right, &self.left);
                }
                Output::Fling { axis: Axis::Horizontal, velocity, total } if state.dragging => {
                    release(state, shell, total.x, Some(velocity.x), &self.right, &self.left);
                }
                Output::Cancel if state.dragging => {
                    release(state, shell, 0.0, None, &self.right, &self.left);
                }
                _ => {}
            }
        } else if matches!(event, Event::Window(window::Event::RedrawRequested(_))) {
            // Il ritorno: avanza finché dura, poi la riga è ferma e non chiede più niente.
            if let Some((_, since)) = state.returning
                && !state.dragging
            {
                if since.elapsed() >= RETURN {
                    state.returning = None;
                    state.offset = 0.0;
                } else {
                    shell.request_redraw(window::RedrawRequest::NextFrame);
                }
            }
        }
        if let Some(at) = state.tracker.deadline() {
            shell.request_redraw(window::RedrawRequest::At(at));
        }

        // Mentre la riga si muove o torna il contenuto non riceve il mouse; al rilascio di un
        // trascinamento col mouse il pulsante sotto si libera senza scattare.
        let moving = state.dragging || was_dragging || state.returning.is_some();
        if moving {
            let released = matches!(
                event,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                    | Event::Mouse(mouse::Event::CursorLeft)
            );
            if released && let Some(child) = layout.children().next() {
                let _ = self.content.as_widget_mut().on_event(
                    &mut tree.children[0],
                    event,
                    child,
                    mouse::Cursor::Unavailable,
                    renderer,
                    clipboard,
                    shell,
                    viewport,
                );
            }
            return event::Status::Captured;
        }
        match layout.children().next() {
            Some(child) => self.content.as_widget_mut().on_event(
                &mut tree.children[0],
                event,
                child,
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            ),
            None => event::Status::Ignored,
        }
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
        let offset = state.shown(Instant::now());
        let bounds = layout.bounds();
        let mut children = layout.children();
        let Some(content) = children.next() else {
            return;
        };
        if offset.abs() < 0.5 {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                content,
                cursor,
                viewport,
            );
            return;
        }
        // Quale lato si scopre, e se la soglia è passata.
        let (side, armed_state) = if offset > 0.0 {
            (self.right.as_ref(), offset >= THRESHOLD)
        } else {
            (self.left.as_ref(), -offset >= THRESHOLD)
        };
        if let Some(side) = side {
            let width = offset.abs().min(bounds.width);
            let revealed = Rectangle {
                x: if offset > 0.0 { bounds.x } else { bounds.x + bounds.width - width },
                y: bounds.y,
                width,
                height: bounds.height,
            };
            let background = if armed_state { side.armed_background } else { side.background };
            renderer.with_layer(revealed, |renderer| {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: revealed,
                        border: Border::default(),
                        shadow: Default::default(),
                    },
                    background,
                );
                // Le due icone di questo lato stanno nei figli dopo il contenuto.
                let first = if offset > 0.0 {
                    1
                } else if self.right.is_some() {
                    3
                } else {
                    1
                };
                let index = if armed_state { first + 1 } else { first };
                if let Some(zone) = layout.children().nth(index) {
                    let element = if armed_state { &side.armed } else { &side.idle };
                    element.as_widget().draw(
                        &tree.children[index],
                        renderer,
                        theme,
                        style,
                        zone,
                        cursor,
                        &revealed,
                    );
                }
            });
        }
        renderer.with_translation(Vector::new(offset, 0.0), |renderer| {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                content,
                mouse::Cursor::Unavailable,
                viewport,
            );
        });
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
        if state.dragging {
            return mouse::Interaction::Grabbing;
        }
        match layout.children().next() {
            Some(child) => self.content.as_widget().mouse_interaction(
                &tree.children[0],
                child,
                cursor,
                viewport,
                renderer,
            ),
            None => mouse::Interaction::None,
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let child = layout.children().next()?;
        self.content.as_widget_mut().overlay(&mut tree.children[0], child, renderer, translation)
    }
}

/// Fine del trascinamento: l'azione, se scatta, e il ritorno a posto.
fn release<Message: Clone, Theme, Renderer>(
    state: &mut State,
    shell: &mut Shell<'_, Message>,
    raw_total: f32,
    fling: Option<f32>,
    right: &Option<Reveal<'_, Message, Theme, Renderer>>,
    left: &Option<Reveal<'_, Message, Theme, Renderer>>,
) {
    let side = if raw_total > 0.0 { right } else { left };
    if let Some(side) = side
        && should_fire(raw_total, fling)
    {
        shell.publish(side.message.clone());
    }
    state.dragging = false;
    state.returning = Some((state.offset, Instant::now()));
    shell.request_redraw(window::RedrawRequest::NextFrame);
}

impl<'a, Message, Theme, Renderer> From<SwipeRow<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(row: SwipeRow<'a, Message, Theme, Renderer>) -> Self {
        Element::new(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_row_follows_the_finger_until_the_threshold_then_resists() {
        assert_eq!(resisted(40.0), 40.0);
        assert_eq!(resisted(-40.0), -40.0);
        assert_eq!(resisted(THRESHOLD), THRESHOLD);
        let beyond = resisted(THRESHOLD + 100.0);
        assert!(beyond > THRESHOLD && beyond < THRESHOLD + 100.0 * 0.5, "{beyond}");
        assert!(resisted(-(THRESHOLD + 100.0)) < -THRESHOLD);
        assert_eq!(resisted(0.0), 0.0);
    }

    #[test]
    fn the_action_fires_past_the_threshold_only() {
        assert!(should_fire(THRESHOLD, None));
        assert!(should_fire(-THRESHOLD - 5.0, None));
        assert!(!should_fire(THRESHOLD - 1.0, None));
        assert!(!should_fire(0.0, None));
    }

    #[test]
    fn a_fast_fling_in_the_action_direction_fires_early() {
        assert!(should_fire(40.0, Some(900.0)));
        assert!(should_fire(-40.0, Some(-900.0)));
        // Contro la direzione, troppo lento o troppo poco spostamento: no.
        assert!(!should_fire(40.0, Some(-900.0)));
        assert!(!should_fire(40.0, Some(500.0)));
        assert!(!should_fire(10.0, Some(1500.0)));
    }

    use crate::ui::gesture::harness::{Rig, blank};
    use iced::Size;

    #[derive(Debug, Clone, PartialEq)]
    enum Msg {
        Queue,
        Like,
    }

    fn row<'a>(right: bool, left: bool) -> Rig<'a, Msg> {
        let mut swipe = swipe_row(blank::<Msg>(400.0, 60.0));
        let reveal = |message| Reveal {
            idle: blank(24.0, 24.0),
            armed: blank(24.0, 24.0),
            message,
            background: iced::Color::BLACK,
            armed_background: iced::Color::WHITE,
        };
        if right {
            swipe = swipe.on_right(reveal(Msg::Queue));
        }
        if left {
            swipe = swipe.on_left(reveal(Msg::Like));
        }
        Rig::new(swipe, Size::new(400.0, 60.0))
    }

    /// Trascina da (200, 30) di `dx`, `dy` in passi, aspetta (il dito si ferma) e rilascia.
    fn drag(rig: &mut Rig<'_, Msg>, dx: f32, dy: f32) {
        rig.press(200.0, 30.0);
        for step in 1..=10 {
            let f = step as f32 / 10.0;
            rig.moved(200.0 + dx * f, 30.0 + dy * f);
        }
        rig.wait(140);
        rig.release(200.0 + dx, 30.0 + dy);
    }

    #[test]
    fn dragging_right_past_the_threshold_adds_to_the_queue() {
        let mut rig = row(true, true);
        drag(&mut rig, 120.0, 4.0);
        assert_eq!(rig.messages, vec![Msg::Queue]);
    }

    #[test]
    fn dragging_left_past_the_threshold_likes() {
        let mut rig = row(true, true);
        drag(&mut rig, -120.0, -3.0);
        assert_eq!(rig.messages, vec![Msg::Like]);
    }

    #[test]
    fn a_short_slow_drag_does_nothing_and_the_row_returns() {
        let mut rig = row(true, true);
        drag(&mut rig, 50.0, 0.0);
        assert!(rig.messages.is_empty());
    }

    #[test]
    fn a_vertical_drag_never_triggers_an_action_and_is_not_captured() {
        let mut rig = row(true, true);
        rig.press(200.0, 30.0);
        for step in 1..=10 {
            // Scorrimento della lista con una deriva orizzontale grande.
            rig.moved(200.0 + step as f32 * 8.0, 30.0 + step as f32 * 12.0);
        }
        rig.wait(140);
        rig.release(280.0, 150.0);
        assert!(rig.messages.is_empty());
    }

    #[test]
    fn a_diagonal_drag_that_the_list_scrolls_does_not_fire_even_when_long() {
        let mut rig = row(true, true);
        // 200 dp a destra e 190 in giù: asse verticale (diagonale), la riga non si muove.
        drag(&mut rig, 200.0, 190.0);
        assert!(rig.messages.is_empty());
    }

    #[test]
    fn a_side_without_action_does_not_move_the_row() {
        let mut rig = row(false, true);
        drag(&mut rig, 150.0, 0.0);
        assert!(rig.messages.is_empty());
        let mut rig = row(true, false);
        drag(&mut rig, -150.0, 0.0);
        assert!(rig.messages.is_empty());
    }

    #[test]
    fn a_fast_flick_fires_before_the_threshold() {
        let mut rig = row(true, true);
        rig.press(200.0, 30.0);
        rig.moved(215.0, 30.0);
        rig.moved(245.0, 30.0);
        // Rilascio subito: velocità altissima, 45 dp bastano.
        rig.release(245.0, 30.0);
        assert_eq!(rig.messages, vec![Msg::Queue]);
    }

    #[test]
    fn the_fired_action_publishes_once_and_a_tap_afterwards_is_a_tap() {
        let mut rig = row(true, true);
        drag(&mut rig, 120.0, 0.0);
        rig.press(200.0, 30.0);
        rig.release(200.0, 30.0);
        assert_eq!(rig.messages, vec![Msg::Queue]);
    }

    #[test]
    fn a_touch_cancel_mid_swipe_fires_nothing() {
        use crate::ui::gesture::tracker::{TOUCH_CANCEL, TOUCH_DOWN};
        use iced::event::Event;
        use iced::{Point, mouse};
        let mut rig = row(true, true);
        let at = Some(Point::new(200.0, 30.0));
        rig.send(Event::Mouse(mouse::Event::CursorMoved { position: Point::new(200.0, 30.0) }), at);
        rig.send(Event::Mouse(mouse::Event::ButtonPressed(TOUCH_DOWN)), at);
        rig.moved(260.0, 30.0);
        rig.moved(330.0, 30.0);
        rig.send(Event::Mouse(mouse::Event::ButtonReleased(TOUCH_CANCEL)), Some(Point::new(330.0, 30.0)));
        assert!(rig.messages.is_empty());
    }

    #[test]
    fn a_finger_swipe_through_the_touch_markers_fires() {
        use crate::ui::gesture::tracker::{TOUCH_DOWN, TOUCH_UP};
        use iced::event::Event;
        use iced::{Point, mouse};
        let mut rig = row(true, true);
        let at = Some(Point::new(200.0, 30.0));
        rig.send(Event::Mouse(mouse::Event::CursorMoved { position: Point::new(200.0, 30.0) }), at);
        rig.send(Event::Mouse(mouse::Event::ButtonPressed(TOUCH_DOWN)), at);
        for step in 1..=10 {
            rig.moved(200.0 + step as f32 * 12.0, 31.0);
        }
        rig.wait(140);
        rig.send(Event::Mouse(mouse::Event::ButtonReleased(TOUCH_UP)), Some(Point::new(320.0, 31.0)));
        assert_eq!(rig.messages, vec![Msg::Queue]);
    }
}
