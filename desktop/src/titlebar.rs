//! Client-side title bar (CSD decorations, Wayland and X11): mark, pixel wordmark,
//! history (Indietro and Avanti, lit only if there is a step), search, the account (the avatar in the circle
//! pixel, «Accedi» or «Accesso in corso…»; nothing if the sign-in is not configured), window controls, plus the
//! window's resize edges.
//! `mark` at rest and `mark_play` (lime) while it plays: the mark is the playback light.

use gpui::{
    AnyElement, App, Bounds, CursorStyle, Div, Entity, HitboxBehavior, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, Point, ResizeEdge, SharedString, Size, StatefulInteractiveElement,
    Styled, Window, canvas, div, point, px,
};
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CHROME, TEXT_PRIMARY, TEXT_TERTIARY};
use psytify_design::tokens::size::cover::AVATAR_S;
use psytify_design::tokens::size::{ICON_BUTTON, TITLE_BAR_H};
use psytify_design::tokens::space::{SPACE_3, SPACE_4};
use psytify_design::tokens::type_::TYPE_META;

use crate::components::IconButton;
use crate::components::search_field::SearchField;
use crate::components::separator::Separator;
use crate::components::tile::Cover;
use crate::components::tooltip::with_tooltip;
use crate::pixel::PixelSprite;
use crate::theme::{TypeStyle, col, dp};

/// Thickness in dp of the resize band along the edges.
const RESIZE_BAND: f32 = 5.0;

fn edge_at(pos: Point<Pixels>, size: Size<Pixels>) -> Option<ResizeEdge> {
    let band = px(RESIZE_BAND);
    let (left, right) = (pos.x < band, pos.x > size.width - band);
    let (top, bottom) = (pos.y < band, pos.y > size.height - band);
    Some(match (left, right, top, bottom) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (_, true, true, _) => ResizeEdge::TopRight,
        (true, _, _, true) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (_, _, true, _) => ResizeEdge::Top,
        (_, _, _, true) => ResizeEdge::Bottom,
        (true, ..) => ResizeEdge::Left,
        (_, true, ..) => ResizeEdge::Right,
        _ => return None,
    })
}

fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// Wraps the window's content with the edges that resize (only with client decorations).
pub fn window_frame(content: impl IntoElement) -> AnyElement {
    let edges = canvas(
        |_, window, _| {
            window.insert_hitbox(
                Bounds::new(point(px(0.0), px(0.0)), window.viewport_size()),
                HitboxBehavior::Normal,
            )
        },
        |_, hitbox, window, _| {
            let size = window.viewport_size();
            if let Some(edge) = edge_at(window.mouse_position(), size) {
                window.set_cursor_style(resize_cursor(edge), &hitbox);
            }
        },
    )
    .absolute()
    .size_full();
    div()
        .relative()
        .size_full()
        .on_mouse_down(MouseButton::Left, |event, window, cx| {
            if let Some(edge) = edge_at(event.position, window.viewport_size()) {
                window.start_window_resize(edge);
                cx.stop_propagation();
            }
        })
        .on_mouse_move(|event, window, _| {
            // The resize cursor updates at paint: it redraws only near the edges.
            if edge_at(event.position, window.viewport_size()).is_some() {
                window.refresh();
            }
        })
        .child(content)
        .child(edges)
        .into_any_element()
}

/// Who closes the window: the gallery quits at once, the app waits for the core to save.
pub type OnClose = std::rc::Rc<dyn Fn(&mut Window, &mut App)>;

/// The history keys: lit only if there is a step to take, with whoever takes it.
#[derive(Clone)]
pub struct HistoryButtons {
    pub on_back: Option<OnClose>,
    pub on_forward: Option<OnClose>,
}

impl HistoryButtons {
    /// Both off (the gallery, or an empty history).
    pub fn none() -> Self {
        Self { on_back: None, on_forward: None }
    }
}

/// The account in the bar, as the core tells it (`SocialView::account`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AccountChip {
    /// Sign-in is not configured: no button.
    #[default]
    Hidden,
    /// A sign-in (or the restoring of the session) is in progress.
    Working,
    /// Sign-in is possible: the person icon opens «Accedi a Psytify».
    SignedOut,
    /// The avatar in the pixel circle opens one's own profile; the name goes in the hint.
    SignedIn { avatar: Option<SharedString>, name: SharedString },
}

/// State that the bar shows.
pub struct TitleBarState {
    pub search: Entity<SearchField>,
    pub playing: bool,
    pub history: HistoryButtons,
    pub account: AccountChip,
    /// Whoever opens the sign-in or the profile (a click on the account).
    pub on_account: Option<OnClose>,
    pub on_close: OnClose,
    /// Ctrl+B: the sidebar between expanded and rail.
    pub on_sidebar: Option<OnClose>,
}

