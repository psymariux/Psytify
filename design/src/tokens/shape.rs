//! Corners, borders, ledges and shadows (section 4). Widths and offsets in art pixels (AP): drawn as
//! `ap_device` physical pixels per AP (see `scale::art_px`). No radius: rounding is a stepped notch.

/// No curvature radius, ever (`RADIUS_NONE = 0`).
pub const RADIUS_NONE: f32 = 0.0;

/// Shape of a corner, all on the AP grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    /// Covers, rows, tiles, panels, separators.
    Square,
    /// Cut of 1 AP per corner: buttons, fields, badges, tooltips, toasts, toggles.
    Notch1,
    /// Two-step cut (1 AP + 1 AP, 45 degrees): menus, dialogs, sheets, cards.
    Notch2,
    /// Circle rasterised at AP resolution (diameter = side / AP): artist photos, avatars.
    PixelCircle,
}

/// How many AP are cut per corner in each notched shape.
pub const NOTCH_0: u32 = 0;
pub const NOTCH_1: u32 = 1;
pub const NOTCH_2: u32 = 2;

/// The only border and separator width.
pub const BORDER_W: u32 = 1;
pub const FOCUS_RING_W: u32 = 1;
/// Gap between the control border and the ring (room for the surface).
pub const FOCUS_RING_GAP: u32 = 1;
/// Selected navigation bar and the toast error marker.
pub const MARKER_W: u32 = 2;

/// Thickness of the hard ledge under the primary and secondary buttons (blur 0): (x, y) in AP.
pub const LEDGE_OFFSET: (u32, u32) = (1, 1);
/// Hard shadow of menus, dialogs, toasts, drag ghost (blur 0, colour `SHADOW_FLOAT`): (x, y) in AP.
pub const SHADOW_FLOAT_OFFSET: (u32, u32) = (2, 2);
/// The blur is 0 everywhere.
pub const BLUR: f32 = 0.0;
