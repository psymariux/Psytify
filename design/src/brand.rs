//! Access provider brands (GitHub, Google, Discord) as monochrome SVG: the only vector exception
//! to the pixel rule (art direction, «Sign-in logos»). Source and licence in
//! `design/assets/brands/README.md` (Simple Icons, CC0 1.0). The frontend tints them with the colour
//! of the button label.

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