/// The account to the right of the search (also in the gallery).
pub fn account(chip: AccountChip, on_account: Option<OnClose>) -> Option<AnyElement> {
    match chip {
        AccountChip::Hidden => None,
        AccountChip::Working => Some(
            div()
                .flex()
                .items_center()
                .gap(dp(SPACE_3))
                .child(PixelSprite::new(sprite::LOADER))
                .child(div().type_role(TYPE_META).text_color(col(TEXT_TERTIARY)).child("Accesso in corso…"))
                .into_any_element(),
        ),
        AccountChip::SignedOut => {
            let button = IconButton::new("account", sprite::ICON_ACCOUNT).tooltip("Accedi", None);
            Some(match on_account {
                Some(open) => button.on_click(move |_, window, cx| open(window, cx)).into_any_element(),
                None => button.into_any_element(),
            })
        }
        AccountChip::SignedIn { avatar, name } => {
            let mut element = div()
                .id("account")
                .flex_none()
                .size(dp(ICON_BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .cursor(CursorStyle::PointingHand)
                .child(Cover::new(AVATAR_S).url(avatar).circle());
            if let Some(open) = on_account {
                element = element.on_click(move |_, window, cx| open(window, cx));
            }
            Some(
                with_tooltip(element, SharedString::from(format!("Il tuo profilo · {name}")), None)
                    .into_any_element(),
            )
        }
    }
}

/// A history key: off without a step to take.
fn history_button(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    keys: &'static str,
    handler: Option<OnClose>,
) -> IconButton {
    let button = IconButton::new(id, icon).disabled(handler.is_none()).tooltip(label, Some(keys));
    match handler {
        Some(handler) => button.on_click(move |_, window, cx| handler(window, cx)),
        None => button,
    }
}

/// The title bar with the hairline under it (the gallery).
pub fn title_bar(state: TitleBarState, window: &mut Window, cx: &mut App) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .bg(col(SURFACE_CHROME))
        .child(title_row(state, window, cx))
        .child(Separator::horizontal())
        .into_any_element()
}

/// The title bar's row, `TITLE_BAR_H` high, without the hairline: `Shell` keeps it in a view in
/// cache and puts the hairline outside (BACKLOG 23).
pub fn title_row(state: TitleBarState, _window: &mut Window, _cx: &mut App) -> Div {
    let on_close = state.on_close;
    let sidebar =
        IconButton::new("sidebar-toggle", sprite::ICON_LIST).tooltip("Barra laterale", Some("Ctrl B"));
    let sidebar = match state.on_sidebar {
        Some(toggle) => sidebar.on_click(move |_, window, cx| toggle(window, cx)),
        None => sidebar,
    };
    let account = account(state.account, state.on_account);
    let mark = if state.playing {
        PixelSprite::new(sprite::MARK_PLAY)
    } else {
        PixelSprite::new(sprite::MARK).tint(TEXT_PRIMARY)
    };
    let drag =
        div().id("title-drag").flex_1().h_full().on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count >= 2 {
                window.zoom_window();
            } else {
                window.start_window_move();
            }
        });
    // As wide as whoever contains it, even alone, as the root of a cached view.
    div()
        .w_full()
        .h(dp(TITLE_BAR_H))
        .flex_none()
        .bg(col(SURFACE_CHROME))
        .flex()
        .items_center()
        .gap(dp(SPACE_4))
        .px(dp(SPACE_3))
        .child(
            div()
                .flex()
                .items_center()
                .gap(dp(SPACE_3))
                .child(mark)
                .child(div().pixel_type(false).text_color(col(TEXT_PRIMARY)).child("Psytify")),
        )
        .child(sidebar)
        .child(
            div()
                .flex()
                .gap(dp(2.0))
                .child(history_button(
                    "nav-back",
                    sprite::ICON_CHEVRON_LEFT,
                    "Indietro",
                    "Alt ←",
                    state.history.on_back,
                ))
                .child(history_button(
                    "nav-fwd",
                    sprite::ICON_CHEVRON_RIGHT,
                    "Avanti",
                    "Alt →",
                    state.history.on_forward,
                )),
        )
        .child(div().flex_1().max_w(dp(480.0)).child(state.search))
        .child(drag)
        .children(account)
        .child(
            div()
                .flex()
                .gap(dp(2.0))
                .child(
                    IconButton::new("win-min", sprite::ICON_MINIMIZE)
                        .tooltip("Riduci a icona", None)
                        .on_click(|_, window, _| window.minimize_window()),
                )
                .child(
                    IconButton::new("win-max", sprite::ICON_MAXIMIZE)
                        .tooltip("Ingrandisci", None)
                        .on_click(|_, window, _| window.zoom_window()),
                )
                .child(
                    IconButton::new("win-close", sprite::ICON_WINDOW_CLOSE)
                        .tooltip("Chiudi", Some("Ctrl Q"))
                        .on_click(move |_, window, cx| on_close(window, cx)),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    #[test]
    fn resize_bands_cover_edges_and_corners_only() {
        let window = size(px(1000.0), px(600.0));
        assert_eq!(edge_at(point(px(500.0), px(300.0)), window), None);
        assert_eq!(edge_at(point(px(2.0), px(300.0)), window), Some(ResizeEdge::Left));
        assert_eq!(edge_at(point(px(998.0), px(300.0)), window), Some(ResizeEdge::Right));
        assert_eq!(edge_at(point(px(500.0), px(1.0)), window), Some(ResizeEdge::Top));
        assert_eq!(edge_at(point(px(500.0), px(599.0)), window), Some(ResizeEdge::Bottom));
        assert_eq!(edge_at(point(px(1.0), px(1.0)), window), Some(ResizeEdge::TopLeft));
        assert_eq!(edge_at(point(px(999.0), px(599.0)), window), Some(ResizeEdge::BottomRight));
    }
}
