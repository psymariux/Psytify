// Generated from psytify-design by `cargo run -p psytify-mobile-gen --bin kotlin-tokens`.
// Do not edit: change the Rust tokens (design/src/tokens) and run the generator again.
package dev.psytify.app.design

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import dev.psytify.app.R

/** Palette and semantic colours. Lime marks a state only. */
object PsytifyColors {
    val DEEP_BLACK = Color(0xFF000000)
    val NEAR_BLACK = Color(0xFF0E0D0C)
    val GRAPHITE = Color(0xFF1A1917)
    val CHARCOAL = Color(0xFF2A2826)
    val IRON = Color(0xFF3D3A37)
    val STONE = Color(0xFF78746D)
    val MEDIUM_GREY = Color(0xFF9A958D)
    val SOFT_GREY = Color(0xFFBDB8AF)
    val WARM_WHITE = Color(0xFFECE8E0)
    val PURE_WHITE = Color(0xFFFFFFFF)
    val LIME = Color(0xFFC6F24E)
    val ERROR = Color(0xFFFF6B5E)
    val SCRIM = Color(0xB8000000)
    val SURFACE_CANVAS = Color(0xFF000000)
    val SURFACE_CHROME = Color(0xFF0E0D0C)
    val SURFACE_RAISED = Color(0xFF1A1917)
    val SURFACE_SELECTED = Color(0xFF2A2826)
    val SURFACE_WELL = Color(0xFF000000)
    val LINE_SUBTLE = Color(0xFF2A2826)
    val LINE_STRONG = Color(0xFF3D3A37)
    val CONTROL_BORDER = Color(0xFF78746D)
    val CONTROL_BORDER_HOVER = Color(0xFFBDB8AF)
    val FOCUS_RING = Color(0xFFFFFFFF)
    val LEDGE_PRIMARY = Color(0xFF78746D)
    val LEDGE_SECONDARY = Color(0xFF2A2826)
    val SHADOW_FLOAT = Color(0xFF000000)
    val TEXT_PRIMARY = Color(0xFFECE8E0)
    val TEXT_SECONDARY = Color(0xFFBDB8AF)
    val TEXT_TERTIARY = Color(0xFF9A958D)
    val TEXT_DISABLED = Color(0xFF78746D)
    val TEXT_ON_LIGHT = Color(0xFF000000)
    val TEXT_ERROR = Color(0xFFFF6B5E)
    val ICON_IDLE = Color(0xFFBDB8AF)
    val ICON_ACTIVE = Color(0xFFECE8E0)
    val ICON_DISABLED = Color(0xFF78746D)
    val STATE_ON = Color(0xFFC6F24E)
    val CARET = Color(0xFFFFFFFF)
}

/** Spacing scale, sizes in dp and art-pixel counts. */
object PsytifySpace {
    /** One art pixel in nominal dp; on screen it is a whole number of physical pixels. */
    const val AP_DP = 2.0f
    val SPACE_0 = Dp(0.0f)
    val SPACE_1 = Dp(2.0f)
    val SPACE_2 = Dp(4.0f)
    val SPACE_3 = Dp(8.0f)
    val SPACE_4 = Dp(12.0f)
    val SPACE_5 = Dp(16.0f)
    val SPACE_6 = Dp(24.0f)
    val SPACE_7 = Dp(32.0f)
    val SPACE_8 = Dp(48.0f)
    val SPACE_9 = Dp(64.0f)
    val CONTROL_H = Dp(32.0f)
    val CONTROL_H_LARGE = Dp(40.0f)
    val ICON = Dp(24.0f)
    val ICON_BUTTON = Dp(32.0f)
    val TRACK_ROW_H = Dp(48.0f)
    val GRID_GAP = Dp(16.0f)
    val SEEK_TRACK_H = Dp(4.0f)
    val SEEK_HIT_H = Dp(24.0f)
    val COVER_XS = Dp(32.0f)
    val COVER_S = Dp(48.0f)
    val COVER_M = Dp(160.0f)
    val COVER_L = Dp(224.0f)
    val AVATAR_S = Dp(32.0f)
    val PHONE_GUTTER = Dp(16.0f)
    val PHONE_TOP_BAR_H = Dp(48.0f)
    val PHONE_TAB_BAR_H = Dp(56.0f)
    val PHONE_MINI_PLAYER_H = Dp(56.0f)
    val PHONE_ROW_H = Dp(64.0f)
    val PHONE_SHELF_COVER = Dp(144.0f)
    val PHONE_PLAY_BUTTON = Dp(64.0f)
    val TOUCH_TARGET = Dp(48.0f)
    val SHEET_ITEM_H = Dp(56.0f)
    // Shapes in art pixels: each one is `artPx` physical pixels (`PsytifyCore.artPx`).
    const val BORDER_W_AP = 1
    const val FOCUS_RING_W_AP = 1
    const val MARKER_W_AP = 2
    const val NOTCH_1_AP = 1
    const val NOTCH_2_AP = 2
    const val LEDGE_X_AP = 1
    const val LEDGE_Y_AP = 1
    const val SHADOW_FLOAT_X_AP = 2
    const val SHADOW_FLOAT_Y_AP = 2
    const val SHEET_HANDLE_W_AP = 16
    const val SHEET_HANDLE_H_AP = 2
    // Durations in ms.
    const val MOTION_HOVER_MS = 70
    const val MOTION_FAST_MS = 100
    const val MOTION_BASE_MS = 160
    const val MOTION_PANEL_MS = 200
    const val MOTION_LYRICS_MS = 280
    const val LONG_PRESS_MS = 450
}

