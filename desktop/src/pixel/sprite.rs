//! `PixelSprite`: draws a `psytify-design` sprite crisp at any scale and zoom.
//!
//! GPUI's sampler is always linear, so the sprite is enlarged on the CPU by a whole factor
//! (`art_px(scale * zoom)`, see `store`) and painted 1:1 on the physical pixel grid: origin rounded to the
//! pixel, size equal to the texture, so the sampler is the identity.
//!
//! The layout box is nominal (`frame * 2 dp`, times the zoom): the sprite sits centred in it with the offset
//! floored to whole pixels, so the layout follows the zoom and the art is at most one AP step smaller, never
//! larger, never blurred.

use gpui::{
    App, BorrowAppContext, Bounds, Corners, IntoElement, RenderOnce, Styled, Window, canvas, point, px, size,
};
use psytify_design::tokens::color::Rgba;
use psytify_design::tokens::space::AP_DP;

use super::anim::{ms_until_next_frame, request_refresh};
use super::store::SpriteStore;
use crate::theme::{Ap, dp};

/// Un elemento che mostra uno sprite per nome (le costanti stanno in `psytify_design::sprite`).
#[derive(IntoElement)]
pub struct PixelSprite {
    name: &'static str,
    tint: Option<Rgba>,
    animate: bool,
    frame: usize,
}

impl PixelSprite {
    pub fn new(name: &'static str) -> Self {
        Self { name, tint: None, animate: true, frame: 0 }
    }

    /// Colore della maschera (un token: `ICON_IDLE`, `STATE_ON`...). Gli sprite a colori lo ignorano.
    pub fn tint(mut self, tint: Rgba) -> Self {
        self.tint = Some(tint);
        self
    }

    /// Con `false` lo sprite resta fermo sul frame indicato da [`frame`](Self::frame) e non programma
    /// ridisegni (equalizzatore in pausa, mascotte fuori vista).
    pub fn animated(mut self, animate: bool) -> Self {
        self.animate = animate;
        self
    }

    /// Frame mostrato quando lo sprite non è animato.
    pub fn frame(mut self, frame: usize) -> Self {
        self.frame = frame;
        self
    }
}

/// Dimensione nominale in dp di uno sprite (frame in AP x 2), `None` se il nome non esiste.
pub fn nominal_size(cx: &App, name: &str) -> Option<(f32, f32)> {
    let sheet = cx.try_global::<SpriteStore>()?.sheet()?;
    let (w, h) = sheet.get(name)?.frame_size();
    Some((w as f32 * AP_DP, h as f32 * AP_DP))
}

impl RenderOnce for PixelSprite {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (w, h) = nominal_size(cx, self.name).unwrap_or((0.0, 0.0));
        let Self { name, tint, animate, frame } = self;
        canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| paint_sprite(window, cx, name, tint, animate, frame, bounds),
        )
        .w(dp(w))
        .h(dp(h))
        .flex_none()
    }
}

/// Dipinge lo sprite nello slot `slot`, centrato e allineato ai pixel fisici. Si usa anche da
/// elementi che disegnano da soli (canvas) e non vogliono un figlio `PixelSprite`.
pub fn paint_sprite(
    window: &mut Window,
    cx: &mut App,
    name: &str,
    tint: Option<Rgba>,
    animate: bool,
    fixed_frame: usize,
    slot: Bounds<gpui::Pixels>,
) {
    let scale = window.scale_factor();
    let ap = Ap::of(window);
    let active = window.is_window_active();
    let drawn = cx.update_global::<SpriteStore, _>(|store, _| {
        store.ensure_scale(ap.effective());
        store.drain_retired(window);
        let sprite = store.sheet()?.get(name)?;
        let moving = animate && sprite.frame_count() > 1 && !store.reduce_motion() && active;
        let (frame, next_ms) = if moving {
            let now = store.elapsed_ms();
            (sprite.frame_index_at(now), Some(ms_until_next_frame(sprite, now)))
        } else {
            (fixed_frame.min(sprite.frame_count().saturating_sub(1)), None)
        };
        let texture = store.texture(name, frame, ap.device, tint)?;
        Some((texture, next_ms))
    });
    let Some((texture, next_ms)) = drawn else { return };
    let (w_dev, h_dev) = (texture.width as f32, texture.height as f32);
    let x = (f32::from(slot.origin.x) * scale).round()
        + ((f32::from(slot.size.width) * scale - w_dev) / 2.0).floor();
    let y = (f32::from(slot.origin.y) * scale).round()
        + ((f32::from(slot.size.height) * scale - h_dev) / 2.0).floor();
    let bounds = Bounds::new(point(px(x / scale), px(y / scale)), size(px(w_dev / scale), px(h_dev / scale)));
    // Se l'immagine non entra nell'atlante non c'è altro da fare che saltare il disegno.
    let _ = window.paint_image(bounds, bounds, Corners::default(), texture.image, 0, false);
    if let Some(ms) = next_ms {
        request_refresh(window, cx, ms);
    }
}
