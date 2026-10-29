# Psytify visual style

The tokens live in `client/src/ui/theme.rs`: every other file uses them and defines no colors or
measures of its own. This document replaces the "true-black flat" direction of 23 October 2026.

The interface text is currently in Italian; the labels quoted below are given in English.

## Why it changed

On 24 October 2026 the project owner asked for an interface closer to Spotify and Apple Music: panels,
gradients taken from the cover, more motion, a right-click menu, Iconly icons. So **gradients and
shadows are now allowed, but only in three places**:

1. the **cover tint** at the top of a page and of the "Now playing" panel (`tinted_panel`, `aurora`): a
   vertical gradient that melts into the gray of the panel;
2. the **shadows of floating elements** (menus, dialogs, notices, the large cover, the Play button on a
   cover);
3. the **drawn cover** of "Liked songs" (pink gradient).

Still forbidden: glass and blur, glows, luminous borders, gradients on buttons, rows or cards, and
shadows on resting elements.

The sites the owner pointed to (ui.unlumen.com, shadergradient.co, magicui.design,
styles.refero.design) were used as inspiration and rebuilt in Rust with Iced's linear gradients: no web
view, no JavaScript.

## Direction

**Three rounded panels on black.** Sidebar (navigation + personal library), page in the middle,
"Now playing" panel on the right, each `PANEL` (`#121212`) with radius 14, separated by 8px of black.
Above sits the top bar (brand, Home and search, account, three window controls), below the player bar:
both directly on black.

**Color from the cover.** The page and the right panel take the cover's average tint (`Covers::tint_of`,
computed at decode time); without a cover, the album's flat color. Pages without a cover use a house
tint (`BRAND_TINT`, `NEUTRAL_TINT`, `LIKED_TINT`).

**Lime is the brand.** `ACCENT` (`#c6f24e`) colors the large Play button, the current track, the
equalizer bars, the heart, active shuffle and repeat, the mark of the active navigation item and the
switches. The primary button stays white with black text (inverted).

## Tokens

| Role | Value | Use |
|---|---|---|
| `GROUND` | `#000000` | Window background, top and bottom bars |
| `PANEL` | `#121212` | The three panels |
| `SURFACE` | `#1b1b1b` | Cards, search, selected row |
| `HOVER` | `#252525` | Mouse hover, thin lines |
| `RAISED` | `#2e2e2e` | Menus, dialogs, slider tracks |
| `CONTROL` | `#4d4d4d` | Control border |
| `TEXT` / `MUTED` / `FAINT` | `#fff` / `#b8b8b8` / `#9c9c9c` | Primary, secondary, meta text |
| `ACCENT` | `#c6f24e` | Brand and active states |
| `NEGATIVE` | `#e5566f` | Errors, destructive items |

Contrast: every text tier exceeds 4.5:1 (WCAG AA) on `GROUND`, `PANEL`, `SURFACE`, `HOVER` and `RAISED`.
The test `every_text_tier_passes_wcag_aa_on_every_surface` in `theme.rs` checks it.

### Measures

| Token | Value |
|---|---|
| Sidebar | 252px |
| Top bar / player bar | 56px / 88px |
| Right panel | 340px |
| Outer margin and gap between panels | 8px |
| Radii | panel and card 14, tile 10, row 8, cover 6, pill 999 |
| Table rows / search rows | 56px / 64px |
| Cover in a page header | 188px |
| Context menu | 276px, items 38px |
| Notice (toast) | 420px |

### Typography

Inter 4.1 (OFL 1.1, `client/assets/fonts`) in weights 400, 500 and 600, with tabular figures.

| Role | Size | Weight |
|---|---|---|
| Title of a track page (hero) | 44 | 600 |
| Page title (h1) | 28 | 600 |
| Section title | 18 | 600 |
| Body, cells, navigation | 14 | 400/500 |
| Meta, labels | 12 | 400 |

Sentence case everywhere.

## Components

- **Track rows**: no zebra striping. At rest the number; on hover Play, heart, Download (if allowed) and
  "more actions"; on the current track the equalizer bars. Right-click opens the menu. A click on the
  current track pauses or resumes.
- **Tiles**: on hover the green Play appears on the cover (tracks); albums open.
- **Context menu** (`ui/context_menu.rs`): Play, Play next, Add to queue, Like, Add to a playlist (list at
  the side, with a check), Remove from this playlist / from the queue, Go to artist and album, Download.
  It opens where the mouse is and stays inside the window.
