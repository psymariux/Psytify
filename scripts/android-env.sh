#!/usr/bin/env bash
# Build environment for Android: SDK, NDK 27, PATH and C compiler/linker for the
# aarch64 and x86_64 targets. To be sourced by `android-apk.sh` and `android-check.sh`.
# Variables: ANDROID_HOME (default ~/Android/Sdk), NDK_VERSION.
export ANDROID_HOME=${ANDROID_HOME:-$HOME/Android/Sdk}
NDK_VERSION=${NDK_VERSION:-27.2.12479018}
export ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/$NDK_VERSION
export ANDROID_NDK_HOME=$ANDROID_NDK_ROOT
TOOLS=$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/bin
export PATH=$TOOLS:$ANDROID_HOME/platform-tools:$ANDROID_HOME/build-tools/35.0.0:$PATH
# The NDK's C compiler for crates with C code (`ring`); audio goes through AAudio (cpal).
for arch in aarch64 x86_64; do
  upper=$(echo "$arch" | tr '[:lower:]' '[:upper:]')
  export "CC_${arch}_linux_android=$TOOLS/${arch}-linux-android26-clang"
  export "CXX_${arch}_linux_android=$TOOLS/${arch}-linux-android26-clang++"
  export "AR_${arch}_linux_android=$TOOLS/llvm-ar"
  export "CARGO_TARGET_${upper}_LINUX_ANDROID_LINKER=$TOOLS/${arch}-linux-android26-clang"
done
