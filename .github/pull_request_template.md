## What and why

<!-- What does this change do, and why? Link the issue it closes: "Closes #123". -->

## How it was tested

<!-- Commands you ran, the platform you tried it on. For UI changes: a screenshot or short clip. -->

## Checklist

- [ ] The PR title follows [Conventional Commits](https://www.conventionalcommits.org) (`feat: …`, `fix: …`)
- [ ] `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass
- [ ] `cargo test --workspace` passes, with tests for new behavior
- [ ] No hot-path regressions (see "Performance" in CONTRIBUTING.md) — numbers attached if the change touches `update`, `view`, audio or caches
- [ ] No secrets, tokens, signed URLs or personal addresses in code, tests, logs or screenshots
- [ ] Docs and `CHANGELOG.md` updated if behavior changed