- **Dialogs** (`ui/dialog.rs`): new playlist, rename, delete confirmation. Enter confirms, Esc or a
  click on the veil cancels.
- **Right panel** (`ui/side_panel.rs`): a pill selector (`SURFACE` track, `RAISED` thumb with a lime
  notch) that slides with a spring between "Now playing", "Queue" and "Lyrics"; the new content enters
  rising. "Now playing": a cover that breathes with Play (or a mini video clip), title, heart, labels
  (format, "Lossless" with a lime outline for FLAC and WAV, duration, downloadable), license, artist,
  next in the queue. "Lyrics": the same engine as the immersive view (`ui/lyrics.rs`), with the current
  line at the top.
- **Settings**: tabs at the top (Playback, Sound, Account, About) and groups of rows with a name and
  explanation on the left and the control on the right; switches for what turns on.
- **Home** (`ui/pages/home.rs`, `ui/shelf.rs`): at the top the `hero` carousel with the five most recent
  releases that have a cover (`hero_card` background with the cover tint, changes every 8 s with a
  fade, dots to choose). Below, in order: shortcuts, "Continue listening", "New releases", "News from
  your artists", "Your artists", "Mixes for you", "Most played by you", "Recently played", "Albums",
  the "Your listening" card (hours, tracks, artist of the moment) and "A to Z". Sections without data
  do not appear. They are horizontal rows of tiles with a 184px cover (`shelf::BIG_COVER`; 148px for
  artists) and a "See all" link to the right of the title.
- **Round covers**: artists have a circular cover, albums a square one with radius 6. Iced 0.13 does not
  round images: the corner comes from masking the pixel alpha at decode time
  (`Covers::handle(url, round)`, `cover()` / `cover_round()` in `ui/cover.rs`).
- **Catalog** (`ui/pages/library.rs`): tabs Albums, Artists, All tracks, Downloadable, Singles and
  sorting (recent, A–Z); large grids of tiles, not lists. "Browse all" (`browse_grid`) uses the
  full-color `browse_card` tiles with the nine `BROWSE` colors.
- **News** (`ui/pages/news.rs`): releases from followed artists, newest first, with the `badge` pill
  (lime, black text) on the sidebar item as long as there are unseen ones.
- **Artist page**: banner with `aurora_full` (cover tint, no added drawing), round Play button
  `fab_button`, "Popular" (five tracks: first the ones you played most, then the most recent; only with
  more than five tracks) and "Discography" as a grid.
- **Immersive view** (`ui/focus.rs`, key F): full screen, cover (up to 640px) and lyrics (up to 760px)
  form a single block, centered in the window; without lyrics the cover sits in the center, up to
  760px. Esc, F or the X at the top close it.
- **Synced lyrics** (`ui/lyrics.rs`): not a list that jumps; each line sits where the current line takes
  it, sliding from one line to the next. The current line grows to full size and lights up word by word
  (estimated on characters, because LRC only gives the start of the line); the others sit at 84% and
  fade towards the edges. Instrumental breaks are three dots that light up. Line breaks are estimated
  with the real widths of Inter SemiBold.
- **Entry splash** (`ui/splash.rs`): on black the three bars of the brand jump up one after another and
  sway, the name lights up letter by letter, then the veil fades while the page enters. Under two
  seconds; a click closes it.
- **Library in the sidebar**: chips Playlists, Artists, Albums; a click on the active chip shows them
  all. Playlists, saved albums and followed artists live here, with right-click to unfollow or remove.
- **Sleep timer and autoplay**: from the player menu (right-click on the bottom bar). The timer pauses
  after 5, 15, 30, 45, 60 minutes or at the end of the track; autoplay adds more tracks by the same
  artist (or, if there are none, from recent releases) when the queue ends, except during a jam.
  "Start radio", from a track's menu, does the same on request.
- **Downloads**: the button appears only if the track has `downloadable: true`. Otherwise it is hidden,
  not disabled: the space stays.
- **Covers**: the item's public image if there is one; otherwise a flat color field chosen from the id and
  the initial of the title. No fake images.
- **States**: gray skeletons at the real size of the content; an empty state with a sentence and an
  action; an error inside the block that failed, with "Retry".
- **Icons**: Iconly (Light and Bold) for the interface, with the player controls drawn in the same
  stroke (`client/assets/icons`, credits in `CREDITS.md`).
