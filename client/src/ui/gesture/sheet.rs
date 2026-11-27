//! Il foglio del player a schermo intero: entra dal basso con la molla, e un trascinamento in giù
//! lo porta fuori seguendo il dito. Al rilascio decide la distanza o la velocità: oltre un terzo
//! dell'altezza, o un lancio verso il basso, il foglio esce e chiude il player; altrimenti torna su
//! con la molla.
//!
//! Il movimento è tutto qui dentro (frazione dell'altezza nello stato del widget): l'app riceve
//! solo tre messaggi per gesto (`on_dragging`, `on_settled`, `on_closed`), non uno per movimento, e
//! la pagina sotto si disegna solo mentre il foglio non è del tutto aperto. I frame si chiedono solo
//! mentre il foglio si muove.
//!
//! Il trascinamento comincia solo verso il basso e solo nella zona di presa (`grab`): sopra la lista
//! del testo o della coda scorrere in giù è della lista, e sul bordo destro sta lo slider del
//! volume.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay, renderer};
use iced::event::{self, Event};
use iced::{Border, Color, Element, Length, Rectangle, Size, Vector, mouse, window};

use super::recognizer::{Axis, Output};
use super::tracker::Tracker;
use crate::ui::motion::{ease_out, spring};

/// Oltre questa frazione dell'altezza il rilascio chiude.
pub const CLOSE_FRACTION: f32 = 0.30;
/// Un lancio verso il basso più veloce di così (dp/s) chiude.
pub const CLOSE_SPEED: f32 = 800.0;
/// Un lancio verso l'alto più veloce di così riporta su il foglio anche se è molto sceso.
const KEEP_SPEED: f32 = 800.0;
/// Entrata e ritorno con la molla; l'uscita con l'ease-out, più breve.
const SPRING: Duration = Duration::from_millis(420);
const EXIT: Duration = Duration::from_millis(220);
/// Quanto si scurisce la pagina sotto a foglio chiuso.
const SCRIM: f32 = 0.55;

/// Cosa decide il rilascio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Close,
    Keep,
}

/// Chiude se il foglio è sceso oltre la frazione, o se è stato lanciato in giù; lo tiene se è
/// stato lanciato in su.
pub fn decide(fraction: f32, velocity_y: Option<f32>) -> Verdict {
    match velocity_y {
        Some(v) if v >= CLOSE_SPEED => Verdict::Close,
        Some(v) if v <= -KEEP_SPEED => Verdict::Keep,
        _ if fraction >= CLOSE_FRACTION => Verdict::Close,
        _ => Verdict::Keep,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Curve {
    Spring,
    EaseOut,
}

#[derive(Debug, Clone, Copy)]
struct Anim {
    from: f32,
    to: f32,
    since: Instant,
    duration: Duration,
    curve: Curve,
    /// A fine corsa il foglio è uscito (`on_closed`) e non solo tornato su (`on_settled`).
    closing: bool,
}

impl Anim {
    fn progress(&self, now: Instant) -> f32 {
        (now.saturating_duration_since(self.since).as_secs_f32() / self.duration.as_secs_f32())
            .clamp(0.0, 1.0)
    }

    fn value(&self, now: Instant) -> f32 {
        let t = self.progress(now);
        let eased = match self.curve {
            Curve::Spring => spring(t),
            Curve::EaseOut => ease_out(t),
        };
        self.from + (self.to - self.from) * eased
    }

    fn done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }
}

pub struct Sheet<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    /// Entra dal basso quando il foglio nasce.
    enter: bool,
    /// Chi sta sopra chiede di uscire (il tasto «giù» dell'intestazione).
    dismiss: bool,
    /// Il trascinamento comincia solo nei primi `grab` dp dall'alto.
    grab: f32,
    /// Fascia a destra (dp) dove il trascinamento non comincia (slider del volume).
    except_right: f32,
    on_dragging: Message,
    on_settled: Message,
    on_closed: Message,
}

/// Il foglio. `on_dragging` parte quando il dito comincia a trascinarlo, `on_settled` quando è
/// tornato del tutto su (la pagina sotto non serve più), `on_closed` quando è uscito.
pub fn sheet<'a, Message: Clone + 'a, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    on_dragging: Message,
    on_settled: Message,
    on_closed: Message,
) -> Sheet<'a, Message, Theme, Renderer> {
    Sheet {
        content: content.into(),
        enter: false,
        dismiss: false,
        grab: f32::MAX,
        except_right: 0.0,
        on_dragging,
        on_settled,
        on_closed,
    }
}

