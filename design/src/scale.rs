//! Scala e griglia dei pixel d'arte (sezione 2 del documento dei token).
//!
//! Un pixel d'arte (AP) vale 2 dp nominali, ma sullo schermo deve occupare un numero intero di pixel
//! fisici, altrimenti gli sprite sfocano o hanno «mixel». `art_px` quantizza; `snap_*` e
//! `center_offset_device` allineano posizioni e offset alla stessa griglia.

use crate::tokens::space::{AP_DP, AP_EPSILON};

/// Pixel fisici per pixel d'arte a un dato fattore di scala: `max(1, floor(AP_DP * scale + 0.001))`.
///
/// Un fattore non finito o non positivo vale 1 (mai 0: dividerebbe per zero chi lo usa).
pub fn art_px(scale_factor: f32) -> u32 {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return 1;
    }
    ((AP_DP * scale_factor + AP_EPSILON).floor() as u32).max(1)
}

/// Dimensione in pixel fisici di uno sprite di `art_pixels` pixel d'arte.
pub const fn sprite_device_px(art_pixels: u32, ap_device: u32) -> u32 {
    art_pixels * ap_device
}

/// Dimensione pixel del font in pixel fisici (11 AP) e relativa interlinea (14 AP): una regola
/// sola per il tipo Departure Mono, dove un pixel del font è un AP.
pub const fn pixel_type_device(ap_device: u32) -> (u32, u32) {
    (crate::tokens::type_::PIXEL_SIZE_AP * ap_device, crate::tokens::type_::PIXEL_LINE_AP * ap_device)
}

/// Converte dp in pixel fisici (non arrotonda).
pub fn dp_to_device(dp: f32, scale_factor: f32) -> f32 {
    dp * scale_factor
}

/// Arrotonda `x` al multiplo più vicino di `unit` (ex aequo verso l'alto). `unit <= 0` o non finito
/// restituisce `x` invariato.
pub fn snap_round(x: f32, unit: f32) -> f32 {
    if unit.is_finite() && unit > 0.0 { (x / unit).round() * unit } else { x }
}

/// Porta `x` al multiplo di `unit` uguale o inferiore. `unit <= 0` o non finito restituisce `x`.
pub fn snap_floor(x: f32, unit: f32) -> f32 {
    if unit.is_finite() && unit > 0.0 { (x / unit).floor() * unit } else { x }
}

/// Origine in pixel fisici interi di una posizione data in dp (arrotondata al pixel fisico più vicino).
pub fn snap_device(x_dp: f32, scale_factor: f32) -> i32 {
    (x_dp * scale_factor).round() as i32
}

/// Offset in pixel fisici (arrotondato per difetto) per centrare uno sprite di `art_pixels` AP
/// nello slot nominale di `slot_dp` dp. Può essere negativo se lo sprite è più largo dello slot.
pub fn center_offset_device(slot_dp: f32, art_pixels: u32, scale_factor: f32) -> i32 {
    let slot = slot_dp * scale_factor;
    let sprite = sprite_device_px(art_pixels, art_px(scale_factor)) as f32;
    ((slot - sprite) / 2.0).floor() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn art_px_table_matches_the_tokens_doc() {
        let table = [
            (1.0, 2),
            (1.25, 2),
            (1.5, 3),
            (1.75, 3),
            (2.0, 4),
            (2.625, 5),
            (2.75, 5),
            (3.0, 6),
            (3.5, 7),
            (4.0, 8),
        ];
        for (scale, expected) in table {
            assert_eq!(art_px(scale), expected, "scale {scale}");
        }
    }

    #[test]
    fn art_px_never_returns_zero() {
        for scale in [0.0, -1.0, 0.1, 0.25, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(art_px(scale) >= 1, "scale {scale}");
        }
        assert_eq!(art_px(0.1), 1);
    }

    #[test]
    fn art_px_is_monotonic_and_never_larger_than_nominal() {
        let mut previous = 0;
        for step in 8..=64 {
            let scale = step as f32 * 0.0625; // da 0,5 a 4,0 (sotto, il minimo di 1 supera il nominale)
            let ap = art_px(scale);
            assert!(ap >= previous, "scale {scale}");
            // Mai più grande del nominale (2 dp x scala), al più un passo più piccolo.
            let nominal = AP_DP * scale;
            assert!(ap as f32 <= nominal + AP_EPSILON, "scale {scale}");
            assert!(nominal - (ap as f32) < 1.0 + AP_EPSILON, "scale {scale}");
            previous = ap;
        }
    }

    #[test]
    fn exact_integers_do_not_fall_through_float_error() {
        // 3 x 2 = 6 esatto, ma qualunque errore binario non deve produrre 5.
        for n in 1..=8u32 {
            assert_eq!(art_px(n as f32 / 2.0), n);
        }
    }

    #[test]
    fn sprite_sizes_follow_the_table() {
        // 12 AP a scala 1,25: 24 px fisici = 19,2 dp; mascotte 32 AP a 1,75: 96 px = 54,86 dp.
        assert_eq!(sprite_device_px(12, art_px(1.25)), 24);
        assert_eq!(sprite_device_px(32, art_px(1.75)), 96);
        assert_eq!(sprite_device_px(12, art_px(4.0)), 96);
    }

    #[test]
    fn pixel_type_is_eleven_device_pixels_per_ap() {
        assert_eq!(pixel_type_device(2), (22, 28));
        assert_eq!(pixel_type_device(art_px(2.0)), (44, 56));
    }

    #[test]
    fn snapping_helpers() {
        assert_eq!(snap_floor(7.0, 3.0), 6.0);
        assert_eq!(snap_round(7.0, 3.0), 6.0);
        assert_eq!(snap_round(8.0, 3.0), 9.0);
        assert_eq!(snap_floor(-1.0, 3.0), -3.0);
        assert_eq!(snap_round(5.5, 0.0), 5.5);
        assert_eq!(snap_floor(5.5, f32::NAN), 5.5);
        assert_eq!(snap_device(10.4, 1.25), 13);
        assert_eq!(snap_device(10.0, 1.5), 15);
    }

    #[test]
    fn centered_sprite_offset_is_floored_to_whole_device_pixels() {
        // Icona 12 AP in uno slot di 24 dp.
        assert_eq!(center_offset_device(24.0, 12, 1.0), 0);
        assert_eq!(center_offset_device(24.0, 12, 1.25), 3); // slot 30, sprite 24
        assert_eq!(center_offset_device(24.0, 12, 1.75), 3); // slot 42, sprite 36
        assert_eq!(center_offset_device(24.0, 12, 2.625), 1); // slot 63, sprite 60
        assert_eq!(center_offset_device(24.0, 12, 2.0), 0);
        // Più largo dello slot: offset negativo, non panico.
        assert!(center_offset_device(8.0, 12, 1.0) < 0);
    }
}
