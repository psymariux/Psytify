//! Contrasto WCAG 2.x: luminanza relativa e rapporto fra due colori opachi. Serve ai test che
//! verificano le affermazioni del documento dei token e a chi calcola colori nuovi.

use crate::tokens::color::Rgba;

/// Canale sRGB (0-255) in luce lineare.
fn linearize(channel: u8) -> f64 {
    let c = f64::from(channel) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Luminanza relativa (0 = nero, 1 = bianco); l'alfa è ignorato.
pub fn relative_luminance(color: Rgba) -> f64 {
    0.2126 * linearize(color.r()) + 0.7152 * linearize(color.g()) + 0.0722 * linearize(color.b())
}

/// Rapporto di contrasto, da 1 (uguali) a 21 (nero su bianco); simmetrico.
pub fn contrast_ratio(a: Rgba, b: Rgba) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (lighter, darker) = if la >= lb { (la, lb) } else { (lb, la) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Soglia WCAG AA per il testo normale.
pub const AA_TEXT: f64 = 4.5;
/// Soglia WCAG per componenti dell'interfaccia (bordi dei controlli).
pub const UI_COMPONENT: f64 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::color::{self, *};

    fn assert_close(actual: f64, expected: f64, what: &str) {
        assert!((actual - expected).abs() <= 0.0051, "{what}: {actual:.4} vs doc {expected}");
    }

    #[test]
    fn extremes() {
        assert_close(contrast_ratio(DEEP_BLACK, PURE_WHITE), 21.0, "black on white");
        assert_close(contrast_ratio(PURE_WHITE, PURE_WHITE), 1.0, "same color");
        assert_eq!(contrast_ratio(IRON, STONE), contrast_ratio(STONE, IRON));
    }

    #[test]
    fn every_text_tier_passes_wcag_aa_on_every_surface() {
        for fg in [TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY, TEXT_ERROR] {
            for bg in color::SURFACES {
                let ratio = contrast_ratio(fg, bg);
                assert!(ratio >= AA_TEXT, "{fg:?} on {bg:?}: {ratio:.2}");
            }
        }
    }

    #[test]
    fn control_borders_reach_three_to_one_on_every_surface() {
        for bg in color::SURFACES {
            let ratio = contrast_ratio(CONTROL_BORDER, bg);
            assert!(ratio >= UI_COMPONENT, "{bg:?}: {ratio:.2}");
        }
        // Il bordo in hover è più chiaro e quindi ancora più contrastato.
        for bg in color::SURFACES {
            assert!(contrast_ratio(CONTROL_BORDER_HOVER, bg) >= contrast_ratio(CONTROL_BORDER, bg));
        }
    }

    /// Ogni cifra della tabella 1.2 del documento, con due decimali di tolleranza di arrotondamento.
    #[test]
    fn contrast_table_of_the_tokens_doc_is_exact() {
        // (primo piano, [su DEEP_BLACK, NEAR_BLACK, GRAPHITE, CHARCOAL]); None = «—».
        let table: [(&str, Rgba, [Option<f64>; 4]); 9] = [
            ("WARM_WHITE", WARM_WHITE, [Some(17.19), Some(15.89), Some(14.38), Some(12.02)]),
            ("PURE_WHITE", PURE_WHITE, [Some(21.00), Some(19.42), Some(17.57), Some(14.69)]),
            ("SOFT_GREY", SOFT_GREY, [Some(10.64), Some(9.84), Some(8.90), Some(7.44)]),
            ("MEDIUM_GREY", MEDIUM_GREY, [Some(7.06), Some(6.53), Some(5.90), Some(4.94)]),
            ("STONE", STONE, [Some(4.52), Some(4.18), Some(3.78), Some(3.16)]),
            ("IRON", IRON, [Some(1.86), Some(1.72), Some(1.56), Some(1.30)]),
            ("CHARCOAL", CHARCOAL, [Some(1.43), Some(1.32), Some(1.20), None]),
            ("LIME", LIME, [Some(16.21), Some(14.99), Some(13.56), Some(11.34)]),
            ("ERROR", ERROR, [Some(7.52), Some(6.95), Some(6.29), Some(5.26)]),
        ];
        for (name, fg, expected) in table {
            for (bg, claim) in color::SURFACES.into_iter().zip(expected) {
                if let Some(claim) = claim {
                    assert_close(contrast_ratio(fg, bg), claim, &format!("{name} on {bg:?}"));
                }
            }
        }
    }

    #[test]
    fn light_fills_and_ledge_claims() {
        assert_close(contrast_ratio(TEXT_ON_LIGHT, WARM_WHITE), 17.19, "on WARM_WHITE");
        assert_close(contrast_ratio(TEXT_ON_LIGHT, PURE_WHITE), 21.00, "on PURE_WHITE");
        assert_close(contrast_ratio(TEXT_ON_LIGHT, LIME), 16.21, "on LIME");
        assert_close(contrast_ratio(TEXT_ON_LIGHT, ERROR), 7.52, "on ERROR");
        assert_close(contrast_ratio(LEDGE_PRIMARY, WARM_WHITE), 3.80, "STONE ledge on WARM_WHITE fill");
    }

    #[test]
    fn decorative_greys_stay_decorative() {
        // IRON non è mai l'unico segnale: sotto 3:1 su ogni superficie, come dichiarato.
        for bg in color::SURFACES {
            assert!(contrast_ratio(IRON, bg) < UI_COMPONENT);
        }
    }
}
