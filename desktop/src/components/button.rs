//! Button (primary, secondary, ghost, destructive) e la base «pressabile» con ledge dura che usano
//! anche IconButton e il play del trasporto.
//!
//! RetroUI: il pulsante ha una ledge dura di 1 AP sotto e a destra; premuto scende di 1 AP e la
//! ledge sparisce (istantaneo, come un tasto). L'hover alza il riempimento o schiarisce il bordo e
//! non sposta mai nulla.

use gpui::{
    App, Div, ElementId, IntoElement, ParentElement, Pixels, RenderOnce, SharedString, Stateful, Styled,
    Window, div, px,
};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    CONTROL_BORDER, CONTROL_BORDER_HOVER, ERROR, ICON_ACTIVE, LEDGE_PRIMARY, LEDGE_SECONDARY, LINE_STRONG,
    PURE_WHITE, Rgba, STONE, SURFACE_RAISED, SURFACE_SELECTED, TEXT_DISABLED, TEXT_ON_LIGHT, TEXT_PRIMARY,
    TEXT_SECONDARY, WARM_WHITE,
};
use psytify_design::tokens::shape::NOTCH_1;
use psytify_design::tokens::size::{CONTROL_H, CONTROL_H_LARGE};
use psytify_design::tokens::space::{SPACE_3, SPACE_5};
use psytify_design::tokens::type_::TYPE_BUTTON;

use super::control::{OnClick, Preview, Visual, Wired, centered, wire, with_click};
use crate::pixel::PixelSprite;
use crate::pixel::shape::{focus_ring, ledge, notch_box, static_ring};
use crate::theme::{Ap, TypeStyle, cased, col, dp};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    /// Solo nei dialoghi: primario con riempimento `ERROR`.
    Destructive,
}

/// Aspetto di un controllo con ledge in un dato stato.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub fill: Option<Rgba>,
    pub border: Option<Rgba>,
    /// Colore di etichetta e icona.
    pub text: Option<Rgba>,
    pub ledge: Option<Rgba>,
    /// Premuto: il corpo scende di 1 AP.
    pub shifted: bool,
}

/// Aspetto del pulsante per tipo e stato (tabella della sezione 1 delle linee guida).
pub fn button_look(kind: ButtonKind, v: Visual, disabled: bool, selected: bool, inert: bool) -> Look {
    let pressed = v.pressed && !inert;
    let hovered = v.hovered && !inert;
    match kind {
        ButtonKind::Primary | ButtonKind::Destructive => {
            let (rest_fill, ledge) =
                if kind == ButtonKind::Primary { (WARM_WHITE, LEDGE_PRIMARY) } else { (ERROR, STONE) };
            if disabled {
                return Look { fill: Some(SURFACE_SELECTED), text: Some(TEXT_DISABLED), ..Look::default() };
            }
            let fill = if hovered && kind == ButtonKind::Primary { PURE_WHITE } else { rest_fill };
            Look {
                fill: Some(if inert { SURFACE_SELECTED } else { fill }),
                text: Some(if inert { TEXT_PRIMARY } else { TEXT_ON_LIGHT }),
                ledge: if pressed || inert { None } else { Some(ledge) },
                shifted: pressed || inert,
                border: None,
            }
        }
        ButtonKind::Secondary => {
            if disabled {
                return Look { border: Some(LINE_STRONG), text: Some(TEXT_DISABLED), ..Look::default() };
            }
            let border = if hovered || selected { CONTROL_BORDER_HOVER } else { CONTROL_BORDER };
            let fill = if pressed {
                Some(SURFACE_SELECTED)
            } else if hovered {
                Some(SURFACE_RAISED)
            } else {
                None
            };
            Look {
                fill,
                border: Some(border),
                text: Some(TEXT_PRIMARY),
                ledge: if pressed || inert { None } else { Some(LEDGE_SECONDARY) },
                shifted: pressed || inert,
            }
        }
        ButtonKind::Ghost => {
            if disabled {
                return Look { text: Some(TEXT_DISABLED), ..Look::default() };
            }
            let fill = if pressed {
                Some(SURFACE_SELECTED)
            } else if hovered {
                Some(SURFACE_RAISED)
            } else {
                None
            };
            Look {
                fill,
                text: Some(if hovered || pressed { TEXT_PRIMARY } else { TEXT_SECONDARY }),
                ..Look::default()
            }
        }
    }
}

