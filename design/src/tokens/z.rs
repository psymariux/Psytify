//! Livelli z (sezione 9). Numeri più alti stanno sopra.

pub const Z_BASE: u16 = 0;
/// Intestazioni e colonne appiccicose.
pub const Z_STICKY: u16 = 10;
/// Pannello laterale sotto `BP_PANEL_DOCK` e flyout della rail.
pub const Z_PANEL_OVERLAY: u16 = 20;
/// Menu contestuale, dropdown, popover.
pub const Z_MENU: u16 = 30;
pub const Z_TOOLTIP: u16 = 40;
pub const Z_SCRIM: u16 = 50;
pub const Z_DIALOG: u16 = 51;
pub const Z_TOAST: u16 = 60;
/// Ghost del drag.
pub const Z_DRAG: u16 = 70;
pub const Z_SPLASH: u16 = 100;

/// Tutti i livelli in ordine di dichiarazione (per i test sull'ordinamento).
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
