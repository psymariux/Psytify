# Psytify pixel-art guidelines

Status: v1, 2026-11-22. Applies to every sprite under `design/assets/pixel/`. Values come from
`PSYTIFY_DESIGN_TOKENS.md`; the reasons are in `PSYTIFY_ART_DIRECTION.md`.

## 1. The one rule: one pixel size

One art pixel (AP) = 2 dp, everywhere, for every sprite, border, separator, focus ring and pixel glyph.
Sprites are never drawn at 2 AP, 1.5 AP or half an AP. A bigger picture is drawn bigger, not scaled.
Mixed pixel sizes ("mixels") are the most common way pixel UI looks cheap; this rule removes them.

## 2. Grids and sizes per use

| Use | Grid (AP) | On screen at 1x | Live area | Files |
|---|---|---|---|---|
| UI icon | 12 x 12 | 24 dp | 10 x 10 (1 AP padding); horizontal arrows may reach the edge | `src/icons/*.txt` |
| State indicator (equalizer, mark, loader) | 12 x 12 | 24 dp | 10 x 10 | `src/motifs/*.txt` |
| Cover placeholder | 24 x 24 | 48 dp | 22 x 22 | `src/motifs/album_placeholder.txt` |
| Mascot | 32 x 32 | 64 dp | 30 x 30, ground shadow on row 26 | `src/mascot/nastro_*.txt` |
| Illustration (empty states) | ≤ 64 x 32 | ≤ 128 x 64 dp | composition on a common ground line | `src/mascot/empty_crate.txt` |
| 9-slice frame (future) | 4 x 4 corners | 8 dp | — | `src/frames/` |

Sizes are multiples of 4 AP so a sprite centred in an even slot lands on whole APs. Odd sizes are not allowed.
New sizes need an entry here first.

## 3. Palettes

There are no free colors. Every sprite uses one of three palettes defined in `manifest.toml`, and every
entry maps to a token. **Extra colors beyond the tokens: 0.** The ramp already has ten steps of warm grey, which is
enough for a 3 to 4 step ramp per material plus outline and highlight; adding hues would compete with album covers.

| Palette | Characters → tokens | Colors | Used by |
|---|---|---|---|
| `mask` | `.` transparent, `#` tint | 1 (tinted at runtime) | icons, eq, eq_rest, mark, mark_play |
| `mascot` | `.` transparent, `o` near_black, `k` graphite, `c` charcoal, `i` iron, `s` stone, `m` medium_grey, `g` soft_grey, `w` warm_white, `p` pure_white, `l` lime | ≤ 10 | Nastro, illustration, placeholder |
| `ramp` | `.` transparent, `i` iron, `m` medium_grey, `g` soft_grey, `w` warm_white | 4 | loader |

Limit per sprite: 15 colors, transparency excluded (the 4bpp discipline of DS/GBA sprites). In practice: masks 1,
loader 4, mascot 9.

Tints for masks (component docs say which applies): `ICON_IDLE` soft_grey, `ICON_ACTIVE` warm_white,
`ICON_DISABLED` stone, `STATE_ON` lime, `TEXT_ERROR` error, `TEXT_ON_LIGHT` deep_black (icons on light buttons).

Lime in a sprite: only `l`, only on Nastro's LED in `nastro_listen`, and only through a tint on the `eq` and
`mark_play` masks. Never anywhere else.

## 4. Light, outline, shading

- **Light comes from the top-left**, for every sprite and every bevel. Lit edges (top, left) are one ramp step lighter;
  shadow edges (bottom, right) one step darker.
- **Outline:** sprites ≥ 24 AP get a silhouette outline in `near_black` (`o`), not `deep_black`, so it separates
  the sprite from `charcoal` and `graphite` surfaces and still sinks into the canvas. Interior lines (window frame,
  label edge) use `o` or one step darker than the local fill.
- **Selective outline:** where a lit edge meets the outline, the edge row/column inside the outline is the
  highlight step (`m` on a `s` body). Where a part overlaps another part, the outline is the darker local color, not `o`.
