# Psytify art direction

Status: v1, 2026-11-22, migration step A. Replaces `docs/STYLE.md` (three rounded panels, cover tint, lime as
brand) for the GPUI desktop and, through `psytify-design`, for the Iced phone layout.
Companions: `PSYTIFY_DESIGN_TOKENS.md` (values), `PSYTIFY_PIXEL_ART_GUIDELINES.md` (sprites),
`PSYTIFY_COMPONENT_GUIDELINES.md` (components). Sprites: `design/assets/pixel/`.

**Owner decisions (2026-11-22):**
- **Brand mark:** Nastro's face everywhere: launcher icon (desktop and Android), in-app mark and splash. The three
  lime bars are retired.
- **Liked heart:** white (`ICON_ACTIVE`) everywhere, player bar included. Lime stays for playing, selected nav,
  toggles that are on, and live.
- **Sign-in logos:** the providers' official vector marks (monochrome variant where one exists) are the only
  exception to the pixel rule.
- **Light theme:** none in v1. Tokens stay named by role, so a light theme can be added later.

## 1. Vision

**A listening instrument, not a storefront.** Psytify looks like a well-made piece of hardware with a tiny screen
from the DS/GBA era, built today with Zed's precision: a monochrome shell of warm greys, edges drawn on a strict
2 dp pixel grid, dense readable lists, and one green-yellow light that is on only when something is live. The
only real color on screen comes from album covers.

Emotional direction: **calm, precise, warm, a little nostalgic.** Calm because nothing moves or glows without
reason. Precise because every edge lands on the same art-pixel grid. Warm because the greys lean slightly brown
and the mascot is a cassette that listens with you. Nostalgic from craft (crisp clusters, stepped animation, a
cassette, a crate of records), not from parody (no CRT, no scanlines, no 8-bit chiptune pastiche).

Three sources, in this proportion:

| Source | What we take | What we leave |
|---|---|---|
| DS / GBA pixel craft | one pixel size, clean silhouettes, selective outlines, limited palettes, characterful idle loops with holds, 9-slice framing, stepped transitions | creature designs, game layouts, fonts, logos, sprites of any franchise |
| Zed | flush panels split by 1 AP lines, 4 dp spacing, 12 to 14 px dense type, 70 to 200 ms state changes, keyboard first | its cool blue-grey and blue accent |
| RetroUI (retroui.io) | hard 1 AP borders, notched corners instead of radii, press = translate + collapse the ledge | its Minecraft font, pastel theme, light surfaces |

## 2. Do and don't

| Do | Because |
|---|---|
| Draw everything on one art pixel (2 dp) | mixed pixel sizes are what makes pixel UI look cheap |
| Keep surfaces to four greys and text to three tiers | more steps turn hierarchy to mud; contrast table in tokens 1.2 |
| Use lime only for playing, selected primary nav, toggles on, live | a light that is always on means nothing |
| Pair every lime cue with a shape cue (bar, dot, filled icon, knob side) | state must not depend on color (color-blind users, grey displays) |
| Divide regions with 1 AP lines, flush | cards inside panels inside panels waste space and read as a template |
| Use Inter for everything the catalogue or the user wrote | catalogue strings are any length and any script |
| Use Departure Mono only for app-authored words and a few numerals | pixel type is a seasoning; under 10 % of the text |
| Step sprite animation by frames; ease UI motion briefly | sprites keep their grid; UI stays responsive |
| Show covers as they are: square, untinted, linear-filtered | covers are the color of the app |
| Keep Nastro to empty, offline, about and sign-in screens, one per screen | a mascot everywhere becomes noise |

