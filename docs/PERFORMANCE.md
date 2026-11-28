# Client performance

Measurements, bottlenecks and the state of the Psytify client's optimisation. The rules for writing
fast code are in `AGENTS.md`, section "Performance rules".

## How it is measured

- Tool: `scripts/perf-sample.py PID [SECONDS]`. It reads `/proc` (CPU per thread, RSS) and
  `nvidia-smi` (compute memory). It needs no privileges.
- **The window must be visible and focused on the active workspace.** With the screen locked or the
  window covered the compositor does not send frame callbacks and the UI's CPU appears close to zero:
  such a measurement is false (it happened, see below).
- Reference scenario: test instance with an isolated configuration (`XDG_CONFIG_HOME`), volume at
  zero, an m4a track with synchronised lyrics, the "Lyrics" panel open (key `T`), video closed. It is
  started and playback is begun via MPRIS (`org.mpris.MediaPlayer2.psytify.instance<PID>`).
- A/B comparisons: two binaries, two rounds each, alternating. RAM varies by ±20 MB between two identical
  starts: repeated differences count, not a single value.
- Sampling profiler (`perf`): not installed. For memory `gdb` is used with the program
  started under gdb (`ptrace_scope=1`): `call (void)malloc_stats()` and `call (int)malloc_trim(0)`.

## Results (m4a playback, lyrics open, video closed)

| | Before | After |
|---|---|---|
| Total CPU | 10.6% of one core | **1.6–1.8%** |
| UI thread | 9.0% | **0.7%** |
| RSS after 15 s | 337–388 MB | **230–237 MB** |
| NVIDIA and Mesa/LLVM libraries loaded | yes (~70 MB) | no |
| NVIDIA context of the process | yes (graphics, 2 MiB) | none |

Server: 21 MB of RSS, no action needed.

### What changed

1. **No continuous frames** (`app.rs`, `subscription`). `window::frames()` sent a message at every
   vsync while playing, and every message redraws the window. Now full frames start only with
   an animation (`is_animating`); the progress bar updates with the Tick (4 times a second, a step
   below the pixel). Instrumenting `view` and `update`: they cost less than 1 µs and 2 µs. The ~1.4 ms per
   redraw is all in iced's renderer, so the only lever is the number of redraws.
2. **Light environment** (`launch.rs`, Linux only). The client re-launches itself once with `exec` (same
   PID) setting:
   - `MALLOC_ARENA_MAX=2`: glibc opened 32 arenas (one per thread) and kept 186 MB of heap for 111 MB in
     use; `malloc_trim` alone brought the RAM from 270 down to 226 MB.
   - `VK_LOADER_DRIVERS_DISABLE=*nvidia*` and `WGPU_BACKEND=vulkan`, only if the driver of an
     integrated GPU (RADV, ANV, AMDVLK) is also present: iced already draws on the low-power GPU, and the NVIDIA
     driver and wgpu's OpenGL were loading for nothing (~70 MB and the dedicated card waking up).
   - To choose the GPU by hand: `PSYTIFY_ALL_GPUS=1`, or one of the variables `VK_*`, `WGPU_*`,
     `DRI_PRIME`, `__NV_PRIME_RENDER_OFFLOAD`: in that case nothing is touched.
3. The option "Video clip on the graphics card" (Settings → Playback) works with the light environment:
   NVDEC loads only when it is active (126 MiB of VRAM), otherwise VA-API.

### 27 October 2026: new animations, cache, release profile

Release binaries, window visible on the laptop (60 Hz), volume at zero, "In the End" (m4a,
synchronised lyrics), two rounds each.

| Scenario | Total CPU | UI thread | RSS |
|---|---|---|---|
| Search + "Now playing", previous profile (A) | 1.6% / 1.6% | 0.4–0.5% | 200 / 194 MB |
| Same, full LTO + 1 codegen unit (B) | 1.5% / 1.4% | 0.4% | 199 / 194 MB |
| Home + "Lyrics" in the panel, scrolling at vsync (650 ms) | 6.9% / 7.2% | 5.8% | 192 / 201 MB |
| Same, scrolling at 30 fps, 480 ms | 4.2% / 4.1% | 2.8–3.0% | 193 / 196 MB |
| Same, 24 fps and the body of the lines at pixel precision (**adopted**) | **3.0% / 2.6%** | **1.6–1.8%** | **190 / 192 MB** |

- **Release profile** (`Cargo.toml`): `lto = "fat"`, `codegen-units = 1`. Binary from 41 to 28 MB, CPU
  equal or slightly lower (within the noise), RAM equal. Build from scratch 2 min 51 s; at every
  change to the client the link with LTO costs ~2 min 15 s (in development the `dev` profile is used).
  `panic` stays `unwind`.
- **The scrolling text** is the most frequent movement (at every line, every ~2 s): at vsync it cost
  5.8% of the UI thread. Now it does not request the window's frames but a timer at 24 fps
  (`AMBIENT_FRAME`), lasts 480 ms, and the body of the growing lines is rounded to the pixel, so the
  text is laid out again only when the measure really changes. The other movements stay at vsync.
- **Cover cache** (`ui/cover.rs`): at most 240 ready covers, discarding the least recently
  drawn one (LRU). The round version (artists) is derived from the square only when needed:
  256 KB less for each album cover. `is_fading` no longer scans the whole cache at every
  message.
- To measure the "Lyrics" panel without a mouse or keyboard there is the `hold-lyrics` step of the automatic
  tour (`--features tour`, `PSYTIFY_TOUR_STEPS=playing,hold-lyrics`): it opens the lyrics and stays
  still for 40 s.

### 28 October 2026: first listen to a track from the official catalogue

Time of `POST /v1/meta/tracks/{id}/resolve` (development server, measured with `curl`):

| Scenario | Time |
|---|---|
| First listen via YouTube, search with yt-dlp (10 tracks) | 3.8–4.9 s, average ~4.3 s |
| Same, direct search and sources together (10 new tracks) | **3.2–4.2 s, average ~3.7 s** |
| First listen via Internet Archive in lossless | 1.4–1.5 s → **1.2 s** |
| Second listen of the same track | **2–4 ms** |
| Two requests together for the same track | a single import: the second waits for the first |

Where the time of a YouTube track goes (yt-dlp 2026.08.19, `-v` with the time of each line):

| Phase | Time |
|---|---|
| Start of yt-dlp (Python) | ~0.25 s per process |
| Search `ytsearch20`: client configuration + API | ~1.3 s (direct: **~0.7 s**) |
| Extraction: video page (1.2 MB, slow even with `curl`) + player API | ~1.4 s |
| Download of the m4a audio (5 MB, a single file) | ~0.8 s |
| Correction of the m4a container | ~0.1 s |

Options tried and discarded: `player_skip=webpage` and `player_skip=webpage,configs` trigger YouTube's
anti-bot check ("Sign in to confirm you're not a bot"); `player_client=default,-tv`
does not change the time; the clients `android_vr`, `ios`, `tv_simply` and `web_safari` do not give the audio
format; `-N 8` (fragments in parallel) is not useful, the audio is a single file. What changed:

- **Direct YouTube search** (`ingest/sources.rs`, `youtube_direct`): the same call as the results
  page (`youtubei/v1/search`), without a process. If it does not answer within 4 s, changes shape or gives no video,
  `ytsearch` is used as before. It also applies to the app's search.
- **The three sources together** (`find_song_anywhere`): YouTube, SoundCloud and Bandcamp start together;
  the first in the order that has the track always wins, the others stop. If YouTube does not have it, the
  answer from SoundCloud is already ready.
- **The download starts while Archive searches** (`ingest/resolve.rs`): the audio from the other source
  is downloaded in parallel with the lossless search (`prepare_source`) and is recorded only if Archive
  does not find it; otherwise the download stops (yt-dlp terminated, folder deleted).
- **A pressed play does not queue** (`app/external.rs`): "Play" imports the first
  track at once, even if a background import (a saved playlist) is in progress. Before, it waited
  for that to finish: up to ~4–5 s more.
- **One track at a time per id** (`ingest/resolve.rs`, `Resolving`): two requests for the same
  track do not start two downloads; whoever waits finds the track in the catalogue.
- **Archive**: the metadata of the 3 candidates is downloaded together instead of one after another, and the chosen file
  stays tied to Deezer's id, so the second listen is immediate even if on Archive the title is
  written differently.
- **Discarded: preparing the track on mouse hover.** Resolving and signing have a limit of 120
  requests per minute per IP: the hover requests would take room from the real click.
- **Still to try: playing while downloading.** Today the client starts when the file is on the server's
  disk (~0.9 s of download and correction). YouTube's m4a is fragmented and yt-dlp rewrites it
  at the end: serving it halfway requires skipping the correction and a decoder that reads the fragments.

### Discarded attempts

- **mimalloc** as the global allocator: RSS 522–540 MB against 344 (it keeps the pages, and the C libraries
  keep using glibc). Removed.
- **Bar at 20 or 10 Hz with `time::every`**: 4.5% and 3.6% of total CPU. The Tick is enough.

## Migration to the core

One line for each measured step of the migration (`docs/transformation/04-core-architecture.md`).
Release binaries, isolated configuration (`XDG_CONFIG_HOME`), public backend, no track. For
each sample: 15 s of settling, then `scripts/perf-sample.py PID 60`, with focus checked
by `niri msg focused-window` before and after. A is `main`, B the step's branch, in alternating rounds (A1 B1
A2 B2). The wake-ups are the context switches of all the threads in the same 60 s, read from `/proc`.

| Date | Step | Scenario | Total CPU A → B | UI thread A → B | RSS A → B | Threads A → B | Wake-ups/s A → B |
|---|---|---|---|---|---|---|---|
| 22 November 2026 | 3: core host (`9e7becd` → `634ff85`) | Home idle, window visible | 1.1 / 1.0% → 1.2 / 0.9% | 0.8 / 0.7% → 0.9 / 0.6% | 174 / 159 → 156 / 151 MB | 49 / 43 → 44 / 44 | 247 / 254 → 247 / 251 |

- **The idle core does no work**: in every sample the `psytify-core` thread has 0 wake-ups and 0 CPU
  ticks. B has one more thread, its own; CPU, RSS and wake-ups stay within the noise between two starts.
- A1 is the first start after compilation: Mesa writes the shader cache and opens 6 more
  `psytify:disk$N` threads, hence 49 threads and 174 MB. In the other rounds the cache is already there.
- A2 lost focus during the sample (a dialogue of another app), with values in
  line with A1. A third round is discarded because focus moved to other windows: A3 has 3.1% and
  749 wake-ups per second, B3 was never focused (99 MB).
- The ~250 wake-ups per second were already there and are the same in the two binaries: ~150 from the 17 tokio threads of
  Iced, ~25 each for the UI thread, smithay-clipboard, the audio output (cpal) and PipeWire.

### Theme (step 3b, Iced theme on `psytify-design`)

Same method as above: release binaries, isolated configuration, Home idle with the window visible and
focused (checked before and after with `niri msg`), 15 s of settling and `scripts/perf-sample.py PID 60`,
A (`main`, 2f4b3fa) and B (branch, pixel theme) in alternating rounds A1 B1 A2 B2. The sprites are enlarged on the CPU
once per `(sprite, frame, factor, tint)` and the view only does a lookup in the cache.

| Date | Step | Scenario | Total CPU A → B | UI thread A → B | RSS A → B | Threads A → B |
|---|---|---|---|---|---|---|
| 22 November 2026 | 3b: pixel theme (`2f4b3fa` → branch `mig/iced-theme`) | Home idle, window visible | 1.1 / 1.1% → 1.0 / 1.0% | 0.8 / 0.8% → 0.7 / 0.7% | 162 / 95 → 164 / 165 MB | 44 / 44 → 44 / 44 |

- CPU and UI thread stay within the noise (B a tenth lower: no gradients or tints to draw). The RSS of
  A2 (95 MB) is the typical low value between two starts (±20 MB and more, see above); B is not above A.
- The sprites visible together weigh a few tenths of a MiB enlarged; the cache has a limit of 128 entries and
  8 MiB (`UpscaleCache`) plus at most 192 Iced handles.
- Not measured: playback (the `eq` bars choose the frame from the clock at every redraw, with no timers
  of their own) and the phone.

### Playback (step 4.3, the playback domain in the core)

