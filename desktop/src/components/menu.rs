//! Menu and context menu: a panel `MENU_W` wide with 32 dp entries. The panel (`MenuPanel`) is pure
//! drawing; `popup` anchors it to the pointer, above everything, inside the window.

use gpui::{
    Anchor, AnyElement, App, ElementId, IntoElement, ParentElement, Point, RenderOnce, SharedString, Styled,
    Window, anchored, deferred, div,
};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    ICON_ACTIVE, ICON_DISABLED, ICON_IDLE, LINE_STRONG, SHADOW_FLOAT, SURFACE_RAISED, SURFACE_SELECTED,
    TEXT_DISABLED, TEXT_ERROR, TEXT_PRIMARY, TEXT_TERTIARY,
};
use psytify_design::tokens::shape::{NOTCH_2, SHADOW_FLOAT_OFFSET};
use psytify_design::tokens::size::{MENU_ITEM_H, MENU_W};
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_4};
use psytify_design::tokens::type_::{TYPE_META, TYPE_NAV};

use super::control::{OnClick, Preview, wire, with_click};
use super::separator::Separator;
use crate::pixel::PixelSprite;
use crate::pixel::shape::{hard_shadow, inset_ring, notch_box};
use crate::theme::{TypeStyle, col, dp};

#[derive(IntoElement)]
pub struct MenuItem {
    id: ElementId,
    label: SharedString,
    icon: Option<&'static str>,
    shortcut: Option<SharedString>,
    checked: bool,
    disabled: bool,
    destructive: bool,
    submenu: bool,
    preview: Preview,
    on_click: Option<OnClick>,
}

impl MenuItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            shortcut: None,
            checked: false,
            disabled: false,
            destructive: false,
            submenu: false,
            preview: Preview::Live,
            on_click: None,
        }
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }

    pub fn submenu(mut self, submenu: bool) -> Self {
        self.submenu = submenu;
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

impl RenderOnce for MenuItem {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let wired = wire(&self.id, self.disabled, self.preview, window, cx);
        let v = wired.visual;
        let lit = v.hovered || v.pressed || v.force_ring;
        let (icon_tint, text) = if self.disabled {
            (ICON_DISABLED, TEXT_DISABLED)
        } else if self.destructive {
            (ICON_IDLE, TEXT_ERROR)
        } else if lit {
            (ICON_ACTIVE, TEXT_PRIMARY)
        } else {
            (ICON_IDLE, TEXT_PRIMARY)
        };
        // The icon column is always there, so the labels stay aligned.
        let icon_name = if self.checked { Some(sprite::ICON_CHECK) } else { self.icon };
        let icon_col =
            div().w(dp(24.0)).flex_none().children(icon_name.map(|n| PixelSprite::new(n).tint(icon_tint)));
        let mut row = wired
            .element
            .relative()
            .h(dp(MENU_ITEM_H))
            .w_full()
            .flex()
            .items_center()
            .gap(dp(SPACE_3))
            .px(dp(SPACE_4))
            .text_color(col(text))
            .type_role(TYPE_NAV)
            .child(icon_col)
            .child(div().flex_1().truncate().child(self.label));
        if let Some(shortcut) = self.shortcut {
            row = row.child(div().type_role(TYPE_META).text_color(col(TEXT_TERTIARY)).child(shortcut));
        }
        if self.submenu {
            row = row.child(PixelSprite::new(sprite::ICON_CHEVRON_RIGHT).tint(icon_tint));
        }
        if lit && !self.disabled {
            row = row.bg(col(SURFACE_SELECTED));
        }
        row =
            row.child(inset_ring(col(psytify_design::tokens::color::PURE_WHITE), wired.focus.clone(), false));
        with_click(row, if self.disabled { None } else { self.on_click })
    }
}

/// Pannello del menu: fill `SURFACE_RAISED`, bordo `LINE_STRONG`, tacca 2, ombra dura di 2 AP.
#[derive(IntoElement)]
pub struct MenuPanel {
    children: Vec<AnyElement>,
}

impl MenuPanel {
    pub fn new() -> Self {
        Self { children: Vec::new() }
    }

    pub fn item(mut self, item: MenuItem) -> Self {
        self.children.push(item.into_any_element());
        self
    }

    /// Any element in the panel (a list that scrolls its entries).
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    /// Separatore fra gruppi: 1 AP con `SPACE_2` sopra e sotto.
    pub fn separator(mut self) -> Self {
        self.children.push(div().py(dp(SPACE_2)).child(Separator::horizontal()).into_any_element());
        self
    }
}

impl Default for MenuPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for MenuPanel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .relative()
            .w(dp(MENU_W))
            .py(dp(SPACE_2))
            .flex()
            .flex_col()
            .child(hard_shadow(NOTCH_2, SHADOW_FLOAT_OFFSET.0, col(SHADOW_FLOAT)))
            .child(notch_box(NOTCH_2, Some(col(SURFACE_RAISED)), Some(col(LINE_STRONG))))
            .children(self.children)
    }
}

/// Anchors an element at the point `at` (top-left corner), above the rest and inside the window.
pub fn popup(at: Point<gpui::Pixels>, content: impl IntoElement) -> impl IntoElement {
    deferred(
        anchored()
            .position(at)
            .anchor(Anchor::TopLeft)
            .snap_to_window_with_margin(gpui::Edges::all(dp(8.0)))
            .child(content),
    )
    .with_priority(psytify_design::tokens::z::Z_MENU as usize)
}
