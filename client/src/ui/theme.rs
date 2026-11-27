//! Token di stile e funzioni di stile per i widget. Ogni colore e ogni misura arriva da
//! `psytify-design` (`docs/design/PSYTIFY_DESIGN_TOKENS.md`): qui si convertono nei tipi di Iced e si
//! danno i nomi che usa il resto di `ui/`. Nessuno definisce un valore suo.
//!
//! L'identità è monocromatica: quattro grigi caldi per le superfici, tre livelli di testo, nessun
//! raggio (gli angoli sono vivi), nessuna sfumatura, nessuna ombra sfocata. Il lime (`LIME`) vale solo
//! come stato: riproduzione in corso, voce di navigazione scelta, interruttore acceso, «dal vivo».
//! Il colore vero sullo schermo è quello delle copertine, mai tinte né usate per colorare altro.
//! Le regioni sono a filo e le divide una linea da 1 AP: la finestra è del colore della linea e
//! gli spazi tra i pannelli (`OUTER_GAP`) la lasciano vedere.

use std::time::Duration;

use iced::border::Radius;
use iced::font::Weight;
use iced::widget::{button, container, scrollable, slider, text_input, toggler};
use iced::{Background, Border, Color, Font, Shadow, Theme, Vector, theme};
use psytify_design::tokens::color as ds;
use psytify_design::tokens::color::Rgba;
use psytify_design::tokens::{size as sz, space, type_};

/// Un token del design come colore di Iced: la conversione si fa una volta, a compilazione.
pub const fn color(token: Rgba) -> Color {
    Color {
        r: token.r() as f32 / 255.0,
        g: token.g() as f32 / 255.0,
        b: token.b() as f32 / 255.0,
        a: token.a() as f32 / 255.0,
    }
}

/// Il verso opposto, per la cache degli sprite (la chiave è un `Rgba`): l'alfa si perde, l'opacità
/// dell'icona la dà il widget.
pub fn rgba_of(c: Color) -> Rgba {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba::rgb(byte(c.r), byte(c.g), byte(c.b))
}

// Superfici (quattro grigi: sezione 1 dei token)
/// La tela del contenuto: nero pieno.
pub const CANVAS: Color = color(ds::SURFACE_CANVAS);
/// Barra del titolo, laterale, pannello destro, barra del player, schede del telefono.
pub const CHROME: Color = color(ds::SURFACE_CHROME);
/// Hover, card, menu, finestre di dialogo, campo del segnaposto delle copertine.
pub const RAISED: Color = color(ds::SURFACE_RAISED);
/// Riga scelta, fill premuto, binario degli slider.
pub const SELECTED: Color = color(ds::SURFACE_SELECTED);
/// Il pozzetto dei campi di testo.
pub const WELL: Color = color(ds::SURFACE_WELL);
/// Le linee a filo che dividono le regioni, i separatori dei menu e delle liste.
pub const LINE: Color = color(ds::LINE_SUBTLE);
/// La finestra, che si vede negli spazi tra le regioni: il colore delle linee.
pub const GROUND: Color = LINE;
/// Bordo dei livelli che galleggiano (menu, dialoghi, avvisi) e separatore forte.
pub const LINE_STRONG: Color = color(ds::LINE_STRONG);
// Controlli
pub const CONTROL: Color = color(ds::CONTROL_BORDER);
pub const CONTROL_HOVER: Color = color(ds::CONTROL_BORDER_HOVER);
pub const FOCUS: Color = color(ds::FOCUS_RING);
// Testo e icone (tutti oltre 4.5:1 sulle quattro superfici: test in `psytify-design`)
pub const TEXT: Color = color(ds::TEXT_PRIMARY);
pub const MUTED: Color = color(ds::TEXT_SECONDARY);
pub const FAINT: Color = color(ds::TEXT_TERTIARY);
/// Un controllo che non si può usare ora (il download di un brano senza licenza).
pub const DISABLED: Color = color(ds::TEXT_DISABLED);
pub const NAV_IDLE: Color = color(ds::ICON_IDLE);
/// Icone attive: bianco caldo, come il testo (il cuore dei brani che piacciono è questo).
pub const ICON_ACTIVE: Color = color(ds::ICON_ACTIVE);
// Stati
/// L'unica luce accesa dell'app: solo stato (riproduzione, navigazione scelta, interruttori, live).
pub const LIME: Color = color(ds::STATE_ON);
pub const ERROR: Color = color(ds::TEXT_ERROR);
pub const ON_PRIMARY: Color = color(ds::TEXT_ON_LIGHT);
/// Il pulsante primario: bianco caldo; al passaggio bianco pieno.
const PRIMARY_HOVER: Color = color(ds::FOCUS_RING);
/// Il nero dei velo e delle ombre.
const BLACK: Color = color(ds::DEEP_BLACK);
/// Il velo dietro le finestre di dialogo: nero al 72 %.
const SCRIM: Color = color(ds::SCRIM);

