# Psytify design tokens

Status: v1, 2026-11-22. Owner: art direction. Replaces the tokens in `docs/STYLE.md` and
`client/src/ui/theme.rs` once `psytify-design` lands (migration step 3b).

These are framework-neutral data. `psytify-design` stores them as plain constants (colors as `[u8; 4]`
RGBA, lengths as `f32` dp, durations as `u32` ms). GPUI and Iced each convert them to their own types.
No frontend defines its own color, length or duration. Identifiers below are the Rust names
(`SCREAMING_SNAKE_CASE`, grouped by module: `color`, `space`, `size`, `type_`, `motion`, `z`).

Units: **dp** = logical pixel (GPUI `px`, Iced logical px, Android dp). **AP** = art pixel = 2 dp nominal.

## 1. Palette (`color`)

Twelve values in total: eight greys that carry the hierarchy, two support greys, one accent, one error.
Everything is slightly warm (R − B = 2 to 14) so album covers, the only real chroma on screen, do not sit on
a cold blue-grey.

| Token | Hex | RGBA bytes | Role |
|---|---|---|---|
| `DEEP_BLACK` | `#000000` | `[0,0,0,255]` | Content canvas, wells (input fill), floating shadows, text on light fills |
| `NEAR_BLACK` | `#0e0d0c` | `[14,13,12,255]` | Chrome: title bar, sidebar, side panel, player bar, phone tab bar |
| `GRAPHITE` | `#1a1917` | `[26,25,23,255]` | Hover fill, cards, menus, tooltips, dialogs, cover placeholder field |
| `CHARCOAL` | `#2a2826` | `[42,40,38,255]` | Selected row, pressed fill, region separators, slider track, secondary ledge |
| `IRON` | `#3d3a37` | `[61,58,55,255]` | Border of floating layers, strong separator, sprite shade (decorative only) |
| `STONE` | `#78746d` | `[120,116,109,255]` | Control borders (≥ 3:1), disabled text and icons, primary-button ledge, resting equalizer |
| `MEDIUM_GREY` | `#9a958d` | `[154,149,141,255]` | Tertiary text: metadata, durations, placeholders, track numbers |
| `SOFT_GREY` | `#bdb8af` | `[189,184,175,255]` | Secondary text: artist lines, idle icons, idle nav labels |
| `WARM_WHITE` | `#ece8e0` | `[236,232,224,255]` | Primary text, primary-button fill, active icons, seek elapsed fill |
| `PURE_WHITE` | `#ffffff` | `[255,255,255,255]` | Focus ring, text caret, primary-button hover fill, seek thumb, sprite glints |
| `LIME` | `#c6f24e` | `[198,242,78,255]` | Semantic state only (section 1.3) |
| `ERROR` | `#ff6b5e` | `[255,107,94,255]` | Errors and destructive actions only |

`SCRIM` = `DEEP_BLACK` at alpha 184 (72 %), `[0,0,0,184]`: dialog and sheet backdrop. It is the only
translucent color. Nothing else uses alpha: no glass, no blur, no tinted overlays.

Removed from the old theme: `ACCENT_HOVER`, all `*_TINT`, `BROWSE[9]`, `NEGATIVE #e5566f` (it failed
4.5:1 on the new `CHARCOAL`; `ERROR` replaces it).

### 1.1 Semantic aliases

Frontends use the aliases, never the raw greys, so a re-tune touches one table.

