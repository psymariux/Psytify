//! Movimento (sezione 7). Due orologi: quello degli sprite (durate in `sprites.json`, mai
//! interpolato) e quello della UI (colori, opacità, traslazioni quantizzate sull'AP).

/// Curva di una transizione della UI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    /// Nessuna interpolazione: il valore scatta (premuto, caret, frame degli sprite).
    Step,
    /// `cubic-bezier(0.2, 0.0, 0.0, 1.0)`; niente molle, niente rimbalzi.
    EaseOut,
}

/// Punti di controllo della curva `EaseOut`: `(x1, y1, x2, y2)`.
pub const EASE_OUT_BEZIER: [f32; 4] = [0.2, 0.0, 0.0, 1.0];

/// Una transizione: durata in ms e curva.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub ms: u32,
    pub curve: Curve,
}

impl Motion {
    /// Durata effettiva: con `reduce_motion` tutte le durate della UI diventano 0.
    pub const fn effective_ms(self, reduce_motion: bool) -> u32 {
        if reduce_motion { 0 } else { self.ms }
    }
}

/// Pressione e collasso della ledge: istantanei, come un tasto.
pub const MOTION_PRESS: Motion = Motion { ms: 0, curve: Curve::Step };
/// Fill, bordo e tinta delle icone in hover.
pub const MOTION_HOVER: Motion = Motion { ms: 70, curve: Curve::EaseOut };
/// Comparsa di tooltip e menu (solo opacità), pomello del toggle (2 passi da 50).
pub const MOTION_FAST: Motion = Motion { ms: 100, curve: Curve::EaseOut };
/// Dissolvenza del contenuto di pagina, toast, dialogo (opacità + scrim).
pub const MOTION_BASE: Motion = Motion { ms: 160, curve: Curve::EaseOut };
/// Pannello laterale, rail, scorrimento degli sheet; offset quantizzati su `ap_device`.
pub const MOTION_PANEL: Motion = Motion { ms: 200, curve: Curve::EaseOut };
/// Scorrimento delle righe del testo, quantizzato a pixel fisici interi.
pub const MOTION_LYRICS: Motion = Motion { ms: 280, curve: Curve::EaseOut };

/// Ritardo del tooltip.
pub const TOOLTIP_DELAY_MS: u32 = 500;
/// Permanenza del toast.
pub const TOAST_HOLD_MS: u32 = 4000;
/// Caret: 530 ms acceso, 530 ms spento (a scatti).
pub const CARET_BLINK_ON_MS: u32 = 530;
pub const CARET_BLINK_OFF_MS: u32 = 530;
/// Aggiornamento di barra di avanzamento e orologio (niente aggiornamenti per frame). Resta attivo
/// anche con `reduce_motion`.
pub const PROGRESS_TICK_MS: u32 = 250;
/// Nessun frame di sprite dura meno di così.
pub const SPRITE_FRAME_MIN_MS: u32 = 80;

/// Le transizioni della UI con nome, per i test.
pub const ALL: [(&str, Motion); 6] = [
    ("MOTION_PRESS", MOTION_PRESS),
    ("MOTION_HOVER", MOTION_HOVER),
    ("MOTION_FAST", MOTION_FAST),
    ("MOTION_BASE", MOTION_BASE),
    ("MOTION_PANEL", MOTION_PANEL),
    ("MOTION_LYRICS", MOTION_LYRICS),
];
