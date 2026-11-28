# Psytify on Android

The Android app is `android-app/` (Kotlin, Jetpack Compose) on top of the Rust core, through the
UniFFI bridge in `mobile/`. The desktop is the GPUI app in `desktop/`.

## The Kotlin app (`android-app/`)

Jetpack Compose on top of the Rust core, through the UniFFI bridge in `mobile/` (`psytify-mobile`).
It replaced the Iced app, which is gone from the tree. Plan and stage notes in
`docs/transformation/android-kotlin.md`.

```bash
QUICK=1 INSTALL=1 scripts/android-kotlin.sh   # debug, x86_64 only, no LTO: the emulator
scripts/android-kotlin.sh                     # debug, arm64-v8a + x86_64, full LTO
RELEASE=1 scripts/android-kotlin.sh           # release: R8, full LTO, both ABIs, signed
FIXTURES=1 QUICK=1 scripts/android-kotlin.sh  # fake social and jam views (debug only)
```

- **Debug** is `dev.psytify.app.kotlin` (debug key): it installs next to the released app.
- **Release** is `dev.psytify.app`, the id the Iced app had, aligned to 16 KB and signed by the script
  with `~/.android/psytify-release.keystore` and its password file (the password is read from the
  file, never on a command line). It installs over the Iced releases and keeps their data: both apps keep `config.json` and `session.json` in the app's
  `filesDir`, and the downloads index holds absolute paths, so the Iced downloads
  (`filesDir/Download`) still play; new downloads go to the app's external Music folder. Nothing is
  migrated.
- **Version**: `versionName` is the release tag without the `v` (`PSYTIFY_BUILD_TAG`, else 0.1.0)
  and `versionCode` is `2·2²⁴ + major·2¹⁶ + minor·2⁸ + patch`, above the Iced APK's `2²⁴ + …`.
- **R8** shrinks the release; `android-app/app/proguard-rules.pro` keeps JNA and the generated
  bindings (`dev.psytify.app.rust`), which JNA reads through reflection and calls from Rust.
- **Self-update**: Profilo → Impostazioni → Informazioni → «Aggiornamento». «Controlla ora» asks the backend
  for the latest release, «Scarica» downloads the `-android.apk` into the app's files (the
  ten-step bar), then the app opens a `PackageInstaller` session and Android asks to confirm
  (`REQUEST_INSTALL_PACKAGES`; the first time Android asks to allow installs from Psytify). A
  release found while the app stays open for hours is a notification. The package must be signed
  with the same key, or Android refuses it and the app says «Aggiornamento non installato».
- **CI** (`public/.github/workflows/android.yml`): the quick debug APK on pull requests, the debug
  APK for both ABIs on `main`, the signed release APK in the release workflow.
- **Accessibility**: every control has a TalkBack label and state, rows and controls are at least
  48 dp and grow with the font size (pixel type stays on the art grid), the system's back gesture
  previews the closing player, and the app follows the system's removed animations and battery
  saver live.

## Requirements

- Android SDK in `~/Android/Sdk` (or `$ANDROID_HOME`) with:
  - `platforms;android-35`
  - `build-tools;35.0.0`
  - `platform-tools`
  - `ndk;27.2.12479018` (`scripts/android-env.sh` sets it up)
- The Rust targets: `rustup target add aarch64-linux-android x86_64-linux-android`.
- `cargo install cargo-ndk --locked`.
- A JDK 21 for Gradle, and the JDK's `keytool` for the signing key.

## Build and backend

The commands are in the first section. The backend of a new configuration is, in order:

```bash
scripts/android-kotlin.sh http://192.168.1.20:8080 # an address given to the script
# else PSYTIFY_DEFAULT_API_URL from .env.build; with QUICK=1, http://10.0.2.2:8080 (the PC seen
# from the emulator)
```

The APK is `android-app/app/build/outputs/apk/{debug/app-debug.apk,release/psytify.apk}`. The address
is stored in the configuration on first launch. To change it, rebuild with another URL and clear the
app's data (Android Settings, Apps, Psytify, Storage, Clear data).

A backend that listens on `0.0.0.0` (`BIND_ADDR`) is reachable from a phone through your computer's
IP, with the firewall open on the port. Plain `http://` is allowed for the home network; use
`https://` in production.

## Signing key

The first release build creates two files:

- `~/.android/psytify-release.keystore`, the key;
- `~/.android/psytify-release.pass`, a random password with mode 600.

They live outside the repository and must never be committed. Keep them: Android installs an update
only if it is signed with the same key. If you lose them, the app has to be uninstalled and installed
again.

## Install and try it

- Phone over USB, with USB debugging on: `INSTALL=1 scripts/android-kotlin.sh`, or
  `adb install -r <apk>`.
- No cable: copy the APK to the phone and open it. Android asks to allow apps from unknown sources.
- Emulator (no sound: always start it with `-no-audio`):

```bash
avdmanager create avd -n psytify -k "system-images;android-35;google_apis;x86_64" -d pixel_7
emulator -avd psytify -no-snapshot -no-audio &
QUICK=1 INSTALL=1 scripts/android-kotlin.sh
adb shell am start -n dev.psytify.app.kotlin/dev.psytify.app.MainActivity
adb logcat | grep -iE "psytify|AndroidRuntime"
```

If the emulator cannot find the AVD, look where `avdmanager` wrote it (for example
`~/.config/.android/avd`) and pass that path in `ANDROID_AVD_HOME`.

## Known limits

- **Video clips**: there are none on Android (the desktop uses GStreamer, which Android lacks).
- **Downloads** go to the app's external Music folder (older ones stay in the app's private
  folder); uninstalling the app removes the private ones.
