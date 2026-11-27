//! Pixel d'arte e spaziature (sezioni 2 e 3). Lunghezze in dp.

/// Un pixel d'arte (AP) vale 2 dp nominali a ogni scala e su ogni piattaforma.
pub const AP_DP: f32 = 2.0;

/// Tolleranza che evita `floor(1.9999999)` quando `AP_DP * scale` cade esattamente su un intero.
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

/// Tutta la scala, dalla più piccola alla più grande.
pub const SCALE: [f32; 10] =
    [SPACE_0, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, SPACE_7, SPACE_8, SPACE_9];

/// Converte pixel d'arte in dp nominali.
pub const fn ap_to_dp(ap: u32) -> f32 {
    ap as f32 * AP_DP
}
