//! Type (section 5, and the desktop v2 ramp of `PSYTIFY_DESKTOP_V2.md` §1). Two faces: Inter for running
//! text and catalogue strings, Departure Mono for page titles, the stage title and small captions. Sizes
//! and line heights in dp, tracking in px (may be negative).
//!
//! Five sizes do the visible work ([`DISPLAY`], [`TITLE`], [`BODY`], [`META`], [`CAPTION`]) plus the lyric
//! pair ([`LYRIC_PANEL`], [`LYRIC_STAGE`]). Every `TYPE_*` role below is one of them, named by its use; a role
//! may change weight, case or figures, never the size.

/// Running text (Inter 4.1: 400, 500, 600).
pub const FAMILY_UI: &str = "Inter";
/// Pixel face (Departure Mono 1.500, Regular), drawn on an 11 px grid: frontends snap its size to a whole
/// multiple of 11 device pixels so the glyphs stay crisp ([`pixel_face_device`]).
pub const FAMILY_PIXEL: &str = "Departure Mono";

/// Letter case asked by the role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCase {
    /// As written (catalogue, user).
    AsWritten,
    /// First letter upper case.
    Sentence,
    /// All upper case (`TYPE_LABEL`).
    Upper,
}

/// A type role.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeRole {
    pub family: &'static str,
    /// Size in dp.
    pub size: f32,
    /// Line height in dp.
    pub line_height: f32,
    /// 400, 500 or 600.
    pub weight: u16,
    /// Letter spacing in px (+ wider, − tighter).
    pub tracking: f32,
    /// Tabular figures (`tnum`).
    pub tabular: bool,
    /// Slashed zero (`zero`), pixel numerals only.
    pub slashed_zero: bool,
    pub case: TextCase,
}

impl TypeRole {
    /// The same size with another weight.
    pub const fn weight(self, weight: u16) -> Self {
        Self { weight, ..self }
    }

    /// The same size with tabular figures.
    pub const fn tabular(self) -> Self {
        Self { tabular: true, ..self }
    }

    /// The same size with another case.
    pub const fn case(self, case: TextCase) -> Self {
        Self { case, ..self }
    }
}

const fn inter(
    size: f32,
    line_height: f32,
    weight: u16,
    tracking: f32,
    tabular: bool,
    case: TextCase,
) -> TypeRole {
    TypeRole { family: FAMILY_UI, size, line_height, weight, tracking, tabular, slashed_zero: false, case }
}

const fn pixel(size: f32, line_height: f32, tabular: bool, slashed_zero: bool, case: TextCase) -> TypeRole {
    TypeRole {
        family: FAMILY_PIXEL,
        size,
        line_height,
        weight: 400,
        tracking: 0.0,
        tabular,
        slashed_zero,
        case,
    }
}

/// Size of the pixel face in the Iced app (11 AP, 22 dp) and its line height (14 AP, 28 dp).
pub const PIXEL_SIZE: f32 = 22.0;
pub const PIXEL_LINE: f32 = 28.0;
/// The same in AP (font pixels): see `scale::pixel_type_device`.
pub const PIXEL_SIZE_AP: u32 = 11;
pub const PIXEL_LINE_AP: u32 = 14;

// The ramp (desktop v2 §1).
/// Page titles, the stage title: Departure Mono 40 / 44.
pub const DISPLAY: TypeRole = pixel(40.0, 44.0, false, false, TextCase::AsWritten);
/// Section headings, object names in panels: Inter 600 22 / 28.
pub const TITLE: TypeRole = inter(22.0, 28.0, 600, -0.22, false, TextCase::Sentence);
/// Track and item titles, nav labels: Inter 500 15 / 20.
pub const BODY: TypeRole = inter(15.0, 20.0, 500, 0.0, false, TextCase::AsWritten);
/// Artist lines, counts, times: Inter 400 13 / 18.
pub const META: TypeRole = inter(13.0, 18.0, 400, 0.0, false, TextCase::AsWritten);
/// Badges (`LOSSLESS`, `NUOVO`, `LIVE`), kbd hints: Departure Mono 11 / 14.
pub const CAPTION: TypeRole = pixel(11.0, 14.0, true, false, TextCase::Upper);
/// Synced lyrics in the side panel: Inter 600 20 / 28.
pub const LYRIC_PANEL: TypeRole = inter(20.0, 28.0, 600, -0.2, false, TextCase::AsWritten);
/// Synced lyrics on the stage: Inter 600 32 / 40.
pub const LYRIC_STAGE: TypeRole = inter(32.0, 40.0, 600, -0.32, false, TextCase::AsWritten);