// Misure (dp). Sezione 3 e 6 dei token.
/// Un pixel d'arte in dp: linee e bordi sono larghi uno.
pub const AP: f32 = space::AP_DP;
pub const PAD: f32 = space::SPACE_6;
/// Margine del contenuto dentro la tela.
pub const GUTTER: f32 = sz::PAGE_GUTTER;
/// Lo spazio tra le regioni: la linea che le divide, larga un AP.
pub const OUTER_GAP: f32 = space::AP_DP;
pub const CONTROL_HEIGHT: f32 = sz::CONTROL_H;
/// Il modulo di accesso: una colonna al centro della pagina.
pub const SIGN_IN_WIDTH: f32 = 360.0;
/// Campi e pulsanti del modulo di accesso.
pub const FIELD_HEIGHT: f32 = sz::CONTROL_H_LARGE;
/// L'avatar grande in testa a un profilo e quello piccolo nelle righe delle persone.
pub const AVATAR_PROFILE: f32 = sz::cover::AVATAR_M;
pub const AVATAR_ROW: f32 = sz::cover::AVATAR_S;
pub const SIDEBAR_WIDTH: f32 = sz::SIDEBAR_W;
pub const TOP_BAR_HEIGHT: f32 = sz::TITLE_BAR_H;
pub const NAV_PITCH: f32 = sz::NAV_ITEM_H + space::SPACE_2;
pub const NOW_PLAYING_HEIGHT: f32 = sz::PLAYER_BAR_H;
/// Pannello destro (in riproduzione, coda, testo).
pub const SIDE_PANEL_WIDTH: f32 = sz::SIDE_PANEL_W;
pub const ROW_PLAIN: f32 = sz::TRACK_ROW_H;
/// Copertina nelle righe delle liste miste.
pub const ROW_COVER: f32 = sz::cover::COVER_XS;
/// Riga di ricerca con copertina da 48.
pub const ROW_MEDIA: f32 = sz::TRACK_ROW_H + space::SPACE_5;
/// Altezza dei filtri e dei segmenti.
pub const PILL_HEIGHT: f32 = sz::CONTROL_H;
/// Lato minimo di una copertina nella griglia degli album: le colonne si calcolano da qui.
pub const ALBUM_TILE_MIN: f32 = sz::TILE_MIN_W;
/// Spazio tra le tessere della griglia, in orizzontale e in verticale.
pub const GRID_GAP: f32 = sz::GRID_GAP;
/// Larghezza di un menu contestuale e altezza di una sua voce.
pub const MENU_WIDTH: f32 = sz::MENU_W;
pub const MENU_ITEM: f32 = sz::MENU_ITEM_H;
/// Riga di una sheet dal basso (telefono).
pub const SHEET_ITEM: f32 = sz::phone::SHEET_ITEM_H;
/// Testo delle voci della sheet.
pub const FS_SHEET: f32 = type_::phone::TYPE_TRACK_TITLE.size;

// Telefono (ui/phone/).
/// Barra delle schede in basso, mini-player sopra di lei, margine laterale delle pagine.
pub const PHONE_NAV_HEIGHT: f32 = sz::phone::PHONE_TAB_BAR_H;
pub const PHONE_MINI_HEIGHT: f32 = sz::phone::PHONE_MINI_PLAYER_H;
pub const PHONE_GUTTER: f32 = sz::phone::PHONE_GUTTER;
/// Barra delle pagine aperte da un'altra: solo «Indietro».
pub const PHONE_TOP_HEIGHT: f32 = sz::phone::PHONE_TOP_BAR_H;
/// Titolo delle schermate principali.
pub const FS_PHONE_TITLE: f32 = type_::PIXEL_SIZE;
/// Copertine degli scaffali.
pub const PHONE_COVER: f32 = sz::phone::PHONE_SHELF_COVER;
pub const PHONE_ARTIST: f32 = 120.0;
/// Righe delle liste: copertina e passo.
pub const PHONE_ROW_COVER: f32 = sz::cover::COVER_S;
pub const PHONE_ROW: f32 = sz::phone::PHONE_ROW_H;
/// Copertina del player a schermo intero: al massimo questa parte dell'altezza.
pub const PHONE_PLAYER_COVER: f32 = 0.42;
/// Spazio per la barra di stato e per quella dei gesti di Android, che stanno sopra l'app.
pub const PHONE_STATUS_INSET: f32 = 28.0;
pub const PHONE_GESTURE_INSET: f32 = 14.0;
/// Bersaglio minimo di un tocco.
pub const TOUCH_TARGET: f32 = sz::phone::TOUCH_TARGET;

