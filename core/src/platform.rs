//! Agganci verso la piattaforma che il core non sa raggiungere da solo: chi li conosce (su Android il
//! guscio in `android/`) li registra all'avvio, prima di `run`. Sul desktop valgono i predefiniti.

use std::sync::OnceLock;

/// Chi apre un indirizzo nel browser del sistema, dove il crate `open` non arriva (su Android il
/// guscio, con un Intent). Restituisce `false` se non ci riesce.
type UrlOpener = Box<dyn Fn(&str) -> bool + Send + Sync>;
static URL_OPENER: OnceLock<UrlOpener> = OnceLock::new();

/// Registra chi apre un indirizzo nel browser del sistema. Vale la prima chiamata.
pub fn set_url_opener(open: impl Fn(&str) -> bool + Send + Sync + 'static) {
    let _ = URL_OPENER.set(Box::new(open));
}

/// Apre `url` nel browser: con l'aggancio del guscio se c'è, altrimenti col crate `open`.
pub fn open_url(url: &str) -> bool {
    match URL_OPENER.get() {
        Some(open) => open(url),
        None => open::that_detached(url).is_ok(),
    }
}

/// Apre una cartella nel gestore di file del sistema (`xdg-open`, Esplora risorse). Su Android non
/// c'è un gestore da aprire da qui: restituisce `false`, e l'interfaccia non offre il tasto.
pub fn open_path(path: &std::path::Path) -> bool {
    #[cfg(target_os = "android")]
    {
        let _ = path;
        false
    }
    #[cfg(not(target_os = "android"))]
    open::that_detached(path).is_ok()
}
