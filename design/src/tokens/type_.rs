//! Scala tipografica (sezione 5). Due famiglie: Inter per tutto il testo corrente e per le stringhe
//! del catalogo; Departure Mono solo per ciò che scrive Psytify. Dimensioni e interlinea in dp,
//! spaziatura in px (può essere negativa).

/// Famiglia per il testo corrente (Inter 4.1: 400, 500, 600).
pub const FAMILY_UI: &str = "Inter";
/// Famiglia pixel (Departure Mono 1.500, Regular). Un pixel del font = 1 AP a 11 px nominali per AP.
pub const FAMILY_PIXEL: &str = "Departure Mono";

/// Maiuscole/minuscole richieste dal ruolo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCase {
    /// Come scritto (catalogo, utente).
    AsWritten,
    /// Prima lettera maiuscola.
    Sentence,
    /// Tutto maiuscolo (`TYPE_LABEL`).
    Upper,
}

/// Un ruolo tipografico.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeRole {
    pub family: &'static str,
    /// Grandezza in dp.
    pub size: f32,
    /// Altezza di riga in dp.
    pub line_height: f32,
    /// 400, 500 o 600.
    pub weight: u16,
    /// Spaziatura fra le lettere in px (+ allarga, − stringe).
    pub tracking: f32,
    /// Cifre tabulari (`tnum`).
    pub tabular: bool,
    /// Zero barrato (`zero`), solo per i numeri a font pixel.
    pub slashed_zero: bool,
    pub case: TextCase,
}

const fn inter(
    size: f32,
    line_height: f32,
    weight: u16,
    tracking: f32,
    tabular: bool,
    case: TextCase,
) -> TypeRole {
    TypeRole { family: FAMILY_UI, size, line_height, weight, tracking, tabular, slashed_zero: false, case }
}

const fn pixel(tabular: bool, slashed_zero: bool, case: TextCase) -> TypeRole {
    TypeRole {
        family: FAMILY_PIXEL,
        size: PIXEL_SIZE,
        line_height: PIXEL_LINE,
        weight: 400,
        tracking: 0.0,
        tabular,
        slashed_zero,
        case,
    }
}

/// Dimensione del tipo pixel: 11 AP (22 dp). Non esistono altre dimensioni pixel.
pub const PIXEL_SIZE: f32 = 22.0;
/// Interlinea del tipo pixel: 14 AP (28 dp).
pub const PIXEL_LINE: f32 = 28.0;
/// Dimensione e interlinea del tipo pixel in AP (pixel del font): vedi `scale::pixel_type_device`.
pub const PIXEL_SIZE_AP: u32 = 11;
pub const PIXEL_LINE_AP: u32 = 14;

