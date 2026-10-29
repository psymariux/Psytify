//! Psytify desktop: avvia la libreria con la finestra del desktop. Con `PSYTIFY_PHONE=1` usa
//! l'impaginazione del telefono in una finestra della sua misura, per provarla senza Android.

// Su Windows, in release, niente console dietro la finestra. Solo lì serve: su Linux non esiste.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() -> iced::Result {
    psytify::prepare_process();
    let phone = std::env::var_os("PSYTIFY_PHONE").is_some_and(|value| value == "1");
    psytify::run(if phone { psytify::Platform::Phone } else { psytify::Platform::Desktop })
}