enum class TypeFamily { UI, PIXEL }

enum class TypeCase { AS_WRITTEN, SENTENCE, UPPER }

/** A type role: size and line height in sp, tracking in px (negative tightens). */
data class TypeRole(
    val family: TypeFamily,
    val size: Float,
    val lineHeight: Float,
    val weight: Int,
    val tracking: Float,
    val tabular: Boolean,
    val slashedZero: Boolean,
    val case: TypeCase,
)

/** Type roles, with the phone overrides in place of the desktop ones. */
object PsytifyType {
    val TYPE_DISPLAY = TypeRole(TypeFamily.UI, 26.0f, 32.0f, 600, -0.32f, false, false, TypeCase.AS_WRITTEN)
    val TYPE_TRACK_TITLE = TypeRole(TypeFamily.UI, 16.0f, 22.0f, 500, 0.0f, false, false, TypeCase.AS_WRITTEN)
    val TYPE_ARTIST = TypeRole(TypeFamily.UI, 14.0f, 20.0f, 400, 0.0f, false, false, TypeCase.AS_WRITTEN)
    val TYPE_BODY = TypeRole(TypeFamily.UI, 15.0f, 22.0f, 400, 0.0f, false, false, TypeCase.SENTENCE)
    val TYPE_NAV = TypeRole(TypeFamily.UI, 11.0f, 16.0f, 500, 0.0f, false, false, TypeCase.SENTENCE)
    val TYPE_LYRIC = TypeRole(TypeFamily.UI, 22.0f, 30.0f, 600, -0.24f, false, false, TypeCase.AS_WRITTEN)
    val TYPE_PAGE_TITLE = TypeRole(TypeFamily.PIXEL, 22.0f, 28.0f, 400, 0.0f, true, false, TypeCase.SENTENCE)
    val TYPE_HEADING = TypeRole(TypeFamily.UI, 16.0f, 24.0f, 600, -0.08f, false, false, TypeCase.SENTENCE)
    val TYPE_META = TypeRole(TypeFamily.UI, 12.0f, 16.0f, 400, 0.0f, true, false, TypeCase.SENTENCE)
    val TYPE_BUTTON = TypeRole(TypeFamily.UI, 13.0f, 16.0f, 600, 0.13f, false, false, TypeCase.SENTENCE)
    val TYPE_LABEL = TypeRole(TypeFamily.UI, 11.0f, 16.0f, 600, 0.66f, true, false, TypeCase.UPPER)
    val TYPE_NUMERIC = TypeRole(TypeFamily.UI, 12.0f, 16.0f, 400, 0.0f, true, false, TypeCase.AS_WRITTEN)
    val TYPE_LYRIC_FOCUS = TypeRole(TypeFamily.UI, 24.0f, 32.0f, 600, -0.24f, false, false, TypeCase.AS_WRITTEN)
    val TYPE_LYRIC_PANEL = TypeRole(TypeFamily.UI, 18.0f, 26.0f, 600, -0.18f, false, false, TypeCase.AS_WRITTEN)
    val TYPE_WORDMARK = TypeRole(TypeFamily.PIXEL, 22.0f, 28.0f, 400, 0.0f, false, false, TypeCase.AS_WRITTEN)
    val TYPE_NUMERIC_PIXEL = TypeRole(TypeFamily.PIXEL, 22.0f, 28.0f, 400, 0.0f, true, true, TypeCase.AS_WRITTEN)
}

/**
 * A sprite of `design/assets/pixel/export/1x`: frames side by side, `frameWidth` x `frameHeight` art
 * pixels each, shown for `durationsMs` (0 for a still). A mask is tinted with a colour token.
 */
