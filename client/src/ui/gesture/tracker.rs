//! Dagli eventi di Iced agli ingressi del riconoscitore.
//!
//! Due sorgenti, un solo flusso:
//!
//! - **Mouse** (desktop, e il telefono provato con `PSYTIFY_PHONE=1`): pulsante sinistro giù, il
//!   cursore che si muove, pulsante su, il cursore che esce dalla finestra come annullamento.
//! - **Dito su Android**: la copia patchata di `iced_winit` (`android/vendor`, `program/touch.rs`)
//!   traduce il tocco in eventi del mouse per i widget di Iced (clic, rotella) e *in più* manda tre
//!   segnali che nessun widget di Iced conosce: `TOUCH_DOWN`, `TOUCH_UP` e `TOUCH_CANCEL`, come
//!   `ButtonPressed`/`ButtonReleased` di un pulsante `Other` con un codice che nessun mouse ha. Il
//!   cursore si muove a ogni passo, anche prima della soglia. Mentre un dito è giù, il pulsante
//!   sinistro (il clic sintetico di un tocco breve) è ignorato qui: lo gestiscono i pulsanti.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iced::event::Event;
use iced::{Point, Rectangle, mouse, window};

use super::recognizer::{Config, Input, Output, Pt, Recognizer};

/// Un dito si è posato (dopo il `CursorMoved` della sua posizione).
pub const TOUCH_DOWN: mouse::Button = mouse::Button::Other(0xF7D0);
/// Il dito si è staccato (dopo l'eventuale clic sintetico).
pub const TOUCH_UP: mouse::Button = mouse::Button::Other(0xF7D1);
/// Il sistema ha annullato il tocco.
pub const TOUCH_CANCEL: mouse::Button = mouse::Button::Other(0xF7D2);

/// L'identificatore del «dito» del mouse e del dito di Android (l'unico che arriva qui).
const POINTER: u64 = 1;

/// Origine dei tempi del riconoscitore.
fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

fn now() -> Duration {
    epoch().elapsed()
}

fn pt(p: Point) -> Pt {
    Pt::new(p.x, p.y)
}

#[derive(Debug, Clone)]
pub struct Tracker {
    recognizer: Recognizer,
    /// Un dito è giù (gli eventi del mouse sono sintetici, vanno ignorati).
    touch: bool,
    /// Il gesto è cominciato dentro i limiti del widget e non è finito.
    tracking: bool,
    /// L'ultima posizione vista (nello spazio degli eventi).
    pos: Option<Point>,
}

impl Default for Tracker {
    fn default() -> Self {
        Self::new(Config::DP)
    }
}

impl Tracker {
    pub fn new(config: Config) -> Self {
        Self { recognizer: Recognizer::new(config), touch: false, tracking: false, pos: None }
    }

    #[allow(dead_code, reason = "API per i test e per altri widget")]
    pub fn recognizer(&self) -> &Recognizer {
        &self.recognizer
    }

    /// Un gesto del widget è in corso.
    pub fn is_tracking(&self) -> bool {
        self.tracking && self.recognizer.is_active()
    }

    /// Quando va chiesto un ridisegno per far scattare la pressione lunga.
    pub fn deadline(&self) -> Option<Instant> {
        if !self.tracking {
            return None;
        }
        self.recognizer.deadline().map(|after| epoch() + after)
    }

    /// Annulla il gesto (per esempio perché il widget ha deciso che non è suo).
    pub fn cancel(&mut self) {
        self.tracking = false;
        let _ = self.recognizer.feed(Input::Cancel);
    }

