//! Patch di Psytify: i gesti del touchscreen diventati eventi del mouse.
//!
//! I widget di Iced 0.13 sono pensati per il mouse. Col tocco, un pulsante cattura il dito e la
//! lista che lo contiene non scorre più; uno `scrollable` orizzontale cattura ogni tocco e la
//! pagina verticale sotto resta ferma. Qui il gesto si decide prima di arrivare ai widget:
//!
//! - tocco breve: clic sinistro dove si è toccato;
//! - trascinamento oltre `SLOP`: rotella in pixel, bloccata sull'asse del gesto (la pagina scorre in
//!   verticale, gli scaffali in orizzontale), poi l'inerzia se il dito si stacca veloce;
//! - pressione lunga: clic destro, che apre il menu contestuale;
//! - un tocco mentre la lista scorre per inerzia la ferma e basta.

use std::time::{Duration, Instant};

use crate::core::mouse;
use crate::core::{Event, Point, Vector};

/// Quanto può muoversi il dito prima che il tocco diventi un trascinamento (in px logici).
const SLOP: f32 = 8.0;
/// Dopo quanto un dito fermo diventa una pressione lunga.
///
/// 500 ms: più dei 450 ms del riconoscitore del client (`client/src/ui/gesture`), così la pressione
/// lunga di una maniglia di riordino scatta prima e il clic destro di qui viene mangiato.
const LONG_PRESS: Duration = Duration::from_millis(500);
/// Quanto lo spostamento orizzontale deve superare il verticale perché l'asse sia orizzontale: una
/// diagonale scorre la pagina. Va tenuto uguale a `HORIZONTAL_BIAS` in
/// `client/src/ui/gesture/recognizer.rs`, che decide lo stesso gesto per le righe trascinabili.
const HORIZONTAL_BIAS: f32 = 1.25;

/// I segnali del dito per i widget del client (`client/src/ui/gesture/tracker.rs`): pulsanti del
/// mouse con un codice che nessun mouse ha, ignorati dai widget di Iced.
pub const TOUCH_DOWN: u16 = 0xF7D0;
pub const TOUCH_UP: u16 = 0xF7D1;
pub const TOUCH_CANCEL: u16 = 0xF7D2;

fn marker(pressed: bool, code: u16) -> Event {
    let button = mouse::Button::Other(code);
    Event::Mouse(if pressed {
        mouse::Event::ButtonPressed(button)
    } else {
        mouse::Event::ButtonReleased(button)
    })
}
/// Sotto questa velocità (px/s) il rilascio non lancia la lista.
const MIN_FLING: f32 = 120.0;
/// Il lancio più veloce (px/s).
const MAX_FLING: f32 = 9000.0;
/// Attrito dell'inerzia: la velocità cala di `e^(-FRICTION · t)`.
const FRICTION: f32 = 3.2;
/// Sotto questa velocità l'inerzia si ferma.
const STOP_FLING: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug)]
struct Finger {
    id: u64,
    start: Point,
    last: Point,
    last_at: Instant,
    since: Instant,
    axis: Option<Axis>,
    /// Velocità del dito (px/s), mediata sugli ultimi movimenti.
    velocity: Vector,
    long_pressed: bool,
    /// Il tocco ha fermato un'inerzia: non è un clic.
    stopped_fling: bool,
}

#[derive(Debug)]
struct Fling {
    velocity: Vector,
    last: Instant,
}

/// Lo stato dei gesti di una finestra.
#[derive(Debug, Default)]
pub struct Gestures {
    finger: Option<Finger>,
    fling: Option<Fling>,
    /// Dopo un clic il puntatore non resta sul pulsante toccato (niente «hover» appeso).
    forget_cursor: bool,
}

impl Gestures {
    /// Un evento del dito, con la posizione in px logici. Restituisce gli eventi del mouse da dare
    /// ai widget al posto del tocco.
    pub fn touch(
        &mut self,
        phase: winit::event::TouchPhase,
        id: u64,
        position: Point,
    ) -> Vec<Event> {
        use winit::event::TouchPhase;

        let tracked = self.finger.as_ref().is_some_and(|f| f.id == id);
        let any = self.finger.is_some();
        let mut events = self.translate(phase, id, position);
        // Il segnale della fine va dopo il clic sintetico: i widget del client lo usano per chiudere il
        // gesto e ignorano il clic nel mezzo.
        match phase {
            TouchPhase::Ended if tracked => {
                events.push(marker(false, TOUCH_UP));
            }
            TouchPhase::Cancelled if any => {
                events.push(marker(false, TOUCH_CANCEL));
            }
            _ => {}
        }
        events
    }