data class Sprite(
    val res: Int,
    val frameWidth: Int,
    val frameHeight: Int,
    val durationsMs: List<Int>,
    val mask: Boolean,
)

object PsytifySprites {
    val ICON_PLAY = Sprite(R.drawable.icon_play, 12, 12, listOf(0), true)
    val ICON_PAUSE = Sprite(R.drawable.icon_pause, 12, 12, listOf(0), true)
    val ICON_PLAY_PAUSE = Sprite(R.drawable.icon_play_pause, 12, 12, listOf(0), true)
    val ICON_NEXT = Sprite(R.drawable.icon_next, 12, 12, listOf(0), true)
    val ICON_PREVIOUS = Sprite(R.drawable.icon_previous, 12, 12, listOf(0), true)
    val ICON_SHUFFLE = Sprite(R.drawable.icon_shuffle, 12, 12, listOf(0), true)
    val ICON_REPEAT = Sprite(R.drawable.icon_repeat, 12, 12, listOf(0), true)
    val ICON_REPEAT_ONE = Sprite(R.drawable.icon_repeat_one, 12, 12, listOf(0), true)
    val ICON_HEART = Sprite(R.drawable.icon_heart, 12, 12, listOf(0), true)
    val ICON_HEART_FILL = Sprite(R.drawable.icon_heart_fill, 12, 12, listOf(0), true)
    val ICON_HEART_HALF = Sprite(R.drawable.icon_heart_half, 12, 12, listOf(0), true)
    val ICON_DOWNLOAD = Sprite(R.drawable.icon_download, 12, 12, listOf(0), true)
    val ICON_DOWNLOADED = Sprite(R.drawable.icon_downloaded, 12, 12, listOf(0), true)
    val ICON_QUEUE = Sprite(R.drawable.icon_queue, 12, 12, listOf(0), true)
    val ICON_SEARCH = Sprite(R.drawable.icon_search, 12, 12, listOf(0), true)
    val ICON_HOME = Sprite(R.drawable.icon_home, 12, 12, listOf(0), true)
    val ICON_LIBRARY = Sprite(R.drawable.icon_library, 12, 12, listOf(0), true)
    val ICON_MORE = Sprite(R.drawable.icon_more, 12, 12, listOf(0), true)
    val ICON_CLOSE = Sprite(R.drawable.icon_close, 12, 12, listOf(0), true)
    val ICON_CHECK = Sprite(R.drawable.icon_check, 12, 12, listOf(0), true)
    val ICON_CHEVRON_LEFT = Sprite(R.drawable.icon_chevron_left, 12, 12, listOf(0), true)
    val ICON_CHEVRON_RIGHT = Sprite(R.drawable.icon_chevron_right, 12, 12, listOf(0), true)
    val ICON_CHEVRON_DOWN = Sprite(R.drawable.icon_chevron_down, 12, 12, listOf(0), true)
    val ICON_PLUS = Sprite(R.drawable.icon_plus, 12, 12, listOf(0), true)
    val ICON_VOLUME = Sprite(R.drawable.icon_volume, 12, 12, listOf(0), true)
    val ICON_VOLUME_LOW = Sprite(R.drawable.icon_volume_low, 12, 12, listOf(0), true)
    val ICON_MUTE = Sprite(R.drawable.icon_mute, 12, 12, listOf(0), true)
    val ICON_LYRICS = Sprite(R.drawable.icon_lyrics, 12, 12, listOf(0), true)
    val ICON_FOCUS = Sprite(R.drawable.icon_focus, 12, 12, listOf(0), true)
    val ICON_FULLSCREEN = Sprite(R.drawable.icon_fullscreen, 12, 12, listOf(0), true)
    val ICON_FULLSCREEN_EXIT = Sprite(R.drawable.icon_fullscreen_exit, 12, 12, listOf(0), true)
    val ICON_MINIMIZE = Sprite(R.drawable.icon_minimize, 12, 12, listOf(0), true)
    val ICON_MAXIMIZE = Sprite(R.drawable.icon_maximize, 12, 12, listOf(0), true)
    val ICON_WINDOW_CLOSE = Sprite(R.drawable.icon_window_close, 12, 12, listOf(0), true)
    val ICON_SETTINGS = Sprite(R.drawable.icon_settings, 12, 12, listOf(0), true)
    val ICON_SETTINGS_FILL = Sprite(R.drawable.icon_settings_fill, 12, 12, listOf(0), true)
    val ICON_ACCOUNT = Sprite(R.drawable.icon_account, 12, 12, listOf(0), true)
    val ICON_PEOPLE = Sprite(R.drawable.icon_people, 12, 12, listOf(0), true)
    val ICON_PEOPLE_FILL = Sprite(R.drawable.icon_people_fill, 12, 12, listOf(0), true)
    val ICON_BELL = Sprite(R.drawable.icon_bell, 12, 12, listOf(0), true)
    val ICON_BELL_FILL = Sprite(R.drawable.icon_bell_fill, 12, 12, listOf(0), true)
    val ICON_RADIO = Sprite(R.drawable.icon_radio, 12, 12, listOf(0), true)
    val ICON_EDIT = Sprite(R.drawable.icon_edit, 12, 12, listOf(0), true)
    val ICON_TRASH = Sprite(R.drawable.icon_trash, 12, 12, listOf(0), true)
    val ICON_COPY = Sprite(R.drawable.icon_copy, 12, 12, listOf(0), true)
    val ICON_KEYBOARD = Sprite(R.drawable.icon_keyboard, 12, 12, listOf(0), true)
    val ICON_VIDEO = Sprite(R.drawable.icon_video, 12, 12, listOf(0), true)
    val ICON_GRIP = Sprite(R.drawable.icon_grip, 12, 12, listOf(0), true)
    val ICON_HOME_FILL = Sprite(R.drawable.icon_home_fill, 12, 12, listOf(0), true)
    val ICON_SEARCH_FILL = Sprite(R.drawable.icon_search_fill, 12, 12, listOf(0), true)
    val ICON_LIBRARY_FILL = Sprite(R.drawable.icon_library_fill, 12, 12, listOf(0), true)
    val ICON_LIST = Sprite(R.drawable.icon_list, 12, 12, listOf(0), true)
    val ICON_NOTE = Sprite(R.drawable.icon_note, 12, 12, listOf(0), true)
    val ICON_MOON = Sprite(R.drawable.icon_moon, 12, 12, listOf(0), true)
    val ICON_SPARKLE = Sprite(R.drawable.icon_sparkle, 12, 12, listOf(0), true)
    val ICON_CHART = Sprite(R.drawable.icon_chart, 12, 12, listOf(0), true)
    val ICON_EQUALIZER = Sprite(R.drawable.icon_equalizer, 12, 12, listOf(0), true)
    val ICON_USER_PLUS = Sprite(R.drawable.icon_user_plus, 12, 12, listOf(0), true)
    val ICON_BELL_RING = Sprite(R.drawable.icon_bell_ring, 12, 12, listOf(600, 80, 80, 80, 80, 80, 80, 80), true)
    val ICON_DOWNLOAD_BUSY = Sprite(R.drawable.icon_download_busy, 12, 12, listOf(140, 140, 140, 140), true)
    val ICON_PEOPLE_LIVE = Sprite(R.drawable.icon_people_live, 12, 12, listOf(260, 260), true)
    val NASTRO_CHEER = Sprite(R.drawable.nastro_cheer, 32, 36, listOf(90, 80, 80, 80, 80, 80, 90, 90, 80, 80, 80, 500), false)
    val EQ = Sprite(R.drawable.eq, 12, 12, listOf(120, 120, 120, 120, 120, 120), true)
    val EQ_REST = Sprite(R.drawable.eq_rest, 12, 12, listOf(0), true)
    val MARK = Sprite(R.drawable.mark, 12, 12, listOf(0), true)
    val MARK_PLAY = Sprite(R.drawable.mark_play, 12, 12, listOf(160, 160), true)
    val LOADER = Sprite(R.drawable.loader, 12, 12, listOf(90, 90, 90, 90, 90, 90, 90, 90), false)
    val ALBUM_PLACEHOLDER = Sprite(R.drawable.album_placeholder, 24, 24, listOf(0), false)
    val NASTRO_IDLE = Sprite(R.drawable.nastro_idle, 32, 32, listOf(1000, 240, 1000, 120), false)
    val NASTRO_LISTEN = Sprite(R.drawable.nastro_listen, 32, 32, listOf(140, 140, 140, 140), false)
    val NASTRO_SLEEP = Sprite(R.drawable.nastro_sleep, 32, 32, listOf(800, 800), false)
    val EMPTY_CRATE = Sprite(R.drawable.empty_crate, 64, 28, listOf(0), false)
    val LIKED_COVER = Sprite(R.drawable.liked_cover, 24, 24, listOf(0), true)
    val APP_ICON = Sprite(R.drawable.app_icon, 16, 16, listOf(0), false)
}
