# Psytify on Android

The APK is the same client as the desktop one (`client/`, used as a library) with the phone layout
(`client/src/ui/phone/`): four tabs at the bottom, a Home and a Library designed for the phone, a mini
player, and a full-screen player with synced lyrics and the queue. The `android/` crate is only the shell: it receives the `AndroidApp` from
the NativeActivity, picks the data folder and wires up the system keyboard.

## Requirements

- Android SDK in `~/Android/Sdk` (or `$ANDROID_HOME`) with:
  - `platforms;android-35`
  - `build-tools;35.0.0`
  - `platform-tools`
  - `ndk;27.2.12479018`
- The Rust targets: `rustup target add aarch64-linux-android x86_64-linux-android`.
- `cargo install cargo-apk` (0.10).
- A JDK for `keytool` and `apksigner`.

## Build

```bash
scripts/android-apk.sh                          # backend: PSYTIFY_DEFAULT_API_URL from .env.build
QUICK=1 scripts/android-apk.sh                  # x86_64 only, no LTO, to try it in the emulator
scripts/android-apk.sh http://10.0.2.2:8080     # backend: your computer, seen from the emulator
scripts/android-apk.sh http://192.168.1.20:8080 # backend: your computer on the LAN, for a real phone
```

The APK is written to `android/target/release/apk/psytify.apk`, for aarch64 (phones) and x86_64
(emulator). The backend address is stored in the configuration on first launch. To change it:

- rebuild with another URL and uninstall the app before reinstalling it;
- or clear its data (Android Settings → Apps → Psytify → Storage → Clear data).

A backend that listens on `0.0.0.0` is reachable from the phone through your computer's IP, with the
firewall open on the port. Plain `http://` is allowed for the home network; use `https://` in
production.

## Signing key

On the first build the script creates two files:

- `~/.android/psytify-release.keystore`, the key;
- `~/.android/psytify-release.pass`, a random password with mode 600.

They live outside the repository and must never be committed. Keep them: Android installs an update
only if it is signed with the same key. If you lose them, the app has to be uninstalled and installed
again.

In CI the same key comes from the repository secrets `ANDROID_KEYSTORE_BASE64` and
`ANDROID_KEYSTORE_PASSWORD`. Without them the workflow signs with a throwaway key.

## Install

- Phone over USB, with USB debugging on: `adb install -r android/target/release/apk/psytify.apk`.
- No cable: copy the APK to the phone and open it. Android asks to allow apps from unknown sources.

## Try it in the emulator

```bash
avdmanager create avd -n psytify -k "system-images;android-35;google_apis;x86_64" -d pixel_7
emulator -avd psytify -no-snapshot &
adb install -r android/target/release/apk/psytify.apk
adb shell am start -n dev.psytify.app/android.app.NativeActivity
adb logcat | grep -iE "psytify|wgpu|AndroidRuntime"
```

If the emulator cannot find the AVD, look where `avdmanager` wrote it (for example
`~/.config/.android/avd`) and pass that path in `ANDROID_AVD_HOME`.

## Touch and gestures

Iced 0.13 widgets are designed for a mouse. The patch `src/program/touch.rs` in the vendored copy of
`iced_winit` decides the gesture before it reaches the widgets:

- **short tap**: click;
- **drag**: past 8 px it scrolls, locked to the axis of the gesture. Pages scroll vertically, shelves
  horizontally, with inertia;
- **long press** (half a second): context menu;
- **tap during inertia**: stops it without clicking.

See `android/vendor/README.md`.

## Every device

- **System bars**: the insets are read from Android over JNI (`android/src/insets.rs`):
  - status bar and notch;
  - gesture or three-button navigation.

  An estimate is used until the window is attached.
- **Configuration changes**: the manifest declares all of them, including the two bits the SDK does not
  accept by name (`scripts/android-manifest.py`). Dark theme, background colors, language, font,
  density and multi-window therefore do not recreate the activity. Recreating it would hang a Rust
  NativeActivity: `onDestroy` waits for an `android_main` that never returns.
- **Alignment**: native libraries are aligned to 16 KB (`zipalign -P 16`), as Android 15 requires on
  phones with 16 KB pages.
- **Widths**: below 420 px the featured panel puts its buttons underneath. Settings put each control
  under its description, and their tabs scroll.

## Verified (Pixel 7 emulator, Android 15, 27 October 2026)

- Launch with the splash screen, Home from the computer's backend, covers and tints.
- Finger scrolling with inertia: pages vertically and shelves horizontally.
- Taps on buttons and a long press that opens the menu inside the safe area.
- Playback from streaming (AAC), mini player with progress, full-screen player with synced lyrics and
  word-by-word highlighting.
- The four-item bottom bar, with Settings on the icon at the top right.
- Android's Back button: closes the player, then returns to the previous page.
- Search: the field stays at the top, the system keyboard opens when entering and closes when leaving. If
  you close it with Back, tap "Search" again to reopen it.
- Queue and page saved between launches.
- In the background music keeps playing and the app does not draw: 0% CPU, about 164 MB of RSS. On
  return the screen is redrawn; before the patch it stayed black (`android/vendor/README.md`).
- No hang when changing theme colors, dark theme, font size and density with the app open.
- Sizes tried:
  - 360 × 760 dp (small phone);
  - 412 × 915 dp;
  - 480 × 1040 dp;
  - 800 × 1280 dp (tablet);
  - gesture and three-button navigation.

## Known limits

- **Keyboard**: the NativeActivity does not declare a text field, so Gboard runs without suggestions or
  correction. Accented letters may not arrive (not verified).
- **Display size**: if it changes while the app is open, the scale stays the old one (winit 0.30 does
  not re-read the density) until the app is reopened.
- **Video clips**: not available (they use GStreamer, which Android lacks).
- **System controls**: no controls in the notification shade or on the lock screen. MPRIS exists only on
  Linux.
- **Downloads**: they end up in the app's private folder. They do not show up in the phone's Downloads
  folder and are lost on uninstall.
- **Orientation**: portrait only. On tablets the layout is the phone one, wider.
- **Press feedback**: buttons do not show a "pressed" state while the finger is down; the click fires when
  the finger lifts without having moved.
