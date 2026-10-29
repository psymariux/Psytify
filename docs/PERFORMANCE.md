# Client performance

Measurements, bottlenecks and optimization status of the Psytify client. The rules for writing fast
code are in [CONTRIBUTING.md](../CONTRIBUTING.md), section "Performance rules".

## How to measure

- Tool: `scripts/perf-sample.py PID [SECONDS]`. It reads `/proc` (CPU per thread, RSS) and `nvidia-smi`
  (compute memory). It needs no privileges.
- **The window must be visible and focused on the active workspace.** With a locked screen or a covered
  window the compositor does not send frame callbacks and the UI CPU looks close to zero: such a
  measurement is false (it happened, see below).
- Reference scenario: a test instance with an isolated configuration (`XDG_CONFIG_HOME`), volume at
  zero, an m4a track with synced lyrics, the "Lyrics" panel open (key `T`), video closed. It is started
  and made to play over MPRIS (`org.mpris.MediaPlayer2.psytify.instance<PID>`).
- A/B comparisons: two binaries, two rounds each, alternated. RAM varies by ±20 MB between two identical
  launches: repeated differences count, not a single value.
- Sampling profiler (`perf`): not installed. For memory, `gdb` is used with the program started by gdb
  (`ptrace_scope=1`): `call (void)malloc_stats()` and `call (int)malloc_trim(0)`.

## Results (m4a playback, lyrics open, video closed)

| | Before | After |
|---|---|---|
| Total CPU | 10.6% of a core | **1.6–1.8%** |
| UI thread | 9.0% | **0.7%** |
| RSS after 15 s | 337–388 MB | **230–237 MB** |
| NVIDIA and Mesa/LLVM libraries loaded | yes (~70 MB) | no |
| NVIDIA context of the process | yes (graphics, 2 MiB) | none |

### What changed

1. **No continuous frames** (`app.rs`, `subscription`). `window::frames()` sent a message on every vsync
   while playing, and every message redraws the window. Full frames now start only with an animation
   (`is_animating`); the progress bar updates with the Tick (4 per second, sub-pixel step). Instrumenting
   `view` and `update`: they cost less than 1 µs and 2 µs. The ~1.4 ms per redraw is all inside iced's
   renderer, so the only lever is the number of redraws.
2. **Light environment** (`launch.rs`, Linux only). The client relaunches itself once with `exec` (same
   PID) setting:
   - `MALLOC_ARENA_MAX=2`: glibc opened 32 arenas (one per thread) and kept 186 MB of heap for 111 MB in
     use; `malloc_trim` alone brought RAM from 270 down to 226 MB.
   - `VK_LOADER_DRIVERS_DISABLE=*nvidia*` and `WGPU_BACKEND=vulkan`, only if the driver of an integrated
     GPU (RADV, ANV, AMDVLK) is also present: iced already draws on the low-power GPU, and the NVIDIA
     driver and wgpu's OpenGL were being loaded for nothing (~70 MB and the discrete card kept awake).
   - To pick the GPU by hand: `PSYTIFY_ALL_GPUS=1`, or one of the `VK_*`, `WGPU_*`, `DRI_PRIME`,
     `__NV_PRIME_RENDER_OFFLOAD` variables: in that case nothing is touched.
3. The "Video clips on the graphics card" option (Settings → Playback) works with the light environment:
   NVDEC is loaded only when it is on (126 MiB of VRAM), otherwise VA-API is used.

### 27 October 2026: new animations, cache, release profile

Release binaries, visible window on the laptop (60 Hz), volume at zero, "In the End" (m4a, synced
lyrics), two rounds each.

| Scenario | Total CPU | UI thread | RSS |
|---|---|---|---|
| Search + "Now playing", previous profile (A) | 1.6% / 1.6% | 0.4–0.5% | 200 / 194 MB |
| Same, full LTO + 1 codegen unit (B) | 1.5% / 1.4% | 0.4% | 199 / 194 MB |
| Home + "Lyrics" in the panel, scrolling at vsync (650 ms) | 6.9% / 7.2% | 5.8% | 192 / 201 MB |
| Same, scrolling at 30 fps, 480 ms | 4.2% / 4.1% | 2.8–3.0% | 193 / 196 MB |
| Same, 24 fps and line body size at pixel precision (**adopted**) | **3.0% / 2.6%** | **1.6–1.8%** | **190 / 192 MB** |