/// La barra di avanzamento dei download: dieci scatti di pixel art, ciascuno largo `STEP_W` e alto
/// `STEP_H`, a distanza `STEP_GAP` (componenti, §19: nessun lime, solo bianco di stato).
pub const STEP_W: f32 = 8.0 * AP;
pub const STEP_H: f32 = 3.0 * AP;
pub const STEP_GAP: f32 = AP;
/// Il segno «scaricato» nelle righe strette: un'icona da 12 AP senza il tasto intorno.
pub const MARK_SLOT: f32 = 12.0 * AP;

// Movimento (vedi ui/motion.rs). Due orologi: gli sprite a scatti, la UI con ease-out brevi.
/// Uscite e hover.
pub const MOTION_FAST: Duration =
    Duration::from_millis(psytify_design::tokens::motion::MOTION_FAST.ms as u64);
/// Ingresso di pagine, avvisi e copertine.
pub const MOTION_BASE: Duration =
    Duration::from_millis(psytify_design::tokens::motion::MOTION_BASE.ms as u64);
/// Pannello laterale e finestre.
pub const MOTION_PANEL: Duration =
    Duration::from_millis(psytify_design::tokens::motion::MOTION_PANEL.ms as u64);
/// Il testo scorre da un verso al successivo.
pub const MOTION_LYRICS: Duration =
    Duration::from_millis(psytify_design::tokens::motion::MOTION_LYRICS.ms as u64);
/// Passo dei movimenti frequenti che non chiedono il vsync (il testo che scorre): 24 al secondo.
pub const AMBIENT_FRAME: Duration = Duration::from_millis(42);
/// Il cassetto del telefono.
pub const MOTION_SPRING: Duration = MOTION_PANEL;
/// La copertina grande non respira più: resta intera anche in pausa.
pub const COVER_REST: f32 = 1.0;
/// Splash d'ingresso: sotto 1,2 s in tutto; da `SPLASH_EXIT` si scopre l'app in un taglio.
pub const SPLASH_TOTAL: Duration = Duration::from_millis(1200);
pub const SPLASH_EXIT: Duration = Duration::from_millis(1100);
/// Una pagina nuova entra con un taglio e una dissolvenza: non sale più.
pub const PAGE_RISE: f32 = 0.0;
/// Durata dell'ingresso di una pagina e distanza fra una riga e la successiva (nessuna).
pub const PAGE_ENTRY: Duration = MOTION_BASE;
pub const STAGGER: Duration = Duration::ZERO;
/// L'avviso compare solo con l'opacità.
pub const TOAST_RISE: f32 = 0.0;
/// Distanza dell'avviso dal player.
pub const TOAST_GAP: f32 = space::SPACE_5;
pub const TOAST_WIDTH: f32 = sz::TOAST_W;
/// Un avviso informativo sparisce da solo dopo tanto; un errore resta finché non si chiude.
pub const TOAST_INFO_TIME: Duration =
    Duration::from_millis(psytify_design::tokens::motion::TOAST_HOLD_MS as u64);
/// Un ciclo della pulsazione degli scheletri, e il colore al suo picco.
pub const PULSE_PERIOD: Duration = Duration::from_millis(1400);
pub const SKELETON_PEAK: Color = SELECTED;

// Tipografia: Inter (assets/fonts). Departure Mono, per i titoli di pagina, non è ancora incorporato:
// finché non c'è, quei titoli usano Inter alla stessa misura.
/// Titolo di una pagina di contenuto (album, playlist): il nome grande in testa.
pub const FS_HERO: f32 = type_::v1::DISPLAY;
/// Titolo del carosello della Home.
pub const FS_FEATURE: f32 = type_::v1::DISPLAY;
/// Titolo di pagina (h1): il nome della schermata.
pub const FS_PAGE: f32 = type_::PIXEL_SIZE;
pub const FS_TITLE: f32 = type_::v1::HEADING;
pub const FS_SECTION: f32 = type_::v1::HEADING;
pub const FS_BODY: f32 = type_::v1::BODY;
pub const FS_SMALL: f32 = type_::v1::META;
pub const FONT: Font = Font::with_name("Inter");
pub const FONT_MEDIUM: Font = Font { weight: Weight::Medium, ..FONT };
pub const FONT_SEMIBOLD: Font = Font { weight: Weight::Semibold, ..FONT };

