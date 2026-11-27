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

**Pixel type size rule:** font pixel = 1 AP. Device size = `11 * ap_device` px (22 dp nominal), line height
`14 * ap_device` px (28 dp), text origin snapped to whole device px, tracking 0. No other pixel size exists:
no 11 dp micro labels (half an AP), no 44 dp display (two APs). Verify in step 3b that cosmic-text (GPUI, Iced)
renders an exact multiple without hinting blur; if it blurs, rasterise the needed strings once into sprites.

| Token | Family / weight | Size / line (dp) | Tracking | Features | Case | Use |
|---|---|---|---|---|---|---|
| `TYPE_DISPLAY` | Inter 600 | 32 / 40 | −0.32 px (−1 %) | — | as written | album, artist, playlist, profile name in a page header |
| `TYPE_PAGE_TITLE` | Departure Mono 400 | 22 / 28 (= 11 / 14 AP-px) | 0 | `tnum` | sentence | app screen titles: Home, Cerca, Libreria, Coda, Testo, Impostazioni |
| `TYPE_WORDMARK` | Departure Mono 400 | 22 / 28 | 0 | — | `Psytify` | title bar, splash, sign-in, about |
| `TYPE_HEADING` | Inter 600 | 16 / 24 | −0.08 px | — | sentence | section headings, dialog titles |
| `TYPE_TRACK_TITLE` | Inter 500 | 14 / 20 | 0 | — | as written | track and tile titles |
| `TYPE_ARTIST` | Inter 400 | 13 / 18 | 0 | — | as written | artist lines, tile subtitles (`TEXT_SECONDARY`) |
| `TYPE_BODY` | Inter 400 | 14 / 20 | 0 | — | sentence | paragraphs, settings descriptions, toasts |
| `TYPE_META` | Inter 400 | 12 / 16 | 0 | `tnum` | sentence | metadata, dates, counts (`TEXT_TERTIARY`) |
| `TYPE_NAV` | Inter 500 | 13 / 20 | 0 | — | sentence | sidebar items, in-page tabs, menu items |
| `TYPE_BUTTON` | Inter 600 | 13 / 16 | +0.13 px | — | sentence | button labels |
| `TYPE_LABEL` | Inter 600 | 11 / 16 | +0.66 px (+6 %) | `tnum` | UPPERCASE | kickers, column headers, badges (`FLAC 24/96`, `LOSSLESS`, `LIVE`) |
| `TYPE_NUMERIC` | Inter 400 | 12 / 16 | 0 | `tnum` | — | durations, track numbers, elapsed/remaining in the player bar, sizes |
| `TYPE_NUMERIC_PIXEL` | Departure Mono 400 | 22 / 28 | 0 | `tnum`, `zero` | — | elapsed/remaining in the focus view, sleep-timer countdown, jam code, listening stats |
| `TYPE_LYRIC` | Inter 600 | 24 / 32 (focus), 18 / 26 (panel) | −0.24 / −0.18 px | — | as written | synced lyrics |

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
| `SIDEBAR_W` | 240 | 120 AP; flush, `SURFACE_CHROME`, 1 AP `LINE_SUBTLE` on its right edge |
| `SIDEBAR_RAIL_W` | 56 | collapsed: icons only |
| `SIDE_PANEL_W` | 320 | now playing / queue / lyrics |
| `PLAYER_BAR_H` | 72 | bottom strip, 1 AP `LINE_SUBTLE` on its top edge |
| `PAGE_GUTTER` | 24 | content padding left/right/top |
| `CONTENT_MAX_W` | 1440 | content column cap on very wide windows |
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
| `SEEK_TRACK_H` | 4 | 2 AP; hit area 24 |
| `VOLUME_W` | 96 | |

Breakpoints (window width): `BP_RAIL = 1100` (below: sidebar becomes the rail), `BP_PANEL_DOCK = 1360`
(below: the side panel overlays the content instead of docking), `BP_WIDE = 1600` (above: tiles grow, content
caps at `CONTENT_MAX_W`).

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

Covers are photos: linear filtering, square, no radius, no tint extraction. Missing cover: `GRAPHITE` field with
`album_placeholder` (24 AP) centred at 1 AP if the cover is ≥ 64 dp, else `icon_library` tinted `STONE`.

### 6.3 Phone (Iced)

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
| `MOTION_PANEL` | 200 | `EASE_OUT` | side panel / rail open, sheet slide; offsets quantised to `ap_device` |
| `MOTION_LYRICS` | 280 | `EASE_OUT` | lyric line scroll, quantised to whole device px |
| `TOOLTIP_DELAY` | 500 | — | |
| `TOAST_HOLD` | 4000 | — | |
| `CARET_BLINK` | 530 on / 530 off | step | text caret |
| `PROGRESS_TICK` | 250 | — | seek bar and time refresh (no per-frame updates) |
| `SPRITE_FRAME_MIN` | 80 | step | no sprite frame shorter than this |

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
embeds the 1x PNGs, decodes them once (`sprite::sheet()`, 159,744 bytes of RGBA for the 72 sprites) and exposes
`sprite::upscale_nearest(&Frame, factor)` and `sprite::upscale_tinted(&Frame, factor, tint)` (tint applies to
mask sprites: the 56 icons plus `eq`, `eq_rest`, `mark`, `mark_play`, `liked_cover`). `factor` is
`scale::art_px(scale_factor)` (this document's `ap_device`; the doc name `space::ap_device` is `scale::art_px`
in code). Cache `sprite::UpscaleCache`, owned by the frontend, key `(sprite, frame, factor, tint)`, LRU, two
declared limits: 128 entries and 8 MiB of pixels. The byte limit is the one that binds: a 32 x 32 sprite at
`ap_device` 8 is 256 KiB, so 128 such entries would be 32 MiB. Details: `PSYTIFY_PIXEL_ART_GUIDELINES.md`
section 10.
