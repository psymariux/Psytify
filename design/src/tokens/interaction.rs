//! Interaction offsets and thresholds (section 8).

/// Pressed translation, in AP (x, y), for controls with a ledge; 0 for the "ghost" ones.
pub const PRESS_TRANSLATE: (u32, u32) = (1, 1);
/// Drag threshold in dp: desktop and phone.
pub const DRAG_THRESHOLD_DESKTOP: f32 = 4.0;
pub const DRAG_THRESHOLD_PHONE: f32 = 8.0;
/// Long press (phone), in ms.
pub const LONG_PRESS_MS: u32 = 450;
/// Maximum interval of a double click, in ms.
pub const DOUBLE_CLICK_MS: u32 = 400;