- **Phone** (`ui/phone.rs`, Android and `PSYTIFY_PHONE=1`): one column, inside the insets Android declares
  for its bars.
  - **Top**: "Back", the brand and the Settings gear.
  - **Middle**: the page at full width with the cover tint, without scroll bars: you scroll with the
    finger. In Search the field stays above the results and the keyboard opens.
  - **Bottom**: the mini player (cover, title, heart, round Play, a progress line; a tap opens the
    player) and the bar with four items, a filled icon and a lime notch on the active one.
  - **Full-screen player**: breathing cover, title, progress, controls, then "Lyrics" or "Queue".
  - **Measures**: 44 px touch targets (`TOUCH_TARGET`). Below 520 px the "Continue listening" card uses
    the round Play without a label; below 420 px the featured panel puts the buttons under the cover.
  - **Settings**: each control goes under its explanation.
  - **Home shortcuts**: titles on two lines.
  - **Gestures**: a long press opens the context menu. Android's Back button closes whatever is on
    top, otherwise it goes back to the previous page.
- **Frameless window**: the top bar drags (double click: maximize); minimize, full screen (also F11) and
  close are on the right. Resizing has handles on the right and bottom edges and on the corner: on
  Wayland the window origin cannot be moved.

## Motion

Iced 0.13 has no animations: `ui/motion.rs` builds them with a `Tween` or with functions of time
(`appear`, `stagger`, `pop`, `wave`, `drift`, `spring_in`), and the app asks for window frames only
while something is moving.

Two curves: **ease-out** for pages, lists, menus and notices; **spring** (`motion::spring`, overshoots
the target by 6% and settles) only for what you touch or what represents the track: the cover, the tab
thumb, the splash brand.

| Motion | Duration | Token / function |
|---|---|---|
| New page: rises 16px | 220 ms | `MOTION_BASE`, `PAGE_RISE` |
| List rows: text appears one after another (first 14) | 28 ms each | `STAGGER`, `motion::stagger` |
| Right panel: opens from the right | 260 ms | `MOTION_PANEL` |
| Menu and Play button on a cover | 120 ms | `MOTION_FAST` |
| Dialog: rises and brightens | 260 ms | `MOTION_PANEL` |
| Heart just pressed: bounce | 260 ms | `motion::pop` |
| Equalizer bars on the current track | continuous | `motion::wave` |
| Gradient of the "Now playing" panel | breathes over 14 s | `motion::drift` |
| Cover that just arrived: fade | 220 ms | `MOTION_BASE` |
| Skeletons | 1.4 s cycle | `PULSE_PERIOD` |
| Notice: enters rising and brightening | 220 ms | `MOTION_BASE` |
| Home carousel: change of release | every 8 s, 420 ms fade | `HERO_PERIOD`, `HERO_SWAP` |
| Large cover: on pause it shrinks to 90%, on Play it springs back | 620 ms | `MOTION_SPRING`, `COVER_REST` |
| Panel tab thumb (spring), content rising 10px | 620 / 260 ms | `MOTION_SPRING`, `MOTION_PANEL` |
| Lyrics: scroll to the new line, which grows and brightens | 480 ms at 24 fps | `MOTION_LYRICS`, `AMBIENT_FRAME` |
| Lyrics: words lit one by one | with the Tick (4 per second) | `lyrics::sung_fraction` |
| Play button: the icon bounces when switching Play/Pause | 260 ms | `motion::pop` |
| Splash: spring bars staggered by 90 ms, letters by 45 ms, fading veil | 1.75 s in total | `SPLASH_TOTAL`, `SPLASH_EXIT` |

## Iced 0.13 limits and workarounds

| Limit | Workaround |
|---|---|
| `Wrapping::None` is ignored | `responsive` provides the real width; texts are cut with "…" on an estimate (`components::line`) |
| No accessible name on widgets | Icon buttons have a tooltip with their name |
| No `drag_resize`, no double click on `mouse_area` | Resizing with handles driven by mouse events; the double click on the bar is counted by hand (`app/chrome.rs`) |
| The mouse position does not reach widgets inside a `scrollable` | A global listener remembers it (`ui/cursor.rs`) and the menu opens there |
| Opacity only on colors, not on widgets | The colors of text and backgrounds are faded (`motion::fade`) |
| Images have no corner radius | Mask on the pixel alpha at decode time (round and rounded covers) |
| `container.clip(true)` only clips to a rectangle | Round shapes come from the mask, not from the clip |
| The window cannot be narrower than 1100px | Below 820px of content, search stacks its columns |