impl<Message, Theme, Renderer> Sheet<'_, Message, Theme, Renderer> {
    pub fn enter(mut self, enter: bool) -> Self {
        self.enter = enter;
        self
    }

    pub fn dismiss(mut self, dismiss: bool) -> Self {
        self.dismiss = dismiss;
        self
    }

    pub fn grab(mut self, height: f32) -> Self {
        self.grab = height;
        self
    }

    pub fn except_right(mut self, width: f32) -> Self {
        self.except_right = width;
        self
    }
}

#[derive(Debug)]
struct State {
    tracker: Tracker,
    /// Quanto è sceso il foglio, da 0 (aperto) a 1 (fuori).
    fraction: f32,
    dragging: bool,
    anim: Option<Anim>,
    /// Il gesto è cominciato nella zona di presa.
    armed: bool,
}

impl State {
    fn shown(&self, now: Instant) -> f32 {
        match self.anim {
            Some(anim) if !self.dragging => anim.value(now).max(0.0),
            _ => self.fraction,
        }
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Sheet<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        let anim = self.enter.then(|| Anim {
            from: 1.0,
            to: 0.0,
            since: Instant::now(),
            duration: SPRING,
            curve: Curve::Spring,
            closing: false,
        });
        tree::State::new(State {
            tracker: Tracker::default(),
            fraction: if self.enter { 1.0 } else { 0.0 },
            dragging: false,
            anim,
            armed: false,
        })
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let limits = limits.width(Length::Fill).height(Length::Fill);
        let child = self.content.as_widget().layout(&mut tree.children[0], renderer, &limits);
        layout::Node::with_children(limits.max(), vec![child])
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
        let state = tree.state.downcast_mut::<State>();
        let was_dragging = state.dragging;
        let now = Instant::now();

        // Il gesto comincia solo nella zona di presa.
        let starts = matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(button))
                if button == super::tracker::TOUCH_DOWN || button == mouse::Button::Left
        );
        if starts {
            state.armed = cursor
                .position_in(bounds)
                .is_some_and(|p| p.y <= self.grab && p.x <= bounds.width - self.except_right);
        }

        let out = if state.armed { state.tracker.feed(&event, bounds, cursor) } else { None };
        if let Some(out) = out {
            match out {
                Output::PanStart { axis: Axis::Vertical, total, .. } if total.y > 0.0 => {
                    state.dragging = true;
                    state.anim = None;
                    state.fraction = (total.y / bounds.height.max(1.0)).clamp(0.0, 1.0);
                    shell.publish(self.on_dragging.clone());
                    shell.request_redraw(window::RedrawRequest::NextFrame);
                }
                // Verso l'alto o in orizzontale: non è del foglio.
                Output::PanStart { .. } => state.tracker.cancel(),
                Output::PanMove { axis: Axis::Vertical, total, .. } if state.dragging => {
                    state.fraction = (total.y / bounds.height.max(1.0)).clamp(0.0, 1.0);
                    shell.request_redraw(window::RedrawRequest::NextFrame);
                }
                Output::PanEnd { total, .. } if state.dragging => {
                    finish(state, shell, total.y / bounds.height.max(1.0), None, now);
                }
                Output::Fling { velocity, total, .. } if state.dragging => {
                    finish(state, shell, total.y / bounds.height.max(1.0), Some(velocity.y), now);
                }
                Output::Cancel if state.dragging => finish(state, shell, 0.0, None, now),
                _ => {}
            }
        }
        if let Some(at) = state.tracker.deadline() {
            shell.request_redraw(window::RedrawRequest::At(at));
        }

        // Le animazioni avanzano a ogni ridisegno richiesto, e chiudono con il loro messaggio.
        if matches!(event, Event::Window(window::Event::RedrawRequested(_))) {
            if self.dismiss
                && !state.dragging
                && state.anim.is_none_or(|a| !a.closing)
                && state.fraction < 1.0
            {
                let from = state.shown(now);
                state.fraction = from;
                state.anim = Some(Anim {
                    from,
                    to: 1.0,
                    since: now,
                    duration: EXIT,
                    curve: Curve::EaseOut,
                    closing: true,
                });
            }
            if let Some(anim) = state.anim.filter(|_| !state.dragging) {
                if anim.done(now) {
                    state.anim = None;
                    state.fraction = anim.to;
                    shell.publish(if anim.closing {
                        self.on_closed.clone()
                    } else {
                        self.on_settled.clone()
                    });
                } else {
                    shell.request_redraw(window::RedrawRequest::NextFrame);
                }
            }
        }

        if state.dragging || was_dragging || state.anim.is_some() {
            // Il foglio si muove: il contenuto non riceve il mouse, ma al rilascio di un
            // trascinamento col mouse il pulsante sotto si libera senza scattare.
            let released = matches!(
                event,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                    | Event::Mouse(mouse::Event::CursorLeft)
            );
            let is_window = matches!(event, Event::Window(_));
            if released && let Some(child) = layout.children().next() {
                let _ = self.content.as_widget_mut().on_event(
                    &mut tree.children[0],
                    event.clone(),
                    child,
                    mouse::Cursor::Unavailable,
                    renderer,
                    clipboard,
                    shell,
                    viewport,
                );
            }
            // Un RedrawRequested arriva comunque ai widget: li si lascia aggiornare.
            if !is_window {
                return event::Status::Captured;
            }
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
        let fraction = state.shown(Instant::now());
        let bounds = layout.bounds();
        let Some(child) = layout.children().next() else {
            return;
        };
        if fraction <= 0.0005 {
            self.content.as_widget().draw(&tree.children[0], renderer, theme, style, child, cursor, viewport);
            return;
        }
        renderer.with_layer(bounds, |renderer| {
            // La pagina sotto si scurisce man mano che il foglio si chiude.
            let dim = (1.0 - fraction) * SCRIM;
            renderer.fill_quad(
                renderer::Quad { bounds, border: Border::default(), shadow: Default::default() },
                Color { a: dim, ..Color::BLACK },
            );
            let shift = Vector::new(0.0, fraction * bounds.height);
            renderer.with_translation(shift, |renderer| {
                self.content.as_widget().draw(
                    &tree.children[0],
                    renderer,
                    theme,
                    style,
                    child,
                    mouse::Cursor::Unavailable,
                    viewport,
                );
            });
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

/// Fine del trascinamento: esce o torna su, con l'animazione giusta.
fn finish<Message: Clone>(
    state: &mut State,
    shell: &mut Shell<'_, Message>,
    fraction: f32,
    velocity_y: Option<f32>,
    now: Instant,
) {
    let fraction = fraction.clamp(0.0, 1.0);
    let from = if state.dragging { state.fraction } else { fraction };
    state.dragging = false;
    state.fraction = from;
    state.anim = Some(match decide(fraction, velocity_y) {
        Verdict::Close => {
            Anim { from, to: 1.0, since: now, duration: EXIT, curve: Curve::EaseOut, closing: true }
        }
        Verdict::Keep => {
            Anim { from, to: 0.0, since: now, duration: SPRING, curve: Curve::Spring, closing: false }
        }
    });
    shell.request_redraw(window::RedrawRequest::NextFrame);
}

impl<'a, Message, Theme, Renderer> From<Sheet<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(sheet: Sheet<'a, Message, Theme, Renderer>) -> Self {
        Element::new(sheet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_slow_drag_springs_back() {
        assert_eq!(decide(0.10, None), Verdict::Keep);
        assert_eq!(decide(0.29, Some(200.0)), Verdict::Keep);
    }

    #[test]
    fn a_long_drag_closes_even_when_slow() {
        assert_eq!(decide(0.30, None), Verdict::Close);
        assert_eq!(decide(0.7, Some(10.0)), Verdict::Close);
    }

    #[test]
    fn a_fast_downward_fling_closes_from_a_short_distance() {
        assert_eq!(decide(0.05, Some(1200.0)), Verdict::Close);
        assert_eq!(decide(0.02, Some(CLOSE_SPEED)), Verdict::Close);
    }

    #[test]
    fn an_upward_fling_keeps_the_sheet_open_even_from_far() {
        assert_eq!(decide(0.6, Some(-1500.0)), Verdict::Keep);
        // Meno veloce: decide la distanza.
        assert_eq!(decide(0.6, Some(-300.0)), Verdict::Close);
    }

    #[test]
    fn the_closing_animation_ends_at_the_bottom_and_the_return_at_the_top() {
        let now = Instant::now();
        let close =
            Anim { from: 0.2, to: 1.0, since: now, duration: EXIT, curve: Curve::EaseOut, closing: true };
        assert_eq!(close.value(now), 0.2);
        assert_eq!(close.value(now + EXIT), 1.0);
        assert!(close.done(now + EXIT));
        assert!(!close.done(now + EXIT / 2));
        let back =
            Anim { from: 0.2, to: 0.0, since: now, duration: SPRING, curve: Curve::Spring, closing: false };
        assert_eq!(back.value(now + SPRING), 0.0);
    }

    use crate::ui::gesture::harness::{Rig, blank};
    use iced::Size;

    #[derive(Debug, Clone, PartialEq)]
    enum Msg {
        Dragging,
        Settled,
        Closed,
    }

    fn rig<'a>(enter: bool, dismiss: bool) -> Rig<'a, Msg> {
        let s = sheet(blank::<Msg>(400.0, 800.0), Msg::Dragging, Msg::Settled, Msg::Closed)
            .enter(enter)
            .dismiss(dismiss)
            .grab(500.0)
            .except_right(60.0);
        Rig::new(s, Size::new(400.0, 800.0))
    }

    fn drag_down(rig: &mut Rig<'_, Msg>, from_y: f32, dy: f32, rest_ms: u64) {
        rig.press(150.0, from_y);
        for step in 1..=10 {
            rig.moved(150.0, from_y + dy * step as f32 / 10.0);
        }
        rig.wait(rest_ms);
        rig.release(150.0, from_y + dy);
    }

    #[test]
    fn a_long_drag_down_follows_the_finger_then_closes_after_the_exit_animation() {
        let mut r = rig(false, false);
        drag_down(&mut r, 100.0, 400.0, 140);
        assert_eq!(r.messages, vec![Msg::Dragging], "un solo messaggio all'inizio");
        // Il foglio sta uscendo: finché l'animazione dura non chiude.
        r.frame(None);
        assert_eq!(r.messages, vec![Msg::Dragging]);
        r.wait(240);
        r.frame(None);
        assert_eq!(r.messages, vec![Msg::Dragging, Msg::Closed]);
    }

    #[test]
    fn a_short_slow_drag_springs_back_and_settles() {
        let mut r = rig(false, false);
        drag_down(&mut r, 100.0, 60.0, 140);
        r.wait(440);
        r.frame(None);
        assert_eq!(r.messages, vec![Msg::Dragging, Msg::Settled]);
    }

    #[test]
    fn a_fast_fling_down_closes_from_a_short_drag() {
        let mut r = rig(false, false);
        r.press(150.0, 100.0);
        r.moved(150.0, 120.0);
        r.moved(150.0, 150.0);
        r.release(150.0, 150.0);
        r.wait(240);
        r.frame(None);
        assert!(r.messages.contains(&Msg::Closed), "{:?}", r.messages);
    }

    #[test]
    fn dragging_up_or_sideways_is_not_the_sheets() {
        let mut r = rig(false, false);
        drag_down(&mut r, 300.0, -200.0, 140);
        assert!(r.messages.is_empty());
        let mut r = rig(false, false);
        r.press(150.0, 100.0);
        r.moved(250.0, 105.0);
        r.moved(330.0, 110.0);
        r.release(330.0, 110.0);
        assert!(r.messages.is_empty());
    }

    #[test]
    fn a_drag_that_starts_below_the_grab_zone_or_on_the_volume_edge_is_ignored() {
        let mut r = rig(false, false);
        drag_down(&mut r, 600.0, 150.0, 10);
        assert!(r.messages.is_empty());
        let mut r = rig(false, false);
        r.press(380.0, 100.0);
        r.moved(380.0, 200.0);
        r.moved(380.0, 400.0);
        r.release(380.0, 400.0);
        assert!(r.messages.is_empty());
    }

    #[test]
    fn a_new_sheet_enters_with_the_spring_and_settles() {
        let mut r = rig(true, false);
        r.frame(None);
        assert!(r.messages.is_empty(), "ancora in movimento");
        r.wait(440);
        r.frame(None);
        assert_eq!(r.messages, vec![Msg::Settled]);
    }

    #[test]
    fn a_dismiss_request_runs_the_exit_animation_then_closes() {
        let mut r = rig(false, true);
        r.frame(None);
        assert!(r.messages.is_empty());
        r.wait(240);
        r.frame(None);
        assert_eq!(r.messages, vec![Msg::Closed]);
    }

    #[test]
    fn a_cancelled_drag_springs_back() {
        use crate::ui::gesture::tracker::{TOUCH_CANCEL, TOUCH_DOWN};
        use iced::event::Event;
        use iced::{Point, mouse};
        let mut r = rig(false, false);
        let at = Some(Point::new(150.0, 100.0));
        r.send(Event::Mouse(mouse::Event::CursorMoved { position: Point::new(150.0, 100.0) }), at);
        r.send(Event::Mouse(mouse::Event::ButtonPressed(TOUCH_DOWN)), at);
        r.moved(150.0, 160.0);
        r.moved(150.0, 400.0);
        r.send(Event::Mouse(mouse::Event::ButtonReleased(TOUCH_CANCEL)), Some(Point::new(150.0, 400.0)));
        r.wait(440);
        r.frame(None);
        assert_eq!(r.messages, vec![Msg::Dragging, Msg::Settled]);
    }
}