pub fn theme() -> Theme {
    Theme::custom(
        "Psytify".to_string(),
        theme::Palette { background: CANVAS, text: TEXT, primary: TEXT, success: LIME, danger: ERROR },
    )
}

/// Un bordo vivo (nessun raggio: gli angoli arrotondati rompono la griglia dei pixel).
fn border(width: f32, color: Color) -> Border {
    Border { width, color, radius: Radius::from(0.0) }
}

fn background(color: Color) -> Option<Background> {
    Some(Background::Color(color))
}

/// L'ombra dei livelli che galleggiano: dura, sfalsata di 2 AP, senza sfocatura.
fn floating_shadow(alpha: f32) -> Shadow {
    Shadow {
        color: Color { a: alpha.clamp(0.0, 1.0), ..BLACK },
        offset: Vector::new(2.0 * AP, 2.0 * AP),
        blur_radius: 0.0,
    }
}

// ---------- Contenitori ----------

/// La finestra: del colore delle linee, che si vede negli spazi tra le regioni.
pub fn ground(_: &Theme) -> container::Style {
    container::Style { background: background(GROUND), text_color: Some(TEXT), ..container::Style::default() }
}

/// Una regione del chrome (barra laterale, pannello destro, barra del player): grigio quasi nero.
pub fn panel_box(_: &Theme) -> container::Style {
    container::Style { background: background(CHROME), text_color: Some(TEXT), ..container::Style::default() }
}

/// La tela del contenuto: nero pieno, piatto. Nessuna tinta, per nessuna pagina.
pub fn canvas(_: &Theme) -> container::Style {
    container::Style { background: background(CANVAS), text_color: Some(TEXT), ..container::Style::default() }
}

pub fn card(_: &Theme) -> container::Style {
    container::Style { background: background(RAISED), text_color: Some(TEXT), ..container::Style::default() }
}

pub fn panel(_: &Theme) -> container::Style {
    container::Style {
        background: background(SELECTED),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Scheletro di un contenuto in arrivo: pulsa tra due gradini di grigio.
pub fn skeleton(_: &Theme) -> container::Style {
    let glow = super::motion::pulse(PULSE_PERIOD);
    container::Style {
        background: background(super::motion::mix(RAISED, SKELETON_PEAK, glow)),
        ..container::Style::default()
    }
}

/// Avviso in basso: sul livello delle card, con il filo dei livelli che galleggiano; `alpha` sotto 1
/// solo mentre entra o esce.
pub fn toast(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(super::motion::fade(RAISED, alpha)),
        border: border(AP, super::motion::fade(LINE_STRONG, alpha)),
        text_color: Some(super::motion::fade(TEXT, alpha)),
        shadow: floating_shadow(alpha),
    }
}

/// Menu contestuale e sottomenu: `alpha` li fa comparire.
pub fn menu_box(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(super::motion::fade(RAISED, alpha)),
        border: border(AP, super::motion::fade(LINE_STRONG, alpha)),
        text_color: Some(super::motion::fade(TEXT, alpha)),
        shadow: floating_shadow(alpha),
    }
}

/// La sheet dal basso del telefono: superficie rialzata con il filo in alto.
pub fn sheet(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(super::motion::fade(RAISED, alpha)),
        border: border(AP, super::motion::fade(LINE_STRONG, alpha)),
        text_color: Some(super::motion::fade(TEXT, alpha)),
        shadow: floating_shadow(alpha),
    }
}

/// Il cassetto da sinistra del telefono: stessa superficie della sheet.
pub fn drawer(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    sheet(alpha)
}

/// Finestra di dialogo al centro.
pub fn dialog_box(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    sheet(alpha)
}

/// Velo scuro dietro una finestra di dialogo.
pub fn scrim(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(Color { a: SCRIM.a * alpha.clamp(0.0, 1.0), ..BLACK }),
        ..container::Style::default()
    }
}