Same method, release binaries of `main` (`04292af`, A) and of the branch `mig/playback` (`526cdec`, B), in alternating
rounds (idle: A1 B1 A2 B2 A3 B3; playback: A1 B1 A2 B2), 15 s of settling and 60 s of sample. The
window is on the MSI screen (HDMI-A-1, 144 Hz), in the active workspace, and it stayed there in all the
valid samples (checked every 2 s with `niri msg`); the focus did not, because whoever uses the computer works on the other
screen: the CPU does not depend on it as long as the window is visible (B1 without focus 2.2%, B2 with focus for 23 samples out of
30 2.3%). In playback the track is only one, an m4a of 273 s from the public backend, at volume 0, without lyrics nor
video; the track starts from the system widget (`playerctl play-pause`), which also serves as a test of MPRIS.

| Date | Step | Scenario | Total CPU A → B | UI thread A → B | RSS A → B | Threads A → B | Wake-ups/s A → B |
|---|---|---|---|---|---|---|---|
| 23 November 2026 | 4.3: playback in the core (`04292af` → `526cdec`) | Home idle | 1.0 / 1.1 / 1.1% → 1.2 / 1.1 / 1.1% | 0.6 / 0.6 / 0.7% → 0.7 / 0.7 / 0.7% | 159 / 168 / 149 → 179 / 162 / 159 MB | 43 → 43 | 290 / 282 / 310 → 314 / 308 / 306 |
| 23 November 2026 | 4.3: playback in the core (`04292af` → `526cdec`) | Playing (m4a, no lyrics nor video) | 2.6 / 2.6% → 2.2 / 2.3% | 1.6 / 1.7% → 1.3 / 1.3% | 147 → 136 MB (the other two: 23 and 45 MB, see below) | 43 → 43 | 400 / 402 → 387 / 387 |

- **The idle core does no work**: at rest `psytify-core` has 0 wake-ups and 0 CPU ticks in all the samples. While
  playing it has about 250 wake-ups in 60 s (the 250 ms Tick: 4 per second) and 0–1 CPU ticks (less than 0.02% of a core; the 0.1%
  that `perf-sample.py` attributes to "psytify-core" is the souvlaki loop, which inherits the name). The Tick is armed only while the track plays and stops when paused and at the end of the queue.
- **Idle**: A and B are in the same range (CPU 1.0–1.1% against 1.1–1.2%, wake-ups 282–310 against
  306–314). The first two rounds seemed to show ~25 more wake-ups per second in B; the third disproved them
  (310 against 306), so it is noise.
- **Playback**: B is 12–15% below A in total CPU (2.2–2.3% against 2.6%) and in the UI thread (1.3% against
  1.6–1.7%), in both pairs. With n = 2 per side I do not declare it an improvement: I only know it is not worse.
  I have not isolated the cause.
- **MPRIS**: with B the core registers `org.mpris.MediaPlayer2.psytify.instance<pid>`; `playerctl` sees the track
  (title, artist, duration), `play-pause` and `position 100` control it, and the state returns to `Paused` or `Playing`
  accordingly. Souvlaki's 10 ms loop (~100 wake-ups per second, the same in A) now lives in a thread that inherits the
  name `psytify-core` instead of `tokio-rt-worker`: same cost, different name in the list.
- **Threads**: 43 in A and in B.
- **RSS**: two samples are anomalous (23 MB in A1 and 45 MB in B2, well below every other start); I kept them out of the
  comparison. The others lie between 136 and 189 MB, within the ±20 MB variation between two starts (see above).
- **Discarded**: the samples with the window on the other screen (Lenovo), in an inactive workspace, where no
  frames arrive (1.0–1.1% and 253–260 wake-ups even with the `Playing` state); one sample with the `Paused` state
  (the `space` key had probably gone to another window); and the playback samples started with the `space` key, whose
  arrival cannot be verified.
- **Not measured**: synchronised lyrics (the core loads them only if a view asks, but the current line stays in the
  client), video, phone, other formats.

### Catalogue and search (step 4.4, the catalogue domain in the core)

Same method: release binaries, isolated configuration (`XDG_CONFIG_HOME`), public backend, 15 s of
settling and 60 s of sample with `scripts/perf-sample.py`. A is `main` (`33ef95a`): the binary is that of
`526cdec`, already compiled for step 4.3, because the two commits differ only in documents. B is
`mig/catalog` (`c3f6d6a`). The rounds are alternating (A1 B1 A2 B2, then A3 B3 B4 A4 and A5 A6 A7 A8 for the RSS). The
window is on the MSI screen (HDMI-A-1), in the active workspace, and in all the samples it stayed there and focused
for 30 checks out of 30 (`niri msg` every 2 s). Two scenarios: **Home idle** and **search**: the configuration
saves the "Search" screen and the text "daft punk", so at start-up the search starts and the response preloads
the pages of the first artists (in A the client loads them, in B the core does).

| Date | Step | Scenario | Total CPU A → B | UI thread A → B | RSS A → B | Threads A → B | Wake-ups/s A → B |
|---|---|---|---|---|---|---|---|
| 23 November 2026 | 4.4: catalogue in the core (`33ef95a` → `c3f6d6a`) | Home idle (8 rounds A, 4 rounds B) | 1.0–1.2% → 1.1–1.2% | 0.5–0.7% → 0.5–0.6% | 147–168 (average 156) → 157–177 (average 168) MB | 43 → 43 | 287–296 → 290 |
| 23 November 2026 | 4.4: catalogue in the core (`33ef95a` → `c3f6d6a`) | Saved search, response and artist pages in advance (2 rounds each) | 0.7 / 0.6% → 0.7 / 0.6% | 0.1 / 0.1% → 0.1 / 0.1% | 181 / 170 → 177 / 183 MB | 43 → 43 | 230 / 234 → 230 / 228 |

- **The idle core does no work**: `psytify-core` has 0 context switches and 0 CPU ticks in all the samples, even with
  the search done at start-up. The 300 ms wait of the search is a timer armed only between a key and its
  deadline: at rest there is none, and the tests fix it (`an_identical_input_changes_nothing_and_an_empty_wait_starts_nothing`,
  `host.rs::the_debounced_search_runs_on_the_runtime_and_the_core_goes_quiet`).
- **CPU, UI thread, threads and wake-ups** are in the same range in A and in B, in both scenarios. The 290
  wake-ups per second were already there (BACKLOG 13): step 4.4 does not move them.
- **RSS at rest: B is about 12 MB higher.** Average 156 MB in A (n = 8: 156, 150, 158, 149, 159, 168, 160, 147),
  168 MB in B (n = 4: 177, 168, 157, 168). The ranges overlap and the variation between two identical starts is
  ±20 MB (see above), but 3 of the 4 rounds of B are above the average of A, and I do not declare it noise. **I have not
  found the cause.** I tried the simplest hypothesis: each domain of the core builds its own `ApiClient` (three
  `reqwest::Client` each). A variant with a single shared client (`ApiClient::shared`, compiled and measured in
  4 rounds: 176, 168, 167, 167 MB, average 169.5) changes nothing, so I discarded it and it is not in the commit. It remains to
  see whether it matters where the JSON responses are decoded (now on the core runtime's threads, with other glibc
  arenas) or whether it is the noise of something else: to be checked in the performance pass after 4.4 (BACKLOG 13).
- **Search**: 0.6–0.7% CPU in both, less than on the Home because the carousel is not in view; the RAM is
  higher than at rest in both (+6 / +30 MB in B, +12 / +25 MB in A: the response, the covers and the artist pages
  in advance). I did not see the results on screen: I know the search started only because the RSS rises.
- **Not measured**: typing (the path with the wait is covered by the tests, not by a sample), opening
  artist and album pages with a click (I did not inject keys or clicks into the session of whoever uses the computer; an
  earlier sample had already lost some), so the RSS after many pages is not measured in the real app. The cache limit
  (40 artist pages, 24 album pages, discarding the least used) is fixed by the reducer's tests
  (`the_artist_cache_forgets_the_least_used_page`, `the_album_cache_is_bounded`). Nor the playback, the
  phone, the lyrics and the video.

### Downloaded tracks: playing from disk (step 4.6b)

Same method (release binaries, isolated `XDG_CONFIG_HOME`, 15 s of settling and 60 s of sample with
`scripts/perf-sample.py`, wake-ups from the voluntary context switches). A is `main` (`cce6a9b`), B is `mig/downloads`
(`3543bdb`); the rounds are alternating. **The track** is a stereo FLAC at 44.1 kHz of 300 s and 49.9 MB (pink noise plus a
440 Hz sine, made with `ffmpeg`), at volume 0. To play from the network a fake local backend is needed
(`stream-url` and `/v1/media` with `Range`, like the real proxy); the Home answers 404, so it shows its error blocks
with "Retry", the same for all the binaries. To play from disk the same track is in the index of downloaded
tracks, with the real file. The track starts with `playerctl play` (MPRIS). The **Home idle** case uses the public
backend as the measurements of 4.3 and 4.4 did.

**The window.** With the window in a column, niri does not say where it is (`tile_pos_in_workspace_view` is `null` for all
windows) and "active workspace" does not prove that frames arrive. In the second series the window is floating at a known point
of the MSI screen (HDMI-A-1), in the active workspace, and halfway through the sample a screenshot with `grim` shows the page and the time of the track advancing (0:48 at about 48 s from the start, progress bar in place): this is why **the table is the second series**. In the first series the window was in a column,
without pixel proof: in playback it gives the same values (CPU 1.1%, 271–272 wake-ups per second, same RSS to the MB),
at idle fewer wake-ups (279–280 against 342–344 with the floating window, equal in A and in B).

| Date | Step | Scenario | Total CPU | UI thread | RSS | Threads | Wake-ups/s |
|---|---|---|---|---|---|---|---|
| 23 November 2026 | 4.6b: B plays from disk | FLAC of 49.9 MB, index of downloaded tracks | 1.2 / 1.1% | 0.3 / 0.3% | **102 / 103 MB** | 45 | 272 / 271 |
| 23 November 2026 | 4.6b: B plays from the network | same FLAC, fake backend | 1.1 / 1.1% | 0.3 / 0.3% | 151 / 151 MB | 45 | 272 / 272 |
| 23 November 2026 | 4.6b: A (`main`) plays from the network | same FLAC, fake backend | 1.1 / 1.1% | 0.3 / 0.3% | 158 / 157 MB | 45 | 272 / 272 |
| 23 November 2026 | 4.6b: Home idle, A → B (`cce6a9b` → `3543bdb`) | public backend, no track | 1.1 / 1.2% → 1.1 / 1.2% | 0.4 / 0.5% → 0.5 / 0.5% | 176 / 148 → 164 / 164 MB | 44 → 44 | 342 / 343 → 344 / 343 |

- **From disk the audio needs no RAM.** B from disk sits at 102–103 MB, B from the network at 151 MB and A from the network at
  157–158 MB, in all four rounds of each case (two per series, same values to the MB): the difference from the
  disk is 48–49 MB, that is the weight of the file (49.9 MB). From the network the player keeps in memory everything it downloads;
  `StreamBuffer::open_file` reads from the file with positional reads and keeps nothing. A lossless FLAC weighs 30–50
  MB per track: it is the item that matters most for the RAM of whoever listens losslessly. The file is open by the process
  (`/proc/PID/fd` lists it), the backend receives neither `stream-url` nor `/v1/media` (0 and 0, against 1 and 1 from the network) and
  a skip at 120 s with MPRIS leaves the `Playing` state.
- **CPU, UI thread, threads and wake-ups: equal** between disk, network and `main` (1.1–1.2%, 0.3%, 45, 271–272), as
  expected: the decoding is in the same thread (`cpal_alsa_out`, 0.4%) and the reads from disk are positional and
  short.
- **Idle: A and B equal.** CPU 1.1–1.2% in all, wake-ups 342–344 (279–280 in the first series), RSS A 148–176 (first
  series 169–171) and B 151–165 MB: within the ±20 MB variation between two starts. Step 4.6 does not move the idle state: the
  download progress timer (250 ms) is armed only while downloading and at rest the core does nothing.
- **B from the network is 7 MB below A from the network** (151 against 157–158 MB, in all four rounds): I did not look for
  the cause and do not declare it an improvement (two rounds per series, and between two idle starts the RAM varies by ±20 MB).
- **Not measured**: a download in progress (CPU and RSS while it writes and computes the SHA-256: the computation is on the same
  pass as the writing, but I did not time it alone); the quick check at start-up with many files (one
  `stat` per track on a blocking thread) and the full one (it reads and hashes everything: it costs as much as reading the
  files); the "Downloaded" page with hundreds of entries (it shows 100 rows at a time, I did not try it with 500); the
  system's page cache (it does not count in the RSS but occupies system RAM); other formats (m4a, ogg), the phone,
  Android and Windows. The position that MPRIS reports does not advance (it stays at a few microseconds), so the
  progress of the track I verified with the screenshot and the file descriptor, not with `playerctl position`.

