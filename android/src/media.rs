//! La sessione multimediale di Android (schermata di blocco, notifica, cuffie): `PsytifyMedia.java`
//! mostra il brano e manda indietro i comandi. Qui si carica la classe, si passano titolo, artista,
//! posizione e copertina, e si riceve ogni comando con `Java_dev_psytify_app_PsytifyMedia_command`.
//!
//! La copertina si converte in pixel ARGB e si manda solo quando cambia il brano o arriva la
//! copertina: gli aggiornamenti di posizione e Play/Pausa sono solo testi e numeri.

use std::path::Path;
use std::sync::{Mutex, OnceLock, PoisonError};

use jni::objects::{GlobalRef, JClass, JObject, JValue};
use jni::sys::{jint, jlong};
use jni::{JNIEnv, JavaVM};
use psytify::NowPlaying;
use winit::platform::android::activity::AndroidApp;

const CLASS: &str = "dev.psytify.app.PsytifyMedia";

static APP: OnceLock<AndroidApp> = OnceLock::new();
static CLASS_REF: OnceLock<GlobalRef> = OnceLock::new();
/// Ciò che il sistema ha già: se titolo, artista, album o la presenza della copertina cambiano, si
/// rimanda anche la copertina.
static SENT: Mutex<Option<Sent>> = Mutex::new(None);

struct Sent {
    title: String,
    artist: String,
    album: Option<String>,
    artwork: bool,
}

/// Carica `PsytifyMedia` con il loader dell'app (il dex è nell'APK, e un thread nativo non lo vede
/// con `FindClass`) e crea la sessione sul thread principale.
pub fn start(app: &AndroidApp) {
    let _ = APP.set(app.clone());
    let Some(vm) = vm(app) else { return };
    let Ok(mut env) = vm.attach_current_thread() else { return };
    let result = env.with_local_frame(16, |env| -> jni::errors::Result<()> {
        // SAFETY: `activity_as_ptr` è un riferimento globale all'Activity tenuto da
        // android-activity; qui lo si usa in prestito, senza cancellarlo.
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
        let loader = env
            .call_method(&activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
            .l()?;
        let name = env.new_string(CLASS)?;
        let class = env
            .call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])?
            .l()?;
        let class = JClass::from(class);
        env.call_static_method(&class, "start", "(Landroid/app/Activity;)V", &[JValue::Object(&activity)])?;
        let _ = CLASS_REF.set(env.new_global_ref(&class)?);
        Ok(())
    });
    if let Err(err) = &result {
        log::warn!("Sessione multimediale non avviata: {err}");
        // La traccia Java va in logcat prima di pulire l'eccezione, altrimenti si perde.
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

/// Mostra il brano al sistema, o niente a player fermo.
pub fn show(now: Option<&NowPlaying<'_>>) {
    let (Some(app), Some(class)) = (APP.get(), CLASS_REF.get()) else { return };
    let Some(vm) = vm(app) else { return };
    let Ok(mut env) = vm.attach_current_thread() else { return };
    let result = env.with_local_frame(32, |env| -> jni::errors::Result<()> {
        let class = JClass::from(env.new_local_ref(class.as_obj())?);
        let Some(now) = now else {
            *SENT.lock().unwrap_or_else(PoisonError::into_inner) = None;
            env.call_static_method(&class, "hide", "()V", &[])?;
            return Ok(());
        };
        let artwork = now.artwork.is_some();
        let changed = {
            let mut sent = SENT.lock().unwrap_or_else(PoisonError::into_inner);
            let same = sent.as_ref().is_some_and(|s| {
                s.title == now.title
                    && s.artist == now.artist
                    && s.album.as_deref() == now.album
                    && s.artwork == artwork
            });
            if !same {
                *sent = Some(Sent {
                    title: now.title.to_owned(),
                    artist: now.artist.to_owned(),
                    album: now.album.map(str::to_owned),
                    artwork,
                });
            }
            !same
        };
        let title = env.new_string(now.title)?;
        let artist = env.new_string(now.artist)?;
        let album = match now.album {
            Some(album) => JObject::from(env.new_string(album)?),
            None => JObject::null(),
        };
        let (side, pixels) = match now.artwork {
            Some((side, rgba)) if changed => (side, JObject::from(argb(&env, side, rgba)?)),
            _ => (0, JObject::null()),
        };
        let duration = i64::try_from(now.duration.as_millis()).unwrap_or(i64::MAX);
        let position = i64::try_from(now.position.as_millis()).unwrap_or(i64::MAX);
        env.call_static_method(&class, "show", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;JJZZI[I)V", &[
            JValue::Object(&title),
            JValue::Object(&artist),
            JValue::Object(&album),
            JValue::Long(duration),
            JValue::Long(position),
            JValue::Bool(u8::from(now.playing)),
            JValue::Bool(u8::from(changed)),
            JValue::Int(i32::try_from(side).unwrap_or(0)),
            JValue::Object(&pixels),
        ])?;
        Ok(())
    });
    if let Err(err) = &result {
        log::warn!("Sessione multimediale: {err}");
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

/// I pixel RGBA di una copertina quadrata di lato `side`, come `int[]` ARGB per `Bitmap`. Un
/// array che non torna con il lato dà `None`: meglio nessuna copertina che una sbagliata.
fn argb<'local>(
    env: &JNIEnv<'local>,
    side: u32,
    rgba: &[u8],
) -> jni::errors::Result<jni::objects::JIntArray<'local>> {
    let expected = (side as usize) * (side as usize) * 4;
    let mut ints = Vec::with_capacity(rgba.len() / 4);
    if rgba.len() == expected {
        ints.extend(rgba.chunks_exact(4).map(|p| i32::from_be_bytes([p[3], p[0], p[1], p[2]])));
    }
    let array = env.new_int_array(i32::try_from(ints.len()).unwrap_or(0))?;
    env.set_int_array_region(&array, 0, &ints)?;
    Ok(array)
}