- **Masks (icons) have no outline and no shading.** Weight comes from strokes: 1 AP lines; 2 AP for stems on the
  vertical axis of a 12-wide grid (an even grid has no centre column) and for the `close` X knot; solid shapes for
  play, pause, next, previous, heart_fill, downloaded.
- No pillow shading (light from the centre), no rim light all around, no gradients except the dithers in section 6.

## 5. Clusters and lines

- **No orphans:** a non-transparent pixel with no neighbour (8-connectivity) is a lint error. Exceptions are listed
  per sprite in `allow_isolated` with a reason (none at present).
- **Single-pixel color islands** (a pixel whose 8 neighbours all differ from it) are allowed only for glints, screws,
  LEDs and hub dots. The lint prints a count per frame; review each one.
- **Line runs** follow monotonic progressions mirrored on the diagonal: the disc edge is 8-2-2-1 / 1-2-2-8, the
  play triangle rises +1, +2, +1, +2. A run sequence that goes up and down (1-3-2-4) is a jaggy: redraw it.
- **No doubles:** a diagonal line is one pixel per step. An L-shaped corner on a diagonal (two pixels touching
  orthogonally) is a double, except at deliberate 2 x 2 knots (shuffle, close, ribbon crossing).
- **No banding:** two parallel 1 AP stripes of neighbouring steps along an edge read as blur. Use one step or a
  clearly different value.
- **Minimum feature:** 1 AP. Gaps between strokes are ≥ 1 AP. Holes inside a shape are ≥ 2 x 2 AP, or a single hub
  dot inside a ring.

## 6. Anti-aliasing and dithering

- **No anti-aliasing.** No intermediate color to soften an edge, no partial alpha (alpha is 0 or 255), no runtime
  smoothing filter, no rotation, no non-integer scaling.
- **Dither** only as a 50 % checkerboard of two adjacent ramp steps, and only for: skeleton fills, the unbuffered part
  of the seek bar, disabled fills. Never on the mascot, icons or covers.

## 7. Animation

Frames are swapped on a timer (`time::every` / a GPUI timer at the frame duration); positions inside a frame
move only by whole APs. Nothing is tweened.

| Sprite | Frames | Timing (ms) | Loop | Runs when |
|---|---|---|---|---|
| `eq` | 6 | 120 each | yes | the row's track is playing and visible |
| `eq_rest` | 1 | — | — | the row's track is paused or buffering |
| `mark_play` | 2 | 160 each | yes | playback active, title bar visible |
| `loader` | 8 | 90 each | yes | a wait longer than 300 ms is visible |
| `nastro_idle` | 4 | 1000, 240, 1000, 120 (hold, lift, hold, blink) | yes | an empty state or the about page is visible |
| `nastro_listen` | 4 | 140 each (hubs turn 45° per frame, lift on 3-4) | yes | Nastro is on screen while music plays |
| `nastro_sleep` | 2 | 800 each | yes | offline / nothing to do |
| `empty_crate` | 1 | — | — | static |
| `icon_bell_ring` | 8 | 600 hold, then 80 each (two swings) | yes | the Novità sidebar entry while there is unread news |
| `icon_download_busy` | 4 | 140 each (the arrow falls into the tray) | yes | the Scaricati sidebar entry while a download runs |
| `icon_people_live` | 2 | 260 each (heads bob in turn) | yes | the Jam sidebar entry while an invite waits |
| `nastro_cheer` | 12 | 80-90 per hop frame, 500 hold (two hops, hubs turning) | yes | the sign-in page |