## GPUI desktop

Component gallery of `psytify-desktop` (pixel kit on pure GPUI, Zed rev `d31d107`), `--release` build
with the workspace profile, Wayland (niri, 1x, 1280x800), `scripts/perf-sample.py` for
30 s with the window still at the top of the gallery.

| State | CPU | RSS | Notes |
| --- | --- | --- | --- |
| Still, `PSYTIFY_REDUCE_MOTION=1` | 0.0% | 86 MB | no frame redrawn: the redraw starts only from input or a timer |
| Still, animations active | 2.2% | 198–203 MB | the visible animated sprites (Nastro, loader, eq) redraw at the times of the frames; RSS stable (HWM unchanged in 60 s) |

- The animations are driven by the drawing: each visible sprite requests its next frame with a single
  deduplicated `request_refresh(delay)`, no loop per frame. Outside the active window or with
  `PSYTIFY_REDUCE_MOTION` the cost goes back to 0.
- The cache of the upscaled sinks is limited (128 entries / 8 MiB); at scale 1.0 the gallery uses 106 of them for
  320 KiB, at 1.5 for 721 KiB. Changing the scale empties the cache and releases the textures with `drop_image`.
- The ~110 MB RSS jump between the two states is to be investigated: probably swapchain/atlas buffers
  allocated by the continuous redraws, not growth over time.
- The list of 10,000 tracks uses `uniform_list`: only the visible rows are drawn.

### Desktop with the real core (step 5): GPUI against Iced

Release binaries of the same commit (the Iced client is not touched by step 5), isolated `XDG_CONFIG_HOME`,
public backend, volume 0, 15 s of settling and 40 s of sample with `scripts/perf-sample.py`, two rounds
alternating for each case. The window is floating on HDMI-A-1, in the active workspace, with focus
(the check every 2 s says 20 samples out of 20 both focused and window on the screen). No input
injected: the search starts with `--search` (GPUI) or with the saved text (Iced), playback starts with
`playerctl play` (MPRIS), the queue panel opens with `--queue`. **The track** is an m4a from the catalogue (a
single track in the queue); in "queue open" the queue has 12 tracks and the search is `eminem`. The UI thread is
`psytify-iced-ma` for Iced and `psytify-desktop` for GPUI; for GPUI at rest it does not appear among the first (below
0.1%). Values "round 1 / round 2".

| Date | Scenario | Binary | Total CPU | UI thread | RSS | Threads | Wake-ups/s |
|---|---|---|---|---|---|---|---|
| 23 November 2026 | Idle, Home or empty Search | Iced | 1.1 / 1.1% | 0.5 / 0.6% | 164 / 169 MB | 43 | 285 / 288 |
| 23 November 2026 | Idle, Home or empty Search | GPUI | 0.5 / 0.5% | ≤ 0.1% | 146 / 145 MB | 37 | 219 / 219 |
| 23 November 2026 | Idle, search "daft punk" with results | Iced | 0.7 / 0.6% | 0.1 / 0.1% | 180 / 180 MB | 43 | 227 / 229 |
| 23 November 2026 | Idle, search "daft punk" with results | GPUI | 0.5 / 0.5% | ≤ 0.1% | 152 / 151 MB | 37 | 220 / 220 |
| 23 November 2026 | Playing (m4a, one track) | Iced | 1.8 / 2.0% | 0.9 / 1.1% | 199 / 190 MB | 44 | 343 / 348 |
| 23 November 2026 | Playing (m4a, one track) | GPUI | 2.3 / 2.3% | 1.4 / 1.5% | 161 / 158 MB | 38 | 303 / 302 |
| 23 November 2026 | Playing, 12-track queue open and search with 50 covers | GPUI | 6.1 / 6.1% | 5.0 / 5.0% | 176 / 172 MB | 38 | 446 / 461 |

- **At rest GPUI costs less: half the CPU, 18–28 MB less, 6 threads fewer.** The redraw starts only from input,
  from a message from the core that changes a domain, or from an animation timer, and at rest none of
  this exists (no timer: `PROGRESS_TICK_MS` is armed only while playing).
- **Playing, GPUI costs more in CPU (2.3% against 1.8–2.0%) and less in RAM (158–161 against 190–199 MB).**
  The UI thread is at 1.4–1.5% against 0.9–1.1%: for a redraw 4 times a second (the 250 ms timer of the bar) that is
  about 3 ms per frame, against 1.4 ms measured for Iced. **I did not profile it** (`perf` is not installed) and
  I do not know how much is drawing the elements and how much is handing the frame to wgpu.
- **With the queue open the cost rises to 6%.** The frames are 16 per second (counted with `WAYLAND_DEBUG`:
  `wl_surface.commit`): the 120 ms equaliser (8.3 per second), the 250 ms timer (4) and the caret of the
  field, at about 3 ms each: the same cost per frame, more frames. Every frame redoes the whole tree of
  the window (no view is in `.cached()`).
- **A defect found and fixed with these measurements.** The `request_refresh` timers were never cancelled:
  two sprites with different phases (the queue's equaliser and the field's caret) made the live timers multiply
  and the window redrew at about 60 per second. Now only one is valid (generation number, `RefreshClock` with a test).
  Same scenario, before and after, two alternating rounds each: 8.5 / 8.6%
  → 6.1 / 6.1% of total CPU, 513 / 546 → 446 / 461 wake-ups per second. The other cases do not change (idle 0.5%,
  playing 2.3–2.4% before and after). The tables above are of the corrected binary.
- **RAM varies between identical starts**: the same binary at rest gave 136 and 145 MB in two different series, so the
  10 MB between the binary before and after the correction is not an effect of the correction.
- **Not measured**: scale 1.5 (the screenshots at 1.5 were made in an Xvfb with Mesa lavapipe, which says nothing
  about real CPU and RAM); scrolling, mouse hover, dragging the seek and volume and typing in the field
  (they require input, which is not injected into the session of whoever is working); lists longer than 50 rows;
  `.cached()` on the views (not tried: the sprite animations are scheduled while painting, and a cached view
  does not paint again, so another route for the timer would be needed); GPU memory
  (`nvidia-smi` shows no compute use); a long listen (leaks); Windows, never run.

### Desktop step 6.0: where the milliseconds of a frame go (BACKLOG 23)

`perf` is not there, so the GPUI profile is read: `cargo build --release -p psytify --features profile` (`-p psytify-desktop` before step 7)
and `PSYTIFY_FRAME_STATS=10` make it print on stderr, every 10 s, the frames drawn with the average time of
`Window::draw` divided by phase (layout, prepaint, paint), the delivery to the GPU, the tasks above 100 µs and the views
above 500 µs (`desktop/src/frame_stats.rs`). Release binaries (workspace profile) of the same tree,
floating window on HDMI-A-1 with focus (20 checks out of 20), volume 0, 15 s of settling and 40 s of
sample, public backend. The `profile` feature costs some CPU per frame: **these numbers serve to
divide the cost, not to compare with the table of step 5**, which is of binaries without the profile.

| State | Binary | Frames/s | Average drawing | Layout / prepaint / paint | UI thread |
|---|---|---|---|---|---|
| Playing (m4a, one track) | before (no view cached) | 10.3 | 0.97–1.02 ms | 0.26 / 0.33 / 0.34 | 1.5% |
| Playing (m4a, one track) | after | 10.3 | 0.91–0.95 ms | 0.25 / 0.30 / 0.31 | 1.4% |
| 12-track queue open, search `eminem` | before | 16.4 | 2.38–2.47 ms | 0.37 / 1.24 / 0.64 | 4.7% |
| 12-track queue open, search `eminem` | after | 16.5 | 1.46–1.49 ms | 0.26 / 0.61 / 0.47 | 3.2% |

- **The cost of a frame is neither the delivery to the GPU nor a task.** The profile reports no delivery time
  on the window's thread and no task above 100 µs apart from the profile's own reader: the work is
  all in `Window::draw`, that is in what the views build at every frame.
- **How much a list weighs.** Redoing a view with a dozen rows of 48 dp (the Search page
  or the queue panel) costs 0.55–0.6 ms of prepaint, about 45 µs per row. The rest of a frame
  (title bar, sidebar, "now playing" bar, sprites) is ~0.9 ms, and it stays below the 500 µs threshold
  of the profile, so it cannot yet be divided per view. In "playing" without lists the frame costs
  that and no more; with the queue and the search open the two lists bring it to 2.4 ms: **the "about 3 ms" of
  step 5 was this, a dozen rows rewritten twice per frame.**
- **Why everything was redone.** `request_refresh` called `Window::refresh`, which redraws the whole window and,
  in GPUI's prepaint, **turns off the reuse of every cached view for that frame**: no view could
  be in `.cached()`. The frames are few by construction of the tokens (equaliser at 120 ms = 8.3/s,
  mark at 160 ms = 6.25/s, beat at 250 ms = 4/s; 16.5 per second with the three together) and they are not touched.