/// Tasto Play che spunta sulla copertina al passaggio del mouse: un quadrato primario, bianco con
/// l'icona nera. Non è lime: non indica uno stato. `alpha` lo fa entrare.
pub fn fab(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(super::motion::fade(TEXT, alpha)),
        text_color: Some(ON_PRIMARY),
        shadow: floating_shadow(alpha),
        ..container::Style::default()
    }
}

/// Il Play della copertina quando è un pulsante vero: si schiarisce al passaggio.
pub fn fab_button(alpha: f32) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let fill = match status {
            button::Status::Hovered => PRIMARY_HOVER,
            button::Status::Pressed => MUTED,
            _ => TEXT,
        };
        button::Style {
            background: background(super::motion::fade(fill, alpha)),
            text_color: ON_PRIMARY,
            border: border(0.0, Color::TRANSPARENT),
            shadow: floating_shadow(alpha),
        }
    }
}

/// Un riquadro che contiene più controlli (le ricerche recenti).
pub fn chip(_: &Theme) -> container::Style {
    container::Style {
        background: background(RAISED),
        border: border(0.0, Color::TRANSPARENT),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Il tasto di una scorciatoia: un riquadro con il bordo dei controlli.
pub fn key_cap(_: &Theme) -> container::Style {
    container::Style {
        background: background(RAISED),
        border: border(AP, CONTROL),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Etichetta della qualità («Lossless»): contorno e testo neutri. La qualità è un fatto, non uno
/// stato, quindi niente lime.
pub fn accent_chip(_: &Theme) -> container::Style {
    container::Style {
        background: None,
        border: border(AP, CONTROL),
        text_color: Some(MUTED),
        ..container::Style::default()
    }
}

/// Binario del selettore a schede del pannello destro.
pub fn segment_track(_: &Theme) -> container::Style {
    container::Style { background: background(RAISED), ..container::Style::default() }
}

/// Il cursore sotto la scheda scelta.
pub fn segment_thumb(_: &Theme) -> container::Style {
    container::Style { background: background(SELECTED), ..container::Style::default() }
}

/// Binario di una scelta a segmenti dentro una card: più scuro della card.
pub fn segment_well(_: &Theme) -> container::Style {
    container::Style { background: background(WELL), ..container::Style::default() }
}

/// Un'opzione di una scelta a segmenti: piena se scelta, trasparente (grigia) altrimenti.
pub fn segment(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let fill = match status {
            _ if selected => Some(TEXT),
            button::Status::Hovered | button::Status::Pressed => Some(SELECTED),
            _ => None,
        };
        button::Style {
            background: fill.and_then(background),
            text_color: if selected { ON_PRIMARY } else { MUTED },
            border: border(0.0, Color::TRANSPARENT),
            shadow: Shadow::default(),
        }
    }
}

/// Il mini-player del telefono: sul chrome, come la barra delle schede su cui poggia.
pub fn mini_player(_: &Theme) -> container::Style {
    container::Style { background: background(CHROME), ..container::Style::default() }
}

/// La barra di navigazione del telefono: il chrome, con il filo in alto fatto dal contenitore.
pub fn phone_nav(_: &Theme) -> container::Style {
    container::Style { background: background(CHROME), ..container::Style::default() }
}

/// Lo splash d'ingresso: la tela nera, che sparisce in un taglio quando l'app compare.
pub fn splash(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(Color { a: alpha.clamp(0.0, 1.0), ..CANVAS }),
        ..container::Style::default()
    }
}

/// L'etichetta sull'angolo di una copertina («Nuovo»): bianco caldo pieno, testo nero.
pub fn badge(_: &Theme) -> container::Style {
    container::Style {
        background: background(TEXT),
        text_color: Some(ON_PRIMARY),
        ..container::Style::default()
    }
}

/// Il campo di una copertina mancante: grigio, con lo sprite del disco al centro.
pub fn cover_field(_: &Theme) -> container::Style {
    container::Style { background: background(RAISED), text_color: Some(TEXT), ..container::Style::default() }
}

/// Il banner dell'aggiornamento: una fascia sul livello delle card con il bordo dei controlli.
pub fn update_banner(_: &Theme) -> container::Style {
    container::Style {
        background: background(RAISED),
        border: border(AP, CONTROL),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Riquadro di uno stato vuoto o d'errore: un bordo pieno al livello dei controlli.
pub fn notice_box(_: &Theme) -> container::Style {
    container::Style {
        background: None,
        border: border(AP, SELECTED),
        text_color: Some(MUTED),
        ..container::Style::default()
    }
}

/// Stato vuoto curato: una card con titolo, una frase e l'azione che riempie la lista.
pub fn empty_card(_: &Theme) -> container::Style {
    card(&theme())
}

/// Una barra piatta (visualizzatore, indicatore).
pub fn bar(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: background(color), ..container::Style::default() }
}

// ---------- Pulsanti ----------

/// Pulsante primario: bianco caldo con testo nero (primario per inversione).
pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    let fill = match status {
        button::Status::Hovered => PRIMARY_HOVER,
        button::Status::Pressed => MUTED,
        button::Status::Disabled => SELECTED,
        button::Status::Active => TEXT,
    };
    button::Style {
        background: background(fill),
        text_color: if status == button::Status::Disabled { DISABLED } else { ON_PRIMARY },
        border: border(0.0, Color::TRANSPARENT),
        shadow: Shadow::default(),
    }
}

/// Il pulsante di riproduzione in evidenza: come il primario. Il lime non è del Play, è dello stato.
pub fn play_fab(theme_: &Theme, status: button::Status) -> button::Style {
    primary(theme_, status)
}

/// Azione secondaria: contorno di 1 AP.
pub fn outlined(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: match status {
            button::Status::Hovered | button::Status::Pressed => background(RAISED),
            _ => None,
        },
        text_color: if status == button::Status::Disabled { DISABLED } else { TEXT },
        border: border(
            AP,
            if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                CONTROL_HOVER
            } else {
                CONTROL
            },
        ),
        shadow: Shadow::default(),
    }
}