| Don't | Because |
|---|---|
| Cover-tinted gradients, auroras, blurred backdrops, glass | owner decision; covers would stop being the only chroma |
| Lime fills larger than 4 x 4 AP, lime text, lime glow, lime decoration | lime inflation kills its meaning |
| Rounded panels, pill chips, horizontal "shelf" carousels, round green Play fab | that is the Spotify silhouette we are leaving |
| Border radius anywhere | anti-aliased curves break the pixel grid; use notches |
| Soft shadows, blur, glow, bloom, scanlines, CRT curvature, neon | fake retro and fake depth |
| Pixel type at sizes other than 22 dp, or for long text | half-AP glyphs are mixels; long pixel text is hard to read |
| Smooth vector icons next to pixel art | two rendering languages on one screen |
| Medieval frames, scrolls, fantasy ornaments, Minecraft textures, 4-color NES pastiche | not our story |
| Any Pokémon (or other franchise) character, sprite, logo, layout or silhouette | copyright, and it would not be ours |
| Animations that loop while nothing happens | CPU at rest must stay near zero (AGENTS.md performance rules) |

## 3. The shell (desktop)

```
┌ title bar 40 ─ mark · Psytify · ‹ › · [ Cerca…  Ctrl K ] ··········· avatar · – □ × ┐
├ sidebar 240 ─┬─ content canvas (deep black) ─────────────────────┬─ side panel 320 ─┤
│ nav          │ page                                              │ tabs              │
│ library list │                                                   │ now / queue / text│
├──────────────┴───────────────────────────────────────────────────┴───────────────────┤
└ player bar 72 ─ cover · title · heart │ transport + seek │ badge · lyrics · queue · vol ┘
```

Chrome (title bar, sidebar, side panel, player bar) is `NEAR_BLACK`; the content canvas is `DEEP_BLACK`; all are
flush and divided by 1 AP `CHARCOAL` lines. No gaps, no radii. The title bar holds the brand mark (12 AP; `mark` at
rest, `mark_play` in lime while music plays: the brand mark *is* the playback light), the wordmark in Departure
Mono, back/forward, the search field (command-palette style, Ctrl+K), the account avatar and pixel window controls.
Below `BP_RAIL` the sidebar collapses to a 56 dp icon rail; below `BP_PANEL_DOCK` the side panel overlays.

## 4. Screens

Each entry: layout, signature detail, lime on that screen.

**Home.** No hero carousel, no tinted banner, no endless shelves. Top: page title `Home` in pixel type and a
greeting line (`TYPE_META`). Then, in order and only when they have data: *Riprendi*: a 2-column grid of up to 8
compact rows (cover 48 + title), like a quick-access panel. *Nuove uscite*: a dated release log, a dense list
grouped by day (`TYPE_LABEL` date headers, rows with cover 32, artist, title, type badge), the Zed/foobar touch
that replaces the shelf. *Dai tuoi artisti*, *Mix per te*, *Ascoltati di recente*: one row of tiles each, cut to
the columns that fit, "Vedi tutto" opens the full grid (no horizontal scrolling). Mix tiles show a 2 x 2 mosaic of
real covers, never color blocks. *Il tuo ascolto*: a card with three stats in `TYPE_NUMERIC_PIXEL` (hours, tracks,
top artist). Lime: the `eq` on whatever is playing, nothing else.

**Search.** The field lives in the title bar; Ctrl+K focuses it from anywhere; typing shows results under it with a
300 ms debounce and `loader` in the field. Empty query: recent searches (rows, `icon_close` to forget) and a plain
text genre index with counts in tabular numerals; no colored browse cards. Results page: in-page tabs (Tutto,
Brani, Album, Artisti, Fonti, Persone). *Tutto*: top result card left (cover 96, `TYPE_HEADING`, kind badge) and the
first 5 tracks right (dense rows); then artists (pixel-circle 96), albums grid, external sources as rows with a
neutral badge (`YOUTUBE`, `BANDCAMP`, `SOUNDCLOUD`) and their licence, people. No results: `empty_crate` and
"Nessun risultato per «…»". Lime: `eq` only.

**Album.** Header on the canvas, no tint: cover `COVER_L` 224 left; right column: kicker `ALBUM · 2024`
(`TYPE_LABEL`), title `TYPE_DISPLAY`, artist link, meta line (tracks, total time, quality badge `LOSSLESS` or
`FLAC 24/96`), action row: Play (primary 40, label "Riproduci"), shuffle, save (heart), download all (only if every
track allows it; otherwise "Scarica i disponibili"), more. 1 AP separator, then stacked track rows with disc
headers. Footer: label, licence badge with holder, source, in `TYPE_META`: the licence is part of the identity.
Lime: `eq` on the playing row.

