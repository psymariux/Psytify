# Psytify component guidelines

Status: v1, 2026-11-22. Every value is a token from `PSYTIFY_DESIGN_TOKENS.md` (names without module prefix).
Desktop = GPUI, phone = Iced; one anatomy, two sizes. States: **rest, hover, pressed, focused, disabled,
selected, playing**; "—" means the state does not exist for that component.

## 0. Shared rules

- **Hover** raises the fill one step (`SURFACE_CANVAS`/`SURFACE_CHROME` → `SURFACE_RAISED`) or lightens the
  border (`CONTROL_BORDER` → `CONTROL_BORDER_HOVER`), in `MOTION_HOVER`. Hover never moves anything.
- **Press** on controls with a ledge: translate by `PRESS_TRANSLATE` and hide the ledge, `MOTION_PRESS` (instant);
  release restores both instantly. Controls without a ledge only change fill.
- **Focus ring** (keyboard focus only, "focus-visible"): `FOCUS_RING_W` 1 AP of `FOCUS_RING` (pure white), separated
  from the control by `FOCUS_RING_GAP` 1 AP, following the control's notch. Rows and list items draw it inset
  (inside their bounds) so it is never clipped by a scroll container. Text fields show focus with a `PURE_WHITE`
  border instead of a ring.
- **Disabled:** text and icons `TEXT_DISABLED`/`ICON_DISABLED`, borders `LINE_STRONG`, no ledge, no hover, cursor
  default. Disabled controls keep their size. Hidden controls (download on non-downloadable tracks) keep their space.
- **Hit areas:** desktop ≥ 32 x 32 dp; phone ≥ `TOUCH_TARGET` 48 x 48 dp, even when the visible control is smaller.
- **Notches** come from `NOTCH_1`/`NOTCH_2`; no `border_radius` anywhere.
- **Lime** only as listed per component; see tokens section 1.3.

## 1. Button

Anatomy: notched box (`NOTCH_1`), label `TYPE_BUTTON`, optional 12 AP icon left (gap `SPACE_3`), padding x
`SPACE_5` (16), height `CONTROL_H` 32 (desktop) / 48 (phone), `CONTROL_H_LARGE` 40 for the page Play.

| State | Primary | Secondary | Ghost |
|---|---|---|---|
| rest | fill `WARM_WHITE`, label/icon `TEXT_ON_LIGHT`, ledge `LEDGE_PRIMARY` (stone) at `LEDGE_OFFSET` | fill none, border 1 AP `CONTROL_BORDER`, label `TEXT_PRIMARY`, ledge `LEDGE_SECONDARY` (charcoal) | no fill, no border, no ledge, label `TEXT_SECONDARY` |
| hover | fill `PURE_WHITE` | fill `SURFACE_RAISED`, border `CONTROL_BORDER_HOVER` | fill `SURFACE_RAISED`, label `TEXT_PRIMARY` |
| pressed | translate 1 AP, ledge hidden | translate 1 AP, ledge hidden, fill `SURFACE_SELECTED` | fill `SURFACE_SELECTED` (no translate) |
| focused | focus ring | focus ring | focus ring |
| disabled | fill `SURFACE_SELECTED`, label `TEXT_DISABLED`, no ledge | border `LINE_STRONG`, label `TEXT_DISABLED`, no ledge | label `TEXT_DISABLED` |
| selected (toggle buttons such as "Segui già") | — | label `TEXT_PRIMARY`, leading `icon_check` (to draw), border `CONTROL_BORDER_HOVER` | — |

Destructive (dialogs only): primary with fill `ERROR`, label `TEXT_ON_LIGHT`, ledge `STONE`. Loading: the label is
replaced by `loader` at the same width; the button stays pressed-looking (ledge hidden) and inert.
One primary button per view region at most.

## 2. IconButton

Anatomy: `ICON_BUTTON` 32 x 32 (phone 48 x 48 hit, 40 x 40 visible), `NOTCH_1`, a 12 AP mask icon centred.

| rest | hover | pressed | focused | disabled | selected (toggle on) | playing |
|---|---|---|---|---|---|---|
| no fill, icon `ICON_IDLE` | fill `SURFACE_RAISED`, icon `ICON_ACTIVE` | fill `SURFACE_SELECTED` | focus ring | icon `ICON_DISABLED` | icon tinted `STATE_ON` (lime) + a 2 x 1 AP lime dot centred 1 AP under the icon | — |

