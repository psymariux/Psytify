//! I marchi dei provider di accesso (GitHub, Google, Discord) come SVG monocromi: l'unica eccezione
//! vettoriale alla regola dei pixel (direzione artistica, «Sign-in logos»). Fonte e licenza in
//! `design/assets/brands/README.md` (Simple Icons, CC0 1.0). Il frontend li tinge con il colore
//! dell'etichetta del pulsante.

pub const GITHUB: &[u8] = include_bytes!("../assets/brands/github.svg");
pub const GOOGLE: &[u8] = include_bytes!("../assets/brands/google.svg");
pub const DISCORD: &[u8] = include_bytes!("../assets/brands/discord.svg");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marks_are_single_color_svgs() {
        for mark in [GITHUB, GOOGLE, DISCORD] {
            let text = std::str::from_utf8(mark).unwrap_or_default();
            assert!(text.starts_with("<svg"), "un SVG");
            assert!(text.contains("fill=\"#ffffff\""), "bianco, da tingere");
        }
    }
}