**Artist.** Header: photo `AVATAR_L` 160 in a pixel-circle mask, name `TYPE_DISPLAY`, listeners/followers in
`TYPE_META`, actions: Play (primary), Segui / Segui già (secondary toggle with check), Radio (secondary), more.
*Popolari*: 5 dense rows. *Discografia*: in-page tabs (Album, Singoli, Raccolte) and a grid. *Altro di …*: external
sources as rows. No banner, no aurora. Lime: `eq` only.

**Player bar / Now playing.** 72 dp strip: left 240 (aligned with the sidebar): cover 48, title and artist, heart
(lime when liked: it sits on the active strip). Centre: transport (shuffle, previous, play/pause 40, next, repeat)
over the seek bar with tabular times. Right: format badge (`FLAC 16/44.1`), lyrics, queue, volume 96, focus view.
Side panel tab *In riproduzione*: cover `COVER_PANEL` 288, title `TYPE_HEADING`, artist, badges (format, licence,
downloadable), credits, then "Prossimo" (the next row). Focus view (F): canvas only, cover up to 640 beside the
lyrics, elapsed/remaining in `TYPE_NUMERIC_PIXEL`, a minimal transport; no backdrop image, no blur. Lime:
`mark_play` in the title bar, `eq`, toggles that are on.

**Queue.** Side panel tab *Coda*: *In riproduzione* (one row with `eq`), *Successivi* (rows reorderable: a 2 x 6 AP
grip appears on hover, drag ghost with hard shadow, 1 AP white drop line), *Dalla riproduzione automatica* (tertiary
heading, rows dimmed to `TEXT_SECONDARY`), *Cronologia* collapsed. Header actions: clear, save as playlist.
Empty: one line and "Sfoglia" (no mascot in a side panel). Lime: `eq`.

**Library.** Title `Libreria` in pixel type; in-page tabs: Playlist, Album, Artisti, Brani, Scaricati. Toolbar:
filter field, sort menu (Recenti, A–Z, Artista), list/grid switch. Brani is a dense table (32 dp rows, sortable
column headers, tabular numerals) for keyboard-first browsing; Album and Artisti are grids. The sidebar below the
nav shows pinned playlists as 32 dp rows with 24 dp covers; no chips. Empty: `empty_crate`. Lime: sidebar marker
on "Libreria", `eq`.

**Playlists.** Same header as Album with a 2 x 2 mosaic of the first four covers (or the disc placeholder),
editable name and description (dialog), owner and visibility badge for public playlists. Rows reorderable like the
queue. "Brani che ti piacciono": the cover is a `GRAPHITE` field with the `liked_cover` sprite (24 AP heart, to draw;
until then `icon_heart_fill` at 1 AP in `WARM_WHITE`); no pink gradient. Lime: `eq`.

**Downloads / offline.** New screen *Scaricati* (also a Library tab). Top: storage block: used/free as a
segmented bar (1 AP outlined, 2 AP tall segments, each 1 % of the quota) with numbers in `TYPE_NUMERIC_PIXEL`; quality
default and folder (secondary buttons). *In corso*: rows with the DownloadIndicator progress, pause/cancel. *Sul
dispositivo*: dense table with quality badge and size (tabular). Offline mode: a 32 dp strip under the title bar,
`SURFACE_RAISED`, text "Offline: suonano solo i brani scaricati" and "Riprova"; non-downloaded rows go to
`TEXT_DISABLED`. Offline with nothing saved: `nastro_sleep`. Lime: none (availability is a fact, not a live state)
apart from `eq`.

