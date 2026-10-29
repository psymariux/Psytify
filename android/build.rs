// Android audio goes through oboe (C++), which `oboe-sys` links to `libc++_static`. Since NDK 23
// the ABI part of libc++ (`__cxa_pure_virtual`, exceptions) lives in a separate archive,
// `libc++abi.a`, which nobody adds: without it, `dlopen` of the library fails at startup. The
// NDK linker finds it by itself in the target's sysroot.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-lib=c++abi");
    }
}