| Alias | Value | Alias | Value |
|---|---|---|---|
| `SURFACE_CANVAS` | `DEEP_BLACK` | `TEXT_PRIMARY` | `WARM_WHITE` |
| `SURFACE_CHROME` | `NEAR_BLACK` | `TEXT_SECONDARY` | `SOFT_GREY` |
| `SURFACE_RAISED` | `GRAPHITE` | `TEXT_TERTIARY` | `MEDIUM_GREY` |
| `SURFACE_SELECTED` | `CHARCOAL` | `TEXT_DISABLED` | `STONE` |
| `SURFACE_WELL` | `DEEP_BLACK` | `TEXT_ON_LIGHT` | `DEEP_BLACK` |
| `LINE_SUBTLE` | `CHARCOAL` | `TEXT_ERROR` | `ERROR` |
| `LINE_STRONG` | `IRON` | `ICON_IDLE` | `SOFT_GREY` |
| `CONTROL_BORDER` | `STONE` | `ICON_ACTIVE` | `WARM_WHITE` |
| `CONTROL_BORDER_HOVER` | `SOFT_GREY` | `ICON_DISABLED` | `STONE` |
| `FOCUS_RING` | `PURE_WHITE` | `STATE_ON` | `LIME` |
| `LEDGE_PRIMARY` | `STONE` | `LEDGE_SECONDARY` | `CHARCOAL` |
| `SHADOW_FLOAT` | `DEEP_BLACK` | `CARET` | `PURE_WHITE` |
| `SHADOW_HARD` | `IRON` | | |

### 1.2 Contrast (WCAG 2.x, computed with the relative-luminance formula)

Text appears only on the four surfaces. Every text tier is ≥ 4.5:1 on every surface; control borders are
≥ 3:1 on every surface.

| Foreground | on `DEEP_BLACK` | on `NEAR_BLACK` | on `GRAPHITE` | on `CHARCOAL` | Use |
|---|---|---|---|---|---|
| `WARM_WHITE` | 17.19 | 15.89 | 14.38 | 12.02 | text, AAA |
| `PURE_WHITE` | 21.00 | 19.42 | 17.57 | 14.69 | focus, caret |
| `SOFT_GREY` | 10.64 | 9.84 | 8.90 | 7.44 | text, AAA |
| `MEDIUM_GREY` | 7.06 | 6.53 | 5.90 | 4.94 | text, AA |
| `STONE` | 4.52 | 4.18 | 3.78 | 3.16 | control border ≥ 3:1; disabled text (WCAG-exempt) |
| `IRON` | 1.86 | 1.72 | 1.56 | 1.30 | decorative only, never the only cue |
| `CHARCOAL` | 1.43 | 1.32 | 1.20 | — | separators (non-essential boundaries) |
| `LIME` | 16.21 | 14.99 | 13.56 | 11.34 | state marks |
| `ERROR` | 7.52 | 6.95 | 6.29 | 5.26 | error text, AA |

Light fills: `DEEP_BLACK` on `WARM_WHITE` 17.19, on `PURE_WHITE` 21.00, on `LIME` 16.21, on `ERROR` 7.52.
`STONE` ledge against `WARM_WHITE` fill 3.80 (the ledge stays visible).

Test to keep: the existing `every_text_tier_passes_wcag_aa_on_every_surface` moves into `psytify-design`
and iterates `{TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY, TEXT_ERROR} x {the four surfaces}` (≥ 4.5) and
`CONTROL_BORDER x surfaces` (≥ 3.0).

### 1.3 Lime rules