/// The ramp, largest first (for tests and the gallery).
pub const RAMP: [(&str, TypeRole); 5] =
    [("DISPLAY", DISPLAY), ("TITLE", TITLE), ("BODY", BODY), ("META", META), ("CAPTION", CAPTION)];

// Roles by use, each one size of the ramp.
/// Album, artist, playlist, profile name in a page header.
pub const TYPE_DISPLAY: TypeRole = DISPLAY;
/// Screen titles: Home, Cerca, Libreria, Impostazioni.
pub const TYPE_PAGE_TITLE: TypeRole = DISPLAY.tabular().case(TextCase::Sentence);
/// The «Psytify» wordmark: the pixel face at the Title size.
pub const TYPE_WORDMARK: TypeRole = pixel(TITLE.size, TITLE.line_height, false, false, TextCase::AsWritten);
/// Section and dialog headings.
pub const TYPE_HEADING: TypeRole = TITLE;
/// Track and tile titles.
pub const TYPE_TRACK_TITLE: TypeRole = BODY;
/// Artist lines, tile subtitles (`TEXT_SECONDARY`).
pub const TYPE_ARTIST: TypeRole = META;
/// Paragraphs, settings explanations, toasts: Body at 400.
pub const TYPE_BODY: TypeRole = BODY.weight(400).case(TextCase::Sentence);
/// Metadata, dates, counts (`TEXT_TERTIARY`).
pub const TYPE_META: TypeRole = META.tabular().case(TextCase::Sentence);
/// Sidebar items, in-page tabs, menu items.
pub const TYPE_NAV: TypeRole = BODY.case(TextCase::Sentence);
/// Button labels: Meta at 600.
pub const TYPE_BUTTON: TypeRole = META.weight(600).case(TextCase::Sentence);
/// Kickers, column heads, badges (`FLAC 24/96`, `LOSSLESS`, `LIVE`).
pub const TYPE_LABEL: TypeRole = CAPTION;
/// Durations, track numbers, elapsed / remaining in the player bar.
pub const TYPE_NUMERIC: TypeRole = META.tabular();
/// Pixel numerals: the stage times, the sleep countdown, the jam code, statistics (the pixel face at the
/// Title size).
pub const TYPE_NUMERIC_PIXEL: TypeRole =
    pixel(TITLE.size, TITLE.line_height, true, true, TextCase::AsWritten);
/// Synced lyrics on the stage (focus view).
pub const TYPE_LYRIC_FOCUS: TypeRole = LYRIC_STAGE;
/// Synced lyrics in the side panel.
pub const TYPE_LYRIC_PANEL: TypeRole = LYRIC_PANEL;

