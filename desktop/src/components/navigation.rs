//! NavigationItem (barra laterale) e Tabs (in pagina).
//!
//! NavigationItem: 32 dp by default, 12 AP icon + `TYPE_NAV` label; hover eases the tone in and nudges the
//! icon 1 AP right; selected = the `MARKER_W` lime bar on the left (it grows from the centre in three frames
//! when the selection moves), filled icon, primary label, no fill. Tabs: sottolineatura di 2 AP
//! `WARM_WHITE` sulla linea di base `LINE_SUBTLE`, niente lime.

use gpui::{
    App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled,
    Window, div,
};
use psytify_design::tokens::color::{
    ICON_ACTIVE, ICON_DISABLED, ICON_IDLE, LIME, PURE_WHITE, SURFACE_RAISED, SURFACE_SELECTED, TEXT_DISABLED,
    TEXT_PRIMARY, TEXT_SECONDARY, WARM_WHITE,
};
use psytify_design::tokens::motion::{MOTION_FAST, STEP_FRAMES};
use psytify_design::tokens::shape::MARKER_W;
use psytify_design::tokens::size::NAV_ITEM_H;
use psytify_design::tokens::space::{SPACE_3, SPACE_4};
use psytify_design::tokens::type_::TYPE_NAV;

use super::badge::{Badge, BadgeKind};
use super::control::{OnClick, Preview, wire, with_click};
use super::separator::Separator;
use super::tooltip::with_tooltip;
use crate::motion::{ap_round, follow, steps};
use crate::pixel::PixelSprite;
use crate::pixel::shape::inset_ring;
use crate::theme::{Ap, TypeStyle, col, dp, fade, to_dp};

#[derive(IntoElement)]
pub struct NavigationItem {
    id: ElementId,
    label: SharedString,
    icon: &'static str,
    icon_selected: Option<&'static str>,
    selected: bool,
    disabled: bool,
    count: Option<u32>,
    rail: bool,
    height: f32,
    preview: Preview,
    on_click: Option<OnClick>,
}

impl NavigationItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, icon: &'static str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon,
            icon_selected: None,
            selected: false,
            disabled: false,
            count: None,
            rail: false,
            height: NAV_ITEM_H,
            preview: Preview::Live,
            on_click: None,
        }
    }

    /// Variante piena dell'icona, usata quando la voce è selezionata.
    pub fn icon_selected(mut self, icon: &'static str) -> Self {
        self.icon_selected = Some(icon);
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

    pub fn count(mut self, count: u32) -> Self {
        self.count = Some(count);
        self
    }

    /// Rail compressa: solo icona.
    pub fn rail(mut self, rail: bool) -> Self {
        self.rail = rail;
        self
    }

    pub fn preview(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    /// Row height in dp (`NAV_ITEM_H` by default).
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }
}

