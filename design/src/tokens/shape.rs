//! Angoli, bordi, ledge e ombre (sezione 4). Larghezze e offset in pixel d'arte (AP): si disegnano
//! `ap_device` pixel fisici per AP (vedi `scale::art_px`). Niente raggio: l'arrotondamento è una tacca a gradini.

/// Nessun raggio di curvatura, mai (`RADIUS_NONE = 0`).
pub const RADIUS_NONE: f32 = 0.0;

/// Forma di un angolo, tutta sulla griglia AP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    /// Copertine, righe, tile, pannelli, separatori.
    Square,
    /// Taglio di 1 AP per angolo: pulsanti, campi, badge, tooltip, toast, toggle.
    Notch1,
    /// Taglio a due gradini (1 AP + 1 AP, 45 gradi): menu, dialoghi, sheet, card.
    Notch2,
    /// Cerchio rasterizzato a risoluzione AP (diametro = lato / AP): foto artista, avatar.
    PixelCircle,
}

/// Quanti AP si tagliano per angolo in ciascuna forma a tacca.
pub const NOTCH_0: u32 = 0;
pub const NOTCH_1: u32 = 1;
pub const NOTCH_2: u32 = 2;

/// Unica larghezza di bordo e separatore.
pub const BORDER_W: u32 = 1;
pub const FOCUS_RING_W: u32 = 1;
/// Spazio fra il bordo del controllo e l'anello (ci passa la superficie).
pub const FOCUS_RING_GAP: u32 = 1;
/// Barra di navigazione selezionata e marcatore d'errore dei toast.
pub const MARKER_W: u32 = 2;

/// Spessore della ledge dura sotto i pulsanti primario e secondario (sfocatura 0): (x, y) in AP.
pub const LEDGE_OFFSET: (u32, u32) = (1, 1);
/// Ombra dura di menu, dialoghi, toast, ghost del drag (sfocatura 0, colore `SHADOW_FLOAT`): (x, y) in AP.
pub const SHADOW_FLOAT_OFFSET: (u32, u32) = (2, 2);
/// La sfocatura è 0 ovunque.
pub const BLUR: f32 = 0.0;