Every IconButton has a tooltip with its name and shortcut. Filled variants (`icon_heart_fill`) replace the outline
icon when the thing is on; liked uses `ICON_ACTIVE`, not lime (liking is not a playback state), everywhere,
player bar included (owner decision, 2026-11-22).

## 3. Card

Used for: "Sta ascoltando" (friend presence), listening stats, top search result. Not used for album/artist
grids (those are tiles, section 12).

Anatomy: fill `SURFACE_RAISED`, `NOTCH_2`, padding `SPACE_5`, no border at rest, no shadow.
| rest | hover (if clickable) | pressed | focused | disabled | selected |
|---|---|---|---|---|---|
| as above | border 1 AP `LINE_STRONG` | fill `SURFACE_SELECTED` | focus ring | content `TEXT_DISABLED` | border 1 AP `CONTROL_BORDER_HOVER` |

## 4. Panel

Chrome regions (sidebar, side panel, player bar, title bar): fill `SURFACE_CHROME`, flush with the window, divided
by 1 AP `LINE_SUBTLE` separators; no radius, no gap, no shadow. Panel header: height 40, title `TYPE_LABEL`
`TEXT_TERTIARY` or in-page tabs (section 15), actions as IconButtons right-aligned. Content canvas between panels:
`SURFACE_CANVAS`. Panels never float except the side panel below `BP_PANEL_DOCK`, which overlays at `Z_PANEL_OVERLAY`
with a 1 AP `LINE_STRONG` left border and `SHADOW_FLOAT_OFFSET`.

## 5. Badge

Anatomy: height 16 dp, padding x `SPACE_2`, `NOTCH_1`, text `TYPE_LABEL`, no icon except the live dot.

| Variant | Look | Use |
|---|---|---|
| neutral | border 1 AP `CONTROL_BORDER`, text `TEXT_SECONDARY` | `MP3 320`, `AAC`, licence short name (`CC BY`) |
| quality | border 1 AP `CONTROL_BORDER_HOVER`, text `TEXT_PRIMARY` | `LOSSLESS`, `FLAC 24/96` (not lime: quality is not a state) |
| count | fill `WARM_WHITE`, text `TEXT_ON_LIGHT`, min width 16 | unread releases, friend requests |
| live | 2 x 2 AP `LIME` square + gap 1 AP + text `TEXT_PRIMARY`, no border | `LIVE` jam, friend listening now |
| error | border 1 AP `ERROR`, text `TEXT_ERROR` | failed download |

Badges have no hover/press; disabled → text `TEXT_DISABLED`.

## 6. Tooltip

Fill `SURFACE_RAISED`, border 1 AP `LINE_STRONG`, `NOTCH_1`, padding 4 x 8 dp, text `TYPE_META` `TEXT_PRIMARY`,
shortcut right in `TYPE_META` `TEXT_TERTIARY` (`Ctrl K`). Max width `TOOLTIP_MAX_W`. Appears after `TOOLTIP_DELAY`,
fades in `MOTION_FAST`, no movement; placed 4 dp below the target (above near the bottom edge). `Z_TOOLTIP`.
Phone: no tooltips; long-press opens the context sheet.

## 7. Menu and context menu

Anatomy: width `MENU_W`, fill `SURFACE_RAISED`, border 1 AP `LINE_STRONG`, `NOTCH_2`, shadow `SHADOW_FLOAT` at
`SHADOW_FLOAT_OFFSET`, padding y `SPACE_2`. Item: height `MENU_ITEM_H` 32, padding x `SPACE_4`, icon 12 AP
`ICON_IDLE` + gap `SPACE_3` + label `TYPE_NAV` `TEXT_PRIMARY`, shortcut right `TYPE_META` `TEXT_TERTIARY`,
submenu `chevron` right. Separator: 1 AP `LINE_SUBTLE` with `SPACE_2` above and below. Opens at the pointer, kept
inside the window; appears in `MOTION_FAST` (opacity only). `Z_MENU`.