/// Costruisce l'elemento «pressabile»: ledge dietro, corpo a tacca che scende da premuto, contenuto,
/// anello di focus. `size` è la misura del controllo (larghezza opzionale: senza, segue il contenuto).
pub fn pressable(
    wired: Wired,
    size: (Option<Pixels>, Pixels),
    notch: u32,
    look: Look,
    ap: Ap,
    content: impl IntoElement,
) -> Stateful<Div> {
    let Wired { element, visual, focus } = wired;
    let ring = if visual.force_ring {
        static_ring(notch, col(PURE_WHITE), ap)
    } else {
        focus_ring(notch, col(PURE_WHITE), focus, ap)
    };
    let (width, height) = size;
    let mut body = div()
        .relative()
        .h(height)
        .flex()
        .items_center()
        .justify_center()
        .text_color(col(look.text.unwrap_or(TEXT_PRIMARY)))
        .child(notch_box(notch, look.fill.map(col), look.border.map(col)));
    if look.shifted {
        body = body.top(ap.one()).left(ap.one());
    }
    let body = match width {
        Some(w) => body.w(w),
        None => body,
    };
    let mut outer = element.relative().h(height).flex_none();
    if let Some(w) = width {
        outer = outer.w(w);
    }
    if let Some(ledge_color) = look.ledge {
        outer = outer.child(ledge(notch, col(ledge_color)));
    }
    outer.child(body.child(content)).child(ring)
}

/// Pulsante con etichetta.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    kind: ButtonKind,
    icon: Option<&'static str>,
    large: bool,
    selected: bool,
    loading: bool,
    disabled: bool,
    preview: Preview,
    min_width: Option<f32>,
    /// Larghezza fissa (i pulsanti a tutta colonna del modulo di accesso), contenuto al centro.
    width: Option<f32>,
    /// Il marchio vettoriale di un provider di accesso, a 20 dp: l'unica eccezione alla regola dei pixel
    /// (la direzione artistica, «Sign-in logos»). Monocromo, tinto come l'etichetta.
    mark: Option<&'static [u8]>,
    on_click: Option<OnClick>,
}

/// Il lato del marchio di un provider (direzione artistica: 20 dp).
const MARK_SIDE: f32 = 20.0;

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind: ButtonKind::Secondary,
            icon: None,
            large: false,
            selected: false,
            loading: false,
            disabled: false,
            preview: Preview::Live,
            min_width: None,
            width: None,
            mark: None,
            on_click: None,
        }
    }

    /// Larghezza fissa in dp, contenuto al centro.
    pub fn width(mut self, dp: f32) -> Self {
        self.width = Some(dp);
        self
    }

    /// Il marchio SVG monocromo di un provider prima dell'etichetta (vedi `psytify_design::brand`).
    pub fn mark(mut self, svg: &'static [u8]) -> Self {
        self.mark = Some(svg);
        self
    }

    pub fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn primary(self) -> Self {
        self.kind(ButtonKind::Primary)
    }

    pub fn secondary(self) -> Self {
        self.kind(ButtonKind::Secondary)
    }

    pub fn ghost(self) -> Self {
        self.kind(ButtonKind::Ghost)
    }

    pub fn destructive(self) -> Self {
        self.kind(ButtonKind::Destructive)
    }

    /// Icona a sinistra (12 AP, colore dell'etichetta).
    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }

    /// Altezza `CONTROL_H_LARGE` (il Play di pagina).
    pub fn large(mut self) -> Self {
        self.large = true;
        self
    }

    /// Pulsante a due stati («Segui già»): solo per il secondario, con la spunta a sinistra.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// L'etichetta lascia il posto al `loader` alla stessa larghezza; il pulsante è inerte.
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
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

    pub fn min_width(mut self, dp: f32) -> Self {
        self.min_width = Some(dp);
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ap = Ap::of(window);
        let inert = self.disabled || self.loading;
        let wired = wire(&self.id, inert, self.preview, window, cx);
        let visual = if self.loading { Visual::default() } else { wired.visual };
        let look = button_look(self.kind, visual, self.disabled, self.selected, self.loading);
        let text = look.text.unwrap_or(TEXT_PRIMARY);
        let height = dp(if self.large { CONTROL_H_LARGE } else { CONTROL_H });

        let leading_icon = if self.selected && self.kind == ButtonKind::Secondary && self.icon.is_none() {
            Some(sprite::ICON_CHECK)
        } else {
            self.icon
        };
        let mut row = div().relative().flex().items_center().gap(dp(SPACE_3)).px(dp(SPACE_5));
        if let Some(width) = self.min_width {
            row = row.min_w(dp(width));
            row = centered(row);
        }
        if let Some(mark) = self.mark {
            row = row.child(gpui::svg().data(mark).flex_none().size(dp(MARK_SIDE)).text_color(col(text)));
        }
        if let Some(icon) = leading_icon {
            let icon_tint = if self.selected { ICON_ACTIVE } else { text };
            row = row.child(PixelSprite::new(icon).tint(icon_tint));
        }
        let label = div().type_role(TYPE_BUTTON).whitespace_nowrap().child(cased(&self.label, TYPE_BUTTON));
        row = row.child(if self.loading { label.invisible() } else { label });
        if self.loading {
            row = row.child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(PixelSprite::new(sprite::LOADER)),
            );
        }
        let on_click = if inert { None } else { self.on_click };
        let width = self.width.map(dp);
        with_click(pressable(wired, (width, height), NOTCH_1, look, ap, row), on_click)
    }
}
