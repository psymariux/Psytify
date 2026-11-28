//! Layout sizes (section 6), in dp unless noted.

// Window.
pub const WINDOW_MIN_W: f32 = 960.0;
pub const WINDOW_MIN_H: f32 = 600.0;

// Desktop (section 6.1).
/// Client-side decorations: mark, wordmark, history, search, account, window controls.
pub const TITLE_BAR_H: f32 = 40.0;
/// Sidebar width.
pub const SIDEBAR_W: f32 = 240.0;
/// Desktop v2 sidebar: expanded by default at this width, resizable by its edge between the two limits.
pub const SIDEBAR_DEFAULT_W: f32 = 264.0;
pub const SIDEBAR_MIN_W: f32 = 220.0;
pub const SIDEBAR_MAX_W: f32 = 360.0;
/// The rail: nav icons and the library as covers.
pub const SIDEBAR_RAIL_W: f32 = 72.0;
/// The grip on the sidebar's right edge that resizes it.
pub const SIDEBAR_GRIP_W: f32 = 8.0;
/// The Jam card above the library while a jam is live.
pub const JAM_CARD_H: f32 = 56.0;
/// Now playing, queue, lyrics.
pub const SIDE_PANEL_W: f32 = 320.0;
pub const PLAYER_BAR_H: f32 = 72.0;
pub const PAGE_GUTTER: f32 = 24.0;
/// Buttons, fields, icon buttons.
pub const CONTROL_H: f32 = 32.0;
/// Page play button, transport play/pause.
pub const CONTROL_H_LARGE: f32 = 40.0;
/// The primary play button of an object page's action row (desktop v2 §6).
pub const PLAY_HERO: f32 = 56.0;
/// Icon: a sprite of 12 AP.
pub const ICON: f32 = 24.0;
/// Click area and hover fill.
pub const ICON_BUTTON: f32 = 32.0;
pub const NAV_ITEM_H: f32 = 32.0;
/// Two lines (title above artist), cover `cover::COVER_XS`.
pub const TRACK_ROW_H: f32 = 48.0;
/// One line, column table (Library «Brani», downloads).
pub const TRACK_ROW_DENSE_H: f32 = 32.0;
/// Number / play / equaliser column.
pub const TRACK_INDEX_W: f32 = 32.0;
/// Minimum width of a tile; columns = floor((width + gap) / (TILE_MIN_W + GRID_GAP)).
pub const TILE_MIN_W: f32 = 160.0;
pub const GRID_GAP: f32 = 16.0;
pub const MENU_W: f32 = 240.0;
pub const MENU_ITEM_H: f32 = 32.0;
pub const TOOLTIP_MAX_W: f32 = 240.0;
/// 16 above the player bar, centred on the content column.
pub const TOAST_W: f32 = 360.0;
pub const DIALOG_W: f32 = 400.0;
pub const SEARCH_FIELD_MAX_W: f32 = 480.0;
/// The column of the sign-in screen (art direction, «Sign-in / social»).
pub const SIGN_IN_W: f32 = 360.0;
/// Seek track (2 AP).
pub const SEEK_TRACK_H: f32 = 4.0;
/// Seek hit area.
pub const SEEK_HIT_H: f32 = 24.0;
pub const VOLUME_W: f32 = 96.0;

// Breakpoints on the window width (dp).
/// Below: the sidebar is a rail whatever the setting.
pub const BP_RAIL: f32 = 1000.0;
/// Below: the side panel floats over the content instead of docking.
pub const BP_PANEL_DOCK: f32 = 1360.0;

// The frame (desktop v2 §2): the canvas fills the window between the sidebar and the side panel.
/// Canvas margin on each side, and the narrower one under `BP_FRAME_NARROW`.
pub const FRAME_MARGIN: f32 = 32.0;
pub const FRAME_MARGIN_NARROW: f32 = 24.0;
pub const BP_FRAME_NARROW: f32 = 1200.0;
/// Fluid tile grids: columns from the smallest tile, then the tiles stretch to fill the row.
pub const TILE_MIN: f32 = 168.0;
pub const TILE_GAP: f32 = 24.0;
/// Row lists that flow into columns: each column at least this wide, one column under `BP_FLOW_2`, two up
/// to `BP_FLOW_3`, three above (canvas width).
pub const FLOW_COL_MIN: f32 = 420.0;
pub const BP_FLOW_2: f32 = 900.0;
pub const BP_FLOW_3: f32 = 1500.0;
/// Gap between flowing columns.
pub const FLOW_GAP: f32 = 24.0;
/// Running text keeps this measure, in characters.
pub const MEASURE_CH: u32 = 72;

/// The canvas margin for a window `window_w` dp wide.
pub fn frame_margin(window_w: f32) -> f32 {
    if window_w < BP_FRAME_NARROW { FRAME_MARGIN_NARROW } else { FRAME_MARGIN }
}

