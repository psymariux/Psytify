//! Z levels (section 9). Higher numbers sit on top.

pub const Z_BASE: u16 = 0;
/// Sticky headers and columns.
pub const Z_STICKY: u16 = 10;
/// Side panel below `BP_PANEL_DOCK` and the rail flyout.
pub const Z_PANEL_OVERLAY: u16 = 20;
/// Context menu, dropdown, popover.
pub const Z_MENU: u16 = 30;
pub const Z_TOOLTIP: u16 = 40;
pub const Z_SCRIM: u16 = 50;
pub const Z_DIALOG: u16 = 51;
pub const Z_TOAST: u16 = 60;
/// Drag ghost.
pub const Z_DRAG: u16 = 70;
pub const Z_SPLASH: u16 = 100;

/// All the levels in declaration order (for the ordering tests).
pub const ALL: [(&str, u16); 10] = [
    ("Z_BASE", Z_BASE),
    ("Z_STICKY", Z_STICKY),
    ("Z_PANEL_OVERLAY", Z_PANEL_OVERLAY),
    ("Z_MENU", Z_MENU),
    ("Z_TOOLTIP", Z_TOOLTIP),
    ("Z_SCRIM", Z_SCRIM),
    ("Z_DIALOG", Z_DIALOG),
    ("Z_TOAST", Z_TOAST),
    ("Z_DRAG", Z_DRAG),
    ("Z_SPLASH", Z_SPLASH),
];