**Settings.** Title `Impostazioni`; in-page tabs: Riproduzione, Suono, Download, Account, Informazioni. Groups of
rows: label (`TYPE_TRACK_TITLE`) and one-line description (`TYPE_ARTIST`) left, control right, 1 AP separators
between groups. Suono: the 10-band equalizer as ten 2 AP pixel sliders with dB values in tabular numerals and a
1 AP 0 dB line. Informazioni: `nastro_idle`, version in Departure Mono, licences (Inter, Departure Mono, icon
credits) and the owner's rights note. Lime: toggles that are on, the sidebar marker.

**Sign-in / social.** Sign-in: a 360 dp column on the canvas, `nastro_idle` above the wordmark, a line that
changes with the step; in-page tabs Accedi / Crea un account; provider buttons are secondary buttons with the
providers' official monochrome marks at 20 dp (the one vector exception: brand guidelines forbid redrawing their
logos); fields 40 dp; errors in `TEXT_ERROR` under the button, never in a toast. Friends: rows with pixel-circle
avatar 32, name, status "Sta ascoltando …" with the live badge; requests with Accetta (secondary) / Rifiuta (ghost).
Profile: avatar 96, name `TYPE_DISPLAY`, top artists as a ranked list (no pills), recent listens as rows; the
presence card ("Sta ascoltando") is a Card with cover 48 and an "Ascolta" button. Jam: the code in
`TYPE_NUMERIC_PIXEL` with a copy button, members as avatars in a row, live badge `LIVE` while connected. Lime: live
badges (presence, jam), sidebar marker.

**Lyrics.** Same engine in the side panel (`TYPE_LYRIC` 18/26) and the focus view (24/32). The current line is
`TEXT_PRIMARY` with a 2 x 3 AP lime cursor block to its left (the "live position" mark, like a text-box cursor);
upcoming lines `TEXT_SECONDARY`, past lines `TEXT_TERTIARY`. No scale-up of the current line, no per-word glow; the
list scrolls to the new line in `MOTION_LYRICS`, quantised to device pixels, at most 24 fps. Instrumental gaps: three
2 x 2 AP dots that light one by one (`STONE` → `WARM_WHITE`) on the progress tick. No lyrics: `nastro_listen` and
«Nessun testo per questo brano» (focus view only; the panel shows just the line). Lime: the lyric cursor, `eq`.

**Splash.** Under 1.2 s, any click skips: on the canvas the `mark` appears in 3 steps (shell, reels, tape guide;
120 ms each), the wordmark types letter by letter at 45 ms in Departure Mono with a `PURE_WHITE` block cursor, then
the app appears in one cut. No lime (nothing is playing yet), no fade-in of sprites.

**Phone layouts (Iced).** Same palette, type, sprites and components, phone sizes from the tokens. Edge-to-edge
canvas, no panels. Tabs at the bottom (Home, Cerca, Libreria, Novità) with the lime top marker on the selected one;
the mini player (56) above them with a 1 AP elapsed line. Screen titles in pixel type in the 48 dp top bar; pages
opened from another show only "Indietro". Home: the same sections as desktop, release log as rows, tiles 144.
Library: one list with in-page tabs instead of pill filters, "+" creates a playlist. Artist: photo 160 pixel-circle
centred, actions in one row. Full player: cover (square, ≤ 42 % of the height, snapped to 16 dp), title
`TYPE_DISPLAY` phone, artist, seek with a visible thumb and tabular times, transport with a 64 dp play button,
Testo / Coda tabs below. Menus are bottom sheets; long-press opens them; swipe a row right to queue it. Lime: the tab
marker, `eq`, toggles, live badges.

## 5. Mascot and motif family

**Nastro** ("tape" in Italian) is a living cassette, 32 x 32 AP. Its face is the cassette window: the two reels
are its eyes. A loose ribbon of tape rises from the lid, loops once and crosses itself, and sways with every
breath. Classic cassette cues make it unmistakable: four corner screws, a label with a printed band, the tape-guide
trapezoid with two guide holes on the lower edge, a ground shadow instead of legs. A single LED on the lid is
charcoal at rest and lime only while music plays: the one lime pixel cluster Nastro ever shows.

