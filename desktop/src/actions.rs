//! Global desktop actions and shortcuts, shared by the gallery and the app.
//!
//! Tab and Shift+Tab move through the controls (the order follows the reading), Ctrl+K puts the focus on the
//! search, Ctrl+Q quits, Alt+← and Alt+→ move through the pages' history, Ctrl+, opens the settings,
//! Ctrl+= / Ctrl+- / Ctrl+0 the UI zoom, Ctrl+B the sidebar. The transport
//! commands work in the `Shell` context (the app's root) and,
//! when the key is a character (Space, Q, T, V, F), not while typing in a text field. Q and T
//! open the panel on the queue or on the lyrics, F the immersive view, V (with the `video` feature) the
//! immersive view on the video.

use gpui::{App, KeyBinding, actions};

actions!(
    psytify,
    [
        FocusNext,
        FocusPrev,
        FocusSearch,
        Quit,
        TogglePlay,
        NextTrack,
        PreviousTrack,
        ToggleShuffle,
        CycleRepeat,
        ToggleQueue,
        ToggleLyrics,
        ToggleFocus,
        GoBack,
        GoForward,
        OpenSettings,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ToggleSidebar
    ]
);

// V opens and closes the immersive view on the video (only with the `video` feature).
#[cfg(feature = "video")]
actions!(psytify, [ToggleVideoFocus]);

/// The context of the app's root: the transport shortcuts live here.
pub const SHELL_CONTEXT: &str = "Shell";

/// Shortcuts that work everywhere (also in the gallery); to register once at start-up.
pub fn bind_global(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", FocusNext, None),
        KeyBinding::new("shift-tab", FocusPrev, None),
        KeyBinding::new("ctrl-k", FocusSearch, None),
        KeyBinding::new("ctrl-q", Quit, None),
    ]);
}

/// The app's transport shortcuts. Space, Q, T, V and F are characters: outside the text fields.
pub fn bind_shell(cx: &mut App) {
    let plain = format!("{SHELL_CONTEXT} && !PsytifyTextInput");
    cx.bind_keys([
        KeyBinding::new("space", TogglePlay, Some(plain.as_str())),
        KeyBinding::new("q", ToggleQueue, Some(plain.as_str())),
        KeyBinding::new("t", ToggleLyrics, Some(plain.as_str())),
        KeyBinding::new("f", ToggleFocus, Some(plain.as_str())),
        KeyBinding::new("ctrl-right", NextTrack, Some(SHELL_CONTEXT)),
        KeyBinding::new("ctrl-left", PreviousTrack, Some(SHELL_CONTEXT)),
        KeyBinding::new("ctrl-s", ToggleShuffle, Some(SHELL_CONTEXT)),
        KeyBinding::new("ctrl-r", CycleRepeat, Some(SHELL_CONTEXT)),
        KeyBinding::new("alt-left", GoBack, Some(SHELL_CONTEXT)),
        KeyBinding::new("alt-right", GoForward, Some(SHELL_CONTEXT)),
        KeyBinding::new("ctrl-,", OpenSettings, Some(SHELL_CONTEXT)),
        // Ctrl+= and Ctrl++ (with or without Shift on most layouts), Ctrl+- and Ctrl+0: the UI zoom.
        KeyBinding::new("ctrl-=", ZoomIn, Some(SHELL_CONTEXT)),
        KeyBinding::new("ctrl-+", ZoomIn, Some(SHELL_CONTEXT)),
        KeyBinding::new("ctrl--", ZoomOut, Some(SHELL_CONTEXT)),
        KeyBinding::new("ctrl-0", ZoomReset, Some(SHELL_CONTEXT)),
        KeyBinding::new("ctrl-b", ToggleSidebar, Some(SHELL_CONTEXT)),
    ]);
    #[cfg(feature = "video")]
    cx.bind_keys([KeyBinding::new("v", ToggleVideoFocus, Some(plain.as_str()))]);
}
