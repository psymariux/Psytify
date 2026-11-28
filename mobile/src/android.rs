//! What a Kotlin app has to give the native side that a NativeActivity gives by itself: the JavaVM
//! and the application context in `ndk-context`, which cpal (AAudio) reads when it opens the audio
//! stream. The app calls `NativeContext.init(applicationContext)` once, before `PsytifyCore.start`.
//!
//! The only `unsafe` code of the crate written by hand is here.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use jni_sys::{JNIEnv, JavaVM, jclass, jobject};

static READY: AtomicBool = AtomicBool::new(false);

/// `ndk-context` has been filled: the audio output can open.
pub(crate) fn ready() -> bool {
    READY.load(Ordering::Acquire)
}

/// `dev.psytify.app.core.NativeContext.init(Context)`. The first call wins; later ones do nothing.
/// The context gets a global reference that is never released: it lives as long as the process,
/// like the application context it points to.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_psytify_app_core_NativeContext_init(
    env: *mut JNIEnv,
    _class: jclass,
    context: jobject,
) {
    if env.is_null() || context.is_null() || READY.load(Ordering::Acquire) {
        return;
    }
    // SAFETY: `env` is the JNIEnv of the calling thread, valid for the duration of this native
    // call; its function table is filled by the JVM. `context` is a live local reference passed by
    // the JVM. `GetJavaVM` writes the process's VM pointer, valid for the life of the process, and
    // `NewGlobalRef` returns a reference valid until it is deleted, which it never is. Both are
    // handed to `ndk-context` exactly once (guarded by `READY`), as its contract requires.
    unsafe {
        let Some(functions) = (*env).as_ref() else { return };
        // Both calls are in the JNI 1.1 table, which every Android VM has.
        let (get_vm, new_global) = (functions.v1_1.GetJavaVM, functions.v1_1.NewGlobalRef);
        let mut vm: *mut JavaVM = std::ptr::null_mut();
        if get_vm(env, &mut vm) != jni_sys::JNI_OK || vm.is_null() {
            return;
        }
        let global = new_global(env, context);
        if global.is_null() {
            return;
        }
        if READY.swap(true, Ordering::AcqRel) {
            return;
        }
        ndk_context::initialize_android_context(vm.cast::<c_void>(), global.cast::<c_void>());
    }
}
