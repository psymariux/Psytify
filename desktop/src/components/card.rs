//! Card (`SURFACE_RAISED`, notch 2, padding `SPACE_5`) and Panel (chrome regions).
//! The Card has no border or shadow at rest; if clickable it has hover, pressed, focus and selected states.

use gpui::{AnyElement, App, ElementId, IntoElement, ParentElement, RenderOnce, Styled, Window, div};
use psytify_design::tokens::color::{
    CONTROL_BORDER_HOVER, LINE_STRONG, PURE_WHITE, SURFACE_CHROME, SURFACE_RAISED, SURFACE_SELECTED,
    TEXT_DISABLED, TEXT_PRIMARY, TEXT_TERTIARY,
};
use psytify_design::tokens::shape::NOTCH_2;
use psytify_design::tokens::space::{SPACE_3, SPACE_5};
use psytify_design::tokens::type_::TYPE_LABEL;

use super::control::{OnClick, Preview, wire, with_click};
use super::separator::Separator;
use crate::pixel::shape::{focus_ring, notch_box, static_ring};
use crate::theme::{Ap, TypeStyle, cased, col, dp};

#[derive(IntoElement)]
pub struct Card {
    id: Option<ElementId>,
    selected: bool,
    disabled: bool,
    preview: Preview,
    on_click: Option<OnClick>,
    children: Vec<AnyElement>,
}

impl Card {
    pub fn new() -> Self {
        Self {
            id: None,
            selected: false,
            disabled: false,
            preview: Preview::Live,
            on_click: None,
            children: Vec::new(),
        }
    }

    /// Makes the card clickable (hover, pressed, focus).
    pub fn interactive(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn preview(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }
}

impl Default for Card {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for Card {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Card {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let body = div().relative().p(dp(SPACE_5)).text_color(col(TEXT_PRIMARY));
        let Some(id) = self.id else {
            return body
                .child(notch_box(NOTCH_2, Some(col(SURFACE_RAISED)), None))
                .child(div().relative().children(self.children))
                .into_any_element();
        };
        let wired = wire(&id, self.disabled, self.preview, window, cx);
        let v = wired.visual;
        let fill = if v.pressed { SURFACE_SELECTED } else { SURFACE_RAISED };
        let border = if self.selected {
            Some(CONTROL_BORDER_HOVER)
        } else if v.hovered {
            Some(LINE_STRONG)
        } else {
            None
        };
        let ring = if v.force_ring {
            static_ring(NOTCH_2, col(PURE_WHITE), ap)
        } else {
            focus_ring(NOTCH_2, col(PURE_WHITE), wired.focus.clone(), ap)
        };
        let element = wired
            .element
            .relative()
            .p(dp(SPACE_5))
            .text_color(col(if self.disabled { TEXT_DISABLED } else { TEXT_PRIMARY }))
            .child(notch_box(NOTCH_2, Some(col(fill)), border.map(col)))
            .child(div().relative().children(self.children))
            .child(ring);
        with_click(element, if self.disabled { None } else { self.on_click }).into_any_element()
    }
}

/// Chrome region: `SURFACE_CHROME` fill, 40 dp header with a `TYPE_LABEL` title and
/// actions on the right, 1 AP separator. No radius, no shadow.
#[derive(IntoElement)]
pub struct Panel {
    title: Option<gpui::SharedString>,
    actions: Vec<AnyElement>,
    children: Vec<AnyElement>,
}

impl Panel {
    pub fn new() -> Self {
        Self { title: None, actions: Vec::new(), children: Vec::new() }
    }

    pub fn title(mut self, title: impl Into<gpui::SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl Default for Panel {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for Panel {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Panel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let header = self.title.map(|title| {
            div()
                .flex()
                .flex_col()
                .flex_none()
                .child(
                    div()
                        .h(dp(40.0))
                        .px(dp(SPACE_5))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .type_role(TYPE_LABEL)
                                .text_color(col(TEXT_TERTIARY))
                                .child(cased(&title, TYPE_LABEL)),
                        )
                        .child(div().flex().items_center().gap(dp(SPACE_3)).children(self.actions)),
                )
                .child(Separator::horizontal())
        });
        div()
            .flex()
            .flex_col()
            .bg(col(SURFACE_CHROME))
            .children(header)
            .child(div().flex_1().min_h_0().children(self.children))
    }
}
