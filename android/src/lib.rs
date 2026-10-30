//! Psytify per Android. Android carica `libpsytify_android.so` in una NativeActivity e chiama
//! `android_main` sul thread dell'app: qui si consegna l'`AndroidApp` a winit (tramite la patch di
//! `iced_winit`), si sceglie la cartella dei dati, si collegano la tastiera, le barre del sistema
//! e il browser e si avvia il client con l'impaginazione da telefono.

mod browser;
mod insets;
mod media;

use winit::platform::android::activity::AndroidApp;

/// Il backend di una configurazione nuova: `PSYTIFY_DEFAULT_API_URL` alla compilazione (la build di
/// distribuzione ci mette il proprio backend), altrimenti il PC che ospita l'emulatore.
const API_URL: &str = match option_env!("PSYTIFY_DEFAULT_API_URL") {
    // Una variabile definita ma vuota (CI senza backend di distribuzione) vale come assente.
    Some(url) if !url.is_empty() => url,
    _ => "http://10.0.2.2:8080",
};

/// L'ingresso che la NativeActivity cerca per nome: deve restare senza «mangling», per questo
/// `no_mangle`. Insieme ai puntatori JNI di `insets.rs` e `browser.rs` è l'unico codice `unsafe`
/// del progetto, e sta fuori dal workspace del desktop.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    if let Some(dir) = app.internal_data_path() {
        psytify::set_data_dir(dir);
    }
    psytify::set_default_api_url(API_URL.to_owned());
    // Iced 0.13 non apre mai la tastiera del sistema: l'app la chiede quando entra in Cerca.
    let keyboard = app.clone();
    psytify::set_soft_keyboard(move |show| {
        if show { keyboard.show_soft_input(true) } else { keyboard.hide_soft_input(true) }
    });
    let bars = app.clone();
    psytify::set_insets_provider(move || insets::read(&bars));
    // Il login con Clerk passa dal browser: lo apre un Intent.
    let activity = app.clone();
    psytify::set_url_opener(move |url| browser::open(&activity, url));
    // La sessione multimediale: schermata di blocco, notifica e cuffie.
    media::start(&app);
    psytify::set_phone_media(media::show);
    psytify::set_phone_installer(media::install);
    psytify::set_phone_update_notifier(media::notify_update);
    let _ = iced_winit::ANDROID_APP.set(app);
    if let Err(err) = psytify::run(psytify::Platform::Phone) {
        log::error!("Psytify si è chiuso: {err}");
    }
}
