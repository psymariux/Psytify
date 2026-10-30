//! La manina al posto delle quattro frecce sugli slider.
//!
//! Lo `slider` di Iced chiede il cursore «afferra» (`Grab`, `Grabbing` mentre si trascina). Su
//! Windows winit lo traduce nel cursore di spostamento, le quattro frecce, che su una barra del
//! volume o dell'avanzamento fa pensare a una finestra da muovere. Questo involucro lascia fare
//! tutto al widget che contiene e cambia solo il cursore: `Grab` e `Grabbing` diventano la manina.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{Clipboard, Shell, overlay, renderer};
use iced::event::{self, Event};
use iced::{Element, Length, Rectangle, Size, Vector, mouse};

pub struct Pointer<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
}

/// Mostra `content` com'è, ma con la manina dove Iced metterebbe «afferra».
pub fn pointer<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Pointer<'a, Message, Theme, Renderer> {
    Pointer { content: content.into() }
}

/// Il cursore da mostrare per quello che chiede il widget interno.
fn tame(interaction: mouse::Interaction) -> mouse::Interaction {
    match interaction {
        mouse::Interaction::Grab | mouse::Interaction::Grabbing => mouse::Interaction::Pointer,
        other => other,
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Pointer<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
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

    fn operate(
        &self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content.as_widget().operate(&mut tree.children[0], layout, renderer, operation);
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
        tame(self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        ))
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

impl<'a, Message, Theme, Renderer> From<Pointer<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(pointer: Pointer<'a, Message, Theme, Renderer>) -> Self {
        Element::new(pointer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grabbing_becomes_a_pointer_and_the_rest_is_left_alone() {
        assert_eq!(tame(mouse::Interaction::Grab), mouse::Interaction::Pointer);
        assert_eq!(tame(mouse::Interaction::Grabbing), mouse::Interaction::Pointer);
        assert_eq!(tame(mouse::Interaction::Text), mouse::Interaction::Text);
        assert_eq!(tame(mouse::Interaction::None), mouse::Interaction::None);
    }
}
