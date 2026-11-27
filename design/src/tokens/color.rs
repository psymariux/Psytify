//! Palette (sezione 1 del documento dei token): dodici valori e l'unico colore traslucido (`SCRIM`).

/// Colore sRGB non premoltiplicato, byte `[R, G, B, A]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    /// Colore opaco da tre canali.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b, 255])
    }

    /// Stesso colore con un altro alfa (0 = trasparente, 255 = opaco).
    pub const fn with_alpha(self, alpha: u8) -> Self {
        Self([self.0[0], self.0[1], self.0[2], alpha])
    }

    pub const fn r(self) -> u8 {
        self.0[0]
    }

    pub const fn g(self) -> u8 {
        self.0[1]
    }

    pub const fn b(self) -> u8 {
        self.0[2]
    }

    pub const fn a(self) -> u8 {
        self.0[3]
    }

    /// I quattro byte `[R, G, B, A]`.
    pub const fn bytes(self) -> [u8; 4] {
        self.0
    }

    /// Valore `0xRRGGBB` senza alfa (comodo per i framework che usano `u32` esadecimali).
    pub const fn hex(self) -> u32 {
        ((self.0[0] as u32) << 16) | ((self.0[1] as u32) << 8) | self.0[2] as u32
    }
}

// Otto grigi che portano la gerarchia, dal più scuro al più chiaro.
/// Tela del contenuto, pozzetti (fill dei campi), ombre flottanti, testo su fill chiari.
pub const DEEP_BLACK: Rgba = Rgba::rgb(0, 0, 0);
/// Chrome: barra del titolo, laterale, pannello, barra del player, tab bar del telefono.
pub const NEAR_BLACK: Rgba = Rgba::rgb(14, 13, 12);
/// Hover, card, menu, tooltip, dialoghi, campo del segnaposto copertina.
pub const GRAPHITE: Rgba = Rgba::rgb(26, 25, 23);
/// Riga selezionata, fill premuto, separatori di regione, binario dello slider, ledge secondario.
pub const CHARCOAL: Rgba = Rgba::rgb(42, 40, 38);
/// Bordo dei livelli flottanti, separatore forte, ombreggiatura degli sprite (solo decorativo).
pub const IRON: Rgba = Rgba::rgb(61, 58, 55);
/// Bordi dei controlli (>= 3:1), testo e icone disabilitati, ledge primario, equalizzatore a riposo.
pub const STONE: Rgba = Rgba::rgb(120, 116, 109);
/// Testo terziario: metadati, durate, segnaposto, numeri di traccia.
pub const MEDIUM_GREY: Rgba = Rgba::rgb(154, 149, 141);
/// Testo secondario: artisti, icone a riposo, etichette di navigazione a riposo.
pub const SOFT_GREY: Rgba = Rgba::rgb(189, 184, 175);
/// Testo primario, fill del pulsante primario, icone attive, riempimento del seek.
pub const WARM_WHITE: Rgba = Rgba::rgb(236, 232, 224);
/// Anello di focus, cursore del testo, hover del pulsante primario, pomello del seek, riflessi.
pub const PURE_WHITE: Rgba = Rgba::rgb(255, 255, 255);
/// Solo stato semantico (sezione 1.3): riproduzione attiva, navigazione selezionata, toggle accesi, live.
pub const LIME: Rgba = Rgba::rgb(198, 242, 78);
/// Solo errori e azioni distruttive.
pub const ERROR: Rgba = Rgba::rgb(255, 107, 94);

/// Sfondo di dialoghi e sheet: `DEEP_BLACK` al 72 % (alfa 184). Unico colore traslucido.
pub const SCRIM: Rgba = DEEP_BLACK.with_alpha(184);

// Alias semantici (sezione 1.1).
pub const SURFACE_CANVAS: Rgba = DEEP_BLACK;
pub const SURFACE_CHROME: Rgba = NEAR_BLACK;
pub const SURFACE_RAISED: Rgba = GRAPHITE;
pub const SURFACE_SELECTED: Rgba = CHARCOAL;
pub const SURFACE_WELL: Rgba = DEEP_BLACK;
pub const LINE_SUBTLE: Rgba = CHARCOAL;
pub const LINE_STRONG: Rgba = IRON;
pub const CONTROL_BORDER: Rgba = STONE;
pub const CONTROL_BORDER_HOVER: Rgba = SOFT_GREY;
pub const FOCUS_RING: Rgba = PURE_WHITE;
pub const LEDGE_PRIMARY: Rgba = STONE;
pub const SHADOW_FLOAT: Rgba = DEEP_BLACK;
pub const TEXT_PRIMARY: Rgba = WARM_WHITE;
pub const TEXT_SECONDARY: Rgba = SOFT_GREY;
pub const TEXT_TERTIARY: Rgba = MEDIUM_GREY;
pub const TEXT_DISABLED: Rgba = STONE;
pub const TEXT_ON_LIGHT: Rgba = DEEP_BLACK;
pub const TEXT_ERROR: Rgba = ERROR;
pub const ICON_IDLE: Rgba = SOFT_GREY;
pub const ICON_ACTIVE: Rgba = WARM_WHITE;
pub const ICON_DISABLED: Rgba = STONE;
pub const STATE_ON: Rgba = LIME;
pub const LEDGE_SECONDARY: Rgba = CHARCOAL;
pub const CARET: Rgba = PURE_WHITE;

/// Le quattro superfici su cui appare il testo (sezione 1.2), dalla più scura alla più chiara.
pub const SURFACES: [Rgba; 4] = [SURFACE_CANVAS, SURFACE_CHROME, SURFACE_RAISED, SURFACE_SELECTED];

/// I dodici colori grezzi della palette, nell'ordine della tabella.
pub const PALETTE: [(&str, Rgba); 12] = [
    ("DEEP_BLACK", DEEP_BLACK),
    ("NEAR_BLACK", NEAR_BLACK),
    ("GRAPHITE", GRAPHITE),
    ("CHARCOAL", CHARCOAL),
    ("IRON", IRON),
    ("STONE", STONE),
    ("MEDIUM_GREY", MEDIUM_GREY),
    ("SOFT_GREY", SOFT_GREY),
    ("WARM_WHITE", WARM_WHITE),
    ("PURE_WHITE", PURE_WHITE),
    ("LIME", LIME),
    ("ERROR", ERROR),
];
