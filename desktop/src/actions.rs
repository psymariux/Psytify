//! Azioni e scorciatoie globali del desktop, condivise da galleria e app.
//!
//! Tab e Maiusc+Tab scorrono i controlli (l'ordine segue la lettura), Ctrl+K porta il focus sulla
//! ricerca, Ctrl+Q chiude, Alt+← e Alt+→ scorrono la cronologia delle pagine, Ctrl+, apre le impostazioni
//! (come nel client Iced). I comandi di trasporto valgono nel contesto `Shell` (la radice dell'app) e,
//! quando il tasto è un carattere (Spazio, Q, T, V, F), non mentre si scrive in un campo di testo. Q e T
//! aprono il pannello sulla coda o sul testo, F la vista immersiva, V (con la feature `video`) la vista
//! immersiva sul videoclip (come nel client Iced).

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
        OpenSettings
    ]
);

// V apre e chiude la vista immersiva sul videoclip (solo con la feature `video`), come nel client Iced.
#[cfg(feature = "video")]
actions!(psytify, [ToggleVideoFocus]);

/// Il contesto della radice dell'app: le scorciatoie di trasporto vivono qui.
pub const SHELL_CONTEXT: &str = "Shell";

/// Scorciatoie che valgono ovunque (anche nella galleria); da registrare una volta all'avvio.
pub fn bind_global(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", FocusNext, None),
        KeyBinding::new("shift-tab", FocusPrev, None),
        KeyBinding::new("ctrl-k", FocusSearch, None),
        KeyBinding::new("ctrl-q", Quit, None),
    ]);
}

/// Scorciatoie di trasporto dell'app. Spazio, Q, T, V e F sono caratteri: fuori dai campi di testo.
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
    ]);
    #[cfg(feature = "video")]
    cx.bind_keys([KeyBinding::new("v", ToggleVideoFocus, Some(plain.as_str()))]);
}