impl RenderOnce for NavigationItem {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let wired = wire(&self.id, self.disabled, self.preview, window, cx);
        let v = wired.visual;
        let (icon_tint, text) = if self.disabled {
            (ICON_DISABLED, TEXT_DISABLED)
        } else if self.selected || v.hovered || v.pressed {
            (ICON_ACTIVE, TEXT_PRIMARY)
        } else {
            (ICON_IDLE, TEXT_SECONDARY)
        };
        let fill = if self.selected || self.disabled {
            None
        } else if v.pressed {
            Some(col(SURFACE_SELECTED))
        } else if v.hover_t > 0.0 {
            Some(fade(SURFACE_RAISED, v.hover_t))
        } else {
            None
        };
        // The selection marker grows from the centre to full height in three frames when the selection
        // moves here (and shrinks the same way when it leaves).
        let marker_id = ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "marker".into());
        let grow = follow(marker_id, if self.selected { 1.0 } else { 0.0 }, MOTION_FAST, window, cx);
        let marker_frames = steps(grow, STEP_FRAMES);
        // The icon nudges 1 AP right on hover.
        let nudge = ap.n(ap_round(v.hover_t));
        let icon = if self.selected { self.icon_selected.unwrap_or(self.icon) } else { self.icon };
        let mut row = wired
            .element
            .relative()
            .h(dp(self.height))
            .w_full()
            .flex()
            .items_center()
            .gap(dp(SPACE_3))
            .px(dp(SPACE_3))
            .text_color(col(text))
            .type_role(TYPE_NAV)
            .child(div().relative().left(nudge).flex_none().child(PixelSprite::new(icon).tint(icon_tint)));
        let label = self.label.clone();
        if self.rail {
            row = row.justify_center();
        } else {
            row = row.child(div().flex_1().truncate().child(self.label));
            if let Some(count) = self.count {
                row = row.child(Badge::new(count.to_string(), BadgeKind::Count));
            }
        }
        if let Some(color) = fill {
            row = row.bg(color);
        }
        if marker_frames > 0 {
            // A 2 AP lime bar on the left, 1 AP in from the top and bottom at full height, centred.
            let full = (self.height - 2.0 * to_dp(ap.one())).max(0.0);
            let h = full * marker_frames as f32 / STEP_FRAMES as f32;
            row = row.child(
                div()
                    .absolute()
                    .left(dp(0.0))
                    .top(dp((self.height - h) / 2.0))
                    .h(dp(h))
                    .w(ap.n(MARKER_W))
                    .bg(col(LIME)),
            );
        }
        row = row.child(inset_ring(col(PURE_WHITE), wired.focus.clone(), v.force_ring));
        let row = with_click(row, if self.disabled { None } else { self.on_click });
        // On the rail the label is the tooltip.
        if self.rail { with_tooltip(row, label, None) } else { row }
    }
}

/// Callback di selezione di una scheda (indice).
type OnSelect = std::rc::Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// Una scheda.
#[derive(Clone)]
pub struct Tab {
    pub id: ElementId,
    pub label: SharedString,
    pub disabled: bool,
}

impl Tab {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self { id: id.into(), label: label.into(), disabled: false }
    }
}

/// Riga di schede; `selected` è l'indice attivo. `preview` forza lo stato della scheda `preview_ix`.
#[derive(IntoElement)]
pub struct Tabs {
    tabs: Vec<Tab>,
    selected: usize,
    previews: Vec<(usize, Preview)>,
    on_select: Option<OnSelect>,
}

impl Tabs {
    pub fn new(tabs: Vec<Tab>, selected: usize) -> Self {
        Self { tabs, selected, previews: Vec::new(), on_select: None }
    }

    pub fn preview(mut self, index: usize, preview: Preview) -> Self {
        self.previews.push((index, preview));
        self
    }

    pub fn on_select(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(std::rc::Rc::new(handler));
        self
    }
}

impl RenderOnce for Tabs {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let mut row = div().flex().items_stretch().h(dp(NAV_ITEM_H));
        for (ix, tab) in self.tabs.iter().enumerate() {
            let preview = self.previews.iter().find(|(i, _)| *i == ix).map_or(Preview::Live, |(_, p)| *p);
            let wired = wire(&tab.id, tab.disabled, preview, window, cx);
            let v = wired.visual;
            let selected = ix == self.selected;
            let color = if tab.disabled {
                TEXT_DISABLED
            } else if selected || v.hovered || v.pressed {
                TEXT_PRIMARY
            } else {
                TEXT_SECONDARY
            };
            let mut cell = wired
                .element
                .relative()
                .px(dp(SPACE_4))
                .flex()
                .items_center()
                .type_role(TYPE_NAV)
                .text_color(col(color))
                .child(div().relative().child(tab.label.clone()));
            if selected {
                // Sottolineatura larga quanto l'etichetta, sopra la linea di base.
                cell = cell.child(
                    div()
                        .absolute()
                        .left(dp(SPACE_4))
                        .right(dp(SPACE_4))
                        .bottom(-ap.one())
                        .h(ap.n(2))
                        .bg(col(WARM_WHITE)),
                );
            }
            cell = cell.child(inset_ring(col(PURE_WHITE), wired.focus.clone(), v.force_ring));
            let on_select = self.on_select.clone();
            let cell = match (on_select, tab.disabled) {
                (Some(f), false) => cell.on_click(move |_, w, c| f(ix, w, c)),
                _ => cell,
            };
            row = row.child(cell);
        }
        div().relative().flex().flex_col().child(row).child(Separator::horizontal())
    }
}
