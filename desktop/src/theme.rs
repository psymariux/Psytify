//! Bridge between the `psytify-design` tokens and GPUI's types. Conversions happen here once: components
//! never write a colour, a length or a font by hand, they only talk to this module.
//!
//! - colours: `col(token)` (from `[u8; 4]` to `Hsla`);
//! - lengths: `dp(v)` for token lengths, times the UI zoom ([`zoom`]); [`Ap`] for what sits on the art
//!   pixel grid (borders, ledges, sprites, the focus ring), from `art_px(scale * zoom)`;
//! - text: [`TypeStyle::type_role`] applies a type role (family, size, line height, weight, tabular
//!   figures), the pixel face snapped to its 11 px grid;
//! - fonts: [`install_fonts`] loads Inter and Departure Mono at start.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use gpui::{App, FontFeatures, FontWeight, Hsla, Pixels, SharedString, Styled, Window, px};
use psytify_design::scale::art_px;
use psytify_design::tokens::color::Rgba;
use psytify_design::tokens::type_::{self, TypeRole};

/// Colour of a token as `Hsla`, the type GPUI uses for fills, borders and text.
pub fn col(token: Rgba) -> Hsla {
    let [r, g, b, a] = token.bytes();
    gpui::Rgba {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: f32::from(a) / 255.0,
    }
    .into()
}

/// `token` with its opacity times `t` (0..=1): a fill that fades in with a hover.
pub fn fade(token: Rgba, t: f32) -> Hsla {
    let mut color = col(token);
    color.a *= t.clamp(0.0, 1.0);
    color
}

/// Between two tokens at `t` (0 = `a`, 1 = `b`), mixed in sRGB: a hover tone that eases.
pub fn mix(a: Rgba, b: Rgba, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    let (a, b) = (a.bytes(), b.bytes());
    let lerp = |i: usize| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t) / 255.0;
    gpui::Rgba { r: lerp(0), g: lerp(1), b: lerp(2), a: lerp(3) }.into()
}

/// The UI zoom (`f32` bits): one for the process, set by the window (`set_zoom`). 1.0 until then.
static ZOOM: AtomicU32 = AtomicU32::new(0x3f80_0000);
/// The window's scale factor as last seen (`f32` bits), for the pixel face snapping.
static SCALE: AtomicU32 = AtomicU32::new(0x3f80_0000);

/// The UI zoom in effect.
pub fn zoom() -> f32 {
    f32::from_bits(ZOOM.load(Ordering::Relaxed))
}

/// Sets the UI zoom; `true` if it changed (the caller then redraws the window with `Window::refresh`, so
/// the cached views lay out again).
pub fn set_zoom(zoom: f32) -> bool {
    let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
    ZOOM.swap(zoom.to_bits(), Ordering::Relaxed) != zoom.to_bits()
}

/// Remembers the window's scale factor (called by the root view as it renders).
pub fn note_scale(scale: f32) {
    if scale.is_finite() && scale > 0.0 {
        SCALE.store(scale.to_bits(), Ordering::Relaxed);
    }
}

fn scale() -> f32 {
    f32::from_bits(SCALE.load(Ordering::Relaxed))
}

/// A token length in dp, times the zoom (a GPUI `Pixels` is a logical pixel).
pub fn dp(value: f32) -> Pixels {
    px(value * zoom())
}

/// A logical length back in dp (bounds measured by GPUI, compared with the tokens).
pub fn to_dp(length: Pixels) -> f32 {
    f32::from(length) / zoom()
}

/// The art pixel (AP) at the window's scale and the zoom.
///
/// On screen an AP is `device` whole physical pixels (`art_px(scale * zoom)`): that keeps borders, ledges and
/// sprites crisp at any scale and zoom. Layout stays in dp; only what is drawn on the grid uses AP.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ap {
    /// Physical pixels per AP (whole, at least 1).
    pub device: u32,
    /// The window's scale factor (logical to physical).
    pub scale: f32,
}

impl Ap {
    /// For a scale factor at zoom 1 (an invalid factor is 1).
    #[cfg(test)]
    pub fn for_scale(scale: f32) -> Self {
        Self::for_scale_zoom(scale, 1.0)
    }

    /// For a scale factor and a zoom.
    pub fn for_scale_zoom(scale: f32, zoom: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
        Self { device: art_px(scale * zoom), scale }
    }

    /// For the current window and the zoom in effect.
    pub fn of(window: &Window) -> Self {
        Self::for_scale_zoom(window.scale_factor(), zoom())
    }

    /// Logical length of one AP: `device / scale`, that is exactly `device` physical pixels.
    pub fn one(self) -> Pixels {
        px(self.device as f32 / self.scale)
    }

    /// Logical length of `n` AP.
    pub fn n(self, n: u32) -> Pixels {
        px((self.device * n) as f32 / self.scale)
    }

    /// The scale times zoom this AP was made for (`SpriteStore::ensure_scale`).
    pub fn effective(self) -> f32 {
        self.scale * zoom()
    }
}

const INTER_REGULAR: &[u8] = include_bytes!("../../design/assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../design/assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../design/assets/fonts/Inter-SemiBold.ttf");
const DEPARTURE_MONO: &[u8] = include_bytes!("../../design/assets/fonts/DepartureMono-Regular.otf");

