//! `psytify-iced`, il binario di sviluppo del client Iced: avvia la libreria con la finestra del
//! desktop. Il desktop distribuito è GPUI (`desktop/`, binario `psytify`); questa finestra serve a
//! provare l'app del telefono senza Android: con `PSYTIFY_PHONE=1` usa l'impaginazione del telefono
//! in una finestra della sua misura.

// Su Windows, in release, niente console dietro la finestra. Solo lì serve: su Linux non esiste.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() -> iced::Result {
    psytify_iced::prepare_process();
    let phone = std::env::var_os("PSYTIFY_PHONE").is_some_and(|value| value == "1");
    psytify_iced::run(if phone { psytify_iced::Platform::Phone } else { psytify_iced::Platform::Desktop })
}
