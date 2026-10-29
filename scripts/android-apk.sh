#!/usr/bin/env bash
# Compila l'APK di Psytify per Android (aarch64 per i telefoni, x86_64 per l'emulatore).
#
# Uso: [QUICK=1] scripts/android-apk.sh [URL_DEL_BACKEND]
#   QUICK=1          solo x86_64 senza LTO, per l'emulatore (android/target-quick)
#   URL_DEL_BACKEND  backend di una configurazione nuova, per esempio http://192.168.1.20:8080
#                    (il PC sulla rete locale). Senza: PSYTIFY_DEFAULT_API_URL da `.env.build` (vedi
#                    `.env.build.example`) e, se manca, http://10.0.2.2:8080, il PC visto dall'emulatore.
#
# Richiede: Android SDK in $ANDROID_HOME (default ~/Android/Sdk) con NDK 27, i target Rust
# aarch64-linux-android e x86_64-linux-android, cargo-apk, un JDK (keytool). Vedi docs/ANDROID.md.
#
# La firma: alla prima esecuzione crea ~/.android/psytify-release.keystore con una password casuale
# in ~/.android/psytify-release.pass (permessi 600). Fuori dal repository: non va mai committata.
# Conservale: un aggiornamento dell'app si installa solo se firmato con la stessa chiave.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
export ANDROID_HOME=${ANDROID_HOME:-$HOME/Android/Sdk}
NDK_VERSION=${NDK_VERSION:-27.2.12479018}
export ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/$NDK_VERSION
export ANDROID_NDK_HOME=$ANDROID_NDK_ROOT
TOOLS=$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/bin
export PATH=$TOOLS:$ANDROID_HOME/platform-tools:$ANDROID_HOME/build-tools/35.0.0:$PATH
# Il compilatore C dell'NDK per le dipendenze con codice C/C++ (oboe per l'audio).
for arch in aarch64 x86_64; do
  upper=$(echo "$arch" | tr '[:lower:]' '[:upper:]')
  export "CC_${arch}_linux_android=$TOOLS/${arch}-linux-android26-clang"
  export "CXX_${arch}_linux_android=$TOOLS/${arch}-linux-android26-clang++"
  export "AR_${arch}_linux_android=$TOOLS/llvm-ar"
  export "CARGO_TARGET_${upper}_LINUX_ANDROID_LINKER=$TOOLS/${arch}-linux-android26-clang"
done

# Il backend di distribuzione sta in `.env.build`, fuori dal repository.
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
  echo "Creata la chiave di firma $KEYSTORE (password in $PASSFILE)."
fi
export CARGO_APK_RELEASE_KEYSTORE=$KEYSTORE
CARGO_APK_RELEASE_KEYSTORE_PASSWORD=$(cat "$PASSFILE")
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD

cd "$ROOT/android"
if [ "${QUICK:-}" = 1 ]; then
  # Per provare nell'emulatore: solo x86_64 e senza LTO, qualche minuto in meno.
  export CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
  export CARGO_TARGET_DIR=$ROOT/android/target-quick
  cargo apk build --release --target x86_64-linux-android
else
  cargo apk build --release
fi
APK=${CARGO_TARGET_DIR:-$ROOT/android/target}/release/apk/psytify.apk

# I cambi di configurazione che aapt non accetta per nome (vedi lo script), poi allineamento a
# 16 KB (Android 15 sui telefoni con pagine da 16 KB) e una firma nuova.
python3 "$ROOT/scripts/android-manifest.py" "$APK" "$APK.unsigned"
zipalign -P 16 -f 4 "$APK.unsigned" "$APK.aligned"
apksigner sign --ks "$KEYSTORE" --ks-pass "file:$PASSFILE" --out "$APK" "$APK.aligned"
rm -f "$APK.unsigned" "$APK.aligned" "$APK.idsig"
echo "APK: $APK"