`LIME` marks exactly four states: **active playback** (equalizer, mark while playing, Nastro's LED),
**selected primary navigation** (sidebar item, phone tab), **toggles that are on** (switches, shuffle, repeat),
**live indicators** (friend listening now, jam in progress). It is never a fill bigger than 4 x 4 AP, never text
longer than one word (`LIVE`), never a border wider than 1 AP, never a glow, never decoration. Every lime cue
comes with a shape cue (marker bar, dot, filled icon, knob position) so state never depends on color alone.
Budget: at most one lime cluster per region (chrome bar, list, panel) at a time, plus the equalizer.

## 2. Art pixel and scale (`space::AP_DP`, `space::ap_device`)

`AP_DP = 2.0`. One art pixel is 2 dp at every scale and on every platform. Sprites, borders, separators,
focus rings, ledges and pixel type all use the same AP, so nothing on screen has two pixel sizes (no mixels).

Device pixels per AP are quantised: `ap_device(scale) = max(1, floor(AP_DP * scale + 0.001))`.
A sprite of `n` art pixels is painted `n * ap_device` device px wide, at a device-px-snapped origin, centred in
its nominal `n * 2` dp slot with the offset floored to whole device px. Borders and separators are drawn
`ap_device` device px thick at snapped positions.

| Scale | Where | `ap_device` | AP in dp | 12 AP icon (dp) | 32 AP mascot (dp) |
|---|---|---|---|---|---|
| 1.0 | desktop 1x, Android mdpi | 2 | 2.00 | 24.0 | 64.0 |
| 1.25 | desktop | 2 | 1.60 | 19.2 | 51.2 |
| 1.5 | desktop, Android hdpi | 3 | 2.00 | 24.0 | 64.0 |
| 1.75 | desktop | 3 | 1.71 | 20.6 | 54.9 |
| 2.0 | desktop HiDPI, Android xhdpi | 4 | 2.00 | 24.0 | 64.0 |
| 2.625 / 2.75 | Android 420 / 440 dpi | 5 | 1.90 / 1.82 | 22.9 / 21.8 | 61.0 / 58.2 |
| 3.0 | Android xxhdpi | 6 | 2.00 | 24.0 | 64.0 |
| 3.5 | Android 560 dpi | 7 | 2.00 | 24.0 | 64.0 |
| 4.0 | Android xxxhdpi | 8 | 2.00 | 24.0 | 64.0 |

Rules: at fractional scales the art is at most one AP-step smaller than nominal, never larger, never blurred.
Layout lengths stay in dp; only sprites, lines and pixel type use `ap_device`. Rebuild the upscaled sprite cache
when the scale factor changes.

## 3. Spacing (`space`)

Base unit 4 dp (2 AP). Every gap, padding and size is a multiple of 2 dp; component sizes are multiples of 4 dp.

| Token | dp | AP | Typical use |
|---|---|---|---|
| `SPACE_0` | 0 | 0 | flush regions |
| `SPACE_1` | 2 | 1 | ledge offset, marker inset, focus gap |
| `SPACE_2` | 4 | 2 | icon-to-text in badges, separator margins |
| `SPACE_3` | 8 | 4 | icon-to-label, row inner gaps, list section gap |
| `SPACE_4` | 12 | 6 | button horizontal padding (compact), menu item padding |
| `SPACE_5` | 16 | 8 | button padding, grid gap, phone gutter |
| `SPACE_6` | 24 | 12 | desktop page gutter, section gap |
| `SPACE_7` | 32 | 16 | header to content |
| `SPACE_8` | 48 | 24 | empty-state vertical rhythm |
| `SPACE_9` | 64 | 32 | splash, sign-in top offset |

## 4. Corners, borders, ledges, shadows

No border radius anywhere (`RADIUS_NONE = 0`). Roundness is a stepped notch drawn on the AP grid
(axis-aligned quads or a 9-slice sprite), never an anti-aliased curve.

| Token | Value | Use |
|---|---|---|
| `NOTCH_0` | 0 AP | covers, rows, tiles, panels, separators |
| `NOTCH_1` | 1 AP cut at each corner | buttons, icon buttons, fields, badges, tooltips, toasts, toggles |
| `NOTCH_2` | 2-step cut (1 AP + 1 AP, 45°) | menus, dialogs, bottom sheets, cards |
| `MASK_PIXEL_CIRCLE` | circle rasterised at AP resolution (diameter = side / AP), edges step by 1 AP | artist photos, avatars |
| `BORDER_W` | 1 AP | every border and separator (the only width) |
| `FOCUS_RING_W` | 1 AP | focus ring |
| `FOCUS_RING_GAP` | 1 AP | gap between control edge and ring (surface shows through) |
| `MARKER_W` | 2 AP | selected-nav marker bar, error marker on toasts |
| `LEDGE_OFFSET` | (1 AP, 1 AP) | hard ledge under primary and secondary buttons (blur 0) |
| `SHADOW_FLOAT_OFFSET` | (2 AP, 2 AP) | menus, dialogs, toasts, drag ghost (blur 0, `SHADOW_FLOAT`) |
| hard ledge | (2 AP, 2 AP) | a hovered tile, the sidebar laid over the canvas (blur 0, `SHADOW_HARD`; desktop v2 §3) |

Blur is 0 everywhere. A ledge reads as the side of a key: the button sits 1 AP up-left of a darker/lighter slab;
pressed, it translates by `LEDGE_OFFSET` and the slab disappears (section 8).

## 5. Type (`type_`)

Two families. Inter for every string the catalogue or the user wrote and for all running UI text.
Departure Mono only for strings Psytify wrote (screen titles, wordmark, a few numerals), because catalogue
strings can be any script and any length.

| Token | Family | File | Licence (verified 2026-11-22) |
|---|---|---|---|
| `FAMILY_UI` | Inter 4.1 (400, 500, 600) | `client/assets/fonts/Inter-*.ttf` (bundled) | SIL OFL 1.1, `OFL.txt` bundled |
| `FAMILY_PIXEL` | Departure Mono 1.500 (Regular) | to bundle: `DepartureMono-Regular.otf` from `github.com/rektdeckard/departure-mono` `public/assets/` | SIL OFL 1.1: `public/assets/LICENSE` in that repo ("Copyright 2022–2024 Helena Zhang"); the repo-root `LICENSE` is MIT and covers only the website. Name table ID 13 also states OFL 1.1. Bundle the OFL text as `DepartureMono-OFL.txt` and add a CREDITS line. sha256 of the checked file: `4d53f663155cf8bf7ffc8e688776e719625f7bbb80a8d90073438b249261a2e0` |

Departure Mono metrics: 550 units/em, one font pixel = 50 units, so at 11 px it is one device px per font pixel;
cap height 8 font px, x-height 6, advance 7. Covers Italian (à è é ì ò ù, « », –, …). Features: `tnum`, `zero`,
`case`, `ss01`, `ss02`.

**Ramp (desktop v2, `type_::RAMP`).** Five sizes and a lyric pair; every role maps onto one of them. Sizes are
dp and follow the UI zoom (section 6.3).

| Step | Family / weight | Size / line (dp) | Case | Roles |
|---|---|---|---|---|
| `DISPLAY` | Departure Mono 400 | 40 / 44 | as written | `TYPE_DISPLAY` (page header names), `TYPE_PAGE_TITLE` (screen titles, `tnum`, sentence) |
| `TITLE` | Inter 600 | 22 / 28 | sentence | `TYPE_HEADING` (section headings, dialog titles); in the pixel face: `TYPE_WORDMARK`, `TYPE_NUMERIC_PIXEL` (`tnum`, `zero`) |
| `BODY` | Inter 500 | 15 / 20 | as written | `TYPE_TRACK_TITLE`, `TYPE_NAV` (sentence); `TYPE_BODY` at 400 |
| `META` | Inter 400 | 13 / 18 | sentence | `TYPE_ARTIST`, `TYPE_META` and `TYPE_NUMERIC` (`tnum`); `TYPE_BUTTON` at 600 |
| `CAPTION` | Departure Mono 400 | 11 / 14 | UPPERCASE | `TYPE_LABEL` (kickers, column headers, badges: `LOSSLESS`, `LIVE`), `tnum` |
| `LYRIC_PANEL` / `LYRIC_STAGE` | Inter 600 | 20 / 28, 32 / 40 | as written | `TYPE_LYRIC_PANEL`, `TYPE_LYRIC_FOCUS` |

**Pixel type size rule:** one font pixel is a whole number of device px. A pixel-face role is drawn at
`11 * k` device px, `k = max(1, round(target / 11))` with ties rounding up (the caption at zoom 1.5 is 22, not 11), where `target` is the role size in
device px (dp x scale x zoom); the line height scales with it (`type_::pixel_face_device`). Text origins snap to
whole device px, tracking 0. So `CAPTION` is 11 device px at 1x, `TITLE` in the pixel face 22, `DISPLAY` 44.

The Iced (Android) app keeps the v1 sizes through `type_::v1` (display 32, heading 16, body 14, meta 12).

Phone overrides (`type_::phone`): `TYPE_DISPLAY` 26 / 32; `TYPE_TRACK_TITLE` 16 / 22; `TYPE_ARTIST` 14 / 20;
`TYPE_BODY` 15 / 22; `TYPE_NAV` (tab labels) Inter 500 11 / 16; `TYPE_LYRIC` 22 / 30. Pixel type is unchanged
(22 dp): it is tied to the AP, not to the platform.

Truncation: one line with "…" for titles and artists in rows and tiles; `TYPE_DISPLAY` wraps to at most two lines.

## 6. Layout (`size`)

### 6.1 Desktop (GPUI)

| Token | dp | Note |
|---|---|---|
| `WINDOW_MIN_W` x `WINDOW_MIN_H` | 960 x 600 | |
| `TITLE_BAR_H` | 40 | client-side decorations; mark, wordmark, history, search, account, window controls |
| `SIDEBAR_W` | 240 | the Iced sidebar |
| `SIDEBAR_DEFAULT_W` | 264 | expanded sidebar (desktop v2 §4); flush, `SURFACE_CHROME`, 1 AP `LINE_SUBTLE` on its right edge |
| `SIDEBAR_MIN_W` / `SIDEBAR_MAX_W` | 220 / 360 | drag range of the right edge (`SIDEBAR_GRIP_W` 8 hit strip) |
| `SIDEBAR_RAIL_W` | 72 | rail: nav icons and 40 dp library covers |
| `JAM_CARD_H` | 56 | the sidebar Jam card while a jam is live |
| `SIDE_PANEL_W` | 320 | now playing / queue / lyrics |
| `PLAYER_BAR_H` | 72 | bottom strip, 1 AP `LINE_SUBTLE` on its top edge |
| `PAGE_GUTTER` | 24 | content padding left/right/top (pages not yet on the frame) |
| `FRAME_MARGIN` / `FRAME_MARGIN_NARROW` | 32 / 24 | canvas margin each side; narrow under `BP_FRAME_NARROW` (1200) of window. No content cap |
| `TILE_MIN` / `TILE_GAP` | 168 / 24 | fluid tile grid: as many columns as fit, tiles stretch to fill the row (`size::tile_grid`) |
| `FLOW_COL_MIN` / `FLOW_GAP` | 420 / 24 | flowing columns for row lists: 1 under `BP_FLOW_2` (900), 2 up to `BP_FLOW_3` (1500), 3 above (`size::flow_columns`) |
| `MEASURE_CH` | 72 | longest line of running text, in characters |
| `CONTROL_H` | 32 | buttons, fields, icon buttons |
| `CONTROL_H_LARGE` | 40 | page Play button, transport play/pause |
| `ICON` | 24 | a 12 AP sprite |
| `ICON_BUTTON` | 32 | hit area and hover fill |
| `NAV_ITEM_H` | 32 | |
| `TRACK_ROW_H` | 48 | two lines (title over artist), cover `COVER_XS` |
| `TRACK_ROW_DENSE_H` | 32 | one line, column table (Library "Brani", downloads) |
| `TRACK_INDEX_W` | 32 | number / play / equalizer column |
| `TILE_MIN_W` | 160 | grid tile; columns = floor((width + gap) / (160 + 16)) |
| `GRID_GAP` | 16 | |
| `MENU_W` | 240 | `MENU_ITEM_H` 32 |
| `TOOLTIP_MAX_W` | 240 | |
| `TOAST_W` | 360 | 16 above the player bar, centred on the content column |
| `DIALOG_W` | 400 | |
| `SEARCH_FIELD_MAX_W` | 480 | |
| `SIGN_IN_W` | 360 | the sign-in column (art direction, "Sign-in / social") |
| `SEEK_TRACK_H` | 4 | 2 AP on hover (1 AP at rest, with a 2 x 4 AP thumb on hover; desktop v2 §3); hit area 24 |
| `VOLUME_W` | 96 | |

Breakpoints (window width in dp, after zoom): `BP_RAIL = 1000` (below: the sidebar is a rail whatever the setting,
the toggle lays the expanded sidebar over the canvas), `BP_PANEL_DOCK = 1360` (below: the side panel overlays the
content instead of docking), `BP_FRAME_NARROW = 1200` (below: 24 dp margins).

### 6.2 Artwork (`size::cover`)

Multiples of 16 dp so the 24 AP placeholder centres on whole APs.

| Token | dp | Use |
|---|---|---|
| `COVER_XS` | 32 | rows, sidebar playlists |
| `COVER_S` | 48 | player bar, menus, search top rows |
| `COVER_M` | 160 | grid tiles (grows with the column width, snapped to 16) |
| `COVER_L` | 224 | album / playlist page header |
| `COVER_PANEL` | 288 | side panel "In riproduzione" |
| `COVER_FOCUS_MAX` | 640 | focus view |
| `AVATAR_S` / `AVATAR_M` / `AVATAR_L` | 32 / 96 / 160 | rows / profile / artist header (pixel-circle mask) |

Desktop v2 art ramp (`cover::ART_RAMP`): `ART_ROW` 40 (rows, sidebar library), `ART_SHORTCUT` 56 (Home
shortcuts), tiles from the fluid grid (at least `TILE_MIN` 168), `ART_HEADER` 232 (page headers), `ART_PANEL` 280
(side panel), `ART_STAGE_MAX` 720 (the stage).

Covers are photos: linear filtering, square, no radius, no tint extraction. Missing cover: `GRAPHITE` field with
`album_placeholder` (24 AP) centred at 1 AP if the cover is ≥ 64 dp, else `icon_library` tinted `STONE`.

### 6.3 Zoom (`zoom`, desktop v2 §1)

Every dp length, type size included, is multiplied by one global zoom `z`. `Auto`: `z = clamp(width / 1600, 1, 1.5)`
snapped to steps of `STEP` 0.125, kept until the window is 48 dp (`HYSTERESIS`) narrower than the step needs.
Pinned values (`PRESETS`): 90, 100, 112, 125, 150 %; Ctrl+= / Ctrl+- step through them, Ctrl+0 goes back to `Auto`.
The art pixel follows the zoom: `ap_device(scale * z)`, so sprites, lines and pixel type stay whole and the covers
and the sprite cache rebuild when it changes.

### 6.4 Phone (Iced)

| Token | dp | Note |
|---|---|---|
| `PHONE_GUTTER` | 16 | |
| `PHONE_TOP_BAR_H` | 48 | title (`TYPE_PAGE_TITLE`) or back |
| `PHONE_TAB_BAR_H` | 56 | + the system gesture inset below |
| `PHONE_MINI_PLAYER_H` | 56 | above the tab bar |
| `PHONE_ROW_H` | 64 | cover 48 |
| `PHONE_SHELF_COVER` | 144 | |
| `PHONE_PLAYER_COVER` | min(width − 32, 0.42 x height), floored to 16 | |
| `PHONE_PLAY_BUTTON` | 64 | transport play/pause |
| `TOUCH_TARGET` | 48 | minimum hit area (Android guideline; above the 44 dp floor). Hit areas never shrink to the sprite |
| `SHEET_ITEM_H` | 56 | bottom-sheet menu rows |
| `SHEET_HANDLE` | 16 x 2 AP | `STONE` bar |

## 7. Motion (`motion`)

Two clocks. The **sprite clock** steps frames at the durations in `design/assets/pixel/export/sprites.json`
(never interpolated). The **UI clock** eases colors, opacity and AP-quantised translation.

| Token | ms | Curve | Use |
|---|---|---|---|
| `MOTION_PRESS` | 0 | — | press translate and ledge collapse (instant, like a key) |
| `MOTION_HOVER` | 70 | `EASE_OUT` | hover fill/border/icon tint |
| `MOTION_FAST` | 100 | `EASE_OUT` | tooltip and menu appear (opacity only), toggle knob (2 steps of 50) |
| `MOTION_BASE` | 160 | `EASE_OUT` | page content fade-in, toast in/out, dialog in (opacity + scrim) |
| `MOTION_PANEL` | 200 | `EASE_OUT` | side panel / rail open, sidebar width, sheet slide; offsets quantised to `ap_device` |
| `MOTION_ENTER` | 220 | `EASE_OUT` | page enter: each section rises `ENTER_RISE_AP` (4 AP) with opacity, staggered by `STAGGER_MS` (40) for at most `STAGGER_MAX_STEPS` (6); no rise on Back and Forward |
| `MOTION_LYRICS` | 280 | `EASE_OUT` | lyric line scroll, quantised to whole device px |
| `TOOLTIP_DELAY` | 500 | — | |
| `TOAST_HOLD` | 4000 | — | |
| `CARET_BLINK` | 530 on / 530 off | step | text caret |
| `PROGRESS_TICK` | 250 | — | seek bar and time refresh (no per-frame updates) |
| `SPRITE_FRAME_MIN` | 80 | step | no sprite frame shorter than this |

Stepped moves (`STEP_FRAMES` = 3): the tile play button, the shortcut play button and the selection marker step
in over three frames instead of easing. Every eased translation lands on whole APs on every frame, and a view asks
for frames only while something runs (desktop v2 §3).

`EASE_OUT` = cubic-bezier(0.2, 0.0, 0.0, 1.0). No springs, no overshoot, no bounce, no parallax, no blur
transitions. Sprite and idle loops run only while visible and the window is focused; they pause on frame 0.
`reduce_motion` (setting, default follows the OS): sprite loops show frame 0, all UI durations become 0,
`PROGRESS_TICK` stays.

## 8. Interaction offsets

| Token | Value |
|---|---|
| `PRESS_TRANSLATE` | (1 AP, 1 AP) for controls with a ledge; 0 for ghost controls |
| `DRAG_THRESHOLD` | 4 dp desktop, 8 dp phone |
| `LONG_PRESS` | 450 ms (phone) |
| `DOUBLE_CLICK` | 400 ms |

## 9. Z layers (`z`)

| Token | Value | Token | Value |
|---|---|---|---|
| `Z_BASE` | 0 | `Z_SCRIM` | 50 |
| `Z_STICKY` | 10 (sticky headers, column headers) | `Z_DIALOG` | 51 |
| `Z_PANEL_OVERLAY` | 20 (side panel below `BP_PANEL_DOCK`, rail flyout) | `Z_TOAST` | 60 |
| `Z_MENU` | 30 (context menu, dropdown, popover) | `Z_DRAG` | 70 (drag ghost) |
| `Z_TOOLTIP` | 40 | `Z_SPLASH` | 100 |

## 10. Sprites (`sprite`)

Native sizes and frame data live in `design/assets/pixel/export/sprites.json` (generated). `psytify-design`
embeds the 1x PNGs, decodes them once (`sprite::sheet()`, 160,896 bytes of RGBA for the 74 sprites) and exposes
`sprite::upscale_nearest(&Frame, factor)` and `sprite::upscale_tinted(&Frame, factor, tint)` (tint applies to
mask sprites: the 56 icons plus `eq`, `eq_rest`, `mark`, `mark_play`, `liked_cover`). `factor` is
`scale::art_px(scale_factor)` (this document's `ap_device`; the doc name `space::ap_device` is `scale::art_px`
in code). Cache `sprite::UpscaleCache`, owned by the frontend, key `(sprite, frame, factor, tint)`, LRU, two
declared limits: 128 entries and 8 MiB of pixels. The byte limit is the one that binds: a 32 x 32 sprite at
`ap_device` 8 is 256 KiB, so 128 such entries would be 32 MiB. Details: `PSYTIFY_PIXEL_ART_GUIDELINES.md`
section 10.