/// Named roles (for the tests).
pub const ALL: [(&str, TypeRole); 15] = [
    ("TYPE_DISPLAY", TYPE_DISPLAY),
    ("TYPE_PAGE_TITLE", TYPE_PAGE_TITLE),
    ("TYPE_WORDMARK", TYPE_WORDMARK),
    ("TYPE_HEADING", TYPE_HEADING),
    ("TYPE_TRACK_TITLE", TYPE_TRACK_TITLE),
    ("TYPE_ARTIST", TYPE_ARTIST),
    ("TYPE_BODY", TYPE_BODY),
    ("TYPE_META", TYPE_META),
    ("TYPE_NAV", TYPE_NAV),
    ("TYPE_BUTTON", TYPE_BUTTON),
    ("TYPE_LABEL", TYPE_LABEL),
    ("TYPE_NUMERIC", TYPE_NUMERIC),
    ("TYPE_NUMERIC_PIXEL", TYPE_NUMERIC_PIXEL),
    ("TYPE_LYRIC_FOCUS", TYPE_LYRIC_FOCUS),
    ("TYPE_LYRIC_PANEL", TYPE_LYRIC_PANEL),
];

/// Device pixels of a pixel-face role at `device_per_dp` (window scale times zoom): the nearest whole
/// multiple of 11 (ties round up, so the caption at 1.5 is 22 and not a cramped 11; never below 11), and
/// the line height in the role's proportion, rounded to whole device pixels.
pub fn pixel_face_device(role: TypeRole, device_per_dp: f32) -> (f32, f32) {
    // Absorbs float noise in `scale * zoom` so an exact tie still rounds up.
    const TIE: f32 = 1e-4;
    let unit = PIXEL_SIZE_AP as f32;
    let target = role.size * device_per_dp;
    let steps = if target.is_finite() { (target / unit + TIE).round().max(1.0) } else { 1.0 };
    let size = steps * unit;
    let line = (size * role.line_height / role.size).round();
    (size, line)
}

/// The sizes the Iced app (Android) keeps: the v1 scale, from before the desktop ramp.
pub mod v1 {
    pub const DISPLAY: f32 = 32.0;
    pub const HEADING: f32 = 16.0;
    pub const BODY: f32 = 14.0;
    pub const META: f32 = 12.0;
}

/// Phone overrides. The pixel face does not change (it follows the AP, not the platform).
pub mod phone {
    use super::{TextCase, TypeRole, inter, pixel};

    pub const TYPE_DISPLAY: TypeRole = inter(26.0, 32.0, 600, -0.32, false, TextCase::AsWritten);
    pub const TYPE_TRACK_TITLE: TypeRole = inter(16.0, 22.0, 500, 0.0, false, TextCase::AsWritten);
    pub const TYPE_ARTIST: TypeRole = inter(14.0, 20.0, 400, 0.0, false, TextCase::AsWritten);
    pub const TYPE_BODY: TypeRole = inter(15.0, 22.0, 400, 0.0, false, TextCase::Sentence);
    /// Tab labels.
    pub const TYPE_NAV: TypeRole = inter(11.0, 16.0, 500, 0.0, false, TextCase::Sentence);
    pub const TYPE_LYRIC: TypeRole = inter(22.0, 30.0, 600, -0.24, false, TextCase::AsWritten);
    // The desktop v2 ramp grew these for wide windows; the phone keeps its sizes.
    pub const TYPE_PAGE_TITLE: TypeRole = pixel(22.0, 28.0, true, false, TextCase::Sentence);
    pub const TYPE_HEADING: TypeRole = inter(16.0, 24.0, 600, -0.08, false, TextCase::Sentence);
    pub const TYPE_META: TypeRole = inter(12.0, 16.0, 400, 0.0, true, TextCase::Sentence);
    pub const TYPE_BUTTON: TypeRole = inter(13.0, 16.0, 600, 0.13, false, TextCase::Sentence);
    pub const TYPE_LABEL: TypeRole = inter(11.0, 16.0, 600, 0.66, true, TextCase::Upper);
    pub const TYPE_NUMERIC: TypeRole = inter(12.0, 16.0, 400, 0.0, true, TextCase::AsWritten);
    pub const TYPE_LYRIC_FOCUS: TypeRole = inter(24.0, 32.0, 600, -0.24, false, TextCase::AsWritten);
    pub const TYPE_LYRIC_PANEL: TypeRole = inter(18.0, 26.0, 600, -0.18, false, TextCase::AsWritten);
}
