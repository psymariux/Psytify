//! Art-pixel scale and grid (section 2 of the token document).
//!
//! One art pixel (AP) is 2 nominal dp, but on screen it must cover a whole number of physical pixels,
//! otherwise sprites blur or show "mixels". `art_px` quantises; `snap_*` and
//! `center_offset_device` align positions and offsets to the same grid.

use crate::tokens::space::{AP_DP, AP_EPSILON};

/// Physical pixels per art pixel at a given scale factor: `max(1, floor(AP_DP * scale + 0.001))`.
///
/// A non-finite or non-positive factor gives 1 (never 0: callers would divide by zero).
pub fn art_px(scale_factor: f32) -> u32 {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return 1;
    }
    ((AP_DP * scale_factor + AP_EPSILON).floor() as u32).max(1)
}

/// Pixel size of a sprite of `art_pixels` art pixels.
pub const fn sprite_device_px(art_pixels: u32, ap_device: u32) -> u32 {
    art_pixels * ap_device
}

/// Font pixel size in physical pixels (11 AP) and its line height (14 AP): one rule
/// for the Departure Mono type, where one font pixel is one AP.
pub const fn pixel_type_device(ap_device: u32) -> (u32, u32) {
    (crate::tokens::type_::PIXEL_SIZE_AP * ap_device, crate::tokens::type_::PIXEL_LINE_AP * ap_device)
}

/// Converts dp to physical pixels (does not round).
pub fn dp_to_device(dp: f32, scale_factor: f32) -> f32 {
    dp * scale_factor
}

/// Rounds `x` to the nearest multiple of `unit` (ties go up). `unit <= 0` or non-finite
/// returns `x` unchanged.
pub fn snap_round(x: f32, unit: f32) -> f32 {
    if unit.is_finite() && unit > 0.0 { (x / unit).round() * unit } else { x }
}

/// Brings `x` down to the multiple of `unit` that is equal or lower. `unit <= 0` or non-finite returns `x`.
pub fn snap_floor(x: f32, unit: f32) -> f32 {
    if unit.is_finite() && unit > 0.0 { (x / unit).floor() * unit } else { x }
}

/// Origin in whole physical pixels of a position given in dp (rounded to the nearest physical pixel).
pub fn snap_device(x_dp: f32, scale_factor: f32) -> i32 {
    (x_dp * scale_factor).round() as i32
}

/// Offset in physical pixels (rounded down) to centre a sprite of `art_pixels` AP
/// in the nominal slot of `slot_dp` dp. It can be negative if the sprite is wider than the slot.
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
            let scale = step as f32 * 0.0625; // from 0.5 to 4.0 (below that, the minimum of 1 exceeds the nominal)
            let ap = art_px(scale);
            assert!(ap >= previous, "scale {scale}");
            // Never larger than nominal (2 dp x scale), at most one step smaller.
            let nominal = AP_DP * scale;
            assert!(ap as f32 <= nominal + AP_EPSILON, "scale {scale}");
            assert!(nominal - (ap as f32) < 1.0 + AP_EPSILON, "scale {scale}");
            previous = ap;
        }
    }

    #[test]
    fn exact_integers_do_not_fall_through_float_error() {
        // 3 x 2 = 6 exactly, but any binary error must not produce 5.
        for n in 1..=8u32 {
            assert_eq!(art_px(n as f32 / 2.0), n);
        }
    }

    #[test]
    fn sprite_sizes_follow_the_table() {
        // 12 AP at scale 1.25: 24 physical px = 19.2 dp; mascot 32 AP at 1.75: 96 px = 54.86 dp.
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
        // Icon 12 AP in a 24 dp slot.
        assert_eq!(center_offset_device(24.0, 12, 1.0), 0);
        assert_eq!(center_offset_device(24.0, 12, 1.25), 3); // slot 30, sprite 24
        assert_eq!(center_offset_device(24.0, 12, 1.75), 3); // slot 42, sprite 36
        assert_eq!(center_offset_device(24.0, 12, 2.625), 1); // slot 63, sprite 60
        assert_eq!(center_offset_device(24.0, 12, 2.0), 0);
        // Wider than the slot: negative offset, not a panic.
        assert!(center_offset_device(8.0, 12, 1.0) < 0);
    }
}