/// Azione pericolosa (elimina): contorno e testo di errore.
pub fn danger(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: match status {
            button::Status::Hovered | button::Status::Pressed => background(RAISED),
            _ => None,
        },
        text_color: ERROR,
        border: border(AP, ERROR),
        shadow: Shadow::default(),
    }
}

/// Filtro a segmento: un gradino sopra la tela a riposo, invertito (bianco, testo nero) quando è scelto.
pub fn pill(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let fill = match status {
            _ if selected => TEXT,
            button::Status::Hovered | button::Status::Pressed => SELECTED,
            _ => RAISED,
        };
        button::Style {
            background: background(fill),
            text_color: if selected { ON_PRIMARY } else { TEXT },
            border: border(0.0, Color::TRANSPARENT),
            shadow: Shadow::default(),
        }
    }
}

/// Azione testuale (Riprova, Svuota, Carica altri).
pub fn text_action(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: match status {
            button::Status::Hovered | button::Status::Pressed => TEXT,
            button::Status::Disabled => DISABLED,
            button::Status::Active => MUTED,
        },
        border: Border::default(),
        shadow: Shadow::default(),
    }
}

/// Icona cliccabile: fondo al passaggio; l'icona la colora chi la disegna.
pub fn icon_button(_on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: match status {
            button::Status::Hovered => background(RAISED),
            button::Status::Pressed => background(SELECTED),
            _ => None,
        },
        text_color: TEXT,
        border: border(0.0, Color::TRANSPARENT),
        shadow: Shadow::default(),
    }
}

/// Pulsante quadrato della barra superiore (Home, account).
pub fn round_button(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: background(if on || hovered { SELECTED } else { RAISED }),
            text_color: TEXT,
            border: border(0.0, Color::TRANSPARENT),
            shadow: Shadow::default(),
        }
    }
}

/// Il campo della ricerca: pozzetto nero con il bordo dei controlli, bianco mentre è aperta.
pub fn search_field(active: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(WELL),
        border: border(AP, if active { FOCUS } else { CONTROL }),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Icona senza fondo al passaggio (la barra del player, dove il fondo disturba).
pub fn icon_plain(_: &Theme, _status: button::Status) -> button::Style {
    button::Style { background: None, text_color: TEXT, border: Border::default(), shadow: Shadow::default() }
}

/// Controlli della finestra.
pub fn window_control(_close: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: if hovered { background(SELECTED) } else { None },
            text_color: if hovered { TEXT } else { MUTED },
            border: border(0.0, Color::TRANSPARENT),
            shadow: Shadow::default(),
        }
    }
}

/// Voce di navigazione: scelta con il fondo del selezionato (il segnalino lime lo disegna la barra
/// laterale), le altre si schiariscono al passaggio.
pub fn nav_item(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: if active {
                background(SELECTED)
            } else if hovered {
                background(RAISED)
            } else {
                None
            },
            text_color: if active || hovered { TEXT } else { NAV_IDLE },
            border: border(0.0, Color::TRANSPARENT),
            shadow: Shadow::default(),
        }
    }
}