/// Album, artista, playlist, nome profilo nell'intestazione di pagina. Va a capo su al massimo due righe.
pub const TYPE_DISPLAY: TypeRole = inter(32.0, 40.0, 600, -0.32, false, TextCase::AsWritten);
/// Titoli delle schermate: Home, Cerca, Libreria, Coda, Testo, Impostazioni.
pub const TYPE_PAGE_TITLE: TypeRole = pixel(true, false, TextCase::Sentence);
/// Barra del titolo, splash, accesso, informazioni. Sempre «Psytify».
pub const TYPE_WORDMARK: TypeRole = pixel(false, false, TextCase::AsWritten);
/// Titoli di sezione e dei dialoghi.
pub const TYPE_HEADING: TypeRole = inter(16.0, 24.0, 600, -0.08, false, TextCase::Sentence);
/// Titoli di brano e di tile.
pub const TYPE_TRACK_TITLE: TypeRole = inter(14.0, 20.0, 500, 0.0, false, TextCase::AsWritten);
/// Righe artista, sottotitoli dei tile (`TEXT_SECONDARY`).
pub const TYPE_ARTIST: TypeRole = inter(13.0, 18.0, 400, 0.0, false, TextCase::AsWritten);
/// Paragrafi, descrizioni delle impostazioni, toast.
pub const TYPE_BODY: TypeRole = inter(14.0, 20.0, 400, 0.0, false, TextCase::Sentence);
/// Metadati, date, conteggi (`TEXT_TERTIARY`).
pub const TYPE_META: TypeRole = inter(12.0, 16.0, 400, 0.0, true, TextCase::Sentence);
/// Voci della laterale, tab nella pagina, voci di menu.
pub const TYPE_NAV: TypeRole = inter(13.0, 20.0, 500, 0.0, false, TextCase::Sentence);
/// Etichette dei pulsanti.
pub const TYPE_BUTTON: TypeRole = inter(13.0, 16.0, 600, 0.13, false, TextCase::Sentence);
/// Kicker, intestazioni di colonna, badge (`FLAC 24/96`, `LOSSLESS`, `LIVE`).
pub const TYPE_LABEL: TypeRole = inter(11.0, 16.0, 600, 0.66, true, TextCase::Upper);
/// Durate, numeri di traccia, trascorso/rimanente nella barra del player, dimensioni.
pub const TYPE_NUMERIC: TypeRole = inter(12.0, 16.0, 400, 0.0, true, TextCase::AsWritten);
/// Trascorso/rimanente nella vista immersiva, conto alla rovescia del timer, codice jam, statistiche.
pub const TYPE_NUMERIC_PIXEL: TypeRole = pixel(true, true, TextCase::AsWritten);
/// Testo sincronizzato nella vista immersiva (24 / 32).
pub const TYPE_LYRIC_FOCUS: TypeRole = inter(24.0, 32.0, 600, -0.24, false, TextCase::AsWritten);
/// Testo sincronizzato nel pannello (18 / 26).
pub const TYPE_LYRIC_PANEL: TypeRole = inter(18.0, 26.0, 600, -0.18, false, TextCase::AsWritten);

/// I ruoli con nome (per i test; `TYPE_LYRIC` conta due volte).
pub const ALL: [(&str, TypeRole); 15] = [
    ("TYPE_DISPLAY", TYPE_DISPLAY),
    ("TYPE_PAGE_TITLE", TYPE_PAGE_TITLE),
    ("TYPE_WORDMARK", TYPE_WORDMARK),
    ("TYPE_HEADING", TYPE_HEADING),
    ("TYPE_TRACK_TITLE", TYPE_TRACK_TITLE),
    ("TYPE_ARTIST", TYPE_ARTIST),
    ("TYPE_BODY", TYPE_BODY),
    ("TYPE_META", TYPE_META),
    ("TYPE_NAV", TYPE_NAV),
    ("TYPE_BUTTON", TYPE_BUTTON),
    ("TYPE_LABEL", TYPE_LABEL),
    ("TYPE_NUMERIC", TYPE_NUMERIC),
    ("TYPE_NUMERIC_PIXEL", TYPE_NUMERIC_PIXEL),
    ("TYPE_LYRIC_FOCUS", TYPE_LYRIC_FOCUS),
    ("TYPE_LYRIC_PANEL", TYPE_LYRIC_PANEL),
];

/// Sostituzioni per il telefono. Il tipo pixel non cambia (è legato all'AP, non alla piattaforma).
pub mod phone {
    use super::{TextCase, TypeRole, inter};

    pub const TYPE_DISPLAY: TypeRole = inter(26.0, 32.0, 600, -0.32, false, TextCase::AsWritten);
    pub const TYPE_TRACK_TITLE: TypeRole = inter(16.0, 22.0, 500, 0.0, false, TextCase::AsWritten);
    pub const TYPE_ARTIST: TypeRole = inter(14.0, 20.0, 400, 0.0, false, TextCase::AsWritten);
    pub const TYPE_BODY: TypeRole = inter(15.0, 22.0, 400, 0.0, false, TextCase::Sentence);
    /// Etichette delle tab.
    pub const TYPE_NAV: TypeRole = inter(11.0, 16.0, 500, 0.0, false, TextCase::Sentence);
    pub const TYPE_LYRIC: TypeRole = inter(22.0, 30.0, 600, -0.24, false, TextCase::AsWritten);
}
