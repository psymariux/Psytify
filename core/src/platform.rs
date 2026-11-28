//! Hooks to the platform that the core cannot reach by itself (on Android the shell in `android/`)
//! register them at startup, before `run`. On desktop the defaults apply.

use std::sync::OnceLock;

/// Who opens an address in the system browser, where the `open` crate does not reach (on Android the
/// shell, with an Intent). Returns `false` if it fails.
type UrlOpener = Box<dyn Fn(&str) -> bool + Send + Sync>;
static URL_OPENER: OnceLock<UrlOpener> = OnceLock::new();

/// Registers who opens an address in the system browser. The first call counts.
pub fn set_url_opener(open: impl Fn(&str) -> bool + Send + Sync + 'static) {
    let _ = URL_OPENER.set(Box::new(open));
}

/// Opens `url` in the browser: with the shell's hook if there is one, otherwise with the `open` crate.
pub fn open_url(url: &str) -> bool {
    match URL_OPENER.get() {
        Some(open) => open(url),
        None => open::that_detached(url).is_ok(),
    }
}

/// Opens a folder in the system file manager (`xdg-open`, Windows Explorer). On Android there is no
/// file manager to open from here: returns `false`, and the interface does not offer the button.
pub fn open_path(path: &std::path::Path) -> bool {
    #[cfg(target_os = "android")]
    {
        let _ = path;
        false
    }
    #[cfg(not(target_os = "android"))]
    open::that_detached(path).is_ok()
}