| Item state | Look |
|---|---|
| rest | as above |
| hover / keyboard highlight | fill `SURFACE_SELECTED`, icon `ICON_ACTIVE` |
| pressed | fill `SURFACE_SELECTED`, label shifts 0 (menus never translate) |
| focused | same as highlight (menus manage focus with arrows) |
| disabled | label and icon `TEXT_DISABLED`, skipped by arrows |
| selected (checked) | `icon_check` (12 AP, to draw) in the icon column, `ICON_ACTIVE` |
| destructive | label `TEXT_ERROR` |

Phone: bottom sheet. Fill `SURFACE_RAISED`, top edge 1 AP `LINE_STRONG` with `NOTCH_2` top corners, handle
`SHEET_HANDLE`, rows `SHEET_ITEM_H` 56 with 12 AP icons, `SCRIM` behind, slides up in `MOTION_PANEL` with offsets
quantised to `ap_device`.

## 8. Separator

1 AP `LINE_SUBTLE` (regions, list groups, menu groups). Strong: 1 AP `LINE_STRONG` (floating layer edges only).
Never thinner, never a gradient, never inset with a margin inside rows.

## 9. ProgressBar / seek

Anatomy: track height `SEEK_TRACK_H` 4 dp (2 AP), hit area 24 dp (phone 48), times `TYPE_NUMERIC` `TEXT_TERTIARY`
left (elapsed) and right (total; click toggles remaining). Segments, left to right: elapsed `WARM_WHITE`, buffered
`IRON`, unbuffered 50 % dither `CHARCOAL`/`GRAPHITE`.

| rest | hover | pressed / dragging | focused | disabled | playing |
|---|---|---|---|---|---|
| no thumb | elapsed `PURE_WHITE`, thumb 2 x 6 AP `PURE_WHITE` at the head, time preview tooltip at the pointer | thumb follows pointer, elapsed `PURE_WHITE`, preview tooltip shows target time | focus ring around the track; ←/→ seek 5 s | elapsed `STONE`, no thumb | updates on `PROGRESS_TICK` (250 ms), never per frame |

Phone: thumb always visible (3 x 8 AP), drag with axis lock, times under the bar. Mini player: 1 AP elapsed line on its
top edge, no thumb, no hit area.

## 10. Slider / volume

Same anatomy as seek, width `VOLUME_W` 96, no buffered segment, value tooltip in percent while dragging.
Leading IconButton toggles mute (`icon_volume`/`icon_mute`, to be drawn). Scroll wheel: ±5 %. Disabled → as seek.
Equalizer bands (settings): ten vertical sliders 2 AP wide, 64 AP tall, value `TYPE_NUMERIC` under each, 0 dB tick
1 AP `LINE_STRONG` across all bands.

## 11. TrackRow

Desktop: height `TRACK_ROW_H` 48, columns: index `TRACK_INDEX_W` 32 | cover `COVER_XS` 32 | title
`TYPE_TRACK_TITLE` `TEXT_PRIMARY` over artist `TYPE_ARTIST` `TEXT_SECONDARY` | album `TYPE_ARTIST` (≥ 1100 dp) |
actions (heart, download, more; 32 each) | duration `TYPE_NUMERIC` `TEXT_TERTIARY` right. Padding x `SPACE_3`.
Dense variant `TRACK_ROW_DENSE_H` 32: one line, columns `# | title | artist | album | added | duration`, column
headers `TYPE_LABEL` `TEXT_TERTIARY`, sticky at `Z_STICKY`.

| State | Look |
|---|---|
| rest | no fill; index number `TYPE_NUMERIC` `TEXT_TERTIARY`; actions hidden except liked heart (`ICON_ACTIVE`) and downloaded mark |
| hover | fill `SURFACE_RAISED`; index → `icon_play` `ICON_ACTIVE`; actions visible (`ICON_IDLE`); download hidden but space kept if not downloadable |
| pressed | fill `SURFACE_SELECTED` (double-click / Enter plays) |
| focused | inset focus ring |
| disabled (unavailable) | title and artist `TEXT_DISABLED`, cover drawn over a 50 % dither of `DEEP_BLACK`, meta "Non disponibile" |
| selected (multi-select) | fill `SURFACE_SELECTED`, index → `icon_check` `ICON_ACTIVE` |
| playing | index → `eq` tinted `STATE_ON` (animated), title stays `TEXT_PRIMARY`; paused → `eq_rest` tinted `STONE`; buffering → `loader` |

