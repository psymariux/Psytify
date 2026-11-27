//! Prova dei widget dei gesti senza finestra: un renderer vuoto, un albero e gli eventi di Iced
//! mandati a mano. Solo per i test.

use std::time::{Duration, Instant};

use iced::advanced::Shell;
use iced::advanced::clipboard;
use iced::advanced::layout::{Layout, Limits, Node};
use iced::advanced::widget::Tree;
use iced::event::{Event, Status};
use iced::{Element, Point, Rectangle, Size, mouse, window};

pub type El<'a, M> = Element<'a, M, (), ()>;

pub struct Rig<'a, M> {
    pub element: El<'a, M>,
    pub tree: Tree,
    pub node: Node,
    pub messages: Vec<M>,
}

impl<'a, M: Clone> Rig<'a, M> {
    pub fn new(element: impl Into<El<'a, M>>, size: Size) -> Self {
        let element = element.into();
        let mut tree = Tree::new(&element);
        let node = element.as_widget().layout(&mut tree, &(), &Limits::new(Size::ZERO, size));
        Self { element, tree, node, messages: Vec::new() }
    }

    pub fn bounds(&self) -> Rectangle {
        Layout::new(&self.node).bounds()
    }

    /// Manda un evento con il cursore in `at` (o assente).
    pub fn send(&mut self, event: Event, at: Option<Point>) -> Status {
        let cursor = at.map_or(mouse::Cursor::Unavailable, mouse::Cursor::Available);
        let viewport = self.bounds();
        let mut shell = Shell::new(&mut self.messages);
        self.element.as_widget_mut().on_event(
            &mut self.tree,
            event,
            Layout::new(&self.node),
            cursor,
            &(),
            &mut clipboard::Null,
            &mut shell,
            &viewport,
        )
    }

    pub fn moved(&mut self, x: f32, y: f32) -> Status {
        self.send(
            Event::Mouse(mouse::Event::CursorMoved { position: Point::new(x, y) }),
            Some(Point::new(x, y)),
        )
    }

    pub fn press(&mut self, x: f32, y: f32) -> Status {
        self.moved(x, y);
        self.send(Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)), Some(Point::new(x, y)))
    }

    pub fn release(&mut self, x: f32, y: f32) -> Status {
        self.send(Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)), Some(Point::new(x, y)))
    }

    pub fn frame(&mut self, at: Option<Point>) -> Status {
        self.send(Event::Window(window::Event::RedrawRequested(Instant::now())), at)
    }

    pub fn wait(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// Un contenuto vuoto di quella misura.
pub fn blank<'a, M: 'a>(width: f32, height: f32) -> El<'a, M> {
    iced::widget::Space::new(width, height).into()
}
