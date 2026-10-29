# Contributing to Psytify

Thanks for wanting to help. This guide keeps contributions quick to review and easy to merge.

## Before you start

- Look through the [open issues](https://github.com/psymariux/Psytify/issues). For anything bigger than a
  small fix, open an issue first so the direction can be agreed before you invest time.
- Security problems do **not** go in public issues: see [SECURITY.md](SECURITY.md).
- By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Set up

```bash
git clone https://github.com/psymariux/Psytify.git
cd Psytify
cargo build -p psytify            # the toolchain in rust-toolchain.toml installs itself
```

System libraries are listed in the [README](README.md#build-from-source). Without GStreamer, build with
`--no-default-features` (video clips are disabled).

Point the client at a backend with `PSYTIFY_API_URL` (see [docs/BACKEND.md](docs/BACKEND.md)).
`PSYTIFY_PHONE=1` opens the phone layout in a desktop window.

## Workflow

1. Fork the repository and branch from `main` (`feat/queue-shuffle`, `fix/seek-after-pause`).
2. Keep each pull request focused on one change.
3. Run the checks locally (the same ones CI runs):

   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets --locked -- -D warnings
   cargo test --workspace --locked
   ```

4. Open a pull request against `main` and fill in the template.
5. A maintainer reviews it. Required checks must pass and conversations must be resolved. Pull requests
   are **squash-merged**, so the title becomes the commit message.

### Pull request titles

Titles follow [Conventional Commits](https://www.conventionalcommits.org/), and a check enforces it:

```
<type>(<optional scope>): <summary in the imperative mood>
```

Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `deps`, `revert`.
Examples: `fix(player): resume at the right position after a pause`, `feat(ui): add a sleep timer`.

## Code rules

- **No `unwrap`, `expect` or `panic!`** outside tests (clippy denies them). `unsafe` is forbidden.
- **Colors and sizes come only from `client/src/ui/theme.rs`.** No glass effects; gradients and shadows
  only where [docs/STYLE.md](docs/STYLE.md) allows them.
- **Never put URLs, tokens or credentials** in code, tests or logs. Signed links are secrets.
- **New dependencies** need a comment in `Cargo.toml` explaining why, and must pass `cargo deny`.
  A dependency on a hot path needs a before/after measurement.
- Comments are short and explain *why*; identifiers are in English. Existing code comments and the app's
  interface text are in Italian for now; new documentation, commit messages and pull requests are in English.
- Add or update tests for behavior you change. Pure logic (queue, feed, parsing) is easy to test; do.

## Performance rules

The app must stay light: low CPU at rest, low memory, no wasted work while playing. Anything that touches
a hot path (`update`, `view`, `subscription`, audio, decoding, caches) follows these rules.

- **Measure before optimizing**, and report CPU and RSS before and after, stating the state (idle or
  playing, format, video, lyrics). Compare two binaries alternately, twice each: memory varies by ±20 MB
  between identical launches. The window must be visible, or no frames arrive.
- **`view` does not compute.** No counting, sorting, parsing, large allocations or image decoding in `view`.
  Compute when the data changes and keep the result in `App` or a cache.
- **No per-frame work.** Use frame subscriptions only while something animates; periodic updates use a
  timer of at least 33 ms, and a timer never produces messages while the app is idle.
- **No needless copies.** Pass queues, lyrics and lists by reference, by index or with `Arc`.
- **Every cache has a capacity and an eviction policy** (LRU). There are no unbounded caches.
- **Never block the UI thread.** Network, file reads, image decoding and heavy computation run in async
  tasks or `spawn_blocking`. The audio thread does not allocate in its callbacks.
- **Long lists** are paged or built only for the visible rows.

## Reporting bugs

Use the bug report form. Include the platform, the app version, what you expected and what happened, and
the exact error text. Please redact backend addresses, tokens and anything personal.

## License

By contributing you agree that your work is released under the [GNU AGPL-3.0](LICENSE).
