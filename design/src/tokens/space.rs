//! Art pixels and spacing (sections 2 and 3). Lengths in dp.

/// One art pixel (AP) is 2 nominal dp at every scale and on every platform.
pub const AP_DP: f32 = 2.0;

/// Tolerance that avoids `floor(1.9999999)` when `AP_DP * scale` falls exactly on an integer.
pub const AP_EPSILON: f32 = 0.001;

pub const SPACE_0: f32 = 0.0;
pub const SPACE_1: f32 = 2.0;
pub const SPACE_2: f32 = 4.0;
pub const SPACE_3: f32 = 8.0;
pub const SPACE_4: f32 = 12.0;
pub const SPACE_5: f32 = 16.0;
pub const SPACE_6: f32 = 24.0;
pub const SPACE_7: f32 = 32.0;
pub const SPACE_8: f32 = 48.0;
pub const SPACE_9: f32 = 64.0;

/// The whole scale, from the smallest to the largest.
pub const SCALE: [f32; 10] =
    [SPACE_0, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, SPACE_7, SPACE_8, SPACE_9];

/// Converts art pixels to nominal dp.
pub const fn ap_to_dp(ap: u32) -> f32 {
    ap as f32 * AP_DP
}
