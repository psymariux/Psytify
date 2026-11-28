# Psytify desktop v2

Status: v2 brief, 2026-11-27. It amends `PSYTIFY_ART_DIRECTION.md` for the GPUI desktop. Everything not named
here stays as it is: palette, the pixel grid, sprites, Nastro, lime only for states, no gradients, no glass, no
radii. Where the two documents disagree, this one wins.

## Why

The owner's review of v1 (2026-11-27): "too basic, too few animations, giant gaps, some things tiny and others
huge, no hover animation, the lyrics hide the album, the video clip is odd and sits apart in its own panel, the
sidebar is bare and cannot be shrunk, full screen looks cut." They like the pixel art. Before screenshots:
`docs/design/before/`.

What the screenshots show, measured at 2560 x 1440, scale 1, full screen (`before/full_home.png`):

| Problem | Evidence |
|---|---|
| Full screen looks cut | the page column stops at 1440 dp and is centred: two empty black bands of ~240 dp, between the sidebar and the content and between the content and the side panel |
| Four scales on one screen | 12–13 px sidebar and row text, 24 dp row covers, 160 dp tiles, 18 px lyrics; the side panel's lyrics are the largest text on screen |
| Rows lose their width | "Aggiunti di recente" is one 1080 dp column whose rows use the left 450 dp |
| Lyrics without the record | side panel *Testo*: title in 12 px, no cover, the first line starts half way down an empty panel |
| Video apart | the clip is a separate 320 dp panel tab, a 16:9 box over an empty panel, and a second place in the focus view |
| Bare sidebar | eight nav rows and nothing else; no library, no collapse, no resize |
| Dead surfaces | nothing reacts to the pointer except a background tone; pages cut in; covers never move |

## 1. Scale: one zoom, one ramp

**Zoom.** The whole UI is laid out in dp multiplied by a zoom `z`. `Auto` (default) follows the window's
logical width: `z = clamp(width / 1600, 1.0, 1.5)`, snapped to steps of 0.125, with 48 dp of hysteresis so a
resize does not flicker between steps. The user can pin it in Impostazioni → Aspetto (`Auto`, 90 %, 100 %, 112 %,
125 %, 150 %) and with Ctrl+= / Ctrl+- / Ctrl+0. It is a frontend preference saved with `SaveUiPrefs`. The art
pixel follows the product of the window scale and the zoom (`art_px(scale * z)`), so sprites and 1 AP lines stay
on whole device pixels.

**Type ramp.** Five sizes do the visible work (before zoom). Anything else is a bug.

| Role | Size / line | Face | Use |
|---|---|---|---|
| Display | 40 / 44 | Departure Mono | page titles, the stage title |
| Title | 22 / 28 | Inter 600 | section headings, object names in panels |
| Body | 15 / 20 | Inter 500 | track and item titles, nav labels |
| Meta | 13 / 18 | Inter 400 | artist lines, counts, times |
| Caption | 11 / 14 | Departure Mono | badges (`LOSSLESS`, `NUOVO`, `LIVE`), kbd hints |

Lyrics have their own pair: 20 / 28 in the side panel, 32 / 40 in the stage.

**Art ramp.** Covers come in five sizes: 40 (rows), 56 (shortcuts, the player bar), tile (fluid, §2), 280 (side
panel), stage (fluid, up to 720). Artist photos use the same sizes in the pixel-circle mask.

## 2. The frame fills the window

- No page column cap. The canvas uses the full width between the sidebar and the side panel with a 32 dp margin
  each side (24 dp under 1200 dp). Only running text (descriptions, credits, settings explanations) keeps a
  72-character measure.
- Tile grids: the number of columns is `floor((width + gap) / (min_tile + gap))` with `min_tile = 168 dp` and a
  24 dp gap; tiles then stretch to fill the row exactly. Shelves show one full row; "Mostra tutto" opens the grid.
- Row lists that are not a single ordered list (Aggiunti di recente, shortcuts, search sections) flow into
  columns of at least 420 dp: one column under 900 dp, two up to 1500, three above.
- Track tables (album, playlist, liked) keep one column and give the extra width to the title and album columns
  (3 : 2), with the duration column right-aligned to the margin.

## 3. Motion: pixel motion

v1 cut every change. v2 moves, with two rules carried over: every moving edge lands on whole APs on every frame
(motion is quantised, never blurred), and nothing animates at rest: an animation asks for frames only while it
runs, and `Reduce motion` makes every duration 0 and freezes sprites on frame 0.

Tokens (ease-out unless noted): `MOTION_HOVER` 70, `MOTION_FAST` 100, `MOTION_BASE` 160, `MOTION_PANEL` 200,
`MOTION_ENTER` 220, `STAGGER` 40 ms (at most 6 steps).

