//! LikeButton: the heart IconButton («mi piace»). Liking fills it in two frames (`icon_heart_half`, then
//! `icon_heart_fill`) with a 1 AP pop on the second; unliking is a cut. Reduced motion: always a cut.

use gpui::{App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use psytify_design::sprite;
use psytify_design::tokens::motion::SPRITE_FRAME_MIN_MS;

use super::control::OnClick;
use super::icon_button::{IconButton, OnStyle};
use crate::theme::Ap;

/// The fill frame shown at `elapsed` ms after a like: the half heart, then the full one with the pop, then
/// nothing (rest). Pure.
pub fn fill_frame(elapsed: Option<u32>) -> Option<(&'static str, bool)> {
    match elapsed? {
        ms if ms < SPRITE_FRAME_MIN_MS => Some((sprite::ICON_HEART_HALF, false)),
        ms if ms < 2 * SPRITE_FRAME_MIN_MS => Some((sprite::ICON_HEART_FILL, true)),
        _ => None,
    }
}

#[derive(IntoElement)]
pub struct LikeButton {
    id: ElementId,
    liked: bool,
    size: Option<f32>,
    on_click: Option<OnClick>,
}

impl LikeButton {
    pub fn new(id: impl Into<ElementId>, liked: bool) -> Self {
        Self { id: id.into(), liked, size: None, on_click: None }
    }

    pub fn size(mut self, dp: f32) -> Self {
        self.size = Some(dp);
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }
}

impl RenderOnce for LikeButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key =
            ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), SharedString::new_static("fill"));
        let elapsed = crate::motion::since_change(key, self.liked, 2 * SPRITE_FRAME_MIN_MS, window, cx);
        let frame = if self.liked { fill_frame(elapsed) } else { None };
        let (icon_on, pop) = frame.unwrap_or((sprite::ICON_HEART_FILL, false));
        let mut button = IconButton::new(self.id, sprite::ICON_HEART)
            .icon_on(icon_on)
            .on_style(OnStyle::Active)
            .selected(self.liked)
            .tooltip(if self.liked { "Rimuovi dai preferiti" } else { "Aggiungi ai preferiti" }, None);
        if let Some(size) = self.size {
            button = button.size(size);
        }
        if let Some(handler) = self.on_click {
            button = button.on_click(move |event, window, cx| handler(event, window, cx));
        }
        let lift = if pop { Ap::of(window).one() } else { gpui::px(0.0) };
        div().relative().top(-lift).child(button)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_like_fills_in_two_frames_and_pops_on_the_second() {
        assert_eq!(fill_frame(None), None);
        assert_eq!(fill_frame(Some(0)), Some((sprite::ICON_HEART_HALF, false)));
        assert_eq!(fill_frame(Some(SPRITE_FRAME_MIN_MS)), Some((sprite::ICON_HEART_FILL, true)));
        assert_eq!(fill_frame(Some(2 * SPRITE_FRAME_MIN_MS)), None);
    }
}