/// Riga o tessera cliccabile: un gradino di grigio al passaggio.
pub fn row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: match status {
            button::Status::Hovered | button::Status::Pressed => background(RAISED),
            _ if selected => background(SELECTED),
            _ => None,
        },
        text_color: TEXT,
        border: border(0.0, Color::TRANSPARENT),
        shadow: Shadow::default(),
    }
}

/// Voce di un menu: evidenziata al passaggio; `danger` la tinge di errore.
pub fn menu_item(danger: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: if hovered { background(SELECTED) } else { None },
            text_color: match (status, danger) {
                (button::Status::Disabled, _) => DISABLED,
                (_, true) => ERROR,
                _ => TEXT,
            },
            border: border(0.0, Color::TRANSPARENT),
            shadow: Shadow::default(),
        }
    }
}

/// Scorciatoia della Home: una barra con la copertina a filo sul bordo sinistro.
pub fn shortcut(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: background(if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            SELECTED
        } else {
            RAISED
        }),
        text_color: TEXT,
        border: border(0.0, Color::TRANSPARENT),
        shadow: Shadow::default(),
    }
}

pub fn tile(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: match status {
            button::Status::Hovered | button::Status::Pressed => background(RAISED),
            _ => None,
        },
        text_color: TEXT,
        border: border(0.0, Color::TRANSPARENT),
        shadow: Shadow::default(),
    }
}

/// Una voce dell'indice dei generi («Sfoglia tutto»): testo su una card, nessun colore.
pub fn browse_card(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: background(if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            SELECTED
        } else {
            RAISED
        }),
        text_color: TEXT,
        border: border(0.0, Color::TRANSPARENT),
        shadow: Shadow::default(),
    }
}

/// Filtro a carta: il bordo e il fondo li disegna il contenitore.
pub fn bare(_: &Theme, _: button::Status) -> button::Style {
    button::Style { background: None, text_color: TEXT, border: Border::default(), shadow: Shadow::default() }
}

// ---------- Input ----------

pub fn search_input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let edge = match status {
        text_input::Status::Focused => FOCUS,
        text_input::Status::Hovered => CONTROL_HOVER,
        _ => CONTROL,
    };
    text_input::Style {
        background: Background::Color(WELL),
        border: border(AP, edge),
        icon: MUTED,
        placeholder: FAINT,
        value: TEXT,
        selection: Color { a: 0.35, ..FOCUS },
    }
}

/// Campo di testo dentro una finestra di dialogo: stesso pozzetto, anello bianco quando è attivo.
pub fn dialog_input(theme_: &Theme, status: text_input::Status) -> text_input::Style {
    search_input(theme_, status)
}

/// Barra di avanzamento: la parte ascoltata è bianco caldo (il lime è per gli stati accesi).
pub fn progress_slider(theme_: &Theme, status: slider::Status) -> slider::Style {
    level_slider(TEXT, 6.0)(theme_, status)
}

/// Volume: è un controllo, non un dato, e resta neutro come la barra.
pub fn volume_slider(theme_: &Theme, status: slider::Status) -> slider::Style {
    level_slider(TEXT, 5.0)(theme_, status)
}