    fn translate(
        &mut self,
        phase: winit::event::TouchPhase,
        id: u64,
        position: Point,
    ) -> Vec<Event> {
        use winit::event::TouchPhase;

        let now = Instant::now();
        match phase {
            TouchPhase::Started => {
                if self.finger.is_some() {
                    // Il secondo dito non conta.
                    return Vec::new();
                }
                let stopped_fling = self.fling.take().is_some();
                self.forget_cursor = false;
                self.finger = Some(Finger {
                    id,
                    start: position,
                    last: position,
                    last_at: now,
                    since: now,
                    axis: None,
                    velocity: Vector::new(0.0, 0.0),
                    long_pressed: false,
                    stopped_fling,
                });
                vec![
                    Event::Mouse(mouse::Event::CursorMoved { position }),
                    marker(true, TOUCH_DOWN),
                ]
            }
            TouchPhase::Moved => {
                let Some(finger) = self.finger.as_mut().filter(|f| f.id == id)
                else {
                    return Vec::new();
                };
                let dt = now.duration_since(finger.last_at).as_secs_f32();
                let step = position - finger.last;
                if dt > 0.0 {
                    let instant = Vector::new(step.x / dt, step.y / dt);
                    finger.velocity = finger.velocity * 0.3 + instant * 0.7;
                }
                finger.last_at = now;

                let axis = match finger.axis {
                    Some(axis) => axis,
                    None => {
                        let moved = position - finger.start;
                        if finger.long_pressed
                            || moved.x.hypot(moved.y) < SLOP
                        {
                            // Il cursore segue comunque il dito: i widget del client leggono il
                            // gesto dai suoi passi, anche sotto la soglia o dopo la pressione lunga.
                            return vec![Event::Mouse(
                                mouse::Event::CursorMoved { position },
                            )];
                        }
                        let axis = if moved.x.abs()
                            > moved.y.abs() * HORIZONTAL_BIAS
                        {
                            Axis::Horizontal
                        } else {
                            Axis::Vertical
                        };
                        finger.axis = Some(axis);
                        // Il primo passo parte dal punto toccato: la lista segue il dito.
                        finger.last = finger.start;
                        axis
                    }
                };
                let delta = position - finger.last;
                finger.last = position;
                vec![
                    Event::Mouse(mouse::Event::CursorMoved { position }),
                    wheel(along(axis, delta)),
                ]
            }
            TouchPhase::Ended => {
                let Some(finger) = self.finger.take().filter(|f| f.id == id)
                else {
                    return Vec::new();
                };
                match finger.axis {
                    Some(axis) => {
                        let velocity = along(axis, finger.velocity);
                        let speed = velocity.x.hypot(velocity.y);
                        let recent = now.duration_since(finger.last_at)
                            < Duration::from_millis(80);
                        if recent && speed > MIN_FLING {
                            let scale = speed.min(MAX_FLING) / speed;
                            self.fling = Some(Fling {
                                velocity: velocity * scale,
                                last: now,
                            });
                        } else {
                            self.forget_cursor = true;
                        }
                        Vec::new()
                    }
                    None if finger.long_pressed || finger.stopped_fling => {
                        self.forget_cursor = true;
                        Vec::new()
                    }
                    None => {
                        self.forget_cursor = true;
                        click(mouse::Button::Left, position)
                    }
                }
            }
            TouchPhase::Cancelled => {
                self.finger = None;
                self.forget_cursor = true;
                Vec::new()
            }
        }
    }

    /// Da chiamare a ogni giro del ciclo degli eventi: la pressione lunga che scade e un passo
    /// dell'inerzia. `true` se il gesto ha bisogno di un altro giro (va chiesto un ridisegno).
    pub fn tick(&mut self, events: &mut Vec<Event>) -> bool {
        let now = Instant::now();
        if let Some(finger) = self.finger.as_mut() {
            if finger.axis.is_none() && !finger.long_pressed {
                if now.duration_since(finger.since) >= LONG_PRESS {
                    finger.long_pressed = true;
                    events.extend(click(mouse::Button::Right, finger.start));
                    return false;
                }
                // Aspetta la scadenza senza altri eventi in arrivo.
                return true;
            }
            return false;
        }
        let Some(fling) = self.fling.as_mut() else {
            return false;
        };
        let dt = now.duration_since(fling.last).as_secs_f32().min(0.05);
        fling.last = now;
        let decay = (-FRICTION * dt).exp();
        let travelled = fling.velocity * ((1.0 - decay) / FRICTION);
        fling.velocity = fling.velocity * decay;
        events.push(wheel(travelled));
        if fling.velocity.x.hypot(fling.velocity.y) < STOP_FLING {
            self.fling = None;
            self.forget_cursor = true;
            return false;
        }
        true
    }

