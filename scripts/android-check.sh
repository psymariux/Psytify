#!/usr/bin/env bash
# Checks that the Android shell compiles for aarch64 without building the APK (`cargo check`).
# `ring` needs the NDK's C compiler even for a plain check: the environment comes from
# `android-env.sh`. Usage: scripts/android-check.sh
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=scripts/android-env.sh
. "$ROOT/scripts/android-env.sh"

cd "$ROOT/android"
cargo check --target aarch64-linux-android --locked
