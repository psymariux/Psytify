//! Psytify design: i token del design (colori `[u8; 4]`, spazi, tipografia, layout, movimento) e i
//! fogli di sprite con l'ingranditore intero nearest-neighbour, come dati semplici senza framework
//! grafico. Ogni frontend li converte nei propri tipi: così Iced (Android) e GPUI (desktop)
//! condividono la stessa identità pixel art.
//!
//! - [`tokens`]: costanti `pub const` (palette, spazi, forme, tipi, layout, movimento, livelli z).
//!   Sorgente dei valori: `docs/design/PSYTIFY_DESIGN_TOKENS.md`.
//! - [`scale`]: pixel di dispositivo per pixel d'arte (`art_px`) e allineamento alla griglia.
//! - [`sprite`]: foglio di sprite incorporato (decodificato una volta), ingrandimento intero e cache
//!   LRU con capacità dichiarata.
//! - [`contrast`]: luminanza relativa e rapporto di contrasto WCAG 2.x.
//! - [`brand`]: i marchi SVG monocromi dei provider di accesso (l'unica eccezione vettoriale).
//!
//! Nessun tipo di GPUI o Iced entra qui; la pipeline completa è in
//! `docs/design/PSYTIFY_PIXEL_ART_GUIDELINES.md` (sezione 10).

pub mod brand;
pub mod contrast;
pub mod scale;
pub mod sprite;
pub mod tokens;
