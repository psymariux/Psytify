//! Psytify: player per il catalogo Psytify (Iced + rodio), come libreria. Il desktop la avvia da
//! `main.rs`, Android dal crate `android/` (che le passa la cartella dei dati dell'app).
//!
//! Il catalogo arriva solo dal backend descritto in docs/BACKEND.md; l'audio si scarica dagli URL
//! firmati che il backend restituisce.

mod api;
mod app;
mod auth;
mod config;
mod error;
mod feed;
mod jam;
#[cfg(target_os = "linux")]
mod launch;
mod player;
mod system_media;
mod ui;

use std::sync::OnceLock;

use iced::{Size, window};

use app::App;

/// Dove l'app è avviata: cambia la finestra e l'impaginazione.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Finestra senza cornice di sistema, almeno 1100 × 700.
    Desktop,
    /// Telefono: tutto lo schermo, impaginazione compatta con la barra in basso.
    Phone,
}

pub use config::{set_data_dir, set_default_api_url};

/// Chi mostra e nasconde la tastiera del sistema. Iced 0.13 non lo chiede mai alla finestra: su
/// Android lo fa il guscio (`android/`), sul desktop non serve.
static SOFT_KEYBOARD: OnceLock<Box<dyn Fn(bool) + Send + Sync>> = OnceLock::new();

/// Registra chi apre (`true`) e chiude (`false`) la tastiera del sistema. Vale la prima chiamata.
pub fn set_soft_keyboard(toggle: impl Fn(bool) + Send + Sync + 'static) {
    let _ = SOFT_KEYBOARD.set(Box::new(toggle));
}

/// Apre o chiude la tastiera del sistema, se qualcuno sa farlo.
pub(crate) fn soft_keyboard(show: bool) {
    if let Some(toggle) = SOFT_KEYBOARD.get() {
        toggle(show);
    }
}

/// Lo spazio che le barre del sistema (stato, gesti o pulsanti, notch) coprono ai bordi della
/// finestra, in px logici.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// Chi legge quegli spazi dal sistema (su Android il guscio, via JNI).
static SYSTEM_INSETS: OnceLock<Box<dyn Fn() -> Option<Insets> + Send + Sync>> = OnceLock::new();

/// Registra chi legge gli spazi delle barre del sistema. `None` se non li conosce ancora (la
/// finestra non è ancora attaccata). Vale la prima chiamata.
pub fn set_insets_provider(read: impl Fn() -> Option<Insets> + Send + Sync + 'static) {
    let _ = SYSTEM_INSETS.set(Box::new(read));
}

/// Gli spazi delle barre del sistema, se qualcuno sa leggerli.
pub(crate) fn system_insets() -> Option<Insets> {
    SYSTEM_INSETS.get().and_then(|read| read())
}

/// Prepara l'ambiente del processo. Va chiamata prima di qualunque thread: glibc e il loader Vulkan
/// leggono l'ambiente solo all'avvio (vedi `launch.rs`).
pub fn prepare_process() {
    #[cfg(target_os = "linux")]
    launch::relaunch_with_lean_env();
}

/// Avvia l'app e torna quando la finestra si chiude.
pub fn run(platform: Platform) -> iced::Result {
    app::set_platform(platform);
    let window = match platform {
        // Niente MSAA: in iced_wgpu serve solo alle mesh del canvas, che qui non c'è. Con
        // l'antialiasing Iced chiede la GPU ad alte prestazioni: sui portatili ibridi (NVIDIA +
        // integrata) su Wayland la swapchain della GPU dedicata non presenta i frame e la finestra
        // resta ferma al primo frame con la CPU al 100%. Senza, si usa la GPU del compositor.
        Platform::Desktop => window::Settings {
            size: Size::new(1280.0, 800.0),
            min_size: Some(Size::new(1100.0, 700.0)),
            // La cornice la disegna l'app: barra superiore trascinabile e tre controlli (ui::top_bar).
            decorations: false,
            // La chiusura passa da App::update per salvare coda, volume e schermata.
            exit_on_close_request: false,
            // L'`app_id` di Wayland (e la classe di X11) deve coincidere con `psytify.desktop`:
            // così il menu e la barra delle applicazioni associano icona e finestra.
            #[cfg(target_os = "linux")]
            platform_specific: window::settings::PlatformSpecific {
                application_id: "psytify".into(),
                ..window::settings::PlatformSpecific::default()
            },
            ..window::Settings::default()
        },
        // Su Android la finestra è lo schermo e la misura la decide il sistema; sul desktop
        // (`PSYTIFY_PHONE=1`) è una finestra della misura di un telefono, per provare la vista.
        Platform::Phone => window::Settings {
            size: Size::new(412.0, 892.0),
            exit_on_close_request: false,
            ..window::Settings::default()
        },
    };
    iced::application(App::title, App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .font(include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-Medium.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice())
        .default_font(ui::theme::FONT)
        .window(window)
        .run_with(App::new)
}