/// Registers Inter (400, 500, 600) and Departure Mono. Call at start, before opening a window. A font that
/// does not load falls back to the system font: the error goes to stderr and the app stays usable.
pub fn install_fonts(cx: &mut App) {
    let fonts = [INTER_REGULAR, INTER_MEDIUM, INTER_SEMIBOLD, DEPARTURE_MONO];
    let blobs = fonts.iter().map(|bytes| Cow::Borrowed(*bytes)).collect();
    if let Err(error) = cx.text_system().add_fonts(blobs) {
        eprintln!("psytify: font non caricati: {error:#}");
    }
}

/// Tabular figures (`tnum`): every digit the same width, so durations and counters do not dance.
fn tabular_features() -> FontFeatures {
    static FEATURES: OnceLock<FontFeatures> = OnceLock::new();
    FEATURES.get_or_init(|| FontFeatures(Arc::new(vec![("tnum".to_owned(), 1)]))).clone()
}

fn weight(value: u16) -> FontWeight {
    FontWeight(f32::from(value))
}

/// Size and line height of a role in logical px: Inter in dp times the zoom; the pixel face snapped to a
/// whole multiple of 11 device pixels (`type_::pixel_face_device`).
pub fn role_metrics(role: TypeRole) -> (Pixels, Pixels) {
    if role.family == type_::FAMILY_PIXEL {
        let scale = scale();
        let (size, line) = type_::pixel_face_device(role, scale * zoom());
        (px(size / scale), px(line / scale))
    } else {
        (dp(role.size), dp(role.line_height))
    }
}

/// Applies a type role to a text element.
pub trait TypeStyle: Styled + Sized {
    /// Family, size, line height, weight and tabular figures of the role. Tracking does not exist in GPUI
    /// and is not applied; upper case comes from [`cased`].
    fn type_role(self, role: TypeRole) -> Self {
        let (size, line) = role_metrics(role);
        let styled = self
            .font_family(SharedString::new_static(role.family))
            .text_size(size)
            .line_height(line)
            .font_weight(weight(role.weight));
        if role.tabular { styled.font_features(tabular_features()) } else { styled }
    }

    /// The pixel face at the Title size (wordmark, pixel numerals).
    fn pixel_type(self, tabular: bool) -> Self {
        self.type_role(if tabular { type_::TYPE_NUMERIC_PIXEL } else { type_::TYPE_WORDMARK })
    }
}

impl<T: Styled + Sized> TypeStyle for T {}

/// The text as the role asks (`Upper` and `Sentence` are copy choices; only upper case is applied here).
pub fn cased(text: &str, role: TypeRole) -> SharedString {
    match role.case {
        type_::TextCase::Upper => text.to_uppercase().into(),
        _ => text.to_owned().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use psytify_design::tokens::color::{LIME, SCRIM, WARM_WHITE};

    #[test]
    fn tokens_convert_to_the_same_srgb_values() {
        let rgba: gpui::Rgba = col(WARM_WHITE).into();
        assert!((rgba.r - 236.0 / 255.0).abs() < 1e-3);
        assert!((rgba.g - 232.0 / 255.0).abs() < 1e-3);
        assert!((rgba.b - 224.0 / 255.0).abs() < 1e-3);
        assert!((rgba.a - 1.0).abs() < 1e-6);
        // The one translucent colour stays translucent.
        assert!(col(SCRIM).a < 1.0);
        assert!(col(LIME).a > 0.99);
    }

    #[test]
    fn ap_is_a_whole_number_of_device_pixels_at_any_scale() {
        for scale in [1.0_f32, 1.1, 1.25, 1.5, 1.7, 2.0, 3.0] {
            let ap = Ap::for_scale(scale);
            assert!(ap.device >= 1);
            let device_px = f32::from(ap.one()) * scale;
            assert!((device_px - ap.device as f32).abs() < 1e-3, "scale {scale}");
            assert!((f32::from(ap.n(12)) * scale - (12 * ap.device) as f32).abs() < 1e-2);
        }
        assert_eq!(Ap::for_scale(1.5).device, 3);
        assert_eq!(Ap::for_scale(f32::NAN).device, 2);
    }

    #[test]
    fn the_art_pixel_follows_scale_times_zoom() {
        assert_eq!(Ap::for_scale_zoom(1.0, 1.5).device, 3);
        assert_eq!(Ap::for_scale_zoom(1.0, 1.125).device, 2);
        assert_eq!(Ap::for_scale_zoom(2.0, 1.25).device, 5);
        let ap = Ap::for_scale_zoom(1.0, 1.5);
        assert!((f32::from(ap.n(2)) - 6.0).abs() < 1e-4, "2 AP = 6 physical pixels at scale 1");
    }

    #[test]
    fn upper_case_roles_shout_and_others_keep_the_text() {
        assert_eq!(cased("lossless", type_::TYPE_LABEL).as_ref(), "LOSSLESS");
        assert_eq!(cased("Ciao", type_::TYPE_BODY).as_ref(), "Ciao");
    }
}