- **Release profile** (`Cargo.toml`): `lto = "fat"`, `codegen-units = 1`. Binary from 41 to 28 MB, CPU the
  same or slightly lower (within noise), RAM the same. A from-scratch build takes 2 min 51 s; on every
  client change the LTO link costs ~2 min 15 s (development uses the `dev` profile). `panic` stays
  `unwind`.
- **Scrolling lyrics** are the most frequent motion (every line, about every 2 s): at vsync they cost 5.8%
  of the UI thread. They no longer request window frames but a 24 fps timer (`AMBIENT_FRAME`), last
  480 ms, and the size of the growing lines is rounded to the pixel, so the text is laid out again only
  when the measure really changes. The other motions stay at vsync.
- **Cover cache** (`ui/cover.rs`): at most 240 ready covers, evicting the least recently drawn (LRU). The
  round version (artists) is derived from the square one only when needed: 256 KB less for every album
  cover. `is_fading` no longer walks the whole cache on every message.
- To measure the "Lyrics" panel with no mouse or keyboard there is the `hold-lyrics` step of the automatic
  tour (`--features tour`, `PSYTIFY_TOUR_STEPS=playing,hold-lyrics`): it opens the lyrics and stays still
  for 40 s.

### Discarded attempts

- **mimalloc** as the global allocator: RSS 522–540 MB against 344 (it keeps pages, and C libraries keep
  using glibc). Removed.
- **Progress bar at 20 or 10 Hz with `time::every`**: 4.5% and 3.6% total CPU. The Tick is enough.

## To do

- **FLAC**: measure FLAC decoding (Symphonia) and the buffer in `player/stream.rs`.
- **Video open**: cost of GStreamer and the decoders, with and without the dedicated option.
- **Main heap ~50 MB in use** (estimate: cosmic-text system fonts, images, texts). A heap profile is
  needed to say more; `perf` and `heaptrack` are not installed.
- **Lyrics**: `verse_heights` recomputes the heights on every redraw (a sum of widths per line, a few
  microseconds). Cache it per track and width if a profile points there.

## Research: what is left to try (27 October 2026)

Ordered by expected gain for our case, which is dominated by the number of redraws (the renderer costs
~1.4–2.6 ms per frame, `view` and `update` a few microseconds).

1. **Fewer redraws, not faster redraws.** It is the lever that worked every time (frames only with an
   animation, Tick at 250 ms, lyrics at 24 fps). Next step: stop the Tick while the lyrics timer is
   active (today both arrive) and do not redraw when the displayed position does not change by at
   least one pixel.
2. **Iced 0.14**: the release notes mention *reactive rendering* (#2662: only what changes is redrawn)
   and primitive *culling* in `column` and `row` (#2611). They are the two problems we work around by
   hand; upgrading deserves a measurement, but the `Task`, `Subscription` and style APIs change and the
   migration touches the whole UI.
3. **Iced's `lazy`** on the parts that do not change during playback (sidebar, Home shelves): the widget
   is rebuilt only when its key changes. Useful if a profile shows a heavy `view`; today it does not.
4. **PGO** (profile-guided optimization, `cargo-pgo`): the Rust Performance Book reports gains of 10% and
   more, but it needs a representative workload and two builds; our bottleneck is the GPU/renderer, not
   the Rust code, so the expectation is low.
5. **`target-cpu=native`**: only for personal builds, the binary would not run on older CPUs.
6. **Alternative allocators** (jemalloc, mimalloc): the Perf Book recommends them, but here mimalloc made
   RAM worse (see above). jemalloc must be measured the same way before adopting it.
7. **Present mode** (`ICED_PRESENT_MODE`, `fifo` by default): `fifo` is vsync and the right choice for a
   player; `mailbox`/`immediate` increase frames, they do not reduce them.

Sources: [Rust Performance Book, Build configuration](https://nnethercote.github.io/perf-book/build-configuration.html),
[iced, release notes](https://github.com/iced-rs/iced/releases),
[iced, performance guide (`lazy`)](https://www.mintlify.com/iced-rs/iced/advanced/performance),
[iced_wgpu, `present_mode_from_env`](https://docs.iced.rs/iced_wgpu/settings/fn.present_mode_from_env.html),
[wgpu, `PresentMode`](https://wgpu.rs/doc/wgpu/enum.PresentMode.html).

## Targets

- Idle with lyrics: CPU ≤ 0.5% (today 0.6–0.7%).
- Playing with lyrics: CPU ≤ 3% (today 2.6–3.0% with animated lyrics; it was 1.6–1.8% without animation).
- RSS while playing ≤ 300 MB (today 190–200 MB).
