//! Select (scelta da un elenco): un pulsante secondario con il valore scelto e la freccia in giù, che apre
//! sotto di sé un menu del kit (`MenuPanel`) con le opzioni; quella scelta ha la spunta. Serve dove il
//! client Iced ha una scelta a segmenti o un menu di durate (qualità, timer di spegnimento, preset
//! dell'equalizzatore): la direzione artistica non ha segmenti né pillole.
//!
//! Il menu è del componente: lo stato aperto/chiuso sta in uno stato per id (`use_keyed_state`), così un
//! Select funziona anche dentro una pagina in `.cached()` (il cambio di stato ridisegna solo la vista che
//! lo contiene). Aperto, un velo trasparente su tutta la finestra chiude il menu al clic fuori; Esc lo
//! chiude e riporta il focus sul pulsante. Il velo e il menu stanno sopra il resto (`deferred`, priorità
//! `Z_MENU`), dentro la finestra.

use std::rc::Rc;

use gpui::{
    App, Bounds, Context, ElementId, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Pixels, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, anchored,
    canvas, deferred, div, point, px,
};
use psytify_design::sprite;
use psytify_design::tokens::color::{ICON_ACTIVE, ICON_DISABLED, ICON_IDLE, TEXT_PRIMARY};
use psytify_design::tokens::shape::NOTCH_1;
use psytify_design::tokens::size::CONTROL_H;
use psytify_design::tokens::space::{SPACE_2, SPACE_3, SPACE_4};
use psytify_design::tokens::type_::TYPE_NAV;
use psytify_design::tokens::z::Z_MENU;

use super::button::{ButtonKind, button_look, pressable};
use super::control::{Preview, wire};
use super::menu::{MenuItem, MenuPanel, popup};
use crate::pixel::PixelSprite;
use crate::theme::{Ap, TypeStyle, col, dp};

/// Callback con l'indice dell'opzione scelta.
pub type OnSelect = Rc<dyn Fn(usize, &mut Window, &mut App)>;

struct SelectState {
    open: bool,
    /// Dove sta il pulsante (dall'ultimo disegno): il menu si apre sotto.
    bounds: Bounds<Pixels>,
    menu: FocusHandle,
}

#[derive(IntoElement)]
pub struct Select {
    id: ElementId,
    options: Vec<SharedString>,
    selected: Option<usize>,
    /// Il testo del pulsante quando nessuna opzione è scelta («Personalizzato»).
    placeholder: SharedString,
    min_width: Option<f32>,
    disabled: bool,
    preview: Preview,
    /// Solo per la galleria: il menu disegnato aperto.
    force_open: bool,
    on_select: Option<OnSelect>,
}

impl Select {
    pub fn new(id: impl Into<ElementId>, options: Vec<SharedString>, selected: Option<usize>) -> Self {
        Self {
            id: id.into(),
            options,
            selected,
            placeholder: SharedString::default(),
            min_width: None,
            disabled: false,
            preview: Preview::Live,
            force_open: false,
            on_select: None,
        }
    }

    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = text.into();
        self
    }

    pub fn min_width(mut self, dp: f32) -> Self {
        self.min_width = Some(dp);
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

    /// La galleria lo mostra aperto senza clic.
    pub fn open(mut self, open: bool) -> Self {
        self.force_open = open;
        self
    }

    pub fn on_select(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Select {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx: &mut Context<SelectState>| {
            SelectState { open: false, bounds: Bounds::default(), menu: cx.focus_handle() }
        });
        let (open, bounds, menu_focus) = {
            let s = state.read(cx);
            (s.open || self.force_open, s.bounds, s.menu.clone())
        };
        let wired = wire(&self.id, self.disabled, self.preview, window, cx);
        let trigger_focus = wired.focus.clone();
        let look = button_look(ButtonKind::Secondary, wired.visual, self.disabled, open, false);
        let text = look.text.unwrap_or(TEXT_PRIMARY);
        let icon_tint = if self.disabled {
            ICON_DISABLED
        } else if open || wired.visual.hovered {
            ICON_ACTIVE
        } else {
            ICON_IDLE
        };
        let value = self
            .selected
            .and_then(|ix| self.options.get(ix).cloned())
            .unwrap_or_else(|| self.placeholder.clone());
        let mut content = div()
            .relative()
            .flex()
            .items_center()
            .justify_between()
            .gap(dp(SPACE_3))
            .px(dp(SPACE_4))
            .type_role(TYPE_NAV)
            .text_color(col(text))
            .child(div().whitespace_nowrap().child(value))
            .child(PixelSprite::new(sprite::ICON_CHEVRON_DOWN).tint(icon_tint));
        if let Some(width) = self.min_width {
            content = content.min_w(dp(width));
        }
        // Il pulsante registra dove sta, per aprire il menu proprio sotto.
        let measure = {
            let state = state.clone();
            canvas(move |bounds, _, cx| state.update(cx, |s, _| s.bounds = bounds), |_, (), _, _| {})
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .size_full()
        };
        let mut trigger = pressable(wired, (None, dp(CONTROL_H)), NOTCH_1, look, ap, content).child(measure);
        if !self.disabled {
            let toggle = state.clone();
            let menu = menu_focus.clone();
            trigger = trigger.on_click(move |_, window, cx| {
                let opened = toggle.update(cx, |s, cx| {
                    s.open = !s.open;
                    cx.notify();
                    s.open
                });
                if opened {
                    window.focus(&menu, cx);
                }
            });
        }

        let mut root = div().relative().flex_none().child(trigger);
        if open && !self.disabled {
            let close_veil = state.clone();
            let size = window.viewport_size();
            // Un clic fuori dal menu lo chiude e non arriva a ciò che c'è sotto.
            let veil = deferred(
                anchored().position(point(px(0.0), px(0.0))).child(
                    div()
                        .w(size.width)
                        .h(size.height)
                        .occlude()
                        .on_mouse_down(MouseButton::Left, {
                            let close = close_veil.clone();
                            move |_, _, cx| close.update(cx, close_menu)
                        })
                        .on_mouse_down(MouseButton::Right, move |_, _, cx| close_veil.update(cx, close_menu)),
                ),
            )
            .with_priority(usize::from(Z_MENU));
            let mut panel = MenuPanel::new();
            for (ix, label) in self.options.iter().enumerate() {
                let choose = state.clone();
                let on_select = self.on_select.clone();
                let back = trigger_focus.clone();
                panel = panel.item(
                    MenuItem::new(
                        (self.id.clone(), SharedString::from(format!("option-{ix}"))),
                        label.clone(),
                    )
                    .checked(self.selected == Some(ix))
                    .on_click(move |_, window, cx| {
                        choose.update(cx, close_menu);
                        window.focus(&back, cx);
                        if let Some(f) = &on_select {
                            f(ix, window, cx);
                        }
                    }),
                );
            }
            let escape = state.clone();
            let back = trigger_focus;
            let menu = div()
                .track_focus(&menu_focus)
                .occlude()
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        escape.update(cx, close_menu);
                        window.focus(&back, cx);
                        cx.stop_propagation();
                    }
                })
                .child(panel);
            let at = point(bounds.origin.x, bounds.origin.y + bounds.size.height + dp(SPACE_2));
            root = root.child(veil).child(popup(at, menu));
        }
        root
    }
}

fn close_menu(state: &mut SelectState, cx: &mut Context<SelectState>) {
    if state.open {
        state.open = false;
        cx.notify();
    }
}