| Where | Move |
|---|---|
| Tile hover | the cover rises 1 AP and a 2 AP hard ledge (`SHADOW_HARD`) appears under its right and bottom edges (`MOTION_HOVER`); a 40 dp square play button steps in from the bottom-right corner in 3 frames; the title goes to `TEXT_PRIMARY` |
| Tile press | down 1 AP, ledge gone (the RetroUI key press), instant |
| Row hover | background tone in `MOTION_HOVER`; the number or cover gets the play glyph; the heart and "Altro" slide in 2 AP from the right with opacity |
| Sidebar item | hover tone and the icon nudges 1 AP right; the lime selection marker grows from the centre to full height in 3 frames when the selection moves |
| Page enter | each section rises 4 AP with opacity in `MOTION_ENTER`, staggered by `STAGGER`; back/forward uses the same move without the rise |
| Panels | the side panel and the sidebar slide their width in `MOTION_PANEL`, quantised to APs |
| Track change | the player bar cover cross-fades in `MOTION_BASE`; title and artist rise 2 AP |
| Seek bar | grows from 1 to 2 AP tall on hover and shows a 2 x 4 AP thumb; volume the same |
| Stage media swap | cover ↔ video is a 4-frame ordered dither dissolve (pixel style), 160 ms |
| Play button | the play ↔ pause glyph swaps through one in-between frame |
| Like | the heart fills in two frames with a 1 AP pop |

Performance budget: at rest 0 frames per second; during a hover or enter only the view that moves repaints. CPU at
rest and while playing is measured before and after with `scripts/perf-sample.py` and written to
`docs/PERFORMANCE.md`.

## 4. Sidebar

Three widths, one toggle (title bar button and Ctrl+B), remembered in `SaveUiPrefs`:

- **Expanded** (default 264 dp, drag the right edge from 220 to 360): nav, then *La tua libreria* with in-place
  tabs (Playlist, Album, Artisti) and a filter field; items are 48 dp rows with a 40 dp cover (artists in the
  pixel circle), name and a meta line ("Playlist · 24 brani", "Album · Daft Punk"). "Brani che ti piacciono" is
  pinned first. While a jam is live, a 56 dp *Jam* card with the `LIVE` badge and the members' avatars sits above
  the library.
- **Rail** (72 dp): nav icons and the library as 40 dp covers, each with a tooltip; the selection marker stays.
- Below 1000 dp of window width the sidebar is a rail whatever the setting; the toggle then overlays the
  expanded sidebar above the canvas with a hard shadow.

The width change follows `MOTION_PANEL`.

## 5. Now playing: one place for the record, the words and the clip

The cover is never hidden by the lyrics, and the video is never somewhere else.

**Side panel** (320 dp, tabs *In riproduzione*, *Testo*, *Coda*):

- *In riproduzione*: the media slot (280 dp square cover, or the video clip 16:9 in the same slot when the track
  has one and video is on: a small Cover / Video switch sits on the slot), title in Title, artist link, badges,
  then the lyrics (current line and the next four, click to open *Testo*), then "Prossimo".
- *Testo*: a 64 dp header with the 56 dp cover, title and artist; lyrics start at the top third, not half way down;
  the current line keeps the lime cursor block.
- The separate *Videoclip* tab and panel are removed; the player bar's video button turns the clip on in the media
  slot (and opens the panel if it is closed).

**Stage** (F, replaces the focus view and the full-screen video): the canvas only, two columns.

- Left (45 %): the media slot at the largest square that fits (up to 720 dp) or 16:9 for the clip, the title in
  Display, the artist, a seek bar with times in tabular pixel numerals, the transport with a 56 dp play button,
  the heart and the Cover / Video switch (V).
- Right: lyrics 32 / 40, the current line at the upper third; no lyrics: `nastro_listen` and the line.
- Bottom: a 72 dp *Prossimo* strip with the next three tracks as 40 dp rows.
- Esc or F leaves; the transition is `MOTION_PANEL` from the player bar cover to the stage media slot (the cover
  grows in AP steps).

## 6. Pages

- **Home:** shortcuts as a 2–4 column grid of 56 dp rows (cover 56, title, play on hover); *Aggiunti di recente*
  in flowing columns (§2) with the date headers spanning the row; shelves as full tile rows; *I tuoi artisti* as a
  row of photos; the listening card. Every section enters with the page-enter move.
- **Album / playlist / liked:** the header grows with the zoom (cover 232), the action row gets the 56 dp primary
  play button; the table follows §2.
- **Artist:** photo 200 in the pixel circle; *Popolari* in two columns of five above 1500 dp.
- **Search, library, downloads, settings, social:** the type ramp, the frame and the hover moves; no new layout.

## 7. Acceptance

- At 2560 x 1440 scale 1 full screen and at 1280 x 800 windowed: no band of empty canvas wider than the 32 dp
  margin beside the content, no text under 11 px after zoom, at most five type sizes on a screen.
- Every interactive tile, row and nav item has a visible hover move; every move has its reduce-motion fallback.
- The side panel *In riproduzione* shows the cover and the lyrics at once; the clip plays in the media slot; the
  *Videoclip* tab is gone.
- Ctrl+B collapses and expands the sidebar; dragging its edge resizes it; both survive a restart.
- CPU at rest is unchanged within noise (`scripts/perf-sample.py`, two runs per binary, window visible).
- fmt, clippy -D warnings and the desktop tests pass; the gallery (`psytify --gallery`) shows every new state.