/// Passa l'APK scaricato a `PsytifyMedia.install`: Android mostra la conferma e installa.
pub fn install(path: &Path) {
    let (Some(app), Some(class)) = (APP.get(), CLASS_REF.get()) else { return };
    let Some(vm) = vm(app) else { return };
    let Ok(mut env) = vm.attach_current_thread() else { return };
    let result = env.with_local_frame(16, |env| -> jni::errors::Result<()> {
        let class = JClass::from(env.new_local_ref(class.as_obj())?);
        let text = path.to_string_lossy();
        let path = env.new_string(text.as_ref() as &str)?;
        env.call_static_method(&class, "install", "(Ljava/lang/String;)V", &[JValue::Object(&path)])?;
        Ok(())
    });
    if let Err(err) = &result {
        log::warn!("Installazione dell'aggiornamento non avviata: {err}");
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

/// Chiede a `PsytifyMedia.notifyUpdate` la notifica «versione nuova disponibile»: un tocco apre l'app.
pub fn notify_update(tag: &str) {
    let (Some(app), Some(class)) = (APP.get(), CLASS_REF.get()) else { return };
    let Some(vm) = vm(app) else { return };
    let Ok(mut env) = vm.attach_current_thread() else { return };
    let result = env.with_local_frame(16, |env| -> jni::errors::Result<()> {
        let class = JClass::from(env.new_local_ref(class.as_obj())?);
        let tag = env.new_string(tag)?;
        env.call_static_method(&class, "notifyUpdate", "(Ljava/lang/String;)V", &[JValue::Object(&tag)])?;
        Ok(())
    });
    if let Err(err) = &result {
        log::warn!("Notifica dell'aggiornamento non mostrata: {err}");
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

fn vm(app: &AndroidApp) -> Option<JavaVM> {
    // SAFETY: `vm_as_ptr` è il `JavaVM*` del processo, valido finché il processo vive.
    unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()
}

/// Un comando dal sistema (`PsytifyMedia.command`): il codice è quello di `phone_command`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_psytify_app_PsytifyMedia_command(_env: JNIEnv<'_>, _class: JClass<'_>, code: jint, value: jlong) {
    psytify::phone_command(code, value);
}