    /// Un evento di Iced. `bounds` e `cursor` sono nello spazio del widget (dentro una lista che
    /// scorre sono già traslati).
    pub fn feed(&mut self, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<Output> {
        match event {
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                self.pos = Some(*position);
                self.step(Input::Move { id: POINTER, at: pt(*position), time: now() })
            }
            Event::Mouse(mouse::Event::ButtonPressed(button)) if *button == TOUCH_DOWN => {
                self.touch = true;
                self.begin(bounds, cursor)
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if !self.touch => {
                self.begin(bounds, cursor)
            }
            Event::Mouse(mouse::Event::ButtonReleased(button)) if *button == TOUCH_UP => {
                self.touch = false;
                self.end()
            }
            Event::Mouse(mouse::Event::ButtonReleased(button)) if *button == TOUCH_CANCEL => {
                self.touch = false;
                self.abort()
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if !self.touch => self.end(),
            Event::Mouse(mouse::Event::CursorLeft) if !self.touch => self.abort(),
            Event::Window(window::Event::RedrawRequested(_)) if self.tracking => self.recognizer.poll(now()),
            _ => None,
        }
    }

    fn begin(&mut self, bounds: Rectangle, cursor: mouse::Cursor) -> Option<Output> {
        self.tracking = false;
        // Il gesto deve cominciare sul widget: il cursore (nello spazio del widget) lo dice meglio
        // della posizione dell'evento, che dentro una lista che scorre non è traslata.
        let over = match cursor {
            mouse::Cursor::Available(p) => bounds.contains(p),
            _ => self.pos.is_some_and(|p| bounds.contains(p)),
        };
        if !over {
            return None;
        }
        let at = self.pos.or_else(|| cursor.position())?;
        self.tracking = true;
        self.recognizer.feed(Input::Cancel);
        self.recognizer.feed(Input::Down { id: POINTER, at: pt(at), time: now() })
    }

    fn step(&mut self, input: Input) -> Option<Output> {
        if !self.tracking {
            return None;
        }
        let out = self.recognizer.feed(input);
        self.finish(out)
    }

    fn end(&mut self) -> Option<Output> {
        if !self.tracking {
            return None;
        }
        let at = self.pos.map(pt).unwrap_or(Pt::ZERO);
        let out = self.recognizer.feed(Input::Up { id: POINTER, at, time: now() });
        self.tracking = false;
        out
    }

    fn abort(&mut self) -> Option<Output> {
        if !self.tracking {
            return None;
        }
        self.tracking = false;
        self.recognizer.feed(Input::Cancel)
    }

    fn finish(&mut self, out: Option<Output>) -> Option<Output> {
        if matches!(out, Some(Output::Cancel)) {
            self.tracking = false;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::Size;

    fn bounds() -> Rectangle {
        Rectangle::new(Point::new(0.0, 0.0), Size::new(200.0, 60.0))
    }

    fn over(x: f32, y: f32) -> mouse::Cursor {
        mouse::Cursor::Available(Point::new(x, y))
    }

    fn moved(x: f32, y: f32) -> Event {
        Event::Mouse(mouse::Event::CursorMoved { position: Point::new(x, y) })
    }

    fn press(button: mouse::Button) -> Event {
        Event::Mouse(mouse::Event::ButtonPressed(button))
    }

    fn release(button: mouse::Button) -> Event {
        Event::Mouse(mouse::Event::ButtonReleased(button))
    }

    #[test]
    fn a_mouse_drag_becomes_a_pan() {
        let mut t = Tracker::default();
        assert_eq!(t.feed(&moved(10.0, 10.0), bounds(), over(10.0, 10.0)), None);
        assert_eq!(t.feed(&press(mouse::Button::Left), bounds(), over(10.0, 10.0)), None);
        assert!(t.is_tracking());
        let out = t.feed(&moved(60.0, 12.0), bounds(), over(60.0, 12.0));
        assert!(matches!(
            out,
            Some(Output::PanStart { axis: super::super::recognizer::Axis::Horizontal, .. })
        ));
        let out = t.feed(&release(mouse::Button::Left), bounds(), over(60.0, 12.0));
        assert!(out.is_some());
        assert!(!t.is_tracking());
    }

    #[test]
    fn a_mouse_press_outside_the_widget_starts_nothing() {
        let mut t = Tracker::default();
        t.feed(&moved(500.0, 10.0), bounds(), mouse::Cursor::Available(Point::new(500.0, 10.0)));
        t.feed(&press(mouse::Button::Left), bounds(), mouse::Cursor::Available(Point::new(500.0, 10.0)));
        assert!(!t.is_tracking());
        assert_eq!(t.feed(&moved(520.0, 10.0), bounds(), over(520.0, 10.0)), None);
    }

    #[test]
    fn the_touch_markers_drive_the_gesture_and_the_synthetic_click_is_ignored() {
        let mut t = Tracker::default();
        t.feed(&moved(10.0, 10.0), bounds(), over(10.0, 10.0));
        assert_eq!(t.feed(&press(TOUCH_DOWN), bounds(), over(10.0, 10.0)), None);
        assert!(t.is_tracking());
        // Il clic sintetico di un tocco breve: sinistro giù e su, poi il segnale del dito su.
        assert_eq!(t.feed(&press(mouse::Button::Left), bounds(), over(10.0, 10.0)), None);
        assert_eq!(t.feed(&release(mouse::Button::Left), bounds(), over(10.0, 10.0)), None);
        assert!(t.is_tracking(), "il clic sintetico non chiude il gesto");
        let out = t.feed(&release(TOUCH_UP), bounds(), over(10.0, 10.0));
        assert_eq!(out, Some(Output::Tap { at: Pt::new(10.0, 10.0) }));
    }

    #[test]
    fn a_finger_drag_uses_event_positions_and_locks_the_axis() {
        let mut t = Tracker::default();
        t.feed(&moved(10.0, 30.0), bounds(), over(10.0, 30.0));
        t.feed(&press(TOUCH_DOWN), bounds(), over(10.0, 30.0));
        let out = t.feed(&moved(10.0, 60.0), bounds(), over(10.0, 60.0));
        assert!(matches!(out, Some(Output::PanStart { axis: super::super::recognizer::Axis::Vertical, .. })));
    }

    #[test]
    fn touch_cancel_ends_the_gesture_with_cancel() {
        let mut t = Tracker::default();
        t.feed(&moved(10.0, 10.0), bounds(), over(10.0, 10.0));
        t.feed(&press(TOUCH_DOWN), bounds(), over(10.0, 10.0));
        t.feed(&moved(40.0, 10.0), bounds(), over(40.0, 10.0));
        assert_eq!(t.feed(&release(TOUCH_CANCEL), bounds(), over(40.0, 10.0)), Some(Output::Cancel));
        assert!(!t.is_tracking());
    }

    #[test]
    fn leaving_the_window_cancels_a_mouse_drag_but_not_a_finger() {
        let mut t = Tracker::default();
        t.feed(&moved(10.0, 10.0), bounds(), over(10.0, 10.0));
        t.feed(&press(mouse::Button::Left), bounds(), over(10.0, 10.0));
        assert_eq!(
            t.feed(&Event::Mouse(mouse::Event::CursorLeft), bounds(), over(10.0, 10.0)),
            Some(Output::Cancel)
        );

        let mut t = Tracker::default();
        t.feed(&moved(10.0, 10.0), bounds(), over(10.0, 10.0));
        t.feed(&press(TOUCH_DOWN), bounds(), over(10.0, 10.0));
        assert_eq!(t.feed(&Event::Mouse(mouse::Event::CursorLeft), bounds(), over(10.0, 10.0)), None);
        assert!(t.is_tracking());
    }

    #[test]
    fn the_long_press_deadline_is_only_set_while_a_finger_is_down() {
        let mut t = Tracker::default();
        assert_eq!(t.deadline(), None);
        t.feed(&moved(10.0, 10.0), bounds(), over(10.0, 10.0));
        t.feed(&press(TOUCH_DOWN), bounds(), over(10.0, 10.0));
        assert!(t.deadline().is_some());
        t.feed(&release(TOUCH_UP), bounds(), over(10.0, 10.0));
        assert_eq!(t.deadline(), None);
    }
}