- **The fix.** `pixel/anim.rs` remembers, for each view that has painted an animated sprite, when it needs
  the next frame (`Window::current_view`), keeps a single timer aimed at the nearest deadline and, when it fires,
  invalidates with `App::notify` only the views whose deadline has come (not all those that asked for a redraw: the first
  version redrew them all at each tick, 12.5 redraws per second of the queue against
  the equaliser's 8.3, and gave 1.65 ms instead of 1.47). The Search page and the queue panel are in
  `.cached()`: a cached view is drawn again only when it notifies itself (it already did so for
  every change that is seen), when its size changes or when the window is redrawn as a whole
  (`Window::refresh`: resizing, arrival of a cover, mouse over the edges). With
  the queue open the Search page no longer redoes at every frame (from 30–48 redraws every 10 s to 1–2) and
  the panel only when its equaliser changes (8.3 per second).
Same binaries **without** the `profile` feature (the ones that are distributed), `scripts/perf-sample.py` for 40 s after
15 s of settling, two alternating rounds per scenario (values "round 1 / round 2"). The "before" binary is the commit
`935db97` (end of step 4.9), the "after" is `mig/gpui-base` at step 6.0.

| Date | Scenario | Binary | Total CPU | UI thread | RSS | Threads | Wake-ups/s |
|---|---|---|---|---|---|---|---|
| 24 November 2026 | Idle, empty Search | before | 0.5 / 0.5% | ≤ 0.1% | 146 / 145 MB | 37 | 219 / 219 |
| 24 November 2026 | Idle, empty Search | after | 0.5 / 0.5% | ≤ 0.1% | 146 / 146 MB | 37 | 219 / 219 |
| 24 November 2026 | Idle, search "daft punk" with results | before | 0.5 / 0.5% | ≤ 0.1% | 162 / 152 MB | 37 | 223 / 219 |
| 24 November 2026 | Idle, search "daft punk" with results | after | 0.5 / 0.5% | ≤ 0.1% | 159 / 153 MB | 37 | 220 / 220 |
| 24 November 2026 | Playing (m4a, one track) | before | 2.1 / 2.1% | 1.4 / 1.5% | 160 / 159 MB | 38 | 342 / 313 |
| 24 November 2026 | Playing (m4a, one track) | after | 2.0 / 2.0% | 1.4 / 1.4% | 161 / 158 MB | 38 | 336 / 325 |
| 24 November 2026 | Playing, 12-track queue open and search `eminem` | before | 5.9 / 5.8% | 5.1 / 5.1% | 176 / 185 MB | 38 | 484 / 468 |
| 24 November 2026 | Playing, 12-track queue open and search `eminem` | after | 3.9 / 3.9% | 3.1 / 3.2% | 174 / 170 MB | 38 | 462 / 453 |

- **The gain is where there are lists that stand still while something else animates**: with the queue and the search open the
  process goes from 5.8–5.9% to 3.9% (−33%) and the UI thread from 5.1% to 3.1–3.2% (−38%); the wake-ups fall by
  4%, the RAM stays within the variation between starts. Playing without lists the gain is within the noise (2.1% to
  2.0%), at rest nothing changes (0.5%, no extra timer). Iced playing one track is at
  1.8–2.0% (table of step 5) and GPUI at 2.0–2.1%; **the scenario with the queue open was not measured on
  Iced**, so it cannot be said from here whether GPUI has reached it. The 16.5 frames per second with the queue
  open are the times of the design's sprites (not a choice of the renderer) and they have not changed.

- **Not done.** The "now playing" bar and the sidebar were not put in cache (the estimated gain is 0.2 ms per frame,
  below the profile's measurement threshold), and the allocations per row (`format!("{:?}")`, `use_keyed_state`)
  were not touched: neither was measured on its own.

### Desktop step 6.1: Library, Downloaded, liked tracks

Release binaries, `scripts/perf-sample.py` for 40 s after 15 s of settling, window visible and focused,
two alternating rounds (values "round 1 / round 2"). The "before" is `main` at `87ee3af` (end of step 6.0, Search
page), the "after" is `mig/gpui-library`. "Fake data" is a test configuration (no real data): 300
liked tracks, 3 playlists (13, 8 and 0 tracks), 2 saved albums, 3 followed artists, 6 tracks on the device
(one missing); when playing, the queue has a real catalogue track (m4a) at the head of the liked ones, volume 0.

| Date | Scenario | Binary | Total CPU | UI thread | RSS | Threads | Wake-ups/s |
|---|---|---|---|---|---|---|---|
| 24 November 2026 | Idle, empty Search | before | 0.5 / 0.5% | ≤ 0.1% | 151 / 152 MB | 37 | 219 / 219 |
| 24 November 2026 | Idle, empty Search | after | 0.5 / 0.4% | ≤ 0.1% | 146 / 147 MB | 37 | 219 / 219 |
| 24 November 2026 | Playing (m4a, one track), Search | before | 2.0 / 2.1% | 1.4 / 1.4% | 162 / 162 MB | 38 | 317 / 329 |
| 24 November 2026 | Playing (m4a, one track), Search | after | 2.0 / 2.0% | 1.4 / 1.4% | 161 / 161 MB | 38 | 318 / 319 |
| 24 November 2026 | Idle, fake data, Search | before | 0.5 / 0.5% | ≤ 0.1% | 154 / 155 MB | 37 | 219 / 219 |
| 24 November 2026 | Idle, fake data, Library | after | 0.5 / 0.5% | ≤ 0.1% | 154 / 153 MB | 37 | 219 / 220 |
| 24 November 2026 | Idle, fake data, Downloaded | after | 0.5 / 0.5% | ≤ 0.1% | 152 / 152 MB | 37 | 220 / 219 |
| 24 November 2026 | Playing, fake data, "Brani che ti piacciono" (301 tracks, the one playing visible) | after | 4.0 / 4.0% | 3.3 / 3.3% | 166 / 165 MB | 38 | 375 / 387 |
| 24 November 2026 | Playing, fake data, Library | after | 2.1 / 2.0% | 1.5 / 1.4% | 162 / 162 MB | 38 | 324 / 318 |
| 24 November 2026 | Playing, fake data, Search | after | 2.0 / 2.1% | 1.4 / 1.4% | 164 / 163 MB | 38 | 325 / 317 |

- **At rest nothing changes**: 0.4–0.5% on every page, with or without the fake data, same wake-ups (219/s)
  and no new timer. The new pages do not work until the core publishes something they show. The RAM
  of the fake data (300 tracks, lyrics of the rows ready) is within the variation between starts (152–155 MB against
  146–152).
- **Playing with the liked list open the process goes to 4.0%** (UI thread 3.3%), against 2.0%
  for the same playback on Library or Search. The cause is the equaliser of the current row: each of its
  frames (120 ms, 8.3 per second) invalidates the view that paints it, and GPUI also marks the ancestors dirty,
  so the page in `.cached()` redoes the visible rows (about 0.55 ms each time, the measure
  of step 6.0). With the current row out of the list (Library) it goes back to 2.0–2.1%. It is the same cost as
  the open queue of step 6.0 (3.9%) and the same remedy still to be done (BACKLOG 23): make the current
  row, or the rows, separate cached views, or an equaliser that does not ask to redo the list. Not
  measured with the current track off screen (the list that scrolls): no input is injected
  into the session of whoever uses the computer.

### Desktop step 6.2: album and artist pages, the equaliser out of the cache (BACKLOG 23)

Release binaries, `scripts/perf-sample.py` for 40 s after 15 s of settling, window visible and focused
on the second screen, two alternating rounds (values "round 1 / round 2"). "Before" is `7dc3050` (album and
artist pages already done, equaliser inside the row), "after" is the final version of the step: the equaliser of the
current track is painted by `Shell` above the page (`components/playing_spot.rs`) and the "now
playing" bar is in `.cached()`. "Only the spot" is the intermediate version without the bar in cache. The data are
those of the fake data of step 6.1 plus a real album of the catalogue (18 tracks, FLAC from Internet Archive, read only)
at the head of the saved albums and its artist at the head of the followed ones; "album queue" means that the queue is
the 18 tracks of the album and the first one plays, volume 0.

| Date | Scenario | Binary | Total CPU | UI thread | RSS | Threads | Wake-ups/s |
|---|---|---|---|---|---|---|---|
| 24 November 2026 | Playing, "Brani che ti piacciono" (301 tracks, the one playing visible) | before | 3.8 / 3.9% | 3.2 / 3.2% | 164 / 167 MB | 38 | 375 / 371 |
| 24 November 2026 | Playing, "Brani che ti piacciono" | only the spot | 3.0 / 3.0% | 2.4 / 2.4% | 163 / 162 MB | 39 | 416 / 384 |
| 24 November 2026 | Playing, "Brani che ti piacciono" | after | 2.7 / 2.6% | 2.0 / 2.0% | 168 / 166 MB | 38 | 389 / 382 |
| 24 November 2026 | Playing (album queue), album page | before | 3.8 / 3.5% | 3.0 / 2.9% | 196 / 196 MB | 38 | 626 / 452 |
| 24 November 2026 | Playing (album queue), album page | only the spot | 3.0 / 3.0% | 2.3 / 2.3% | 197 / 196 MB | 38 | 476 / 518 |
| 24 November 2026 | Playing (album queue), album page | after | 2.6 / 2.6% | 1.9 / 1.9% | 199 / 197 MB | 38 | 487 / 465 |
| 24 November 2026 | Playing (album queue), album page and queue open | before | 4.7 / 4.5% | 3.8 / 3.8% | 198 / 199 MB | 38 | 611 / 460 |
| 24 November 2026 | Playing (album queue), album page and queue open | only the spot | 3.3 / 3.3% | 2.6 / 2.6% | 199 / 198 MB | 38 | 560 / 496 |
| 24 November 2026 | Playing (album queue), album page and queue open | after | 2.8 / 2.8% | 2.2 / 2.2% | 181 / 199 MB | 38 | 489 / 469 |
| 24 November 2026 | Playing (m4a, one track), Search | before | 2.1% (one valid round) | 1.4% | 162 MB | 38 | 315 |
| 24 November 2026 | Playing (m4a, one track), Search | after | 1.8 / 1.9% | 1.2 / 1.2% | 155 / 161 MB | 38 | 350 / 319 |
| 24 November 2026 | Playing (album queue), artist page | after | 2.0 / 2.0% | 1.3 / 1.3% | 209 / 222 MB | 38 | 412 / 414 |
| 24 November 2026 | Idle, artist page | after | 0.5 / 0.5% | ≤ 0.1% | 167 / 159 MB | 37 | 219 / 220 |
| 24 November 2026 | Idle, album page | before | 0.6 / 0.6% | ≤ 0.1% | 155 / 155 MB | 37 | 277 / 272 |
| 24 November 2026 | Idle, album page | after | 0.6 / 0.6% | ≤ 0.1% | 155 / 156 MB | 37 | 272 / 283 |

- **The cause (BACKLOG 23, left from step 6.1).** The equaliser sprite changes frame every
  120 ms and `pixel/anim.rs` invalidates the view that painted it; GPUI also marks the ancestors dirty
  (`Window::mark_view_dirty`), so a page in `.cached()` with the current row visible redid the whole
  list 8.3 times per second. Making the row a separate view is not enough, for the same reason.
- **The remedy.** The row no longer paints the equaliser: in its place there is a placeholder of the same size
  that, when the page paints, notes in a shared cell (`PlayingSpot`) where it is and under which mask
  (the scrolling list cuts it). `Shell`, which is not cached, paints the sprite at that point above the
  page, after it in the same frame; the sprite's timer thus invalidates only `Shell` and the page
  is reused from the cache (`AnyView::cached`: prepaint and paint reused until the view is dirty, the
  size does not change and the window is not refreshing, `view.rs` of the pinned Zed revision). The page
  clears the spot at the start of `render`: if the row is no longer among those painted (scrolled off, track
  changed), the equaliser disappears with it. It applies to the pages with track rows (Search, Downloaded,
  liked and playlist tracks, album, artist) and to the queue. Then, since now it is `Shell` that redraws at every
  tick, the "now playing" bar is also in `.cached()` (it notifies itself with its 250 ms beat).
- **The result.** Playing with the current row visible it drops by 1.1–1.9 points: liked tracks from
  3.8–3.9% to 2.6–2.7%, album from 3.5–3.8% to 2.6%, album with the queue from 4.5–4.7% to 2.8% (UI thread from 3.8% to
  2.2%). The cached bar alone is worth 0.3–0.5 points and also lowers playback without lists (Search: from
  2.1% to 1.8–1.9%). It stays about 0.7–0.8 points above Search: it is the redraw of `Shell` at every tick (title bar,
  sidebar, layers) and the reuse of the page, which costs little but not zero.
- **The idle album wakes the process 53 times a second more** (272 against 219), before and after in the
  same way, and it is the core: opening an album warms its first track (`PlaybackCmd::Warm`, as in the Iced
  client), the header arrives and the download stays waiting without reading (1.4–1.8 MB standing in the socket queue),
  checking every 25 ms whether someone is listening (`HOLD_POLL` in `core/src/api/client.rs`), until the track
  plays or the cache of warm tracks lets it go. It costs less than 0.1 points; BACKLOG 33.
- **The new pages at rest** are at 0.5% (artist) and 0.6% (album, for the warm track), with no new
  timers. The RAM of the artist page (up to 222 MB while playing) is the discography: the covers of the
  visible releases, in the cache of 128 textures.
- **Not measured.** The scrolling with the current row entering and leaving the list and the mouse hover
  over the rows (no input is injected into the session of whoever uses the computer); they are tried only
  in the Xvfb of the screenshots.

### Desktop step 6.3: Home, New, the track buffer without polling (BACKLOG 33)

Release binaries, `scripts/perf-sample.py` for 40 s after 15 s of settling, window visible and focused
on the second screen, two alternating rounds (values "round 1 / round 2"). "Step" is the final version of the step
(`e9c9d47`); "without remedy" is the same Home with the previous core (`56f9ec9`: polling wait in the buffer);
"main" is `053122e`, which has no Home and opens on Search. The `main` binary was compiled in the same
`target` folder as the step: its pages are those of `main`, the core may already be the new one (none
of the compared scenarios warms a track, so nothing changes). Fake data from `lib63/seed.py`: those of
step 6.2 plus 14 real recent listens from the catalogue (read only), 10 chart entries, 12.4 hours
listened to, Linkin Park followed and "New" seen up to 28 October at 21:00 (one release to listen to). When playing: the m4a track as always, volume 0, started from the system widget.

| Date | Scenario | Binary | Total CPU | UI thread | RSS | Threads | Wake-ups/s |
|---|---|---|---|---|---|---|---|
| 24 November 2026 | Idle, Home | step | 0.3 / 0.3% | ≤ 0.1% | 167 / 158 MB | 37-38 | 224 / 226 |
| 24 November 2026 | Idle, Home | without remedy | 0.3 / 0.3% | ≤ 0.1% | 162 / 159 MB | 37 | 225 / 225 |
| 24 November 2026 | Idle, start (Search) | main | 2.3 / 0.3% | 2.0 / ≤ 0.1% | 153 / 150 MB | 37-38 | 356 / 226 |
| 24 November 2026 | Idle, Library | main | 0.3 / 0.3% | ≤ 0.1% | 154 / 155 MB | 37 | 224 / 224 |
| 24 November 2026 | Idle, Library | step | 0.3 / 0.3% | ≤ 0.1% | 154 / 154 MB | 37 | 224 / 224 |
| 24 November 2026 | Idle, album page (first track warmed) | without remedy | 0.4 / 0.4% | ≤ 0.1% | 156 / 156 MB | 38 / 37 | 281 / 280 |
| 24 November 2026 | Idle, album page (first track warmed) | step | 0.6 / 0.3% | 0.2 / ≤ 0.1% | 155 / 155 MB | 37 | 238 / 228 |
| 24 November 2026 | Playing (m4a, one track), Home | step | 2.8 / 2.8% | 2.1 / 2.1% | 167 / 167 MB | 38 | 377 / 377 |
| 24 November 2026 | Playing (m4a, one track), Search | main | 1.9 / 1.9% | 1.2 / 1.2% | 159 / 160 MB | 38 | 312 / 321 |
| 24 November 2026 | Playing (m4a, one track), Search | step | 2.0 / 1.9% | 1.2 / 1.3% | 160 / 159 MB | 38 | 322 / 332 |

- **The idle Home costs as much as the other pages**: 0.3% and 224–226 wake-ups per second, like the Library of
  `main` and of the step. The shelves have no timers: the arrows and the wheel move the index and redo the view
once; the page is in `.cached(fill())` and is redone only when the core publishes a change that touches it.
  The first round of `main` idle on Search (2.3%, 125 wake-ups per second of the UI thread) did not repeat in the
  second (0.3%) and has no known cause; the focus stayed on the window in 18 samples out of 20.
- **Playing on the Home** it is at 2.8%, 0.8–0.9 points above Search: the current track is in *Resume*, at the top of the
  chart and in *Recently listened*, so the equaliser is on the page and `Shell` redraws at each of its ticks
  (8.3 per second), as on the album page of step 6.2 (2.6%, BACKLOG 23). The cached page
  is not redone: the equaliser spots are up to 8 per view (`PlayingSpot`).
- **BACKLOG 33: the idle album no longer wakes the process 53 times a second.** With the previous core the
  download of the first track, warmed and standing on the header, checked every 25 ms whether someone was listening: 54
  wake-ups per second of the `psytify-core-io` thread, 280–281 in total. Now it waits for a `Notify` that every change
  of state wakes (bytes, `release`, a reader beyond the header, cancellation, failure): `psytify-core-io` goes to
  0 and the process to 228, as much as a page without a warm track (224–226). The first "step" round (0.6%, 238)
  lost focus in 15 samples out of 20 (the UI thread redrew the window); the second, focused, is
  the clean one. The same wait applies to `StreamBuffer::ready` (it was a 20 ms poll), used
  at the start of each track.
- **RAM.** The idle Home sits at 158–167 MB, 4–13 MB above the Library: the covers of the visible shelves
  (at most 128 textures, the cache of `covers.rs`).
- **Not measured.** Scrolling the Home, the shelf arrows and "Show all" (no input is injected
  into the session of whoever uses the computer): tried only in the Xvfb of the screenshots.

### Desktop step 6.4: synchronised lyrics and the immersive view

Release binaries, `scripts/perf-sample.py` for 40 s after 15 s of settling, window visible on the second
screen (1280 x 800), two alternating rounds plus a third as a check (values "round 1 / round 2 / round 3"). "Step" is
`c612e75`, "main" is `e345031`. Minimal configuration with only one real track with synchronised lyrics in the queue
(Eminem, "Say Goodbye Hollywood", m4a, volume 0, started from the system widget), on the Library; the lyrics come
from LRCLIB through the server (read only). In the second round the window lost focus for most of the
sample (in brackets the samples with focus out of 20): the third round, all focused, repeats the main scenarios.

| Date | Scenario | Binary | Total CPU | UI thread | RSS | Wake-ups/s |
|---|---|---|---|---|---|---|
| 24 November 2026 | Playing, panel closed | main | 2.3 (15) / 1.2 (0) / 1.7% | 1.6 / 0.6 / 1.1% | 158 / 159 / 159 MB | 372 / 282 / 317 |
| 24 November 2026 | Playing, panel closed | step | 1.8 / 1.6 (7) / 1.8% | 1.2 / 1.0 / 1.2% | 160 / 159 / 162 MB | 315 / 312 / 314 |
| 24 November 2026 | Playing, lyrics panel (synchronised) | step | 2.4 / 2.2 (13) / 2.5% | 1.7 / 1.6 / 1.8% | 160 / 163 / 162 MB | 351 / 329 / 354 |
| 24 November 2026 | Playing, immersive view | step | 2.3 / 2.4% | 1.7 / 1.7% | 160 / 161 MB | 356 / 349 |
| 24 November 2026 | Idle, panel closed | main | 0.3 / 0.6% (4) | ≤ 0.1 / 0.3% | 153 / 151 MB | 229 / 255 |
| 24 November 2026 | Idle, lyrics panel open | step | 0.3 / 0.4 (2) / 0.3% | ≤ 0.1% | 154 / 152 / 153 MB | 223 / 232 / 223 |

- **Idle with the lyrics open costs nothing**: 0.3%, 223 wake-ups per second and the UI thread out of the list
  (no timer), like `main` idle. The lyrics are loaded and shown; without playback the view has no deadlines.
- **Playing, the panel costs 0.6–0.7 points** above the closed panel (1.8% → 2.4–2.5%, UI thread
  1.2% → 1.7–1.8%, +12 wake-ups per second of the UI thread and +8 of GPUI's `Timer`). It is not the timer of the line
  (it wakes once per line): it is the scrolling in `MOTION_LYRICS`, 7 frames of 42 ms at each change of
  line, in which the lyrics view is redone (a few whole lines, heights already in the cache) and `Shell`, while the page
  stays cached. With `reduce_motion` the scrolling is a cut and only one redraw per line remains (not measured).
- **The immersive view costs as much as the panel** (2.3–2.4%): the text is the same engine, the bar has no
  timer while it is covered, and the view's 250 ms beat takes its place.
- **Closed panel: the step as `main`** (1.6–1.8% against 1.2–2.3%, where the low and high values of `main`
  come from the rounds with focus lost; in the third round, focused, 1.8 against 1.7%). With the closed panel the
  core does not read the lyrics (`PlaybackCmd::Lyrics(false)`).
- **RAM**: 152–163 MB in all the scenarios, within the ±20 MB noise between two starts; the large cover of the immersive
  view (at most 2, 256 px) is 256 KB each.
- **Not measured**: plain text (no timer, only the manual scrolling), the cropping of the black bands of the
  covers (one pass over the border rows and columns, on the blocking pool, at most a few milliseconds
  for a 1000 px JPEG, once per cover).

### Desktop step 6.5: Settings

Release binaries, `scripts/perf-sample.py` for 40 s after 15 s of settling, window visible on the second
screen (1280 x 800; visible in 20 samples out of 20 in each round, the focus almost always elsewhere), values "round 1 /
round 2". "Step" is `6ce8540`, "main" is `d3872f0` (which has no Settings: the comparison is on the Library).
Minimal configuration with a real track in the queue (Eminem, m4a, volume 0, started from the system widget). The second
round stopped after the three idle scenarios: the computer's memory dropped to 1 GB (another application) and
the round was interrupted, so the playing scenarios have one round only. In this session the PipeWire thread
`data-loop.0` costs 0.4–0.6% and ~189 wake-ups per second in all the binaries, `main` included: the floor is higher
than that of step 6.4 (0.3%) because of the state of the system's audio, not of the app.

| Date | Scenario | Binary | Total CPU | UI thread | RSS | Wake-ups/s |
|---|---|---|---|---|---|---|
| 24 November 2026 | Idle, Library | main | 0.7 / 0.9% | out of list / out of list | 150 / 126 MB | 323 / 319 |
| 24 November 2026 | Idle, Library | step | 0.6 / 1.0% | out of list / 0.2% | 152 / 107 MB | 325 / 348 |
| 24 November 2026 | Idle, Settings (Playback) | step | 0.6 / 0.7% | out of list / out of list | 154 / 153 MB | 324 / 324 |
| 24 November 2026 | Playing, Library | main | 2.5% | 1.4% | 159 MB | 436 |
| 24 November 2026 | Playing, Library | step | 2.0% | 0.8% | 160 MB | 389 |
| 24 November 2026 | Playing, Settings | step | 1.9% | 0.7% | 161 MB | 390 |

- **Idle on Settings costs nothing**: 0.6–0.7%, the UI thread out of the list (0 wake-ups per
  second), like `main` idle on the Library (0.7–0.9%). The page has no timer: it redraws only when a
  value it shows changes.
- **Playing, Settings cost as much as the Library** (1.9 against 2.0%, UI thread 0.7 against 0.8%, 16
  wake-ups per second of the UI thread in both: the bar's beat). The playback publications do not redraw the
  page: the mirror of the values (`Mirror`) is the same and the handler exits at once. `main` playing
  2.5% with focus held in 19 samples out of 20 (with focus GPUI draws more); one round only.
- **RAM**: 150–161 MB, within the ±20 MB noise between two starts (the 107 and 126 MB of the second round come from
  the system memory under pressure, which pushed pages into swap).
- **Not measured**: dragging an equaliser band (an `EqualizerBand` per 0.5 dB step, a publication and a page
  redraw per step, the save only on release) and reduced motion while playing with the lyrics open (the scrolling becomes
  a cut: one redraw per line instead of 7).

### Desktop step 6.5b: "Reduce motion" follows the system

Release binaries, `scripts/perf-sample.py` for 40 s after 15 s of settling, on the Settings (Playback tab),
no track, two alternating rounds ("round 1 / round 2"). "main" is `8dae4a3`, "branch" is `7cdcc00`. The
computer had only the laptop screen, where whoever uses it works, so the window was in an Xvfb of mine (1280 x 800,
lavapipe): the absolute values are not comparable with the rows above (lavapipe brings the threads to 105), only with
each other. The D-Bus buses are the real ones, read only: the system was in energy saving (on battery), so
"Follow the system" ran with reduced motion.

| Date | Scenario | Binary | Total CPU | RSS | Wake-ups/s | Threads | New zbus threads |
|---|---|---|---|---|---|---|---|
| 25 November 2026 | Idle, Settings | main | 0.9 / 1.1% | 144 / 221 MB | 293 / 291 | 105 | - |
| 25 November 2026 | Idle, Settings, "Follow the system" | branch | 1.1 / 1.2% | 219 / 216 MB | 294 / 294 | 107 | 2, at 0.00 wake-ups/s |
| 25 November 2026 | Idle, Settings, "Full" | branch | 1.0 / 1.1% | 216 / 216 MB | 296 / 294 | 105 | none |

- **Listening to the system wakes nothing**: with "Follow the system" there are two threads more, the executors of the
  two zbus connections (session bus for the portal, system bus for power-profiles-daemon), and in the 40 s
  they had no context switch (0.00 per second in both rounds). The total wake-ups stay 291–296
  in all three binaries, within the noise. The stream only waits for signals: no timer.
- **"Full" and "Reduced" open nothing** (`WatchSystemMotion(false)`): same threads as `main`.
- **CPU** 0.9–1.2% in all, within the noise (the floor of this Xvfb with lavapipe); **RSS** 144–221 MB, within the noise
  of ±20 MB between two starts plus the difference of the first round of `main` (the first start after compilation).
- **Not measured**: the live switch (a signal, a publication of the settings, a redraw of the
  windows only if the motion in force changes: once per system change) and the real window on a
  screen (no free screen in this session).

### Desktop step 6.6: sign-in, Friends and profiles

Release binaries without the `fixtures` feature, `scripts/perf-sample.py` for 40 s after 15 s of settling, without
sign-in (configuration and data in a disposable folder), no track, two alternating rounds ("round 1 / round 2").
"main" is `83dddd7`, "branch" is `23703e6`. As in step 6.5b the window was in an Xvfb of mine (1280 x 800, lavapipe),
so the values can be compared only with each other. `main` has no Friends page: the like-for-like comparison is on
Settings, and Friends is measured on the branch.

| Date | Scenario | Binary | Total CPU | RSS | Wake-ups/s | Threads |
|---|---|---|---|---|---|---|
| 25 November 2026 | Idle, Settings, no sign-in | main | 0.5 / 0.6% | 178 / 149 MB | 291 / 300 | 107 |
| 25 November 2026 | Idle, Settings, no sign-in | branch | 0.5 / 0.5% | 160 / 238 MB | 293 / 295 | 107 |
| 25 November 2026 | Idle, Friends, no sign-in (Nastro and "Sign in") | branch | 0.5 / 0.5% | 235 / 237 MB | 292 / 293 | 107 |

- **Idle Friends costs nothing**: CPU and wake-ups equal to Settings and to `main`, in the floor of this
  Xvfb. The page has no timer: it redraws only when the social view changes, and without sign-in it does not change.
  `RefreshFriends` on opening, without a session, the core ignores it.
- **No new thread** (107 in all): the social pages use only the social domain the core already had.
- **RSS** 149–238 MB: the same branch binary on Settings goes from 160 to 238 MB between two starts, so the
  difference between Friends and Settings is within the noise. The noise is wider than the usual ±20 MB: the computer was in use,
  with less than 5 GB free for part of the measurement.
- **Not measured**: with a real sign-in (social poll every 15 s, avatars from the network) and the real window on
  a screen. The screens with friends and profiles come only from the fake views of `--fixture`.

### Desktop step 6.7: Jam
Release binaries without the `fixtures` feature, `scripts/perf-sample.py` for 40 s after 15 s of settling, without
sign-in and outside a jam (configuration and data in a disposable folder), no track, two alternating rounds
("round 1 / round 2"). "main" is `033f5a2`, "branch" is `mig/gpui-jam` (`3ac76cc`, compiled while the touch-up of the
rows in a room was coming in, which does not run outside a jam). The window was in an Xvfb of mine
(1280 x 800, lavapipe) and, unlike 6.6, without the session bus of whoever works (no MPRIS in their
session), so the values can be compared only with each other. `main` has no Jam: the like-for-like comparison is on
Settings, and the Jam is measured on the branch.

| Date | Scenario | Binary | Total CPU | RSS | Wake-ups/s | Threads |
|---|---|---|---|---|---|---|
| 25 November 2026 | Idle, Settings, no sign-in | main | 0.5 / 0.5% | 239 / 237 MB | 293 / 291 | 107 |
| 25 November 2026 | Idle, Settings, no sign-in | branch | 0.4 / 0.5% | 239 / 238 MB | 293 / 291 | 107 |
| 25 November 2026 | Idle, Jam, no sign-in and no jam ("Listen together") | branch | 0.5 / 0.5% | 239 / 236 MB | 295 / 291 | 107 |

- **The idle Jam costs nothing**: CPU, RSS and wake-ups equal to Settings and to `main`. The page has no timer
  and redraws only when the jam or social views change; outside a room the friends' presence does not
  enter the model, so the social poll (every 15 s, only with a sign-in) does not redraw it. The sign in the
  "now playing" bar exists only in a jam and changes only with the phase, the code or the number of people.
- **No new thread** (107 in all): the jam uses the domain and the link the core already had (4.8).
- **Not measured**: in a real jam (the WebSocket link, the alignment of the queue, the 5 s beat while
  playing) and with a real sign-in: no room was opened (BACKLOG 37). The screens in a room come only from the
  fake views of `--fixture jam-*`.

### Desktop step 6.8: update banner and download

Release binaries without the `fixtures` feature, `scripts/perf-sample.py` for 40 s after 15 s of settling, idle
on the Home, without sign-in and without offered updates (the banner hidden: a local copy has no `BUILD_TAG` and does not
check), configuration and data in a disposable folder, no track, two alternating rounds ("round 1 / round 2").
"main" is `ccb0f0b`, "branch" is `mig/gpui-updates` (`85dc142`). My Xvfb (1280 x 800, lavapipe), without the
session bus of whoever works: the values can be compared only with each other.

| Date | Scenario | Binary | Total CPU | RSS | Wake-ups/s | Threads |
|---|---|---|---|---|---|---|
| 25 November 2026 | Idle, Home, no sign-in, banner hidden | main | 0.5 / 0.5% | 251 / 258 MB | 292 / 285 | 107 |
| 25 November 2026 | Idle, Home, no sign-in, banner hidden | branch | 0.5 / 0.5% | 282 / 258 MB | 287 / 288 | 107 |

- **The hidden banner costs nothing**: CPU, wake-ups and threads equal to `main`. It has no timer; a publication of
  the downloads costs a comparison of `update_line`, and `Shell` redraws only when the banner appears or disappears.
- **RSS**: 282 MB in the first round of the branch against 251–258 in the other three; the second round is equal (258 / 258). The Home
  loads the covers from the network and the RAM varies by ±20 MB between two identical starts: it is not a cost of the banner, which
  when hidden allocates nothing.
- **Not measured**: the banner shown while the package downloads (the loader redraws only the banner, cached at its
  height) and a real download of an update (never downloaded: BACKLOG 38, 39).

### Desktop step 6.9: video clip, the two gates against Iced

Release binaries (`psytify-desktop --features video` at `751d217`, the Iced client with its default features, which
include video, from the same tree), `scripts/perf-sample.py` for 45 s after 30 s of settling (10 s of start-up, Play
via MPRIS on a private session bus, 20 s for the pipeline to open), two alternating rounds ("round 1 / round 2").
Window 1280 x 800 in my Xvfb, **focused** with `xdotool windowfocus` (without a window manager no window
is active and GPUI draws an inactive window every 33 ms at most, 20.8 fps with the 16 ms step of X11: BACKLOG 44).
Vulkan on the real Radeon 780M (`radeon_icd`) with `MESA_VK_WSI_DEBUG=sw`: Xvfb has no DRI3, so Mesa presents each
frame by copying it with the CPU (the `WSI swapchain q` thread), a copy that a real display does not have. With lavapipe
the numbers are of no use: the rasteriser runs on the CPU (proof: GPUI 36%, Iced 160% with 720p30 in the mini player).
Decoder: `vah264dec` (VA-API, rank primary+1) for both; the NVIDIA decoders at rank zero for both.

Local fake backend (`stream-url` and `video-url` served from local files, every write rejected, the other GETs read only
towards the public server). Queue of three catalogue tracks with the video clip ready, read once only from the
server with the normal path (`GET /v1/tracks/{id}/video-url`): `17e84543…` (Eminem, "We Made You", 1280 x 720,
23.976 fps, H.264 High, ~290 kbps), `f7b47240…` (726 x 720, 25 fps) and `0c52a878…` (1280 x 720, 25 fps). The first is
the one measured. Clips:

- **R720**: the real video clip; **R360**: the same reduced here to 640 x 360 with ffmpeg (the server does not keep the 360p
  of a video clip already complete).
- **S720 / S360**: 1280 x 720 and 640 x 360 at **30 fps** synthetic (`testsrc2` with noise in every frame, H.264 Main
  at 2 Mb/s and 0.7 Mb/s): the worst case, because the real video clips in the catalogue are at 24–25 fps and compress a lot.
- The audio is pink noise, AAC at 128 kb/s, the length of the track (volume at zero), the same for both clients.

"Audio only" scenario: GPUI with the panel on the queue, Iced with the "Now playing" panel on the cover. Video:
the mini player of the panel (GPUI: 288 x 162 dp, Iced: the width of its panel, 16:9). Frames shown from
`PSYTIFY_VIDEO_STATS` (GPUI only).

| Date | Scenario | Binary | Total CPU | Δ over audio only | UI thread | Presentation (Xvfb) | Decoding | Frames shown |
|---|---|---|---|---|---|---|---|---|
| 26 November 2026 | Audio only | GPUI | 4.8 / 4.4% | — | 2.8 / 2.5% | 1.1 / 1.1% | — | — |
| 26 November 2026 | Audio only | Iced | 4.7 / 4.7% | — | 2.4 / 2.3% | 1.4 / 1.5% | — | — |
| 26 November 2026 | Mini player, R360 (real, 24 fps) | GPUI | 10.4 / 10.7% | **+5.6 / +6.3** | 5.8 / 5.6% | 2.4 / 2.9% | 1.1 / 1.1% | 23.8/s of 23.8 |
| 26 November 2026 | Mini player, R360 (real, 24 fps) | Iced | 8.5 / 8.5% | +3.8 / +3.8 | 3.9 / 3.9% | 2.7 / 2.7% | 0.7 / 0.7% | — |
| 26 November 2026 | Mini player, R720 (real, 24 fps) | GPUI | 10.4 / 11.2% | **+5.6 / +6.8** | 5.4 / 6.1% | 2.6 / 2.6% | 1.3 / 1.4% | 23.8/s of 23.8 |
| 26 November 2026 | Mini player, R720 (real, 24 fps) | Iced | 8.9 / 8.7% | +4.2 / +4.0 | 4.1 / 4.0% | 2.7 / 2.7% | 0.9 / 0.9% | — |
| 26 November 2026 | Mini player, S360 (30 fps) | GPUI | 11.4 / 11.3% | +6.6 / +6.9 | 6.0 / 5.8% | 2.8 / 2.8% | 1.4 / 1.5% | 29.7/s of 29.7 |
| 26 November 2026 | Mini player, S360 (30 fps) | Iced | 9.9 / 9.8% | +5.2 / +5.1 | 4.5 / 4.4% | 3.2 / 3.2% | 0.9 / 0.9% | — |
| 26 November 2026 | Mini player, S720 (30 fps) | GPUI | 12.3 / 11.8% | +7.5 / +7.4 | 6.7 / 6.2% | 2.7 / 2.7% | 1.7 / 1.7% | 29.8/s of 29.8 |
| 26 November 2026 | Mini player, S720 (30 fps) | Iced | 10.3 / 10.3% | +5.6 / +5.6 | 4.7 / 4.4% | 3.1 / 3.4% | 1.1 / 1.1% | — |
| 26 November 2026 | Immersive view, S720 (784 x 442) | GPUI | 13.7 / 13.9% | +8.9 / +9.5 | 5.1 / 5.2% | 2.6 / 2.6% | 4.8 / 4.8% | 29.8/s of 29.8 |

"Presentation" is Mesa's thread that copies each frame to Xvfb (`WSI swapchain q`), "Decoding" the thread
of the pipeline (`multiqueue0:src`: reading, VA-API, and in GPUI also `videoscale` and the conversion to BGRA). Xvfb, a
separate process, spent another 2.1–3.5% at rest and 4.8–6.7% with the video, for both clients.

- **Gate 1 (CPU): 720p passes, 360p sits on the limit.** Real video clips in the mini player: 720p +5.6 / +6.8 points (threshold
  +10), 360p +5.6 / +6.3 (threshold +6: one round under, one 0.3 over). In the worst synthetic case at 30 fps: 720p +7.5 /
  +7.4, 360p +6.6 / +6.9. In GPUI 360p and 720p cost almost the same, because the pipeline reduces both to 288 x 162
  before converting. **Without Xvfb's presentation copy** (which a real display does not have, for both) the
  real 360p is +4.3 / +4.5 and the synthetic +4.9 / +5.2: within the threshold.
- **GPUI costs 1.4–2.8 points more than Iced** with the same video clip: about 1.5 in the interface thread (each
  frame redraws the window: the `Shell` root is always redone, the cached views reuse their drawing; plus
  the new `RenderImage` and its upload into the atlas) and 0.4–0.6 in the pipeline (the reduction and the conversion to
  BGRA on the CPU, which Iced does on the GPU with the NV12 shader).
- **No frame lost** with the window focused: all those copied are shown (23.8/s of 23.8, 29.7 of
  29.7). Without focus GPUI shows 20.6 per second of 29.7 (BACKLOG 44).
- **The immersive view** (784 x 442 at scale 1) costs +8.9 / +9.5: here what weighs is the conversion of a frame 7 times
  larger (decoding 4.8%), not the interface.

Gate 2 (memory), 10 minutes of S720 (30 fps, the worst case) in the mini player, a new track every 60 s via MPRIS
(10 changes, queue of three tracks on repeat), memory every 20 s. The machine had ~9 GB in swap and the kernel
was reclaiming pages even with 5–6 GB available: Iced's RSS fell from 335 to 54 MB in the first round (file pages
removed, not memory freed), so in the third round the anonymous part plus the swap (`RssAnon + VmSwap`) is also read, which
is where a leak shows.

| Date | Round | Binary | Average CPU (10 min) | RSS start → end (maximum) | RSS growth | Anonymous + swap start → end (maximum) | Frames shown per pipeline |
|---|---|---|---|---|---|---|---|
| 26 November 2026 | 1 | GPUI | 12.0% | 285 → 294 (294) MB | **+9 MB** | not read | 1798–1799 of 1800–1801 in 60 s |
| 26 November 2026 | 1 | Iced | 10.8% | 335 → 97 (335) MB | not valid (kernel reclaim) | not read | — |
| 26 November 2026 | 2 | GPUI | 12.4% | 286 → 296 (296) MB | **+10 MB** | not read | 1797–1800 of 1800–1802 |
| 26 November 2026 | 3 | Iced | 10.8% | 327 → 97 (327) MB | not valid (93 MB in swap) | 139 → 158 (167) MB, **+19 MB** | — |
| 26 November 2026 | 3 | GPUI | 11.9% | 278 → 283 (290) MB | **+5 MB** | 94 → 107 (107) MB, **+13 MB**, no swap | 1798–1800 of 1801–1802 |

- **Gate 2: passes.** GPUI's RSS grows by 5–10 MB in 10 minutes with 10 track changes (threshold 20), the
  anonymous part by 13 MB, in steps at the track changes (94, 99, 100, 102, 106, 103, 104, 100, 107: it rises in the first 5 minutes and
  then oscillates). Iced, on the same video clip, +19 MB of anonymous part. No backlog: for each pipeline the
  frames shown are those copied minus 1–4 (replaced in the spot before the UI took them), the spot keeps
  one by construction. A one-hour run would tell whether the anonymous part really stops: not done.
- **The video's memory.** With the video clip open GPUI sits at 278–296 MB against 207–222 MB for audio only: ~70 MB between
  GStreamer libraries, the VA-API driver and the pipeline buffers, which close with the video clip (when paused, on a track
  change without a video clip, with the panel closed).

### Desktop QA 1: idle wake-ups, unfocused window, video clip, cached bars

First performance pass after step 6 (`docs/transformation/04-core-architecture.md` §8, "QA 1"). Release binaries
of `psytify-desktop` (`--features video` where there is the video clip): `main` is `857add5` (end of step 6.9),
the others are the commits of this pass. Each case runs in my Xvfb (1280 x 800, no window manager: the
window is focused with `xdotool windowfocus`, except the "unfocused" cases), with disposable data, a private
session bus (Play and Pause via MPRIS) and the fake backend of step 6.9: synthetic audio and video clips (S360 and
S720, 30 fps) from local files, no video clip asked of the server, the other GETs read only towards the
public server. Vulkan on the Radeon with `MESA_VK_WSI_DEBUG=sw`: **Xvfb has no DRI3**, so Mesa presents each
frame by copying it with the CPU (`WSI swapchain q`, which a real display does not have). 40 s of sample after 20 s
of settling, two alternating rounds ("round 1 / round 2"), CPU in % of one core; the wake-ups are the context switches
per second (voluntary and not) of each thread, read from `/proc/PID/task/*/status` in the same 40 s.

**The state of the machine changes between the matrices.** The machine restarted twice during the work. Matrices B and D ran on
battery with the `power-saver` profile (amd_pstate): the CPU is slower and the same work costs more points (audio only with the queue: 8.7% against 4.4–4.8% in step 6.9),
so there only the alternating comparisons count. F and G ran plugged in with the `performance` profile. In addition GPUI reads the profile
from power-profiles-daemon on the system bus and in `power-saver` limits each window to one draw every 33.3 ms
(`low_power_frame_interval`: 20.8 fps with the 16 ms step of X11), and the core derives "Reduce motion" from it
(step 6.5b): matrix B ran that way and only served to discover it. In matrices D, F and G the app's system bus is the private one: neither GPUI nor the core
sees the machine's profile, and the sprites animate.

**Idle wake-ups (BACKLOG 13).** Matrix A (before a restart of the machine, real system bus, profile not in
`power-saver`: the unfocused video clip reached 29.8 fps), Home idle, window unfocused. `b1` is `main`
with our MPRIS service (`83f902f`) and `inactive_frame_interval: None` (`6989ec1`).

| Date | State | Binary | Total CPU | Wake-ups/s | RSS |
|---|---|---|---|---|---|
| 26 November 2026 | Idle on the Home, unfocused | main | 0.4 / 0.4% | 217 / 216 | 192 / 211 MB |
| 26 November 2026 | Idle on the Home, unfocused | b1 | 0.3 / 0.3% | **116 / 118** | 214 / 198 MB |

| Thread | main (round 1) | b1 (round 1) | Whose | Why |
|---|---|---|---|---|
| `psytify-core` | 99.6/s | — | ours: **removed** | souvlaki's MPRIS thread (unnamed, it inherits the core's name) ran on `recv_timeout(10 ms)` |
| main thread | 62.9/s, 0.12% | 63.0/s, 0.12% | GPUI on X11 | the 16 ms refresh timer: one `read`, one `recvmsg` and one `epoll_pwait` at each tick (strace: 62.5/s) |
| `cpal_alsa_out` | 26.1/s, 0.15% | 25.4/s, 0.12% | rodio and cpal | the output device stays open and plays silence even when idle (strace: `poll` and `read` 23/s) |
| `data-loop.0` | 23.7/s | 23.5/s | PipeWire (ALSA plugin in the process) | the data cycle of the open stream; ~95/s in other sessions on the same machine: it depends on PipeWire's graph |
| `blocking-1`, `blocking-2` | 2.0/s each | 2.0/s each | `blocking` crate (zbus) | the idle threads wait for work with a 500 ms timeout |
| `alsa-pipewire` | 0.2/s | 0.2/s | PipeWire | |

- The rest is not ours: X11 (Wayland uses frame callbacks and has no timer), the open audio device and the threads of the
  `blocking` crate. **Closing the device after a few seconds without a track** would remove ~50 wake-ups per second and
  0.15–0.25 points of CPU, but the first Play after a pause would reopen the device: the owner's decision, not made.
- Matrix D (private system bus, `data-loop.0` at 95/s): 291 → 194 wake-ups per second between `main` and the branch, the
  same difference of ~100.
- The MPRIS service is in the core, so it also applies to the Iced client, which was not measured again.

**Unfocused window (BACKLOG 44).** Matrix A, S720 in the mini player, window unfocused.

| Date | Scenario | Binary | Total CPU | UI thread | Presentation (Xvfb) | Frames shown |
|---|---|---|---|---|---|---|
| 26 November 2026 | S720 in the mini player, unfocused | main-video | 8.3 / 8.4% | 3.95 / 3.92% | 1.8 / 1.8% | 20.7/s of 29.8 |
| 26 November 2026 | S720 in the mini player, unfocused | b1-video | 11.8 / 11.9% | 6.3 / 6.3% | 3.0 / 3.1% | **29.8/s of 29.8** |

- `inactive_frame_interval: None`: without focus the window draws at the rhythm of its content. At rest it costs
  nothing (above: 0.4 → 0.3%, no drawing); with the video clip the price is the frames that were lost before, the
  same cost as the focused window in step 6.9 (12.3 / 11.8%).

**Video clip, title bar and sidebar (BACKLOG 41 and 23).** Matrix F (after a restart: laptop plugged in
with the `performance` profile, private system bus, animated sprites), window focused. Three binaries with the video:
`main` (`857add5`), `b4a` (`95b00af`: `Shell` stays the root and the stage is its last child, the panel and the immersive
view have the well) and `b4` (`f8690fb`: in addition the title bar and the sidebar in cache). Order of the
binaries reversed in the second round; round 1 of the immersive view and idle is redone separately (matrix G),
because in F the private bus did not start with the longer labels (socket beyond 108 bytes). Audio only: the
panel on the queue; video: the mini player of the panel (288 x 162) or the immersive view (784 x 442).

| Date | Scenario | Binary | Total CPU | Δ over audio only | UI thread | Presentation (Xvfb) | Decoding | Frames shown |
|---|---|---|---|---|---|---|---|---|
| 26 November 2026 | Audio only (queue) | main-video | 4.6 / 4.5% | — | 2.60 / 2.62% | 1.2 / 1.2% | — | — |
| 26 November 2026 | Audio only (queue) | b4a-video | 4.4 / 4.5% | — | 2.65 / 2.70% | 1.1 / 1.2% | — | — |
| 26 November 2026 | Audio only (queue) | b4-video | **3.9 / 3.9%** | — | 2.15 / 2.17% | 1.2 / 1.2% | — | — |
| 26 November 2026 | Liked tracks, current row | main-video | 4.3 / 4.3% | — | 2.35 / 2.35% | 1.1 / 1.1% | — | — |
| 26 November 2026 | Liked tracks, current row | b4a-video | 4.2 / 4.2% | — | 2.42 / 2.35% | 1.2 / 1.2% | — | — |
| 26 November 2026 | Liked tracks, current row | b4-video | **3.7 / 3.6%** | — | 1.90 / 1.82% | 1.2 / 1.1% | — | — |
| 26 November 2026 | Mini player, S360 (30 fps) | main-video | 13.2 / 13.1% | +8.6 / +8.6 | 7.19 / 7.09% | 3.3 / 3.2% | 1.6 / 1.6% | 29.8/s of 29.8 |
| 26 November 2026 | Mini player, S360 (30 fps) | b4a-video | 12.6 / 13.0% | +8.2 / +8.5 | 6.84 / 7.04% | 3.2 / 3.3% | 1.6 / 1.6% | 29.8/s of 29.8 |
| 26 November 2026 | Mini player, S360 (30 fps) | b4-video | **11.1 / 10.9%** | +7.2 / +7.0 | 5.02 / 5.04% | 3.4 / 3.2% | 1.6 / 1.6% | 29.8/s of 29.8 |
| 26 November 2026 | Mini player, S720 (30 fps) | main-video | 14.5 / 14.0% | +9.9 / +9.5 | 7.89 / 7.42% | 3.5 / 3.5% | 1.8 / 1.9% | 29.8/s of 29.8 |
| 26 November 2026 | Mini player, S720 (30 fps) | b4a-video | 14.0 / 13.7% | +9.6 / +9.2 | 7.47 / 7.27% | 3.5 / 3.4% | 1.9 / 1.9% | 29.8/s of 29.8 |
| 26 November 2026 | Mini player, S720 (30 fps) | b4-video | **11.6 / 11.9%** | +7.7 / +8.0 | 5.22 / 5.54% | 3.4 / 3.5% | 1.9 / 1.8% | 29.8/s of 29.8 |
| 26 November 2026 | Immersive view, S720 (784 x 442) | main-video | 15.8 / 15.7% | +11.2 / +11.2 | 6.72 / 6.67% | 3.3 / 3.2% | 4.6 / 4.6% | 29.8/s of 29.8 |
| 26 November 2026 | Immersive view, S720 (784 x 442) | b4a-video | 14.8 / 14.8% | +10.4 / +10.3 | 5.84 / 5.79% | 3.3 / 3.4% | 4.7 / 4.7% | 29.8/s of 29.8 |
| 26 November 2026 | Immersive view, S720 (784 x 442) | b4-video | **13.8 / 13.6%** | +9.9 / +9.7 | 4.79 / 4.67% | 3.3 / 3.2% | 4.6 / 4.6% | 29.8/s of 29.8 |

- **Title bar and sidebar in cache (`b4a` → `b4`): −0.5 / −0.6 points with a track playing, from −1.0 to
  −2.4 with the video clip.** `Shell` is the root and GPUI redoes it at every drawing: at every tick of the equaliser
  that paints over the pages, of the mark, of the bar's beat and at every frame. Before it also redid the two bars (the search
  field, the history, the account, eight entries with the counts); now they are two views in `.cached()` that are redone
  only when what they show changes. The UI thread drops by ~0.5 points with a track playing (queue, liked tracks)
  and by 1.0–2.3 with the video clip (30 drawings per second).
- **The stage outside the panel and the immersive view (`main` → `b4a`): from −0.1 to −0.6 points in the mini player,
  −1.0 / −0.9 in the immersive view, nothing without a video clip.** A frame redid the panel or the immersive view
  (the stage was their child); now it redoes only `Shell` and the stage, which paints in the well marked by the panel. The panel is small, the immersive view
  is not. Together (`main` → `b4`): −0.6 / −0.7 points with a track playing, from −2.0 to −2.9 with the video clip.
- **An attempt discarded.** The first step (`27648e9`) put `Shell` in `.cached()` under a new root, with
  the stage beside it. In GPUI (rev `d31d107`) a cached view that is redone redraws everything it contains with
  `refreshing` on, so also the cached views inside it: at every tick of a sprite the
  pages, panels and bar were redone. Matrix D (private system bus, on battery in `power-saver`), audio only with the
  queue: UI thread 5.0 / 5.2% on `main`, 11.1 / 12.3% with `Shell` in cache. Removed in `95b00af`. With the
  reduced motion from the profile (matrix B, 20.8 fps for all) the same attempt looked like a gain (mini player S720:
  UI 9.35 → 5.9%), because without sprites `Shell` almost never got dirty.
- **Gate 1 of step 6.9, measured again with the synthetic clips.** Δ over audio only of the same binary in
  the same round: S360 from +8.6 / +8.6 to **+7.2 / +7.0** (threshold +6), S720 from +9.9 / +9.5 to **+7.7 / +8.0** (threshold
  +10), immersive view from +11.2 / +11.2 to **+9.9 / +9.7**. The 720p passes with a margin; the synthetic 360p at 30 fps
  (the worst case, not the real clip of the gate, which was not asked of the server) stays one point above. Today `main` measures ~2 points more than in step 6.9
  (S360 +6.6 / +6.9 then) on the same machine in another state: the real difference is the alternating one.
- **Idle, unfocused, at the end of the work**: `main` 0.4 / 0.5% and 211 / 211 wake-ups per second, `b4` 0.4 / 0.4% and
  114 / 114 (in F round 1 the private bus was off: without MPRIS even `main` was at 110).

### Desktop v2, slice A: zoom, frame, pixel motion, sidebar v2

Release builds. `/usr/bin/psytify` (the installed v1 desktop) is "before", `feat/desktop-redesign` at `029a543` is
"after". Each run: the window full screen on the 2560 x 1440 monitor (visible, Auto zoom 1.5 in the new build),
pointer moved off the window, 10 s to settle, then `scripts/perf-sample.py PID 30`. Idle on the Home with a track
loaded and stopped, the same throwaway config for both, two alternating runs ("run 1 / run 2").

| Date | Scenario | Binary | Total CPU | RSS | Threads |
|---|---|---|---|---|---|
| 27 November 2026 | Idle, Home, 2560 x 1440 full screen | before | 1.9 / 1.7% | 170 / 176 MB | 39 |
| 27 November 2026 | Idle, Home, 2560 x 1440 full screen | after | 1.9 / 1.6% | 63 / 173 MB | 39 |

- **At rest the motion layer costs nothing**: tweens ask for frames only while they run (hover 70 ms, marker 100 ms,
  sidebar width 200 ms, page enter at most 460 ms), so with the pointer away no frame is requested.
- **RSS**: 63 MB in the first "after" run is an outlier I did not chase (cause not investigated); the second
  run (173 MB) matches "before". RAM varies by ±20 MB between two identical launches.
- **Not measured**: CPU while hovering or dragging the sidebar edge (bounded by the durations above), and the
  1280 x 800 window.

### Desktop v2, slice B: side panel v2, stage, media slot, pixel motion

Release builds. Slice A (`029a543`) is "before", `feat/desktop-redesign` at `1e88b41` is "after". Each run: the
window full screen on the 2560 x 1440 monitor (visible, Auto zoom 1.5), 10 s to settle, then
`scripts/perf-sample.py PID 30`; idle on the Home with a track loaded and stopped, the same throwaway config for
both. Runs alternate between the binaries.

| Date | Scenario | Binary | Total CPU | RSS | Threads |
|---|---|---|---|---|---|
| 27 November 2026 | Idle, Home, 2560 x 1440 full screen | before | 0.7 / 3.0% | 158 / 97 MB | 39 |
| 27 November 2026 | Idle, Home, 2560 x 1440 full screen | after | 0.4% (one run) | 181 MB | 39 |
| 27 November 2026 | Idle, Home (window state not captured) | before | 0.5 / 2.1% | 24 / 158 MB | 39 |
| 27 November 2026 | Idle, Home (window state not captured) | after | 0.4 / 0.4% | 95 / 156 MB | 39 |

- **At rest slice B costs nothing more**: the new motion (cover cross-fade, title rise, play/pause frame, like
  fill, dissolve, panel width, stage reveal) asks for frames only while it runs; the stage's 250 ms tick runs
  only while the stage is open and playing. The 3.0% and 2.1% "before" runs are UI-thread spikes I did not
  chase; RSS varies by more than the ±20 MB noted above between identical launches (24 MB is an outlier).
- **Not measured: playing with the stage on lyrics.** The runs I made are not valid: keystrokes from `wtype`
  did not always reach the window (shared keyboard focus), and with MPRIS `Play` the track stopped after
  about 8 s, before the 30 s sample. Test playback must also go to a silent sink, and routing the ALSA
  output to it was not verified, so I stopped. To redo: route the output to a null sink first, start with
  MPRIS `Play`, and check from a screenshot that the clock has moved past 0:38 when the sample ends.

### Desktop v2, slice C: page frame on every page, cover tiers 256 / 512

Release builds. "Before" is slice A (`31f95b0`), "after" is `feat/desktop-redesign-pages` at `926a0b2`. The external
monitors were off during these runs, so every run is on the laptop panel (eDP-1, 1706 x 1066 logical, scale 1.5):
the window visible, nothing playing (a track loaded and stopped), the pointer left where it was, the same throwaway
config for both binaries, alternating runs ("run 1 / run 2"). `scripts/perf-sample.py PID 30` on the liked tracks
page (the collection page: the same header and track rows as an album page; no server album was reachable from the
test config, see the slice C report), `PID 20` on the artist page full screen (a photo header and a grid of 7 x 2
visible discography tiles, which is where the tiers show).

| Date | Scenario | Binary | Total CPU | RSS | Threads |
|---|---|---|---|---|---|
| 27 November 2026 | Idle, Liked tracks, 1280 x 800 window | before | 2.3 / 1.1% | 159 / 152 MB | 39 |
| 27 November 2026 | Idle, Liked tracks, 1280 x 800 window | after | 2.8 / 1.1% | 161 / 159 MB | 39 |
| 27 November 2026 | Idle, artist page, full screen | before | 2.9 / 0.1% | 168 / 173 MB | 39 |
| 27 November 2026 | Idle, artist page, full screen | after | 0.2 / 2.5% | 178 / 177 MB | 39 |

- **CPU at rest is unchanged**: the page enter asks for frames only while it runs (at most 460 ms); the spread
  between runs (0.1 to 2.9%) is the panel and the rest of the desktop, the same for both binaries.
- **Cover tiers cost about 5 to 10 MB here** (artist page, both runs), in line with about 30 tiles at 256 px
  (256 KiB each) and one 512 header. The first version of the tiers asked for 512 for every tile at scale 1.5
  (a 200 dp tile is 300 device px) and measured 184 / 212 / 212 / 214 MB against 168 / 180 / 162 / 175 MB:
  a 256 tier now serves up to 320 device px (`TIER_STRETCH` 1.25), and the whole 256 image is made only when a tier
  waits for it. The tier deposit is bounded at 112 images and 28 MiB (`covers::MAX_TIERS`, `MAX_TIER_BYTES`).
- **Not measured**: the 2560 x 1440 monitor (off during the runs), CPU while hovering, and a page with more than
  about 100 large tiles on screen (beyond the tier bound the least recent tiles are evicted and asked for again).

## To do

- **FLAC**: in the local catalogue all 52 tracks are m4a with an external licence; Internet Archive's FLAC
  arrives after an import (it writes to the database, and the user's authorisation is needed). The FLAC
  decoding (Symphonia) and the buffer of `player/stream.rs` must be measured.
- **Video open**: measured in the GPUI desktop and in the Iced client at step 6.9 and again in "Desktop QA 1" (above:
  a frame no longer redoes the panel, the immersive view and the bars). What remains is the cost on the GPU, the battery and Windows
  (BACKLOG 40, 41, 42) and the decoders of the dedicated card (`dedicated_video`).
- **Main heap ~50 MB in use** (estimate: cosmic-text's system fonts, images, texts). A heap profile is needed to say
  more; `perf` or `heaptrack` are not installed.
- **Lyrics**: `verse_heights` recalculates the heights at every redraw (a sum of widths per line,
  a few microseconds). To be kept in cache per track and width if a profile indicates it.
- **Idle wake-ups**: with the app idle on the Home, ~250 context switches per second, ~150 of which
  in Iced's tokio workers (see "Migration to the core"). To be understood: who wakes them. In the GPUI desktop all are attributed
  ("Desktop QA 1"): ~100 were souvlaki's MPRIS thread, removed; the same MPRIS service also runs in the
  Iced client, to be measured again.

## Research: what remains to try (27 October 2026)

Ordered by expected gain on our case, which is dominated by the number of redraws (the renderer
costs ~1.4–2.6 ms per frame, `view` and `update` a few microseconds).

1. **Fewer redraws, not faster redraws.** It is the lever that has worked every time (frames only
   with an animation, Tick at 250 ms, text at 24 fps). Next step: stop the Tick while the lyrics
   timer is active (today both arrive) and do not redraw when the displayed position
   does not change by at least one pixel.
2. **Iced 0.14**: the release notes cite the *reactive rendering* (#2662: only what changes is redrawn) and the
   *culling* of primitives in `column` and `row` (#2611). They are the two problems we work around here by hand; upgrading is worth a measurement, but the API of `Task`, `Subscription` and styles changes
   and the migration touches the whole UI.
3. **Iced's `lazy`** on the parts that do not change during playback (sidebar, Home shelves):
   the widget is rebuilt only if the key changes. Useful if a profile shows a heavy `view`; today it is not.
4. **PGO** (profile-guided optimisation, `cargo-pgo`): the Rust Performance Book reports
   gains of 10% and more, but it needs a representative load and two builds; our bottleneck is the GPU/the renderer, not the Rust code, so the expected gain is low.
5. **`target-cpu=native`**: only for personal builds, the binary would not run on older CPUs.
6. **Alternative allocators** (jemalloc, mimalloc): the Perf Book recommends them, but here mimalloc worsened the RAM (see below). jemalloc must be measured the same way before adopting it.
7. **Presentation mode** (`ICED_PRESENT_MODE`, `fifo` by default): `fifo` is vsync and the right choice for a player; `mailbox`/`immediate` increase frames, they do not reduce them.

Sources: [Rust Performance Book, Build configuration](https://nnethercote.github.io/perf-book/build-configuration.html),
[iced, release notes](https://github.com/iced-rs/iced/releases),
[iced, performance guide (`lazy`)](https://www.mintlify.com/iced-rs/iced/advanced/performance),
[iced_wgpu, `present_mode_from_env`](https://docs.iced.rs/iced_wgpu/settings/fn.present_mode_from_env.html),
[wgpu, `PresentMode`](https://wgpu.rs/doc/wgpu/enum.PresentMode.html).

## Goals

- Idle with lyrics: CPU ≤ 0.5% (today 0.6–0.7%).
- Playing with lyrics: CPU ≤ 3% (today 2.6–3.0% with the animated text; it was 1.6–1.8% without animation).
- RSS while playing ≤ 300 MB (today 190–200 MB).

### Desktop step 7: the `psytify` binary (GPUI) against `psytify-iced`

Release binaries of `main` after step 7 (`7b4e766`): `psytify` is the GPUI desktop with the video clip on by
default, `psytify-iced` the Iced client. Real Wayland session (niri), isolated `XDG_CONFIG_HOME` with a track in the queue
("Instant Crush", official catalogue), volume 0, Discord off, public backend. The window is on the laptop screen
(eDP-1, scale 1.5), in the active and empty workspace: **visible but without focus** (the focus stays with whoever
works on the other screen; the check every 2 s says visible 20 times out of 20 in each round). 15 s of settling, playback starts with
`playerctl play` (MPRIS) and another 15 s, then 40 s of sample with `scripts/perf-sample.py`; the
wake-ups are the context switches per second of all the threads, from `/proc/PID/task/*/status`. Two alternating rounds,
"round 1 / round 2".

| Date | State | Binary | Total CPU | UI thread | RSS | Threads | Wake-ups/s |
|---|---|---|---|---|---|---|---|
| 26 November 2026 | Idle on the Home, visible unfocused | `psytify` (GPUI) | 0.4 / 0.4% | < 0.1% | 171 / 182 MB | 38 | 127 / 129 |
| 26 November 2026 | Idle on the Home, visible unfocused | `psytify-iced` | 1.5 / 1.4% | 1.0 / 0.9% | 185 / 198 MB | 42 | 233 / 215 |
| 26 November 2026 | One track playing, visible unfocused | `psytify` (GPUI) | 1.5 / 1.4% | 0.7 / 0.6% | 170 / 170 MB | 39 | 202 / 198 |
| 26 November 2026 | One track playing, visible unfocused | `psytify-iced` | 2.3 / 2.3% | 1.4 / 1.4% | 194 / 192 MB | 43 | 304 / 292 |

- **GPUI costs less in every case.** Idle: a quarter of Iced's CPU and half of the wake-ups; while playing 0.8–0.9
  points less, with the UI thread at half. At step 5 GPUI while playing cost more than Iced (2.3 against 1.8–2.0%):
  the cached views (6.x, QA 1) and our MPRIS service (QA 1) changed the comparison.
- **RAM: GPUI 14–16 MB less at idle and 22–24 MB less while playing.** The difference at idle lies within the variation between
  two identical starts (±20 MB), the one while playing just outside it.
- **What remains is not the UI:** `cpal_alsa_out` (0.1% idle, 0.5% while playing) and PipeWire's `data-loop.0`
  (0.2–0.3%) are equal in the two binaries; closing the audio device at rest is still an open decision.
- **A first series is not valid and is discarded:** with the windows open in the workspace of whoever was working, focus
  kept going to the other window and the Psytify window ended up off screen (focus 0 of 20 almost always, Iced's UI thread
  at 0.1%): without frames the UI seems free. Hence the move to eDP-1.
- **Not measured:** the window with focus, scale 1, the video clip (the video measurements remain those of
  step 6.9 and QA 1), battery, Windows.
