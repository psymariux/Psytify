#!/usr/bin/env bash
# Builds the APK of the Kotlin app (android-app/): the Rust core as libpsytify_mobile.so with
# cargo-ndk, the UniFFI Kotlin bindings read from that library, then Gradle.
#
# Usage: [QUICK=1] [RELEASE=1] [FIXTURES=1] [INSTALL=1] scripts/android-kotlin.sh [BACKEND_URL]
#   QUICK=1      x86_64 only and without LTO (profile `android-quick`), for the emulator
#   RELEASE=1    the release APK (`dev.psytify.app`, R8, signed with the release key: it installs
#                over the earlier releases). Without it, the debug APK (`dev.psytify.app.kotlin`,
#                next to the installed app).
#   FIXTURES=1   fake social and jam views for screenshots (`psytify-mobile`'s `fixtures` feature,
#                started with `adb shell am start ... --es fixture NAME`). Debug APKs only: a release
#                build must never pass it.
#   INSTALL=1    install the APK on the device or emulator that adb sees
#   BACKEND_URL  backend of a new configuration. Without it: the
#                PSYTIFY_DEFAULT_API_URL of `.env.build`, or http://10.0.2.2:8080 with QUICK=1.
#
# Requires: the Android SDK, NDK 27 (scripts/android-env.sh) and the Rust Android targets, cargo-ndk
# (`cargo install cargo-ndk --locked`) and a JDK 21 for Gradle (JAVA_HOME, or one that Gradle has
# provisioned in ~/.gradle/jdks). Every build runs inside a memory-capped scope, one at a time.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=scripts/android-env.sh
. "$ROOT/scripts/android-env.sh"

# The distribution backend lives in `.env.build`, outside the repository. Never printed.
if [ -f "$ROOT/.env.build" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$ROOT/.env.build"
  set +a
fi
if [ "${1:-}" != "" ]; then
  export PSYTIFY_DEFAULT_API_URL=$1
elif [ "${QUICK:-}" = 1 ]; then
  # The emulator: the PC as the emulator sees it, even if `.env.build` names the public backend.
  export PSYTIFY_DEFAULT_API_URL=http://10.0.2.2:8080
fi

LIMIT=(systemd-run --user --scope -q -p MemoryMax=6G -p MemoryHigh=5G -p MemorySwapMax=1G nice)
# A CI runner has no user session to put the scope in, and the machine to itself.
if [ -n "${CI:-}" ]; then
  LIMIT=()
fi
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}

if [ "${QUICK:-}" = 1 ]; then
  TARGETS=(-t x86_64)
  PROFILE=android-quick
else
  TARGETS=(-t arm64-v8a -t x86_64)
  PROFILE=release
fi

FEATURES=()
if [ "${FIXTURES:-}" = 1 ]; then
  if [ "${RELEASE:-}" = 1 ]; then
    echo "FIXTURES=1 is for debug APKs only" >&2
    exit 1
  fi
  FEATURES=(--features fixtures)
fi

# The release key, outside the repository, created on the first release build. The password stays
# in its file: never on a command line or in a log.
KEYSTORE=$HOME/.android/psytify-release.keystore
PASSFILE=$HOME/.android/psytify-release.pass
if [ "${RELEASE:-}" = 1 ] && [ ! -f "$KEYSTORE" ]; then
  mkdir -p "$HOME/.android"
  (
    umask 077
    head -c 24 /dev/urandom | base64 | tr -d '/+=' > "$PASSFILE"
  )
  keytool -genkeypair -keystore "$KEYSTORE" -storepass:file "$PASSFILE" -alias psytify \
    -keyalg RSA -keysize 4096 -validity 10000 -dname "CN=Psytify" >/dev/null
  echo "Created the signing key $KEYSTORE (password in $PASSFILE)."
fi

OUT=$ROOT/android-app/app/build/generated/rust
rm -rf "$OUT/jniLibs" "$OUT/kotlin"
cd "$ROOT"
"${LIMIT[@]}" cargo ndk "${TARGETS[@]}" -P 26 -o "$OUT/jniLibs" \
  build -p psytify-mobile --profile "$PROFILE" --locked "${FEATURES[@]}"

# Library mode: the bindings come from the UniFFI metadata inside the library just built, so they
# always match it.
LIB=$ROOT/target/x86_64-linux-android/$PROFILE/libpsytify_mobile.so
"${LIMIT[@]}" cargo run -q -p psytify-mobile-gen --bin uniffi-bindgen --locked -- \
  generate --library "$LIB" --language kotlin --no-format --out-dir "$OUT/kotlin"

# Gradle and AGP do not run on every JDK: use a JDK 21 unless JAVA_HOME already is one.
if ! "${JAVA_HOME:-/nonexistent}/bin/java" -version 2>&1 | grep -q '"21\.'; then
  for jdk in "$HOME"/.gradle/jdks/*-21-* "$HOME"/.local/*jdk*21*; do
    if [ -x "$jdk/bin/java" ]; then
      export JAVA_HOME=$jdk
      break
    fi
  done
fi

cd "$ROOT/android-app"
TASK=:app:assembleDebug
if [ "${RELEASE:-}" = 1 ]; then
  TASK=:app:assembleRelease
fi
status=0
"${LIMIT[@]}" ./gradlew --console=plain "$TASK" || status=$?
# The daemon holds 2 GB: stop it unless asked to keep it for the next build.
if [ "${KEEP_DAEMON:-}" != 1 ]; then
  ./gradlew --stop >/dev/null 2>&1 || true
fi
[ "$status" = 0 ] || exit "$status"

APK=$ROOT/android-app/app/build/outputs/apk/debug/app-debug.apk
if [ "${RELEASE:-}" = 1 ]; then
  # Aligned to 16 KB (Android 15 phones with 16 KB pages), then signed with the release key.
  OUTPUTS=$ROOT/android-app/app/build/outputs/apk/release
  APK=$OUTPUTS/psytify.apk
  zipalign -P 16 -f 4 "$OUTPUTS/app-release-unsigned.apk" "$OUTPUTS/aligned.apk"
  apksigner sign --ks "$KEYSTORE" --ks-pass "file:$PASSFILE" --out "$APK" "$OUTPUTS/aligned.apk"
  rm -f "$OUTPUTS/aligned.apk" "$APK.idsig"
fi
echo "APK: $APK"
if [ "${INSTALL:-}" = 1 ]; then
  adb install -r "$APK"
fi