States: `nastro_idle` (eyes with a glint, slow lift and blink, 1000 ms holds), `nastro_listen` (the reel hubs turn
45° per frame, small lift on the beat, ribbon sways, LED lime), `nastro_sleep` (eyes shut, ribbon slack on the lid).
Palette: nine tokens, no extra color. Where: sign-in, empty states (with `empty_crate`), offline, "no lyrics",
about. Never in lists, headers, the player bar or error states; at most one per screen; frozen on frame 0 under
reduce-motion.

Originality check: Nastro is drawn from scratch on our grid. It is not a creature that carries a cassette, not a
robot or animal that turns into one (the nearest known neighbours, Transformers' cassette characters, are robots
and beasts, not a cassette with reel eyes), not a TV-headed or console-shaped character, and it shares no
silhouette, color scheme, eye shape or pose with any Pokémon or Nintendo character. Its distinguishing marks
(reel eyes, crossing tape ribbon, guide-hole trapezoid, single LED) are ours.

**Motif family** (all in `design/assets/pixel/`, all original):

| Motif | Sprite | Meaning |
|---|---|---|
| Brand mark: Nastro's face in one color (shell, two reels, guide) | `mark`, `mark_play` | identity at rest; playback light while playing |
| Equalizer: three 2 AP bars | `eq`, `eq_rest` | this track is playing / paused |
| Reel loader: eight blocks walking clockwise | `loader` | waiting |
| Disc | `album_placeholder` | a cover is missing |
| Record crate with an A–Z divider | `empty_crate` | nothing collected yet |
| Pixel icons on 12 x 12 | `icon_*` | UI actions (56 drawn; every role of the old svg set is covered) |

## 6. Motion principles

Two clocks, never mixed on the same element:

1. **Sprite clock**: frame-stepped pixel animation. Frames swap on a timer; positions move by whole APs; nothing is
   interpolated. Durations: 80 to 1000 ms per frame (table in the pixel guidelines, section 7). Idle loops carry a
   long hold so they breathe instead of twitching.
2. **UI clock**: short eased changes of color and opacity (`MOTION_HOVER` 70, `MOTION_FAST` 100, `MOTION_BASE` 160,
   `MOTION_PANEL` 200, `MOTION_LYRICS` 280 ms, ease-out, no springs, no overshoot). Moving surfaces (side panel,
   sheets, lyric scroll) are quantised to device pixels each frame so 1 AP lines never blur.

Press is instant (translate 1 AP, ledge gone), as a physical key. Pages change with a cut plus a 160 ms content
fade, no rising or sliding. Nothing animates at rest: idle CPU stays near zero (frames only while something moves,
`PROGRESS_TICK` 250 ms for the seek bar). Reduce-motion freezes sprites on frame 0 and sets UI durations to 0.

## 7. Lime by screen

| Screen | Lime allowed |
|---|---|
| Shell | `mark_play` (playing), sidebar selected marker, active shuffle/repeat in the player bar |
| Home, Search, Album, Artist, Playlists, Library | `eq` on the playing row/tile |
| Queue | `eq` on the current row |
| Lyrics | lyric cursor block, `eq` |
| Downloads / offline | `eq` only |
| Settings | toggles on, equalizer on/off toggle |
| Sign-in | none |
| Social, Jam | live badge dot (presence, jam) |
| Splash | none |
| Phone | tab marker, `eq`, toggles, live badge, Nastro's LED |

If a screen shows more than these, it is a bug.

## 8. Open items for the asset engineer

Done on 2026-11-22: 56 icons on the 12 x 12 grid (every role of the Iced app's `assets/icons/*.svg` except the provider
logos), `liked_cover` (24 x 24), `app_icon` (16 x 16 Nastro face) and the launcher set generated by
`design/tools/launcher.py` (Android mipmaps at integer scales 2/3/4/7/9, notification vector, desktop svg).
`window_close` shares `icon_close`'s art on purpose.

Still open:
- 9-slice frame set for menus/dialogs, only if plain quads prove insufficient in GPUI.
- Android adaptive icon (foreground/background layers inside the 66 dp safe zone): the legacy mipmaps ship for now.
