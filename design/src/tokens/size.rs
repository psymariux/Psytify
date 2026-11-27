//! Dimensioni di layout (sezione 6), in dp salvo dove indicato.

// Finestra.
pub const WINDOW_MIN_W: f32 = 960.0;
pub const WINDOW_MIN_H: f32 = 600.0;

// Desktop (sezione 6.1).
/// Decorazioni lato client: marchio, wordmark, cronologia, ricerca, account, comandi finestra.
pub const TITLE_BAR_H: f32 = 40.0;
pub const SIDEBAR_W: f32 = 240.0;
/// Laterale compressa: solo icone.
pub const SIDEBAR_RAIL_W: f32 = 56.0;
/// In riproduzione, coda, testo.
pub const SIDE_PANEL_W: f32 = 320.0;
pub const PLAYER_BAR_H: f32 = 72.0;
pub const PAGE_GUTTER: f32 = 24.0;
/// Tetto della colonna del contenuto sulle finestre molto larghe.
pub const CONTENT_MAX_W: f32 = 1440.0;
/// Pulsanti, campi, pulsanti icona.
pub const CONTROL_H: f32 = 32.0;
/// Pulsante Play di pagina, play/pausa del trasporto.
pub const CONTROL_H_LARGE: f32 = 40.0;
/// Icona: uno sprite di 12 AP.
pub const ICON: f32 = 24.0;
/// Area di click e fill dell'hover.
pub const ICON_BUTTON: f32 = 32.0;
pub const NAV_ITEM_H: f32 = 32.0;
/// Due righe (titolo sopra artista), copertina `cover::COVER_XS`.
pub const TRACK_ROW_H: f32 = 48.0;
/// Una riga, tabella a colonne (Libreria «Brani», download).
pub const TRACK_ROW_DENSE_H: f32 = 32.0;
/// Colonna numero / play / equalizzatore.
pub const TRACK_INDEX_W: f32 = 32.0;
/// Larghezza minima di un tile; colonne = floor((larghezza + gap) / (TILE_MIN_W + GRID_GAP)).
pub const TILE_MIN_W: f32 = 160.0;
pub const GRID_GAP: f32 = 16.0;
pub const MENU_W: f32 = 240.0;
pub const MENU_ITEM_H: f32 = 32.0;
pub const TOOLTIP_MAX_W: f32 = 240.0;
/// 16 sopra la barra del player, centrato sulla colonna del contenuto.
pub const TOAST_W: f32 = 360.0;
pub const DIALOG_W: f32 = 400.0;
pub const SEARCH_FIELD_MAX_W: f32 = 480.0;
/// La colonna della schermata di accesso (direzione artistica, «Sign-in / social»).
pub const SIGN_IN_W: f32 = 360.0;
/// Binario del seek (2 AP).
pub const SEEK_TRACK_H: f32 = 4.0;
/// Area sensibile del seek.
pub const SEEK_HIT_H: f32 = 24.0;
pub const VOLUME_W: f32 = 96.0;

// Breakpoint sulla larghezza della finestra.
/// Sotto: la laterale diventa rail.
pub const BP_RAIL: f32 = 1100.0;
/// Sotto: il pannello laterale si sovrappone al contenuto invece di agganciarsi.
pub const BP_PANEL_DOCK: f32 = 1360.0;
/// Sopra: i tile crescono e il contenuto si ferma a `CONTENT_MAX_W`.
pub const BP_WIDE: f32 = 1600.0;

/// Numero di colonne di una griglia di tile in una colonna larga `width` dp (almeno una).
pub fn grid_columns(width: f32) -> u32 {
    let cols = ((width + GRID_GAP) / (TILE_MIN_W + GRID_GAP)).floor();
    if cols.is_finite() && cols >= 1.0 { cols as u32 } else { 1 }
}

/// Artwork (sezione 6.2): multipli di 16 dp, così il segnaposto di 24 AP cade su AP interi.
pub mod cover {
    /// Righe, playlist della laterale.
    pub const COVER_XS: f32 = 32.0;
    /// Barra del player, menu, prime righe della ricerca.
    pub const COVER_S: f32 = 48.0;
    /// Tile della griglia (cresce con la colonna, agganciato a 16).
    pub const COVER_M: f32 = 160.0;
    /// Intestazione di album / playlist.
    pub const COVER_L: f32 = 224.0;
    /// Pannello «In riproduzione».
    pub const COVER_PANEL: f32 = 288.0;
    pub const COVER_FOCUS_MAX: f32 = 640.0;
    /// Avatar con maschera a cerchio pixel: righe, profilo, intestazione artista.
    pub const AVATAR_S: f32 = 32.0;
    pub const AVATAR_M: f32 = 96.0;
    pub const AVATAR_L: f32 = 160.0;
    /// Sotto questo lato il segnaposto è `icon_library` tinta `STONE`, sopra `album_placeholder`.
    pub const PLACEHOLDER_ART_MIN: f32 = 64.0;
}

/// Telefono (sezione 6.3).
pub mod phone {
    pub const PHONE_GUTTER: f32 = 16.0;
    /// Titolo (`TYPE_PAGE_TITLE`) o indietro.
    pub const PHONE_TOP_BAR_H: f32 = 48.0;
    /// Più l'inset del gesto di sistema sotto.
    pub const PHONE_TAB_BAR_H: f32 = 56.0;
    /// Sopra la tab bar.
    pub const PHONE_MINI_PLAYER_H: f32 = 56.0;
    /// Copertina 48.
    pub const PHONE_ROW_H: f32 = 64.0;
    pub const PHONE_SHELF_COVER: f32 = 144.0;
    /// Play/pausa del trasporto.
    pub const PHONE_PLAY_BUTTON: f32 = 64.0;
    /// Area minima sensibile (linea guida Android, sopra il minimo di 44): non si restringe allo sprite.
    pub const TOUCH_TARGET: f32 = 48.0;
    /// Righe del menu a tendina dal basso.
    pub const SHEET_ITEM_H: f32 = 56.0;
    /// Maniglia dello sheet, barra `STONE`: larghezza e altezza in AP (16 x 2).
    pub const SHEET_HANDLE_AP: (u32, u32) = (16, 2);

    /// Lato della copertina del player a schermo intero: min(larghezza − 32, 0,42 x altezza), a multipli di 16.
    pub fn player_cover(width: f32, height: f32) -> f32 {
        let side = (width - 2.0 * PHONE_GUTTER).min(0.42 * height);
        if side.is_finite() && side > 0.0 { (side / 16.0).floor() * 16.0 } else { 0.0 }
    }
}
