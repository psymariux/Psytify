// L'audio di Android passa da oboe (C++), che `oboe-sys` collega a `libc++_static`. Dall'NDK 23
// la parte ABI di libc++ (`__cxa_pure_virtual`, le eccezioni) sta in un archivio a parte,
// `libc++abi.a`, che nessuno aggiunge: senza, `dlopen` della libreria fallisce all'avvio. Il
// linker dell'NDK la trova da solo nel sysroot del target.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-lib=c++abi");
    }
}