Right click opens the context menu; drag starts after `DRAG_THRESHOLD`, ghost = row at `SHADOW_FLOAT_OFFSET`,
drop line 1 AP `WARM_WHITE`. Phone: `PHONE_ROW_H` 64, cover 48, title `TYPE_TRACK_TITLE` phone, more button 48 hit
area right, long-press = context sheet, swipe right = add to queue (reveals a `SURFACE_SELECTED` strip with
`icon_queue`, snaps back).

## 12. AlbumTile (and ArtistTile)

Anatomy: cover square, width = column width (min `TILE_MIN_W` 160), no radius, no frame; gap `SPACE_3`; title
`TYPE_TRACK_TITLE` 1 line; subtitle `TYPE_ARTIST` `TEXT_TERTIARY` 1 line (year · type, or artist).
ArtistTile: cover with `MASK_PIXEL_CIRCLE`, centred text.

| rest | hover | pressed | focused | disabled | selected | playing |
|---|---|---|---|---|---|---|
| cover + text | 1 AP `CONTROL_BORDER_HOVER` frame drawn inside the cover edge; Play IconButton (primary look: `WARM_WHITE` fill, `TEXT_ON_LIGHT` icon, stone ledge, `NOTCH_1`) at cover bottom-right, inset `SPACE_3` | Play button presses (translate); tile itself does not move | focus ring around the cover | cover over dither, text `TEXT_DISABLED` | frame 1 AP `WARM_WHITE` | `eq` (lime) before the subtitle text, Play button shows `icon_pause` |

Missing cover: `GRAPHITE` field + `album_placeholder` centred at 1 AP. Phone: shelves of `PHONE_SHELF_COVER` 144,
no hover; tap opens, long-press menu.

## 13. PlayerControl (transport)

Group: shuffle (IconButton toggle), previous (IconButton), play/pause, next (IconButton), repeat (IconButton,
3-state: off, all = `icon_repeat` lime, one = `icon_repeat_one` lime). Gaps `SPACE_3`.
Play/pause: `CONTROL_H_LARGE` 40 x 40 (phone `PHONE_PLAY_BUTTON` 64), primary button look (fill `WARM_WHITE`,
icon `TEXT_ON_LIGHT`, stone ledge, `NOTCH_1`); pressed translates 1 AP.

| State | play/pause | shuffle / repeat |
|---|---|---|
| rest | `icon_play` | `ICON_IDLE` |
| hover | fill `PURE_WHITE` | IconButton hover |
| pressed | translate, ledge hidden | IconButton pressed |
| focused | focus ring | focus ring |
| disabled (empty queue) | fill `SURFACE_SELECTED`, icon `ICON_DISABLED` | `ICON_DISABLED` |
| selected | — | lime icon + lime dot (section 2) |
| playing | `icon_pause`; the button stays warm white: the lime cue for playback lives in `mark_play` and `eq` | — |

Loading a track: play/pause swaps its fill to `SURFACE_SELECTED` and shows `loader` (the loader keeps its own grey ramp).
Keyboard: Space, Ctrl+→/←, Ctrl+S shuffle, Ctrl+R repeat.

## 14. NavigationItem (sidebar, phone tab)

Desktop: height `NAV_ITEM_H` 32, padding x `SPACE_3`, icon 12 AP + gap `SPACE_3` + label `TYPE_NAV`; optional count
badge right.

| rest | hover | pressed | focused | disabled | selected |
|---|---|---|---|---|---|
| icon outline `ICON_IDLE`, label `TEXT_SECONDARY` | fill `SURFACE_RAISED`, label `TEXT_PRIMARY` | fill `SURFACE_SELECTED` | inset focus ring | `TEXT_DISABLED` | `MARKER_W` 2 AP `LIME` bar on the left edge, inset 1 AP top/bottom; filled icon variant `ICON_ACTIVE`; label `TEXT_PRIMARY`; no fill |