/// Columns and tile side of a fluid grid `inner` dp wide (margins already taken off): `floor((inner + gap)
/// / (min + gap))` columns, at least one, and the tile that fills the row exactly.
pub fn tile_grid(inner: f32, min: f32, gap: f32) -> (usize, f32) {
    let fits = ((inner + gap) / (min + gap)).floor();
    // Small positive numbers: the truncation is the point.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let columns = if fits.is_finite() && fits >= 1.0 { fits as usize } else { 1 };
    let side = if inner.is_finite() && inner > 0.0 {
        ((inner - gap * (columns as f32 - 1.0)) / columns as f32).max(1.0)
    } else {
        min
    };
    (columns, side)
}

/// Columns of a flowing row list `inner` dp wide.
pub fn flow_columns(inner: f32) -> usize {
    let by_width = if inner < BP_FLOW_2 {
        1
    } else if inner < BP_FLOW_3 {
        2
    } else {
        3
    };
    // Never narrower than `FLOW_COL_MIN`.
    let fits = ((inner + FLOW_GAP) / (FLOW_COL_MIN + FLOW_GAP)).floor();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let fits = if fits.is_finite() && fits >= 1.0 { fits as usize } else { 1 };
    by_width.min(fits).max(1)
}

/// Number of columns of a tile grid in a column `width` dp wide (at least one).
pub fn grid_columns(width: f32) -> u32 {
    let cols = ((width + GRID_GAP) / (TILE_MIN_W + GRID_GAP)).floor();
    if cols.is_finite() && cols >= 1.0 { cols as u32 } else { 1 }
}

/// Artwork (section 6.2): multiples of 16 dp, so the 24 AP placeholder falls on whole APs.
pub mod cover {
    // The desktop v2 art ramp (§1): 40 rows, 56 shortcuts and the player bar, tiles fluid, 280 the side
    // panel, the stage fluid up to 720; 232 the object page header. Artist photos use the same sizes.
    pub const ART_ROW: f32 = 40.0;
    pub const ART_SHORTCUT: f32 = 56.0;
    pub const ART_PANEL: f32 = 280.0;
    pub const ART_HEADER: f32 = 232.0;
    /// The artist photo of the artist page header, in the pixel circle.
    pub const ART_ARTIST: f32 = 200.0;
    pub const ART_STAGE_MAX: f32 = 720.0;
    /// The art ramp, smallest first (the tile and stage are fluid: their floor and ceiling).
    pub const ART_RAMP: [f32; 5] = [ART_ROW, ART_SHORTCUT, super::TILE_MIN, ART_PANEL, ART_STAGE_MAX];

    /// Sidebar rows and playlists.
    pub const COVER_XS: f32 = 32.0;
    /// Player bar, menu, first search rows.
    pub const COVER_S: f32 = 48.0;
    /// Grid tile (grows with the column, snapped to 16).
    pub const COVER_M: f32 = 160.0;
    /// Album / playlist header.
    pub const COVER_L: f32 = 224.0;
    /// «In riproduzione» panel.
    pub const COVER_PANEL: f32 = 288.0;
    pub const COVER_FOCUS_MAX: f32 = 640.0;
    /// Avatar with a pixel circle mask: rows, profile, artist header.
    pub const AVATAR_S: f32 = 32.0;
    pub const AVATAR_M: f32 = 96.0;
    pub const AVATAR_L: f32 = 160.0;
    /// Below this side the placeholder is `icon_library` tinted `STONE`, above it `album_placeholder`.
    pub const PLACEHOLDER_ART_MIN: f32 = 64.0;
}

/// Phone (section 6.3).
pub mod phone {
    pub const PHONE_GUTTER: f32 = 16.0;
    /// Title (`TYPE_PAGE_TITLE`) or back.
    pub const PHONE_TOP_BAR_H: f32 = 48.0;
    /// Plus the system gesture inset below.
    pub const PHONE_TAB_BAR_H: f32 = 56.0;
    /// Above the tab bar.
    pub const PHONE_MINI_PLAYER_H: f32 = 56.0;
    /// Cover 48.
    pub const PHONE_ROW_H: f32 = 64.0;
    pub const PHONE_SHELF_COVER: f32 = 144.0;
    /// Transport play/pause.
    pub const PHONE_PLAY_BUTTON: f32 = 64.0;
    /// Minimum hit area (Android guideline, above the 44 minimum): it does not shrink to the sprite.
    pub const TOUCH_TARGET: f32 = 48.0;
    /// Dropdown menu rows, from below.
    pub const SHEET_ITEM_H: f32 = 56.0;
    /// Sheet handle, `STONE` bar: width and height in AP (16 x 2).
    pub const SHEET_HANDLE_AP: (u32, u32) = (16, 2);

    /// Side of the full-screen player cover: min(width - 32, 0.42 x height), in multiples of 16.
    pub fn player_cover(width: f32, height: f32) -> f32 {
        let side = (width - 2.0 * PHONE_GUTTER).min(0.42 * height);
        if side.is_finite() && side > 0.0 { (side / 16.0).floor() * 16.0 } else { 0.0 }
    }
}
