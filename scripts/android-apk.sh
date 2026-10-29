#!/usr/bin/env bash
# Builds the Psytify APK for Android (aarch64 for phones, x86_64 for the emulator).
#
# Usage: [QUICK=1] scripts/android-apk.sh [BACKEND_URL]
#   QUICK=1          x86_64 only, no LTO, for the emulator (android/target-quick)
#   BACKEND_URL      backend of a fresh configuration, for example http://192.168.1.20:8080
#                    (your computer on the LAN). Without it: PSYTIFY_DEFAULT_API_URL from `.env.build` (see
#                    `.env.build.example`) and, if missing, http://10.0.2.2:8080, the computer seen from the emulator.
#
# Requires: Android SDK in $ANDROID_HOME (default ~/Android/Sdk) with NDK 27, the Rust targets
# aarch64-linux-android and x86_64-linux-android, cargo-apk, a JDK (keytool). See docs/ANDROID.md.
#
# Signing: on the first run it creates ~/.android/psytify-release.keystore with a random password
# in ~/.android/psytify-release.pass (mode 600). Outside the repository: never commit it.
# Keep them: an app update installs only if it is signed with the same key.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
export ANDROID_HOME=${ANDROID_HOME:-$HOME/Android/Sdk}
NDK_VERSION=${NDK_VERSION:-27.2.12479018}
export ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/$NDK_VERSION
export ANDROID_NDK_HOME=$ANDROID_NDK_ROOT
TOOLS=$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/bin
export PATH=$TOOLS:$ANDROID_HOME/platform-tools:$ANDROID_HOME/build-tools/35.0.0:$PATH
# The NDK's C compiler for dependencies with C/C++ code (oboe for audio).
for arch in aarch64 x86_64; do
  upper=$(echo "$arch" | tr '[:lower:]' '[:upper:]')
  export "CC_${arch}_linux_android=$TOOLS/${arch}-linux-android26-clang"
  export "CXX_${arch}_linux_android=$TOOLS/${arch}-linux-android26-clang++"
  export "AR_${arch}_linux_android=$TOOLS/llvm-ar"
  export "CARGO_TARGET_${upper}_LINUX_ANDROID_LINKER=$TOOLS/${arch}-linux-android26-clang"
done

# The distribution backend lives in `.env.build`, outside the repository.
if [ -f "$ROOT/.env.build" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$ROOT/.env.build"
  set +a
fi
if [ "${1:-}" != "" ]; then
  export PSYTIFY_DEFAULT_API_URL=$1
fi

KEYSTORE=$HOME/.android/psytify-release.keystore
PASSFILE=$HOME/.android/psytify-release.pass
if [ ! -f "$KEYSTORE" ]; then
  mkdir -p "$HOME/.android"
  umask 077
  head -c 24 /dev/urandom | base64 | tr -d '/+=' > "$PASSFILE"
  keytool -genkeypair -keystore "$KEYSTORE" -storepass "$(cat "$PASSFILE")" -alias psytify \
    -keyalg RSA -keysize 4096 -validity 10000 -dname "CN=Psytify" >/dev/null
  echo "Created the signing key $KEYSTORE (password in $PASSFILE)."
fi
export CARGO_APK_RELEASE_KEYSTORE=$KEYSTORE
CARGO_APK_RELEASE_KEYSTORE_PASSWORD=$(cat "$PASSFILE")
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD

cd "$ROOT/android"
if [ "${QUICK:-}" = 1 ]; then
  # To try it in the emulator: x86_64 only and no LTO, a few minutes less.
  export CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
  export CARGO_TARGET_DIR=$ROOT/android/target-quick
  cargo apk build --release --target x86_64-linux-android
else
  cargo apk build --release
fi
APK=${CARGO_TARGET_DIR:-$ROOT/android/target}/release/apk/psytify.apk

# Il codice Java (la sessione multimediale, `android/java/`) diventa un classes.dex: cargo-apk compila
# solo Rust. javac e d8 vengono dal JDK e dall'SDK; `--release 11` è ciò che d8 legge.
JAVA_OUT=$(mktemp -d)
trap 'rm -rf "$JAVA_OUT"' EXIT
javac --release 11 -Xlint:-options -classpath "$ANDROID_HOME/platforms/android-35/android.jar" \
  -d "$JAVA_OUT/classes" $(find "$ROOT/android/java" -name '*.java')
d8 --release --min-api 26 --lib "$ANDROID_HOME/platforms/android-35/android.jar" \
  --output "$JAVA_OUT" $(find "$JAVA_OUT/classes" -name '*.class')

# The configuration changes aapt does not accept by name (see the script), then alignment to
# 16 KB (Android 15 on phones with 16 KB pages) and a fresh signature.
python3 "$ROOT/scripts/android-manifest.py" "$APK" "$APK.unsigned" "$JAVA_OUT/classes.dex"
zipalign -P 16 -f 4 "$APK.unsigned" "$APK.aligned"
apksigner sign --ks "$KEYSTORE" --ks-pass "file:$PASSFILE" --out "$APK" "$APK.aligned"
rm -f "$APK.unsigned" "$APK.aligned" "$APK.idsig"
echo "APK: $APK"
