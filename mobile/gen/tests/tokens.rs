//! The Kotlin tokens committed in the Android app match the Rust tokens.

#[test]
fn the_committed_kotlin_tokens_are_up_to_date() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(psytify_mobile_gen::TOKENS_KT);
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        Ok(committed) == psytify_mobile_gen::kotlin_tokens(),
        "{} is stale: run `cargo run -p psytify-mobile-gen --bin kotlin-tokens`",
        path.display()
    );
}

#[test]
fn the_committed_brand_marks_are_up_to_date() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(psytify_mobile_gen::BRANDS_DIR);
    for (name, xml) in psytify_mobile_gen::brand_vectors().unwrap() {
        let committed = std::fs::read_to_string(dir.join(&name)).unwrap_or_default();
        assert!(
            committed == xml,
            "{name} is stale: run `cargo run -p psytify-mobile-gen --bin kotlin-tokens`"
        );
    }
}
