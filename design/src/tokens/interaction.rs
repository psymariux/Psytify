//! Offset e soglie di interazione (sezione 8).

/// Traslazione da premuto, in AP (x, y), per i controlli con ledge; 0 per quelli «ghost».
pub const PRESS_TRANSLATE: (u32, u32) = (1, 1);
/// Soglia di trascinamento in dp: desktop e telefono.
pub const DRAG_THRESHOLD_DESKTOP: f32 = 4.0;
pub const DRAG_THRESHOLD_PHONE: f32 = 8.0;
/// Pressione lunga (telefono), in ms.
pub const LONG_PRESS_MS: u32 = 450;
/// Intervallo massimo del doppio clic, in ms.
pub const DOUBLE_CLICK_MS: u32 = 400;
