//! Ponte fra i token di `psytify-design` e i tipi di GPUI. Si converte una volta qui: i componenti
//! non scrivono mai un colore, una misura o un font a mano, parlano solo con questo modulo.
//!
//! - colori: `col(token)` (da `[u8; 4]` a `Hsla`);
//! - misure: `dp(v)` per i dp dei token, [`Ap`] per ciò che sta sulla griglia dei pixel d'arte
//!   (bordi, ledge, sprite, anello di focus);
//! - testo: [`TypeStyle::type_role`] applica un ruolo tipografico (famiglia, taglia, interlinea,
//!   peso, cifre tabulari);
//! - font: [`install_fonts`] carica Inter e Departure Mono all'avvio.

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use gpui::{App, FontFeatures, FontWeight, Hsla, Pixels, SharedString, Styled, Window, px};
use psytify_design::scale::art_px;
use psytify_design::tokens::color::Rgba;
use psytify_design::tokens::type_::{self, TypeRole};

/// Colore di un token come `Hsla`, il tipo che GPUI usa per riempimenti, bordi e testo.
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

/// Lunghezza in dp di un token (in GPUI un dp è un `Pixels` logico).
pub fn dp(value: f32) -> Pixels {
    px(value)
}

/// Misura del pixel d'arte (AP) alla scala corrente della finestra.
///
/// Sullo schermo un AP vale `device` pixel fisici interi (`art_px`): è ciò che tiene nitidi bordi,
/// ledge e sprite a ogni scala. Il layout resta in dp; solo ciò che si disegna sulla griglia usa AP.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ap {
    /// Pixel fisici per AP (intero, almeno 1).
    pub device: u32,
    /// Fattore di scala della finestra.
    pub scale: f32,
}

impl Ap {
    /// Misura per un fattore di scala (un fattore non valido vale 1).
    pub fn for_scale(scale: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        Self { device: art_px(scale), scale }
    }

    /// Misura per la finestra corrente.
    pub fn of(window: &Window) -> Self {
        Self::for_scale(window.scale_factor())
    }

    /// Lunghezza logica di un AP: `device / scale`, cioè esattamente `device` pixel fisici.
    pub fn one(self) -> Pixels {
        px(self.device as f32 / self.scale)
    }

    /// Lunghezza logica di `n` AP.
    pub fn n(self, n: u32) -> Pixels {
        px((self.device * n) as f32 / self.scale)
    }

    /// Dimensione del tipo pixel (11 pixel del font per AP) e interlinea, in pixel logici.
    pub fn pixel_type(self) -> (Pixels, Pixels) {
        let (size, line) = psytify_design::scale::pixel_type_device(self.device);
        (px(size as f32 / self.scale), px(line as f32 / self.scale))
    }
}

const INTER_REGULAR: &[u8] = include_bytes!("../../client/assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../client/assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../client/assets/fonts/Inter-SemiBold.ttf");
const DEPARTURE_MONO: &[u8] = include_bytes!("../../design/assets/fonts/DepartureMono-Regular.otf");

/// Registra Inter (400, 500, 600) e Departure Mono nel sistema di testo. Da chiamare all'avvio, prima
/// di aprire una finestra. Se un font non si carica il testo ricade sul font di sistema: l'errore si
/// segnala su stderr e l'app resta usabile.
pub fn install_fonts(cx: &mut App) {
    let fonts = [INTER_REGULAR, INTER_MEDIUM, INTER_SEMIBOLD, DEPARTURE_MONO];
    let blobs = fonts.iter().map(|bytes| Cow::Borrowed(*bytes)).collect();
    if let Err(error) = cx.text_system().add_fonts(blobs) {
        eprintln!("psytify: font non caricati: {error:#}");
    }
}

/// Cifre tabulari (`tnum`): la stessa larghezza per ogni cifra, così durate e contatori non ballano.
fn tabular_features() -> FontFeatures {
    static FEATURES: OnceLock<FontFeatures> = OnceLock::new();
    FEATURES.get_or_init(|| FontFeatures(Arc::new(vec![("tnum".to_owned(), 1)]))).clone()
}

fn weight(value: u16) -> FontWeight {
    FontWeight(f32::from(value))
}

/// Applica un ruolo tipografico a un elemento di testo.
pub trait TypeStyle: Styled + Sized {
    /// Famiglia, taglia, interlinea, peso e cifre tabulari del ruolo. La spaziatura fra le lettere
    /// (`tracking`) non esiste in GPUI e non si applica; le maiuscole si ottengono con [`cased`].
    fn type_role(self, role: TypeRole) -> Self {
        let styled = self
            .font_family(SharedString::new_static(role.family))
            .text_size(px(role.size))
            .line_height(px(role.line_height))
            .font_weight(weight(role.weight));
        if role.tabular { styled.font_features(tabular_features()) } else { styled }
    }

    /// Tipo pixel (Departure Mono): taglia e interlinea seguono la griglia dei pixel d'arte, quindi
    /// dipendono dalla scala (`Ap::pixel_type`).
    fn pixel_type(self, ap: Ap, tabular: bool) -> Self {
        let (size, line) = ap.pixel_type();
        let styled = self
            .font_family(SharedString::new_static(type_::FAMILY_PIXEL))
            .text_size(size)
            .line_height(line)
            .font_weight(weight(400));
        if tabular { styled.font_features(tabular_features()) } else { styled }
    }
}

impl<T: Styled + Sized> TypeStyle for T {}

/// Testo come lo richiede il ruolo (`Upper` e `Sentence` sono scelte di copia, qui solo il caso
/// maiuscolo si applica).
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
        // L'unico colore traslucido resta traslucido.
        assert!(col(SCRIM).a < 1.0);
        assert!(col(LIME).a > 0.99);
    }

    #[test]
    fn ap_is_a_whole_number_of_device_pixels_at_any_scale() {
        for scale in [1.0_f32, 1.1, 1.25, 1.5, 1.7, 2.0, 3.0] {
            let ap = Ap::for_scale(scale);
            assert!(ap.device >= 1);
            let device_px = f32::from(ap.one()) * scale;
            assert!((device_px - ap.device as f32).abs() < 1e-3, "scala {scale}");
            assert!((f32::from(ap.n(12)) * scale - (12 * ap.device) as f32).abs() < 1e-2);
        }
        assert_eq!(Ap::for_scale(1.5).device, 3);
        assert_eq!(Ap::for_scale(f32::NAN).device, 2);
    }

    #[test]
    fn upper_case_roles_shout_and_others_keep_the_text() {
        assert_eq!(cased("lossless", type_::TYPE_LABEL).as_ref(), "LOSSLESS");
        assert_eq!(cased("Ciao", type_::TYPE_BODY).as_ref(), "Ciao");
    }
}