Rail (collapsed): icon only, 56 wide, the lime bar stays on the left edge, tooltip with the label.
Phone tab: 4 tabs across `PHONE_TAB_BAR_H` 56; icon 12 AP over label `TYPE_NAV` phone. Selected: `MARKER_W` 2 AP
lime bar 12 AP wide centred on the tab's top edge, filled icon and label `TEXT_PRIMARY`; rest `TEXT_SECONDARY`.
Unread dot: 2 x 2 AP `WARM_WHITE` at the icon's top-right (not lime: unread is not live).

## 15. Tabs (in-page)

Settings sections, side panel (In riproduzione / Coda / Testo), library filters, search groups. Replaces every pill
chip. Anatomy: labels `TYPE_NAV`, padding x `SPACE_4`, height 32, 1 AP `LINE_SUBTLE` baseline across the tab row.

| rest | hover | pressed | focused | disabled | selected |
|---|---|---|---|---|---|
| `TEXT_SECONDARY` | `TEXT_PRIMARY` | `TEXT_PRIMARY` | inset focus ring | `TEXT_DISABLED` | `TEXT_PRIMARY` + 2 AP `WARM_WHITE` underline over the baseline, width = label |

The underline jumps to the new tab instantly (no sliding spring). Lime is not used: one lime selection per screen
belongs to primary navigation. Phone: same, scrollable row, height 48.

## 16. SearchField

Anatomy: height `CONTROL_H` 32 (phone 48), max width `SEARCH_FIELD_MAX_W`, fill `SURFACE_WELL`, border 1 AP
`CONTROL_BORDER`, `NOTCH_1`, leading `icon_search` `ICON_IDLE`, text `TYPE_BODY` `TEXT_PRIMARY`, placeholder
"Cerca brani, album, artisti" `TEXT_TERTIARY`, trailing shortcut hint `Ctrl K` (`TYPE_META`, `TEXT_TERTIARY`, 1 AP
`LINE_STRONG` box) when empty and unfocused, `icon_close` clear button when not empty. Caret 1 AP x 16 dp `CARET`,
blinking `CARET_BLINK`.

| rest | hover | pressed | focused | disabled | loading |
|---|---|---|---|---|---|
| as above | border `CONTROL_BORDER_HOVER` | — | border `PURE_WHITE`, shortcut hint hidden | border `LINE_STRONG`, placeholder `TEXT_DISABLED` | `loader` replaces `icon_search` (after 300 ms) |

Esc clears then blurs; ↓ moves into results; Enter opens the top result.

## 17. Toggle (switch)

Anatomy: track 16 x 8 AP (32 x 16 dp), border 1 AP, fill `SURFACE_WELL`, `NOTCH_1`; knob 6 x 6 AP square, inset
1 AP. Label left (`TYPE_TRACK_TITLE`), description below (`TYPE_ARTIST`), switch right. Hit area 32 (phone 48) high.

| off | on | hover | pressed | focused | disabled |
|---|---|---|---|---|---|
| border `CONTROL_BORDER`, knob `STONE` left | border `LIME`, knob `LIME` right | off: border and knob `SOFT_GREY`; on: unchanged | knob moves in 2 steps of 50 ms (`MOTION_FAST`), quantised to AP | focus ring | border and knob `LINE_STRONG`, position kept |

## 18. LoadingIndicator

- Inline wait (button, field, row, region header): `loader` sprite 12 AP, shown only if the wait passes 300 ms.
- Content placeholder: skeleton blocks at the real size of the content, fill `SURFACE_RAISED`, stepping to
  `SURFACE_SELECTED` and back every 600 ms (2-frame step, no fade); stops under `reduce_motion`.
- Long page load (> 2 s): `loader` + one line `TYPE_META` `TEXT_TERTIARY` ("Carico la pagina dell'artista…").
- Never a spinner, never a progress ring, never a full-screen blocker after the splash.

## 19. DownloadIndicator

12 AP icon in the row's download slot or in the album header.

| State | Look |
|---|---|
| available | `icon_download` `ICON_IDLE` (visible on hover in rows; always in headers) |
| not downloadable | nothing drawn, space kept |
| queued | `icon_download` `ICON_DISABLED` + tooltip "In coda" |
| downloading | `icon_download` `ICON_ACTIVE`; its tray row becomes a 10-step progress bar (each step 1 AP, `WARM_WHITE` over `IRON`) |
| done (on this device) | `icon_downloaded` `ICON_ACTIVE` (always visible) |
| error | `icon_download` tinted `ERROR` + tooltip with the reason and "Riprova" |

