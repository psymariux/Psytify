# Patched dependencies for Android

`iced_winit/` is `iced_winit` 0.13.0 from crates.io (MIT license, see its `Cargo.toml`) with four
changes, marked "Psytify patch". The first: on Android `program::run` creates winit's event loop
with the `AndroidApp` received from `android_main` (`iced_winit::ANDROID_APP`). Without it, winit stops with
"An `AndroidApp` as passed to android_main() is required to create an `EventLoop` on Android".

The patch applies only to the `android/` project (`[patch.crates-io]` in its `Cargo.toml`): the desktop
client uses the original `iced_winit`. It has to be redone when Iced is upgraded.

Second change, in `src/conversion.rs`: on Android winit has no `platform::modifier_supplement`;
`logical_key` and `text` are used there, as the web version already does.

Third change, in `src/program.rs`: going to the background, Android destroys the native window and
gives a new one on return. Iced 0.13 uses `resumed` only for startup and left the screen black; with the
patch `suspended` stops redraws (no work or error loop in the background) and `resumed`
recreates the surface of every window and redraws. Audio keeps playing meanwhile.

Fourth change, `src/program/touch.rs` (hooked into `src/program.rs`, with
`State::forget_cursor`). Iced 0.13 widgets are designed for a mouse: a button captures the
finger and the list containing it does not scroll, and a horizontal `scrollable` captures every touch (there is a
"TODO: bubble up" in iced_widget). Here the touch does not reach the widgets as such, but becomes:

- a click if it is short;
- a pixel wheel, locked to the axis of the gesture, if the finger moves more than 8 px, with inertia
  on release;
- a right click if it stays still for half a second.

After the gesture the pointer is removed, so no "hover" is left hanging.
