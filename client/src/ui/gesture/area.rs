//! L'area generica dei gesti: avvolge un widget, lo lascia funzionare com'è e traduce i gesti in
//! messaggi. Serve dove il widget sotto non si muove (la striscia del mini-player che si tira su,
//! il cassetto che si chiude con uno scorrimento).
//!
//! Se `axis` è impostato, solo un trascinamento su quell'asse è suo: sull'altro il gesto è
//! annullato subito e il resto passa al contenuto (la pagina che scorre). Quando il trascinamento è
//! suo, il pulsante sotto non riceve il rilascio come clic.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{Clipboard, Shell, overlay, renderer};
use iced::event::{self, Event};
use iced::{Element, Length, Rectangle, Size, Vector, mouse, window};

use super::recognizer::{Axis, Output};
use super::tracker::Tracker;

type Mapper<'a, Message> = Box<dyn Fn(Output) -> Option<Message> + 'a>;

pub struct Area<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    axis: Option<Axis>,
    on_gesture: Mapper<'a, Message>,
}

/// Avvolge `content` e manda a `on_gesture` ogni gesto riconosciuto.
pub fn area<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    on_gesture: impl Fn(Output) -> Option<Message> + 'a,
) -> Area<'a, Message, Theme, Renderer> {
    Area { content: content.into(), axis: None, on_gesture: Box::new(on_gesture) }
}

impl<Message, Theme, Renderer> Area<'_, Message, Theme, Renderer> {
    /// Solo i trascinamenti su questo asse sono dell'area.
    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = Some(axis);
        self
    }
}

#[derive(Debug, Default)]
struct State {
    tracker: Tracker,
    /// Il trascinamento è dell'area: il contenuto non vede più il mouse.
    claimed: bool,
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Area<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<State>()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(&self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, op: &mut dyn Operation) {
        self.content.as_widget().operate(&mut tree.children[0], layout, renderer, op);
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
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        let was_claimed = state.claimed;
        if let Some(out) = state.tracker.feed(&event, bounds, cursor) {
            match out {
                Output::PanStart { axis, .. } => {
                    if self.axis.is_none_or(|wanted| wanted == axis) {
                        state.claimed = true;
                    } else {
                        // Un altro asse: scorre la pagina, non è un gesto dell'area.
                        state.tracker.cancel();
                    }
                }
                Output::PanEnd { .. } | Output::Fling { .. } | Output::Cancel => state.claimed = false,
                _ => {}
            }
            if let Some(message) = (self.on_gesture)(out) {
                shell.publish(message);
            }
        }
        if let Some(at) = state.tracker.deadline() {
            shell.request_redraw(window::RedrawRequest::At(at));
        }

        let released = matches!(
            event,
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                | Event::Mouse(mouse::Event::CursorLeft)
        );
        if was_claimed || state.claimed {
            // Il rilascio arriva al contenuto come se il cursore fosse altrove: il pulsante sotto si
            // libera senza scattare.
            if released {
                let _ = self.content.as_widget_mut().on_event(
                    &mut tree.children[0],
                    event,
                    layout,
                    mouse::Cursor::Unavailable,
                    renderer,
                    clipboard,
                    shell,
                    viewport,
                );
            }
            return event::Status::Captured;
        }
        self.content.as_widget_mut().on_event(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        )
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
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, translation)
    }
}

impl<'a, Message, Theme, Renderer> From<Area<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(area: Area<'a, Message, Theme, Renderer>) -> Self {
        Element::new(area)
    }
}