No lime: availability offline is a fact, not a live state. Batch downloads show a count badge on the Downloads nav item.

## 20. Equalizer (playing indicator)

`eq` 12 AP mask tinted `STATE_ON`, 6 frames x 120 ms, in: the playing TrackRow index, the playing tile's subtitle, the
queue's "In riproduzione" row, the Downloads/Library row of the playing track. Paused: `eq_rest` tinted `STONE`.
Buffering: `loader`. The title-bar brand mark is `mark` (`WARM_WHITE`) at rest and `mark_play` (lime) while playing.
Never more than one animated `eq` per list (only the playing track).

## 21. Toast

Width `TOAST_W` 360, `SPACE_5` above the player bar, centred on the content column; fill `SURFACE_RAISED`, border
1 AP `LINE_STRONG`, `NOTCH_1`, `SHADOW_FLOAT_OFFSET`; text `TYPE_BODY` `TEXT_PRIMARY`; optional ghost action right
("Annulla"). In/out: opacity over `MOTION_BASE`, no movement; holds `TOAST_HOLD` 4 s, paused on hover. Max one at a
time (new replaces old). Error toasts add a `MARKER_W` `ERROR` bar on the left edge, but errors belong inside the
failed block first; toasts are for confirmations ("Aggiunto alla coda"). `Z_TOAST`. Phone: full width minus 2 x
`PHONE_GUTTER`, above the mini player.

## 22. Dialog

Width `DIALOG_W` 400 (phone: full width minus 32, anchored to the bottom like a sheet), fill `SURFACE_RAISED`, border
1 AP `LINE_STRONG`, `NOTCH_2`, `SHADOW_FLOAT_OFFSET`, padding `SPACE_6`; title `TYPE_HEADING`, body `TYPE_BODY`
`TEXT_SECONDARY`, field (if any) as SearchField without icons, buttons right-aligned: secondary (Annulla) then
primary (or destructive). `SCRIM` behind at `Z_SCRIM`, dialog `Z_DIALOG`; opacity in `MOTION_BASE`. Enter confirms,
Esc or scrim click cancels, focus trapped and returned to the trigger.

## 23. EmptyState

Centred column, max width 360: illustration at 1 AP (`empty_crate`, `nastro_idle` or `nastro_sleep`), gap
`SPACE_6`, title `TYPE_HEADING` `TEXT_PRIMARY`, gap `SPACE_3`, one sentence `TYPE_BODY` `TEXT_SECONDARY`, gap
`SPACE_6`, one secondary button (the next action). Vertical position: 1/3 from the top of the region. One mascot per
screen at most. Copy says what is empty and what to do ("Ancora nessun disco. Cerca un artista e salva un album.").

| Case | Sprite |
|---|---|
| empty library, playlist, downloads, search with no results | `empty_crate` |
| offline with nothing downloaded | `nastro_sleep` |
| no lyrics for this track (playing) | `nastro_listen` |
| about page, first run | `nastro_idle` |
| errors | no mascot: text + "Riprova" in the failed block |

## 24. Keyboard focus ring (summary)

1 AP `PURE_WHITE`, 1 AP gap, follows the notch, shown only for keyboard focus; inset for rows, tiles' ring around the
cover only; text fields use a white border instead. Focus order follows reading order: title bar → sidebar →
content → side panel → player bar. Shortcuts panel (`?`) lists every binding in `TYPE_META` with key caps drawn as
1 AP `LINE_STRONG` boxes.

## 25. Phone variants (summary)

| Component | Phone difference |
|---|---|
| Button | height 48, label `TYPE_BUTTON` 14/16 |
| IconButton | 48 hit area, 40 visible, no tooltip |
| TrackRow | 64 high, cover 48, long-press sheet, swipe right to queue |
| Seek / volume | thumb always visible, 48 hit, axis-locked drag; volume is a vertical sheet |
| Menu | bottom sheet with `SCRIM`, rows 56 |
| Tabs | 48 high, scrollable |
| Navigation | bottom tab bar with top lime marker |
| Toast | full width above the mini player |
| Dialog | bottom-anchored, full width |
| Tooltip | none |
