//! Writes the Kotlin design tokens. Usage: `kotlin-tokens [OUT]`; without `OUT`, the file in the
//! Android app (`psytify_mobile_gen::TOKENS_KT`, from the repository root), and then the providers'
//! marks in `psytify_mobile_gen::BRANDS_DIR`.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let out = std::env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(psytify_mobile_gen::TOKENS_KT)
    });
    if let Some(dir) = out.parent()
        && let Err(err) = std::fs::create_dir_all(dir)
    {
        eprintln!("kotlin-tokens: {}: {err}", dir.display());
        return ExitCode::FAILURE;
    }
    let kotlin = match psytify_mobile_gen::kotlin_tokens() {
        Ok(kotlin) => kotlin,
        Err(err) => {
            eprintln!("kotlin-tokens: {err}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = std::fs::write(&out, kotlin) {
        eprintln!("kotlin-tokens: {}: {err}", out.display());
        return ExitCode::FAILURE;
    }
    if std::env::args_os().nth(1).is_some() {
        return ExitCode::SUCCESS;
    }
    let brands = match psytify_mobile_gen::brand_vectors() {
        Ok(brands) => brands,
        Err(err) => {
            eprintln!("kotlin-tokens: {err}");
            return ExitCode::FAILURE;
        }
    };
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(psytify_mobile_gen::BRANDS_DIR);
    for (name, xml) in brands {
        if let Err(err) = std::fs::write(dir.join(&name), xml) {
            eprintln!("kotlin-tokens: {name}: {err}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
