//! Apre un indirizzo nel browser del sistema con un Intent `ACTION_VIEW`, via JNI: su Android il
//! crate `open` non sa farlo. Serve al login con Clerk, che passa dal browser e torna all'app sul
//! loopback.

use jni::JavaVM;
use jni::objects::{JObject, JValue};
use winit::platform::android::activity::AndroidApp;

/// `true` se Android ha accettato l'Intent.
pub fn open(app: &AndroidApp, url: &str) -> bool {
    // SAFETY: `vm_as_ptr` è il `JavaVM*` del processo, valido finché il processo vive.
    let Ok(vm) = (unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }) else { return false };
    let Ok(mut env) = vm.attach_current_thread() else { return false };
    let result = env.with_local_frame(8, |env| -> jni::errors::Result<()> {
        // SAFETY: `activity_as_ptr` è un riferimento globale all'Activity tenuto da
        // android-activity; qui lo si usa in prestito, senza cancellarlo.
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
        let text = env.new_string(url)?;
        let uri = env
            .call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[
                JValue::Object(&text),
            ])?
            .l()?;
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;Landroid/net/Uri;)V", &[
            JValue::Object(&action),
            JValue::Object(&uri),
        ])?;
        env.call_method(&activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
        Ok(())
    });
    if let Err(err) = &result {
        log::warn!("Browser non aperto: {err}");
        // Un'eccezione Java lasciata in sospeso farebbe cadere la prossima chiamata JNI.
        let _ = env.exception_clear();
    }
    result.is_ok()
}
