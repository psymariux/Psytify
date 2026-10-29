//! La finestra senza cornice di sistema: la barra superiore la trascina, tre pulsanti la riducono,
//! la mettono a schermo intero e la chiudono, e tre maniglie ai bordi la ridimensionano.
//!
//! Iced 0.13 non sa avviare il ridimensionamento del sistema da un bordo, quindi lo facciamo noi:
//! mentre la maniglia è premuta ascoltiamo il mouse e portiamo la finestra alla dimensione
//! corrispondente. Funziona per i bordi destro e basso e per l'angolo, che sono quelli che non
//! spostano l'origine della finestra (su Wayland la posizione non si può cambiare).

use std::time::{Duration, Instant};

use iced::window::Mode;
use iced::{Event, Point, Size, Task, event, mouse, window};

use super::{App, Message};

/// Dimensione minima della finestra: sotto, la barra del player e il pannello non ci stanno.
pub const MIN_SIZE: Size = Size::new(1100.0, 700.0);
/// Due pressioni sulla barra superiore entro questo tempo la ingrandiscono o la riportano com'era.
const DOUBLE_PRESS: Duration = Duration::from_millis(380);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    Right,
    Bottom,
    Corner,
}

/// Un ridimensionamento in corso: la dimensione di partenza e il punto in cui è iniziato il
/// trascinamento (il primo movimento del mouse dopo la pressione).
#[derive(Debug, Clone, Copy)]
pub struct ResizeDrag {
    edge: ResizeEdge,
    start: Size,
    origin: Option<Point>,
}

impl ResizeDrag {
    pub fn new(edge: ResizeEdge, start: Size) -> Self {
        Self { edge, start, origin: None }
    }

    /// La dimensione per il mouse in `at`. Il primo punto fissa l'origine e non cambia nulla.
    pub fn size_for(&mut self, at: Point) -> Option<Size> {
        let Some(origin) = self.origin else {
            self.origin = Some(at);
            return None;
        };
        let width = match self.edge {
            ResizeEdge::Right | ResizeEdge::Corner => self.start.width + at.x - origin.x,
            ResizeEdge::Bottom => self.start.width,
        };
        let height = match self.edge {
            ResizeEdge::Bottom | ResizeEdge::Corner => self.start.height + at.y - origin.y,
            ResizeEdge::Right => self.start.height,
        };
        Some(Size::new(width.max(MIN_SIZE.width), height.max(MIN_SIZE.height)))
    }
}

/// Eventi che servono solo mentre una maniglia è premuta: movimento e rilascio del mouse.
pub fn resize_events(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::ResizeMove(position)),
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Message::ResizeEnd),
        _ => None,
    }
}

impl App {
    fn with_window(&self, task: impl FnOnce(window::Id) -> Task<Message>) -> Task<Message> {
        self.window.map_or_else(Task::none, task)
    }

    /// Trascina la finestra; due pressioni ravvicinate la ingrandiscono o la ripristinano.
    pub(super) fn title_bar_pressed(&mut self) -> Task<Message> {
        if self.fullscreen {
            return Task::none();
        }
        let now = Instant::now();
        let double = self.last_title_press.is_some_and(|before| now.duration_since(before) < DOUBLE_PRESS);
        self.last_title_press = if double { None } else { Some(now) };
        self.with_window(|id| if double { window::toggle_maximize(id) } else { window::drag(id) })
    }

    pub(super) fn minimize(&self) -> Task<Message> {
        self.with_window(|id| window::minimize(id, true))
    }

    pub(super) fn toggle_fullscreen(&mut self) -> Task<Message> {
        self.fullscreen = !self.fullscreen;
        let mode = if self.fullscreen { Mode::Fullscreen } else { Mode::Windowed };
        self.with_window(|id| window::change_mode(id, mode))
    }

    /// Chiude passando da `CloseRequested`, che salva coda, volume e schermata.
    pub(super) fn quit(&self) -> Task<Message> {
        self.with_window(|id| Task::done(Message::CloseRequested(id)))
    }

    pub(super) fn resize_started(&mut self, edge: ResizeEdge) {
        if !self.fullscreen {
            self.resizing = Some(ResizeDrag::new(edge, self.window_size));
        }
    }

    pub(super) fn resize_moved(&mut self, at: Point) -> Task<Message> {
        let Some(size) = self.resizing.as_mut().and_then(|drag| drag.size_for(at)) else {
            return Task::none();
        };
        self.with_window(|id| window::resize(id, size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drag(edge: ResizeEdge) -> ResizeDrag {
        ResizeDrag::new(edge, Size::new(1280.0, 800.0))
    }

    #[test]
    fn the_first_point_only_fixes_the_origin() {
        let mut drag = drag(ResizeEdge::Corner);
        assert_eq!(drag.size_for(Point::new(1278.0, 798.0)), None);
    }

    #[test]
    fn the_corner_follows_the_mouse_on_both_axes() {
        let mut drag = drag(ResizeEdge::Corner);
        drag.size_for(Point::new(1278.0, 798.0));
        assert_eq!(drag.size_for(Point::new(1378.0, 848.0)), Some(Size::new(1380.0, 850.0)));
    }

    #[test]
    fn an_edge_changes_only_its_own_axis() {
        let mut right = drag(ResizeEdge::Right);
        right.size_for(Point::new(1278.0, 400.0));
        assert_eq!(right.size_for(Point::new(1478.0, 100.0)), Some(Size::new(1480.0, 800.0)));
        let mut bottom = drag(ResizeEdge::Bottom);
        bottom.size_for(Point::new(600.0, 798.0));
        assert_eq!(bottom.size_for(Point::new(10.0, 748.0)), Some(Size::new(1280.0, 750.0)));
    }

    #[test]
    fn the_window_never_shrinks_below_the_minimum() {
        let mut drag = drag(ResizeEdge::Corner);
        drag.size_for(Point::new(1278.0, 798.0));
        assert_eq!(drag.size_for(Point::new(100.0, 100.0)), Some(MIN_SIZE));
    }
}
