//! Dove stanno le barre del sistema, chiesto ad Android via JNI: barra di stato, barra dei gesti o
//! dei tre pulsanti, notch. Cambiano da telefono a telefono (e con le impostazioni di
//! navigazione), quindi l'app non può fissarle.

use jni::JavaVM;
use jni::objects::{JObject, JValue};
use psytify::Insets;
use winit::platform::android::activity::AndroidApp;

/// `Build.VERSION_CODES.R`: da qui c'è `WindowInsets.getInsets(int)`.
const API_30: i32 = 30;

/// Gli spazi in px logici, o `None` se la finestra non è ancora attaccata (o JNI fallisce).
pub fn read(app: &AndroidApp) -> Option<Insets> {
    // La stessa scala di winit: densità in dpi diviso 160.
    let scale = app.config().density().map_or(1.0, |dpi| dpi as f32 / 160.0);
    // SAFETY: `vm_as_ptr` è il `JavaVM*` del processo, valido finché il processo vive.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    // Un frame locale: i riferimenti creati qui si liberano all'uscita (la funzione si richiama).
    let found = env.with_local_frame(16, |env| -> jni::errors::Result<Option<[i32; 4]>> {
        // SAFETY: `activity_as_ptr` è un riferimento globale all'Activity tenuto da
        // android-activity; qui lo si usa in prestito, senza cancellarlo.
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
        let window = env.call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?.l()?;
        let decor = env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?.l()?;
        let insets =
            env.call_method(&decor, "getRootWindowInsets", "()Landroid/view/WindowInsets;", &[])?.l()?;
        if insets.is_null() {
            return Ok(None);
        }
        let sdk = env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()?;
        if sdk >= API_30 {
            let kind = "android/view/WindowInsets$Type";
            let bars = env.call_static_method(kind, "systemBars", "()I", &[])?.i()?;
            let cutout = env.call_static_method(kind, "displayCutout", "()I", &[])?.i()?;
            let edges = env
                .call_method(
                    &insets,
                    "getInsets",
                    "(I)Landroid/graphics/Insets;",
                    &[JValue::Int(bars | cutout)],
                )?
                .l()?;
            let mut side = |name: &str| env.get_field(&edges, name, "I").and_then(|v| v.i());
            Ok(Some([side("top")?, side("right")?, side("bottom")?, side("left")?]))
        } else {
            let mut side = |name: &str| env.call_method(&insets, name, "()I", &[]).and_then(|v| v.i());
            Ok(Some([
                side("getSystemWindowInsetTop")?,
                side("getSystemWindowInsetRight")?,
                side("getSystemWindowInsetBottom")?,
                side("getSystemWindowInsetLeft")?,
            ]))
        }
    });
    let [top, right, bottom, left] = match found {
        Ok(found) => found?,
        Err(err) => {
            log::warn!("Spazi delle barre di sistema non letti: {err}");
            let _ = env.exception_clear();
            return None;
        }
    };
    let logical = |px: i32| px as f32 / scale;
    Some(Insets { top: logical(top), right: logical(right), bottom: logical(bottom), left: logical(left) })
}
