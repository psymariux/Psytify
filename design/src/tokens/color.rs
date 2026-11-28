//! Palette (section 1 of the token document): twelve values and the only translucent colour (`SCRIM`).

/// Non-premultiplied sRGB colour, bytes `[R, G, B, A]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    /// Opaque colour from three channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b, 255])
    }

    /// Same colour with another alpha (0 = transparent, 255 = opaque).
    pub const fn with_alpha(self, alpha: u8) -> Self {
        Self([self.0[0], self.0[1], self.0[2], alpha])
    }

    pub const fn r(self) -> u8 {
        self.0[0]
    }

    pub const fn g(self) -> u8 {
        self.0[1]
    }

    pub const fn b(self) -> u8 {
        self.0[2]
    }

    pub const fn a(self) -> u8 {
        self.0[3]
    }

    /// The four bytes `[R, G, B, A]`.
    pub const fn bytes(self) -> [u8; 4] {
        self.0
    }

    /// Value `0xRRGGBB` without alpha (handy for frameworks that use hex `u32`).
    pub const fn hex(self) -> u32 {
        ((self.0[0] as u32) << 16) | ((self.0[1] as u32) << 8) | self.0[2] as u32
    }
}

// Eight greys that carry the hierarchy, from darkest to lightest.
/// Content canvas, wells (field fills), floating shadows, text on light fills.
pub const DEEP_BLACK: Rgba = Rgba::rgb(0, 0, 0);
/// Chrome: title bar, sidebar, panel, player bar, phone tab bar.
pub const NEAR_BLACK: Rgba = Rgba::rgb(14, 13, 12);
/// Hover, cards, menus, tooltips, dialogs, cover placeholder field.
pub const GRAPHITE: Rgba = Rgba::rgb(26, 25, 23);
/// Selected row, pressed fill, region separators, slider track, secondary ledge.
pub const CHARCOAL: Rgba = Rgba::rgb(42, 40, 38);
/// Border of floating layers, strong separator, sprite shading (decorative only).
pub const IRON: Rgba = Rgba::rgb(61, 58, 55);
/// Control borders (>= 3:1), disabled text and icons, primary ledge, equaliser at rest.
pub const STONE: Rgba = Rgba::rgb(120, 116, 109);
/// Tertiary text: metadata, durations, placeholders, track numbers.
pub const MEDIUM_GREY: Rgba = Rgba::rgb(154, 149, 141);
/// Secondary text: artists, icons at rest, navigation labels at rest.
pub const SOFT_GREY: Rgba = Rgba::rgb(189, 184, 175);
/// Primary text, primary button fill, active icons, seek fill.
pub const WARM_WHITE: Rgba = Rgba::rgb(236, 232, 224);
/// Focus ring, text caret, primary button hover, seek knob, highlights.
pub const PURE_WHITE: Rgba = Rgba::rgb(255, 255, 255);
/// Semantic state only (section 1.3): playback active, selected navigation, toggles on, live.
pub const LIME: Rgba = Rgba::rgb(198, 242, 78);
/// Errors and destructive actions only.
pub const ERROR: Rgba = Rgba::rgb(255, 107, 94);

/// Background of dialogs and sheets: `DEEP_BLACK` at 72 % (alpha 184). The only translucent colour.
pub const SCRIM: Rgba = DEEP_BLACK.with_alpha(184);

// Semantic aliases (section 1.1).
pub const SURFACE_CANVAS: Rgba = DEEP_BLACK;
pub const SURFACE_CHROME: Rgba = NEAR_BLACK;
pub const SURFACE_RAISED: Rgba = GRAPHITE;
pub const SURFACE_SELECTED: Rgba = CHARCOAL;
pub const SURFACE_WELL: Rgba = DEEP_BLACK;
pub const LINE_SUBTLE: Rgba = CHARCOAL;
pub const LINE_STRONG: Rgba = IRON;
pub const CONTROL_BORDER: Rgba = STONE;
pub const CONTROL_BORDER_HOVER: Rgba = SOFT_GREY;
pub const FOCUS_RING: Rgba = PURE_WHITE;
pub const LEDGE_PRIMARY: Rgba = STONE;
pub const SHADOW_FLOAT: Rgba = DEEP_BLACK;
/// The hard ledge under a hovered tile (desktop v2 §3): on the black canvas it reads as a step.
pub const SHADOW_HARD: Rgba = IRON;
pub const TEXT_PRIMARY: Rgba = WARM_WHITE;
pub const TEXT_SECONDARY: Rgba = SOFT_GREY;
pub const TEXT_TERTIARY: Rgba = MEDIUM_GREY;
pub const TEXT_DISABLED: Rgba = STONE;
pub const TEXT_ON_LIGHT: Rgba = DEEP_BLACK;
pub const TEXT_ERROR: Rgba = ERROR;
pub const ICON_IDLE: Rgba = SOFT_GREY;
pub const ICON_ACTIVE: Rgba = WARM_WHITE;
pub const ICON_DISABLED: Rgba = STONE;
pub const STATE_ON: Rgba = LIME;
pub const LEDGE_SECONDARY: Rgba = CHARCOAL;
pub const CARET: Rgba = PURE_WHITE;

/// The four surfaces the text appears on (section 1.2), from darkest to lightest.
pub const SURFACES: [Rgba; 4] = [SURFACE_CANVAS, SURFACE_CHROME, SURFACE_RAISED, SURFACE_SELECTED];

/// The twelve raw palette colours, in the order of the table.
pub const PALETTE: [(&str, Rgba); 12] = [
    ("DEEP_BLACK", DEEP_BLACK),
    ("NEAR_BLACK", NEAR_BLACK),
    ("GRAPHITE", GRAPHITE),
    ("CHARCOAL", CHARCOAL),
    ("IRON", IRON),
    ("STONE", STONE),
    ("MEDIUM_GREY", MEDIUM_GREY),
    ("SOFT_GREY", SOFT_GREY),
    ("WARM_WHITE", WARM_WHITE),
    ("PURE_WHITE", PURE_WHITE),
    ("LIME", LIME),
    ("ERROR", ERROR),
];
