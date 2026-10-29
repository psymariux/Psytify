//! Il trascinamento col dito sulla barra di avanzamento del telefono.
//!
//! Su Android i gesti arrivano ai widget già tradotti (`android/vendor/iced_winit`, `touch.rs`): un
//! tocco breve è un clic, ma un trascinamento diventa una rotella in pixel, e lo `slider` di Iced
//! la ignora. Questo strato invisibile sta sopra la barra (`stack!`): quando il dito scorre in
//! orizzontale sulla barra, la posizione segue il dito; quando il dito si stacca, il brano salta lì.
//! Tutto il resto (il tocco breve, il mouse) passa alla barra sotto.
//!
//! Il dito che si stacca si riconosce da due segni: il puntatore che se ne va (`CursorLeft`, dopo
//! un trascinamento lento) o una rotella senza movimento del puntatore prima (l'inerzia, che parte
//! solo a dito alzato).

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, renderer};
use iced::event::{self, Event};
use iced::{Element, Length, Rectangle, Size, mouse};

pub struct TouchSeek<Message> {
    on_change: Box<dyn Fn(f32) -> Message>,
    on_release: Message,
}

/// Lo strato che rende trascinabile col dito la barra sotto: `on_change` riceve la frazione sotto
/// il dito, `on_release` arriva quando il dito si stacca.
pub fn touch_seek<Message>(
    on_change: impl Fn(f32) -> Message + 'static,
    on_release: Message,
) -> TouchSeek<Message> {
    TouchSeek { on_change: Box::new(on_change), on_release }
}

#[derive(Debug, Default)]
struct State {
    /// Il dito sta trascinando la barra.
    dragging: bool,
    /// Il puntatore si è mosso dall'ultima rotella: il dito è ancora giù.
    moved: bool,
}

impl<Message: Clone, Theme, Renderer: renderer::Renderer> Widget<Message, Theme, Renderer>
    for TouchSeek<Message>
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&self, _tree: &mut Tree, _renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) -> event::Status {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        let fraction = |x: f32| ((x - bounds.x) / bounds.width.max(1.0)).clamp(0.0, 1.0);
        match event {
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                state.moved = true;
                if state.dragging {
                    shell.publish((self.on_change)(fraction(position.x)));
                    return event::Status::Captured;
                }
                event::Status::Ignored
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta: mouse::ScrollDelta::Pixels { x, y } }) => {
                let moved = std::mem::take(&mut state.moved);
                if state.dragging {
                    if !moved {
                        state.dragging = false;
                        shell.publish(self.on_release.clone());
                    }
                    return event::Status::Captured;
                }
                match cursor.position_over(bounds) {
                    Some(at) if moved && x.abs() > y.abs() => {
                        state.dragging = true;
                        shell.publish((self.on_change)(fraction(at.x)));
                        event::Status::Captured
                    }
                    _ => event::Status::Ignored,
                }
            }
            Event::Mouse(mouse::Event::CursorLeft | mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.dragging =>
            {
                state.dragging = false;
                shell.publish(self.on_release.clone());
                event::Status::Ignored
            }
            _ => event::Status::Ignored,
        }
    }
}

impl<'a, Message: Clone + 'a, Theme: 'a, Renderer: renderer::Renderer + 'a> From<TouchSeek<Message>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(seek: TouchSeek<Message>) -> Self {
        Element::new(seek)
    }
}