fn level_slider(fill: Color, grown: f32) -> impl Fn(&Theme, slider::Status) -> slider::Style {
    move |_, status| {
        // La maniglia compare solo quando serve: sopra una barra ferma resta un filo pulito.
        let handle = match status {
            slider::Status::Hovered | slider::Status::Dragged => grown,
            slider::Status::Active => 0.0,
        };
        let fill = if matches!(status, slider::Status::Active) { fill } else { FOCUS };
        slider::Style {
            rail: slider::Rail {
                backgrounds: (Background::Color(fill), Background::Color(SELECTED)),
                width: sz::SEEK_TRACK_H,
                border: border(0.0, Color::TRANSPARENT),
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Rectangle {
                    width: (2.0 * handle).round() as u16,
                    border_radius: Radius::from(0.0),
                },
                background: Background::Color(FOCUS),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        }
    }
}

/// Il volume dei player a schermo intero. `open` (0 a riposo, 1 toccato) allarga la barra e fa
/// comparire la maniglia: a riposo è un filo sottile e spento.
pub fn side_volume(open: f32) -> impl Fn(&Theme, slider::Status) -> slider::Style {
    move |_, _| {
        let width = sz::SEEK_TRACK_H + 6.0 * open;
        slider::Style {
            rail: slider::Rail {
                backgrounds: (
                    Background::Color(super::motion::mix(super::motion::fade(TEXT, 0.55), TEXT, open)),
                    Background::Color(super::motion::mix(super::motion::fade(TEXT, 0.18), SELECTED, open)),
                ),
                width,
                border: border(0.0, Color::TRANSPARENT),
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Rectangle {
                    width: (2.0 + 14.0 * open).round() as u16,
                    border_radius: Radius::from(0.0),
                },
                background: Background::Color(FOCUS),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        }
    }
}

/// La pastiglia dietro il volume: invisibile a riposo, un fondo pieno quando è toccato.
pub fn volume_pill(open: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(super::motion::fade(RAISED, 0.94 * open)),
        ..container::Style::default()
    }
}

/// Interruttore: lime quando è acceso (lo stato è anche nella posizione del pomello).
pub fn switch(_: &Theme, status: toggler::Status) -> toggler::Style {
    let (on, hovered) = match status {
        toggler::Status::Active { is_toggled } => (is_toggled, false),
        toggler::Status::Hovered { is_toggled } => (is_toggled, true),
        toggler::Status::Disabled => (false, false),
    };
    let off = if hovered { CONTROL_HOVER } else { CONTROL };
    toggler::Style {
        background: if on { LIME } else { off },
        background_border_width: 0.0,
        background_border_color: Color::TRANSPARENT,
        foreground: if on { ON_PRIMARY } else { TEXT },
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
    }
}

pub fn scroll(_: &Theme, status: scrollable::Status) -> scrollable::Style {
    let active = !matches!(status, scrollable::Status::Active);
    // Sul telefono si scorre col dito: la barra non serve e coprirebbe il bordo delle righe.
    let color = if crate::app::is_phone() {
        Color::TRANSPARENT
    } else if active {
        CONTROL
    } else {
        SELECTED
    };
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller { color, border: border(0.0, Color::TRANSPARENT) },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
    }
}

#[cfg(test)]
mod tests {
    use psytify_design::contrast::{AA_TEXT, contrast_ratio};

    use super::*;

    // I rapporti di contrasto dei token (testo su ogni superficie, bordi dei controlli, testo scuro
    // sui fondi chiari) li verifica `psytify-design::contrast`. Qui si controlla solo che i nomi del
    // client puntino ai token giusti e che la conversione in `Color` non perda un byte.

    #[test]
    fn the_conversion_to_iced_colours_is_lossless() {
        for (_, token) in ds::PALETTE {
            assert_eq!(rgba_of(color(token)), Rgba::rgb(token.r(), token.g(), token.b()));
        }
        assert!((SCRIM.a - 184.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn the_client_names_are_the_design_aliases() {
        assert_eq!(rgba_of(TEXT), ds::TEXT_PRIMARY);
        assert_eq!(rgba_of(MUTED), ds::TEXT_SECONDARY);
        assert_eq!(rgba_of(FAINT), ds::TEXT_TERTIARY);
        assert_eq!(rgba_of(DISABLED), ds::TEXT_DISABLED);
        assert_eq!(rgba_of(LIME), ds::LIME);
        assert_eq!(rgba_of(ERROR), ds::ERROR);
        assert_eq!(rgba_of(CANVAS), ds::DEEP_BLACK);
        assert_eq!(rgba_of(CHROME), ds::NEAR_BLACK);
        assert_eq!(rgba_of(RAISED), ds::GRAPHITE);
        assert_eq!(rgba_of(SELECTED), ds::CHARCOAL);
    }

    #[test]
    fn every_text_colour_of_the_client_passes_wcag_aa_on_the_four_surfaces() {
        for text in [TEXT, MUTED, FAINT, NAV_IDLE, ERROR] {
            for surface in ds::SURFACES {
                assert!(contrast_ratio(rgba_of(text), surface) >= AA_TEXT, "{text:?} on {surface:?}");
            }
        }
    }

    #[test]
    fn the_primary_label_passes_wcag_aa_on_its_fill() {
        assert!(contrast_ratio(rgba_of(ON_PRIMARY), rgba_of(TEXT)) >= AA_TEXT);
        assert!(contrast_ratio(rgba_of(ON_PRIMARY), rgba_of(LIME)) >= AA_TEXT);
    }
}