    /// Se il puntatore va tolto dopo un clic o la fine di uno scorrimento (una volta sola).
    pub fn take_forget_cursor(&mut self) -> bool {
        std::mem::take(&mut self.forget_cursor)
    }
}

fn along(axis: Axis, delta: Vector) -> Vector {
    match axis {
        Axis::Horizontal => Vector::new(delta.x, 0.0),
        Axis::Vertical => Vector::new(0.0, delta.y),
    }
}

fn wheel(delta: Vector) -> Event {
    Event::Mouse(mouse::Event::WheelScrolled {
        delta: mouse::ScrollDelta::Pixels {
            x: delta.x,
            y: delta.y,
        },
    })
}

fn click(button: mouse::Button, position: Point) -> Vec<Event> {
    vec![
        Event::Mouse(mouse::Event::CursorMoved { position }),
        Event::Mouse(mouse::Event::ButtonPressed(button)),
        Event::Mouse(mouse::Event::ButtonReleased(button)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::event::TouchPhase;

    fn p(x: f32, y: f32) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn a_short_touch_is_a_left_click() {
        let mut g = Gestures::default();
        let _ = g.touch(TouchPhase::Started, 1, p(10.0, 10.0));
        let events = g.touch(TouchPhase::Ended, 1, p(11.0, 10.0));
        assert!(events.contains(&Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left
        ))));
    }

    #[test]
    fn a_vertical_drag_scrolls_only_vertically_and_does_not_click() {
        let mut g = Gestures::default();
        let _ = g.touch(TouchPhase::Started, 1, p(10.0, 100.0));
        let events = g.touch(TouchPhase::Moved, 1, p(13.0, 60.0));
        assert!(events.contains(&wheel(Vector::new(0.0, -40.0))));
        std::thread::sleep(Duration::from_millis(100));
        let ended = g.touch(TouchPhase::Ended, 1, p(13.0, 60.0));
        assert!(!ended.contains(&Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left
        ))));
    }

    fn is_marker(event: &Event, code: u16) -> bool {
        matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Other(c)))
                | Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Other(c)))
                if *c == code
        )
    }

    #[test]
    fn a_touch_is_announced_with_down_and_up_markers_after_the_click() {
        let mut g = Gestures::default();
        let started = g.touch(TouchPhase::Started, 1, p(10.0, 10.0));
        assert!(is_marker(started.last().unwrap(), TOUCH_DOWN));
        let ended = g.touch(TouchPhase::Ended, 1, p(10.0, 10.0));
        assert!(is_marker(ended.last().unwrap(), TOUCH_UP));
        assert!(ended.contains(&Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left
        ))));
    }

    #[test]
    fn a_cancelled_touch_sends_the_cancel_marker_only_if_one_was_down() {
        let mut g = Gestures::default();
        assert!(g.touch(TouchPhase::Cancelled, 1, p(0.0, 0.0)).is_empty());
        let _ = g.touch(TouchPhase::Started, 1, p(10.0, 10.0));
        let events = g.touch(TouchPhase::Cancelled, 1, p(10.0, 10.0));
        assert!(is_marker(events.last().unwrap(), TOUCH_CANCEL));
    }

    #[test]
    fn small_moves_still_move_the_cursor_for_the_client_gestures() {
        let mut g = Gestures::default();
        let _ = g.touch(TouchPhase::Started, 1, p(10.0, 10.0));
        let events = g.touch(TouchPhase::Moved, 1, p(12.0, 11.0));
        assert_eq!(
            events,
            vec![Event::Mouse(mouse::Event::CursorMoved {
                position: p(12.0, 11.0)
            })]
        );
    }

    #[test]
    fn a_diagonal_drag_scrolls_vertically() {
        let mut g = Gestures::default();
        let _ = g.touch(TouchPhase::Started, 1, p(10.0, 100.0));
        // 12 contro 10: non supera 1,25 volte, quindi verticale.
        let events = g.touch(TouchPhase::Moved, 1, p(22.0, 90.0));
        assert!(events.contains(&wheel(Vector::new(0.0, -10.0))));
    }

    #[test]
    fn a_second_finger_sends_nothing() {
        let mut g = Gestures::default();
        let _ = g.touch(TouchPhase::Started, 1, p(10.0, 10.0));
        assert!(g.touch(TouchPhase::Started, 2, p(50.0, 50.0)).is_empty());
        assert!(g.touch(TouchPhase::Ended, 2, p(50.0, 50.0)).is_empty());
    }
}