Craft rules: idles carry one hold of ≥ 1000 ms (calm, not twitchy); secondary motion lags the body by one frame
(Nastro's ribbon sways while the shell lifts); one change per frame, at most two (shape + offset).
`SPRITE_FRAME_MIN` = 80 ms. Loops stop on frame 0 when the sprite leaves the viewport, the window loses focus or
is hidden, or `reduce_motion` is on. No sprite animates while the window is idle and nothing is playing, except a
visible empty state.

## 8. Source of truth and file format

- **Source:** `design/assets/pixel/manifest.toml` (tokens, palettes, sprite list) plus one UTF-8 text grid per
  sprite. One character = one art pixel. `@frame [ms=N]` starts a frame; `# ` (hash + space) starts a comment; pixel
  rows never contain spaces. Grids diff line by line in review.
- **Export:** `python3 -I design/tools/sprites.py` (Pillow) writes
  - `export/1x/<name>.png`: indexed PNG (mode P), frames side by side, index 0 transparent, palette order = manifest
    order (masks: index 1 = tint, previewed as warm_white);
  - `export/sprites.json`: frame size, frame count, durations, palette roles per sprite;
  - `export/contact/<group>@4x.png`: review sheets, every frame on `deep_black` and `charcoal` at 4x with a 1 AP
    grid, plus each frame at real size (1x);
  - `export/preview/<name>@4x.gif`: animated previews.
  Output is deterministic; commit the grid and the exports together.
- `--check` lints only and exits 1 on errors (for CI once `psytify-design` exists).
- Never edit an exported PNG by hand. Never commit a sprite without its grid.

## 9. Naming

`snake_case`, `<family>_<name>[_<state>]`. Families: `icon_` (UI masks), `eq`, `mark`, `loader`, `album_`,
`nastro_` (mascot states), illustrations by subject (`empty_crate`). Grid file name = sprite name without the
family prefix for icons (`src/icons/heart_fill.txt` → `icon_heart_fill`), full name elsewhere. Outline and filled
pairs: `icon_heart` / `icon_heart_fill`. Rust: `sprite::ICON_HEART_FILL`, `sprite::NASTRO_LISTEN`.

## 10. Runtime pipeline (for `psytify-design`, step 3b)

1. Embed `export/1x/*.png` and `sprites.json` with `include_bytes!`; decode once at startup (PNG decode only).
2. `ap = ap_device(scale)` (tokens section 2).
3. Nearest-neighbour upscale on the CPU: every art pixel becomes an `ap x ap` block. Mask sprites: replace index 1
   with the tint RGBA before upscaling.
4. GPUI: swap to BGRA, wrap in `RenderImage`, paint with `canvas` + `window.paint_image` at a device-snapped origin
   with bounds equal to the texture size (texel = device pixel, so the Linear sampler is the identity). Iced: same
   pre-upscaled RGBA as an `image::Handle` drawn at its exact size with `FilterMethod::Nearest`.
5. Cache key `(sprite, frame, ap, tint)`, LRU, capped at 128 entries and 8 MiB (`psytify_design::sprite::UpscaleCache`,
   owned by the frontend); rebuild on scale-factor change. Frames are cached one by one, not as a shared strip.
6. Animation state is `(sprite, frame_index, next_deadline)` in the view model; the timer only fires while some
   sprite animation is visible.

## 11. Review checklist (every new or changed sprite)

- [ ] Grid size is in the table of section 2; every frame has the same size.
- [ ] `sprites.py` runs clean: no errors; every single-pixel island in the warning list is a glint, screw, LED or hub.
- [ ] Palette is one of the three; no new token; lime only where section 3 allows.
- [ ] Light from the top-left; outline `near_black` on sprites ≥ 24 AP; no outline on masks.
- [ ] Line runs are monotonic, no doubles, no banding, no anti-aliasing.
- [ ] Readable at real size (the "real size (1x)" row of the contact sheet) on `deep_black` and `charcoal`.
- [ ] Icons: distinct silhouette from every other icon at 1x (shuffle vs close, repeat vs repeat_one).
- [ ] Animation: durations in the section 7 table; one hold ≥ 1000 ms on idles; stops when hidden.
- [ ] Original: no traced, ripped or "inspired-by" game sprite, logo or character; no franchise silhouette.
- [ ] Grid, manifest entry and exports committed together.
