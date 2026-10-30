//! Token di stile e funzioni di stile per i widget (vedi docs/STYLE.md).
//!
//! Struttura "tre pannelli su nero" come nei player moderni: la finestra è nera, la barra laterale,
//! il contenuto e il pannello destro sono pannelli arrotondati un gradino sopra. L'elevazione
//! si fa con gradini di luminosità; l'ombra compare solo su ciò che galleggia (menu, finestre di
//! dialogo) e i gradienti solo come tinta della copertina in testa a una pagina. Il lime è il
//! colore del marchio: avanzamento, brano in riproduzione, "mi piace", interruttori accesi e il
//! pulsante di riproduzione in evidenza.

use std::f32::consts::{FRAC_PI_2, PI};
use std::time::Duration;

use iced::border::Radius;
use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{button, container, scrollable, slider, svg, text_input, toggler};
use iced::{Background, Border, Color, Font, Gradient, Shadow, Theme, Vector, theme};

const fn hex(rgb: u32) -> Color {
    Color {
        r: ((rgb >> 16) & 0xff) as f32 / 255.0,
        g: ((rgb >> 8) & 0xff) as f32 / 255.0,
        b: (rgb & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

// Superfici
/// La finestra: nero puro, si vede tra i pannelli.
pub const GROUND: Color = hex(0x000000);
/// Barra laterale, contenuto e pannello destro.
pub const PANEL: Color = hex(0x121212);
/// Card dentro un pannello e riga selezionata.
pub const SURFACE: Color = hex(0x1b1b1b);
/// Passaggio del mouse.
pub const HOVER: Color = hex(0x252525);
/// Binari degli slider, bordo degli stati vuoti, menu.
pub const RAISED: Color = hex(0x2e2e2e);
pub const SKELETON: Color = hex(0x1f1f1f);
// Linee
pub const CONTROL: Color = hex(0x4d4d4d);
// Testo (tutti oltre 4.5:1 su GROUND, PANEL, SURFACE, HOVER e RAISED)
pub const TEXT: Color = hex(0xffffff);
pub const MUTED: Color = hex(0xb8b8b8);
pub const FAINT: Color = hex(0x9c9c9c);
/// Un controllo che non si può usare ora (il download di un brano senza licenza).
pub const DISABLED: Color = hex(0x636363);
pub const NAV_IDLE: Color = hex(0xbdbdbd);
// Marchio, dati e stati
pub const ACCENT: Color = hex(0xc6f24e);
pub const ACCENT_HOVER: Color = hex(0xd6ff5e);
pub const NEGATIVE: Color = hex(0xe5566f);
pub const ON_PRIMARY: Color = hex(0x000000);

// Misure
/// Tinte delle pagine che non hanno una copertina: la casa (il verde del marchio, molto scuro),
/// la neutra delle impostazioni e quella dei brani che piacciono.
pub const BRAND_TINT: Color = hex(0x4d6a14);
pub const NEUTRAL_TINT: Color = hex(0x3a3a3a);
pub const LIKED_TINT: Color = hex(0x8a2f48);

pub const RADIUS_PANEL: f32 = 14.0;
pub const RADIUS_CARD: f32 = 14.0;
pub const RADIUS_TILE: f32 = 10.0;
/// Angolo di una copertina, in frazione del lato: 7% (a 188px sono 13px, a 40px quasi 3).
pub const COVER_CORNER: f32 = 0.07;
pub const RADIUS_ROW: f32 = 8.0;
pub const RADIUS_PILL: f32 = 999.0;
pub const PAD: f32 = 22.0;
/// Margine del contenuto dentro il pannello centrale.
pub const GUTTER: f32 = 28.0;
/// Spazio tra i pannelli e dal bordo della finestra.
pub const OUTER_GAP: f32 = 8.0;
pub const CONTROL_HEIGHT: f32 = 36.0;
/// Il modulo di accesso: una colonna stretta al centro della pagina.
pub const SIGN_IN_WIDTH: f32 = 380.0;
/// Campi e pulsanti del modulo di accesso: più alti dei controlli, per il pollice e per leggere.
pub const FIELD_HEIGHT: f32 = 44.0;
/// L'avatar grande in testa a un profilo e quello piccolo nelle righe delle persone.
pub const AVATAR_PROFILE: f32 = 128.0;
pub const AVATAR_ROW: f32 = 44.0;
pub const SIDEBAR_WIDTH: f32 = 252.0;
pub const TOP_BAR_HEIGHT: f32 = 56.0;
pub const NAV_PITCH: f32 = 40.0;
pub const NOW_PLAYING_HEIGHT: f32 = 88.0;
/// Pannello destro (in riproduzione, coda, testo).
pub const SIDE_PANEL_WIDTH: f32 = 340.0;
pub const ROW_PLAIN: f32 = 56.0;
/// Copertina nelle righe delle liste miste: 40px su un passo di 56 (40 + 16).
pub const ROW_COVER: f32 = 40.0;
/// Riga di ricerca con copertina da 50px.
pub const ROW_MEDIA: f32 = 64.0;
/// Filtri a pillola della Libreria.
pub const PILL_HEIGHT: f32 = 32.0;
/// Lato minimo di una copertina nella griglia degli album: le colonne si calcolano da qui.
pub const ALBUM_TILE_MIN: f32 = 168.0;
/// Spazio tra le tessere della griglia, in orizzontale e in verticale.
pub const GRID_GAP: f32 = 16.0;
/// Larghezza di un menu contestuale e altezza di una sua voce.
pub const MENU_WIDTH: f32 = 276.0;
pub const MENU_ITEM: f32 = 38.0;
/// Riga di una sheet dal basso (telefono): alta quanto basta a un dito.
pub const SHEET_ITEM: f32 = 56.0;
/// Testo delle voci della sheet.
pub const FS_SHEET: f32 = 16.0;
/// Angoli superiori della sheet.
pub const SHEET_RADIUS: f32 = 20.0;

// Telefono (ui/phone/).
/// Barra di navigazione in basso, mini-player sopra di lei, margine laterale delle pagine.
pub const PHONE_NAV_HEIGHT: f32 = 60.0;
pub const PHONE_MINI_HEIGHT: f32 = 60.0;
pub const PHONE_GUTTER: f32 = 16.0;
/// Barra delle pagine aperte da un'altra: solo «Indietro».
pub const PHONE_TOP_HEIGHT: f32 = 48.0;
/// Titolo delle quattro schermate principali: grande, come nelle app da telefono.
pub const FS_PHONE_TITLE: f32 = 30.0;
/// Copertine degli scaffali: due e mezza per riga, quella tagliata invita a scorrere.
pub const PHONE_COVER: f32 = 148.0;
pub const PHONE_ARTIST: f32 = 120.0;
/// Righe delle liste: copertina e passo.
pub const PHONE_ROW_COVER: f32 = 56.0;
pub const PHONE_ROW: f32 = 68.0;
/// Copertina del player a schermo intero: al massimo questa parte dell'altezza.
pub const PHONE_PLAYER_COVER: f32 = 0.42;
/// Spazio per la barra di stato e per quella dei gesti di Android, che stanno sopra l'app.
pub const PHONE_STATUS_INSET: f32 = 28.0;
pub const PHONE_GESTURE_INSET: f32 = 14.0;
/// Bersaglio minimo di un tocco.
pub const TOUCH_TARGET: f32 = 44.0;

// Movimento (vedi ui/motion.rs).
/// Uscite: chi se ne va si toglie di mezzo più in fretta di chi arriva.
pub const MOTION_FAST: Duration = Duration::from_millis(120);
/// Ingresso di pagine, avvisi e copertine.
pub const MOTION_BASE: Duration = Duration::from_millis(220);
/// Pannello laterale e finestre: sono larghi, gli serve un po' più tempo per non sembrare uno scatto.
pub const MOTION_PANEL: Duration = Duration::from_millis(260);
/// Il testo scorre da un verso al successivo.
pub const MOTION_LYRICS: Duration = Duration::from_millis(480);
/// Passo dei movimenti frequenti che non chiedono il vsync (il testo che scorre): 24 al secondo,
/// come il cinema. Scorre di una riga in 480 ms: undici passi bastano a non vederli.
pub const AMBIENT_FRAME: Duration = Duration::from_millis(42);
/// Movimenti con la molla (`motion::spring`): la copertina che respira con il Play, il cursore delle
/// schede del pannello. Più lunghi dell'ease-out perché il rimbalzo deve avere il tempo di posarsi.
pub const MOTION_SPRING: Duration = Duration::from_millis(620);
/// In pausa la copertina grande scende a questa scala; al Play torna intera con la molla.
pub const COVER_REST: f32 = 0.9;
/// Splash d'ingresso: dura tanto in tutto, e da `SPLASH_EXIT` il velo sfuma e scopre l'app.
pub const SPLASH_TOTAL: Duration = Duration::from_millis(1750);
pub const SPLASH_EXIT: Duration = Duration::from_millis(1250);
/// Una pagina nuova sale di tanto mentre entra.
pub const PAGE_RISE: f32 = 16.0;
/// Le righe e le tessere di una pagina entrano una dopo l'altra, a distanza di tanto.
/// Durata complessiva dell'ingresso di una pagina con la sua lista (quattordici righe scaglionate).
pub const PAGE_ENTRY: Duration = Duration::from_millis(14 * 28 + 220);
pub const STAGGER: Duration = Duration::from_millis(28);
/// L'avviso sale di tanto dal bordo del player.
pub const TOAST_RISE: f32 = 12.0;
/// Distanza dell'avviso dal player.
pub const TOAST_GAP: f32 = 16.0;
pub const TOAST_WIDTH: f32 = 420.0;
/// Un avviso informativo sparisce da solo dopo tanto; un errore resta finché non si chiude.
pub const TOAST_INFO_TIME: Duration = Duration::from_secs(4);
/// Un ciclo della pulsazione degli scheletri, e il colore al suo picco.
pub const PULSE_PERIOD: Duration = Duration::from_millis(1400);
pub const SKELETON_PEAK: Color = hex(0x2a2a2a);

// Tipografia: Inter con cifre tabellari incorporate (assets/fonts, vedi docs/STYLE.md).
/// Titolo di una pagina di contenuto (album, playlist): il nome grande in testa.
pub const FS_HERO: f32 = 44.0;
/// Titolo del carosello della Home; con poco spazio scende a `FS_PAGE` e va a capo.
pub const FS_FEATURE: f32 = 40.0;
/// Titolo di pagina (h1): il nome della schermata, la cosa che si legge per prima.
pub const FS_PAGE: f32 = 28.0;
pub const FS_TITLE: f32 = 20.0;
pub const FS_SECTION: f32 = 18.0;
pub const FS_BODY: f32 = 14.0;
pub const FS_SMALL: f32 = 12.0;
pub const FONT: Font = Font::with_name("Inter");
pub const FONT_MEDIUM: Font = Font { weight: Weight::Medium, ..FONT };
pub const FONT_SEMIBOLD: Font = Font { weight: Weight::Semibold, ..FONT };

pub fn theme() -> Theme {
    Theme::custom(
        "Psytify".to_string(),
        theme::Palette { background: GROUND, text: TEXT, primary: TEXT, success: ACCENT, danger: NEGATIVE },
    )
}

fn border(width: f32, color: Color, radius: f32) -> Border {
    Border { width, color, radius: Radius::from(radius) }
}

fn background(color: Color) -> Option<Background> {
    Some(Background::Color(color))
}

/// La tinta di una copertina per la testa di una pagina: il suo colore, scurito e sciolto nel pannello.
pub fn header_tint(cover: Color) -> Color {
    super::motion::mix(PANEL, cover, 0.62)
}

// ---------- Contenitori ----------

pub fn ground(_: &Theme) -> container::Style {
    container::Style { background: background(GROUND), text_color: Some(TEXT), ..container::Style::default() }
}

/// Un pannello arrotondato sul fondo nero.
pub fn panel_box(_: &Theme) -> container::Style {
    container::Style {
        background: background(PANEL),
        border: border(0.0, Color::TRANSPARENT, RADIUS_PANEL),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Un pannello con la tinta di una copertina in alto, che si scioglie nel grigio del pannello a
/// metà altezza. Il pannello centrale e il pannello "In riproduzione" prendono il colore di ciò
/// che si guarda o si ascolta.
pub fn tinted_panel(tint: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Gradient(Gradient::Linear(
            Linear::new(PI).add_stop(0.0, header_tint(tint)).add_stop(0.5, PANEL).add_stop(1.0, PANEL),
        ))),
        border: border(0.0, Color::TRANSPARENT, RADIUS_PANEL),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Un oggetto sollevato dalla pagina (la copertina grande): solo un'ombra.
pub fn lifted(_: &Theme) -> container::Style {
    container::Style {
        shadow: Shadow {
            color: Color { a: 0.5, ..GROUND },
            offset: Vector::new(0.0, 10.0),
            blur_radius: 30.0,
        },
        ..container::Style::default()
    }
}

/// Come `tinted_panel`, ma la sfumatura respira: l'angolo oscilla piano e un secondo colore, più
/// chiaro della tinta, sale e scende. Per il pannello «In riproduzione» mentre il brano suona.
pub fn aurora(tint: Color, drift: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let base = header_tint(tint);
        let glow = super::motion::mix(base, super::motion::mix(tint, ACCENT, 0.3), 0.45);
        container::Style {
            background: Some(Background::Gradient(Gradient::Linear(
                Linear::new(PI + 0.4 * drift)
                    .add_stop(0.0, base)
                    .add_stop((0.32 + 0.1 * drift).clamp(0.1, 0.6), glow)
                    .add_stop(0.78, PANEL)
                    .add_stop(1.0, PANEL),
            ))),
            border: border(0.0, Color::TRANSPARENT, RADIUS_PANEL),
            text_color: Some(TEXT),
            ..container::Style::default()
        }
    }
}

/// Il banner in cima alla Home: la tinta della copertina a sinistra, sciolta nel pannello a destra.
pub fn hero_card(tint: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Gradient(Gradient::Linear(
            Linear::new(FRAC_PI_2)
                .add_stop(0.0, super::motion::mix(PANEL, tint, 0.8))
                .add_stop(1.0, super::motion::mix(PANEL, tint, 0.22)),
        ))),
        border: border(0.0, Color::TRANSPARENT, RADIUS_CARD),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// I colori delle schede di «Sfoglia tutto»: toni pieni e scuri, tutti con lo stesso peso.
pub const BROWSE: [Color; 9] = [
    hex(0x8d2f5a),
    hex(0x1e5f74),
    hex(0x5e3a98),
    hex(0x9a5b13),
    hex(0x2e6b3d),
    hex(0x7a2a2a),
    hex(0x2a4d8f),
    hex(0x56622a),
    hex(0x2f5f58),
];

/// Una scheda di «Sfoglia tutto»: un colore pieno, angoli come le copertine.
pub fn browse_card(color: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let fill = match status {
            button::Status::Hovered | button::Status::Pressed => {
                super::motion::mix(color, Color::WHITE, 0.08)
            }
            _ => color,
        };
        button::Style {
            background: background(fill),
            text_color: TEXT,
            border: border(0.0, Color::TRANSPARENT, RADIUS_CARD),
            shadow: Shadow::default(),
        }
    }
}

/// Come `aurora`, ma a tutta finestra: senza angoli arrotondati e con la tinta che arriva più in
/// basso. Per la vista immersiva.
pub fn aurora_full(tint: Color, drift: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let base = super::motion::mix(GROUND, tint, 0.7);
        let glow = super::motion::mix(base, super::motion::mix(tint, ACCENT, 0.3), 0.4);
        container::Style {
            background: Some(Background::Gradient(Gradient::Linear(
                Linear::new(PI + 0.5 * drift)
                    .add_stop(0.0, base)
                    .add_stop((0.4 + 0.12 * drift).clamp(0.15, 0.7), glow)
                    .add_stop(1.0, GROUND),
            ))),
            text_color: Some(TEXT),
            ..container::Style::default()
        }
    }
}

/// Velo sopra la copertina sfocata delle viste a tutto schermo: scurisce quanto basta a leggere il
/// testo bianco, di più ai lati, dove l'immagine ingrandita finisce.
pub fn veil(_: &Theme) -> container::Style {
    let shade = |a: f32| Color { a, ..GROUND };
    container::Style {
        background: Some(Background::Gradient(Gradient::Linear(
            Linear::new(FRAC_PI_2)
                .add_stop(0.0, shade(0.86))
                .add_stop(0.14, shade(0.6))
                .add_stop(0.86, shade(0.6))
                .add_stop(1.0, shade(0.86)),
        ))),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

pub fn card(_: &Theme) -> container::Style {
    container::Style {
        background: background(SURFACE),
        border: border(0.0, Color::TRANSPARENT, RADIUS_CARD),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

pub fn panel(_: &Theme) -> container::Style {
    container::Style {
        background: background(RAISED),
        border: border(0.0, Color::TRANSPARENT, 6.0),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Scheletro di un contenuto in arrivo: pulsa tra due gradini di luminosità.
pub fn skeleton(_: &Theme) -> container::Style {
    let glow = super::motion::pulse(PULSE_PERIOD);
    container::Style {
        background: background(super::motion::mix(SKELETON, SKELETON_PEAK, glow)),
        border: border(0.0, Color::TRANSPARENT, 4.0),
        ..container::Style::default()
    }
}

/// Avviso in basso: un gradino sopra il pannello, opaco a riposo; `alpha` sotto 1 solo mentre entra
/// o esce. Galleggia, quindi ha un'ombra.
pub fn toast(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(super::motion::fade(RAISED, alpha)),
        border: border(0.0, Color::TRANSPARENT, RADIUS_TILE),
        text_color: Some(super::motion::fade(TEXT, alpha)),
        shadow: floating_shadow(alpha),
    }
}

/// Menu contestuale e sottomenu: `alpha` e `scale` li fanno comparire.
pub fn menu_box(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(super::motion::fade(RAISED, alpha)),
        border: border(1.0, super::motion::fade(hex(0x3a3a3a), alpha), RADIUS_TILE + 2.0),
        text_color: Some(super::motion::fade(TEXT, alpha)),
        shadow: floating_shadow(alpha),
    }
}

/// La sheet dal basso del telefono: superficie rialzata con gli angoli superiori tondi.
pub fn sheet(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(super::motion::fade(hex(0x1c1c1c), alpha)),
        border: iced::Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: iced::border::Radius::new(0.0).top(SHEET_RADIUS),
        },
        text_color: Some(super::motion::fade(TEXT, alpha)),
        shadow: floating_shadow(alpha),
    }
}

/// Il cassetto da sinistra del telefono: stessa superficie della sheet, angoli tondi tutti intorno
/// perché galleggia staccato dai bordi.
pub fn drawer(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(super::motion::fade(hex(0x1c1c1c), alpha)),
        border: iced::Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: iced::border::Radius::new(SHEET_RADIUS),
        },
        text_color: Some(super::motion::fade(TEXT, alpha)),
        shadow: floating_shadow(alpha),
    }
}

/// Finestra di dialogo al centro.
pub fn dialog_box(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(super::motion::fade(hex(0x1c1c1c), alpha)),
        border: border(1.0, super::motion::fade(hex(0x333333), alpha), RADIUS_CARD + 2.0),
        text_color: Some(super::motion::fade(TEXT, alpha)),
        shadow: floating_shadow(alpha),
    }
}

/// Velo scuro dietro una finestra di dialogo.
pub fn scrim(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(Color { a: 0.62 * alpha.clamp(0.0, 1.0), ..GROUND }),
        ..container::Style::default()
    }
}

/// Tasto Play tondo che spunta sulla copertina al passaggio del mouse: `alpha` lo fa entrare.
pub fn fab(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(super::motion::fade(ACCENT, alpha)),
        border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
        text_color: Some(ON_PRIMARY),
        shadow: floating_shadow(alpha * 0.8),
    }
}

/// Il Play tondo della copertina quando è un pulsante vero: si schiarisce al passaggio.
pub fn fab_button(alpha: f32) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let fill = match status {
            button::Status::Hovered => ACCENT_HOVER,
            button::Status::Pressed => hex(0xa9d63a),
            _ => ACCENT,
        };
        button::Style {
            background: background(super::motion::fade(fill, alpha)),
            text_color: ON_PRIMARY,
            border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
            shadow: floating_shadow(alpha * 0.8),
        }
    }
}

/// Etichetta sull'angolo di una copertina («Nuovo»): il verde del marchio, piena.
/// Una pillola che contiene più controlli (le ricerche recenti): il grigio delle pillole spente.
pub fn chip(_: &Theme) -> container::Style {
    container::Style {
        background: background(RAISED),
        border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Il tasto di una scorciatoia: un riquadro rialzato con il bordo, come un tasto vero.
pub fn key_cap(_: &Theme) -> container::Style {
    container::Style {
        background: background(RAISED),
        border: border(1.0, CONTROL, RADIUS_ROW - 2.0),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Etichetta della qualità «Lossless»: contorno e testo lime, senza riempimento (il pieno è del Play).
pub fn accent_chip(_: &Theme) -> container::Style {
    container::Style {
        background: None,
        border: border(1.0, ACCENT, RADIUS_PILL),
        text_color: Some(ACCENT),
        ..container::Style::default()
    }
}

/// Binario del selettore a schede del pannello destro.
pub fn segment_track(_: &Theme) -> container::Style {
    container::Style {
        background: background(SURFACE),
        border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
        ..container::Style::default()
    }
}

/// Il cursore che scivola, con la molla, sotto la scheda scelta.
pub fn segment_thumb(_: &Theme) -> container::Style {
    container::Style {
        background: background(RAISED),
        border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
        ..container::Style::default()
    }
}

/// Binario di una scelta a segmenti dentro una card: più scuro della card, così anche le opzioni
/// spente si leggono come parte di un controllo.
pub fn segment_well(_: &Theme) -> container::Style {
    container::Style {
        background: background(PANEL),
        border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
        ..container::Style::default()
    }
}

/// Un'opzione di una scelta a segmenti: piena se scelta, trasparente (grigia) altrimenti.
pub fn segment(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let fill = match status {
            _ if selected => Some(TEXT),
            button::Status::Hovered | button::Status::Pressed => Some(RAISED),
            _ => None,
        };
        button::Style {
            background: fill.and_then(background),
            text_color: if selected { ON_PRIMARY } else { MUTED },
            border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
            shadow: Shadow::default(),
        }
    }
}

/// Il mini-player del telefono: la tinta della copertina, scurita, su un riquadro arrotondato.
pub fn mini_player(tint: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(super::motion::mix(SURFACE, tint, 0.45)),
        // Appoggiata sulla barra delle schede: niente angoli arrotondati.
        border: border(0.0, Color::TRANSPARENT, 0.0),
        ..container::Style::default()
    }
}

/// La pagina del telefono: il nero della finestra con, in alto, la tinta di ciò che si guarda che
/// si scioglie nel nero prima di metà schermo. Niente pannello: la pagina va da bordo a bordo.
pub fn phone_wash(tint: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Gradient(Gradient::Linear(
            Linear::new(PI)
                .add_stop(0.0, super::motion::mix(GROUND, tint, 0.5))
                .add_stop(0.42, GROUND)
                .add_stop(1.0, GROUND),
        ))),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// La barra di navigazione del telefono: il nero della finestra, senza linee.
pub fn phone_nav(_: &Theme) -> container::Style {
    container::Style {
        background: background(GROUND),
        border: border(0.0, Color::TRANSPARENT, 0.0),
        ..container::Style::default()
    }
}

/// Lo splash d'ingresso: il nero della finestra, che sfuma quando l'app compare.
pub fn splash(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(Color { a: alpha.clamp(0.0, 1.0), ..GROUND }),
        ..container::Style::default()
    }
}

pub fn badge(_: &Theme) -> container::Style {
    container::Style {
        background: background(ACCENT),
        border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
        text_color: Some(ON_PRIMARY),
        ..container::Style::default()
    }
}

fn floating_shadow(alpha: f32) -> Shadow {
    Shadow {
        color: Color { a: 0.55 * alpha.clamp(0.0, 1.0), ..GROUND },
        offset: Vector::new(0.0, 8.0),
        blur_radius: 28.0,
    }
}

/// Copertina disegnata: colore piatto con l'angolo arrotondato di `radius` (proporzionale al lato).
pub fn cover(color: Color, radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(color),
        border: border(0.0, Color::TRANSPARENT, radius),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Copertina disegnata con una sfumatura (playlist, "Brani che ti piacciono").
pub fn cover_gradient(from: Color, to: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Gradient(Gradient::Linear(
            Linear::new(FRAC_PI_2 * 0.55).add_stop(0.0, from).add_stop(1.0, to),
        ))),
        border: border(0.0, Color::TRANSPARENT, 7.0),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Il banner dell'aggiornamento: una fascia sul livello delle card con un filo lime, perché si veda
/// senza gridare (niente fondo colorato).
pub fn update_banner(_: &Theme) -> container::Style {
    container::Style {
        background: background(SURFACE),
        border: border(1.0, ACCENT, RADIUS_PILL),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Riquadro di uno stato vuoto o d'errore: linea tratteggiata non disponibile in Iced, quindi
/// un bordo pieno di 1px al livello dei controlli.
pub fn notice_box(_: &Theme) -> container::Style {
    container::Style {
        background: None,
        border: border(1.0, RAISED, 10.0),
        text_color: Some(MUTED),
        ..container::Style::default()
    }
}

/// Stato vuoto curato: una card con titolo, una frase e l'azione che riempie la lista.
pub fn empty_card(_: &Theme) -> container::Style {
    container::Style {
        background: background(SURFACE),
        border: border(0.0, Color::TRANSPARENT, RADIUS_CARD),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Una barra piatta con gli angoli tondi (visualizzatore, indicatore).
pub fn bar(color: Color, radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(color),
        border: border(0.0, Color::TRANSPARENT, radius),
        ..container::Style::default()
    }
}

// ---------- Pulsanti ----------

/// Pulsante primario: pillola bianca con testo nero (primario per inversione).
pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    let fill = match status {
        button::Status::Hovered => hex(0xf0f0f0),
        button::Status::Pressed => hex(0xcfcfcf),
        button::Status::Disabled => RAISED,
        button::Status::Active => TEXT,
    };
    button::Style {
        background: background(fill),
        text_color: if status == button::Status::Disabled { FAINT } else { ON_PRIMARY },
        border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
        shadow: Shadow::default(),
    }
}

/// Il pulsante di riproduzione in evidenza: tondo e lime, come il marchio.
pub fn play_fab(_: &Theme, status: button::Status) -> button::Style {
    let fill = match status {
        button::Status::Hovered => ACCENT_HOVER,
        button::Status::Pressed => hex(0xa9d63a),
        button::Status::Disabled => RAISED,
        button::Status::Active => ACCENT,
    };
    button::Style {
        background: background(fill),
        text_color: ON_PRIMARY,
        border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
        shadow: Shadow::default(),
    }
}

/// Azione secondaria: contorno di 1px, più chiaro dei bordi strutturali.
pub fn outlined(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: match status {
            button::Status::Hovered | button::Status::Pressed => background(HOVER),
            _ => None,
        },
        text_color: if status == button::Status::Disabled { FAINT } else { TEXT },
        border: border(1.0, CONTROL, RADIUS_PILL),
        shadow: Shadow::default(),
    }
}

/// Azione pericolosa (elimina): contorno rosso.
pub fn danger(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: match status {
            button::Status::Hovered | button::Status::Pressed => background(Color { a: 0.14, ..NEGATIVE }),
            _ => None,
        },
        text_color: NEGATIVE,
        border: border(1.0, Color { a: 0.6, ..NEGATIVE }, RADIUS_PILL),
        shadow: Shadow::default(),
    }
}

/// Filtro a pillola: un gradino sopra il pannello a riposo, invertito (bianco, testo nero) quando è
/// scelto.
pub fn pill(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let fill = match status {
            _ if selected => TEXT,
            button::Status::Hovered | button::Status::Pressed => RAISED,
            _ => SURFACE,
        };
        button::Style {
            background: background(fill),
            text_color: if selected { ON_PRIMARY } else { TEXT },
            border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
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
            button::Status::Disabled => FAINT,
            button::Status::Active => MUTED,
        },
        border: Border::default(),
        shadow: Shadow::default(),
    }
}

/// Icona cliccabile: grigia a riposo, bianca al passaggio; `on` la tiene bianca.
pub fn icon_button(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: match status {
            button::Status::Hovered | button::Status::Pressed => background(HOVER),
            _ => None,
        },
        text_color: if on || matches!(status, button::Status::Hovered) { TEXT } else { MUTED },
        border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
        shadow: Shadow::default(),
    }
}

/// Pulsante tondo della barra superiore (Home, account): grigio, più chiaro al passaggio.
pub fn round_button(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: background(if on || hovered { RAISED } else { SURFACE }),
            text_color: TEXT,
            border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
            shadow: Shadow::default(),
        }
    }
}

/// La pillola della ricerca: grigia, con un contorno chiaro mentre la ricerca è aperta.
pub fn search_field(active: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(SURFACE),
        border: border(
            if active { 1.5 } else { 0.0 },
            if active { TEXT } else { Color::TRANSPARENT },
            RADIUS_PILL,
        ),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// Icona senza fondo al passaggio (la barra del player, dove il fondo disturba).
pub fn icon_plain(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            TEXT
        } else {
            MUTED
        },
        border: Border::default(),
        shadow: Shadow::default(),
    }
}

/// Controlli della finestra: il tasto di chiusura diventa rosso al passaggio.
pub fn window_control(close: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: match (hovered, close) {
                (true, true) => background(hex(0xc42b3b)),
                (true, false) => background(HOVER),
                _ => None,
            },
            text_color: if hovered { TEXT } else { MUTED },
            border: border(0.0, Color::TRANSPARENT, RADIUS_ROW),
            shadow: Shadow::default(),
        }
    }
}

/// Voce di navigazione: attiva con un fondo e l'etichetta bianca, le altre sfumano al passaggio.
pub fn nav_item(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: if active {
                background(SURFACE)
            } else if hovered {
                background(Color { a: 0.55, ..SURFACE })
            } else {
                None
            },
            text_color: if active || hovered { TEXT } else { NAV_IDLE },
            border: border(0.0, Color::TRANSPARENT, RADIUS_TILE),
            shadow: Shadow::default(),
        }
    }
}

/// Riga o tessera cliccabile: un gradino di luminosità al passaggio.
pub fn row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: match status {
            button::Status::Hovered | button::Status::Pressed => background(HOVER),
            _ if selected => background(SURFACE),
            _ => None,
        },
        text_color: TEXT,
        border: border(0.0, Color::TRANSPARENT, RADIUS_ROW),
        shadow: Shadow::default(),
    }
}

/// Voce di un menu: evidenziata al passaggio; `danger` la tinge di rosso.
pub fn menu_item(danger: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: if hovered { background(hex(0x3a3a3a)) } else { None },
            text_color: match (status, danger) {
                (button::Status::Disabled, _) => FAINT,
                (_, true) => NEGATIVE,
                _ => TEXT,
            },
            border: border(0.0, Color::TRANSPARENT, 6.0),
            shadow: Shadow::default(),
        }
    }
}

/// Scorciatoia della Home: una barra grigia con la copertina a filo sul bordo sinistro.
pub fn shortcut(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: background(if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            RAISED
        } else {
            SURFACE
        }),
        text_color: TEXT,
        border: border(0.0, Color::TRANSPARENT, RADIUS_ROW),
        shadow: Shadow::default(),
    }
}

pub fn tile(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: match status {
            button::Status::Hovered | button::Status::Pressed => background(SURFACE),
            _ => None,
        },
        text_color: TEXT,
        border: border(0.0, Color::TRANSPARENT, RADIUS_TILE),
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
        text_input::Status::Focused => TEXT,
        text_input::Status::Hovered => hex(0x6a6a6a),
        _ => CONTROL,
    };
    text_input::Style {
        background: Background::Color(SURFACE),
        border: border(1.0, edge, RADIUS_PILL),
        icon: MUTED,
        placeholder: FAINT,
        value: TEXT,
        selection: Color { a: 0.35, ..ACCENT },
    }
}

/// Campo di testo dentro una finestra di dialogo: rettangolare, con il bordo lime quando è attivo.
pub fn dialog_input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let edge = match status {
        text_input::Status::Focused => ACCENT,
        text_input::Status::Hovered => hex(0x6a6a6a),
        _ => CONTROL,
    };
    text_input::Style {
        background: Background::Color(hex(0x141414)),
        border: border(1.0, edge, RADIUS_TILE),
        icon: MUTED,
        placeholder: FAINT,
        value: TEXT,
        selection: Color { a: 0.35, ..ACCENT },
    }
}

/// Barra di avanzamento: la parte ascoltata è il dato, quindi è nel colore accento.
pub fn progress_slider(theme_: &Theme, status: slider::Status) -> slider::Style {
    level_slider(ACCENT, 6.0)(theme_, status)
}

/// Volume: è un controllo, non un dato, quindi resta neutro.
pub fn volume_slider(theme_: &Theme, status: slider::Status) -> slider::Style {
    level_slider(TEXT, 5.0)(theme_, status)
}

fn level_slider(fill: Color, grown: f32) -> impl Fn(&Theme, slider::Status) -> slider::Style {
    move |_, status| {
        // La maniglia compare solo quando serve: sopra una barra ferma resta un filo pulito.
        let handle_radius = match status {
            slider::Status::Hovered | slider::Status::Dragged => grown,
            slider::Status::Active => 0.0,
        };
        let fill = if matches!(status, slider::Status::Active) { fill } else { lift(fill) };
        slider::Style {
            rail: slider::Rail {
                backgrounds: (Background::Color(fill), Background::Color(CONTROL)),
                width: 4.0,
                border: border(0.0, Color::TRANSPARENT, 2.0),
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Circle { radius: handle_radius },
                background: Background::Color(TEXT),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        }
    }
}

/// Il volume dei player a schermo intero. `open` (0 a riposo, 1 toccato) allarga la barra, la
/// accende di lime e fa comparire la maniglia: a riposo è un filo sottile e spento.
pub fn side_volume(open: f32) -> impl Fn(&Theme, slider::Status) -> slider::Style {
    move |_, _| {
        let width = 4.0 + 6.0 * open;
        slider::Style {
            rail: slider::Rail {
                backgrounds: (
                    Background::Color(super::motion::mix(super::motion::fade(TEXT, 0.55), ACCENT, open)),
                    Background::Color(super::motion::mix(super::motion::fade(TEXT, 0.18), CONTROL, open)),
                ),
                width,
                border: border(0.0, Color::TRANSPARENT, width / 2.0),
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Circle { radius: 1.0 + 8.0 * open },
                background: Background::Color(TEXT),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        }
    }
}

/// La pastiglia dietro il volume: invisibile a riposo, un fondo pieno quando è toccato.
pub fn volume_pill(open: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: background(super::motion::fade(SURFACE, 0.94 * open)),
        border: border(0.0, Color::TRANSPARENT, RADIUS_PILL),
        ..container::Style::default()
    }
}

/// Un velo più chiaro: il colore dei controlli che si accendono al passaggio.
fn lift(color: Color) -> Color {
    super::motion::mix(color, Color::WHITE, 0.18)
}

/// Interruttore: lime quando è acceso.
pub fn switch(_: &Theme, status: toggler::Status) -> toggler::Style {
    let (on, hovered) = match status {
        toggler::Status::Active { is_toggled } => (is_toggled, false),
        toggler::Status::Hovered { is_toggled } => (is_toggled, true),
        toggler::Status::Disabled => (false, false),
    };
    let off = if hovered { hex(0x5a5a5a) } else { CONTROL };
    toggler::Style {
        background: if on { if hovered { ACCENT_HOVER } else { ACCENT } } else { off },
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
        RAISED
    };
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller { color, border: border(0.0, Color::TRANSPARENT, RADIUS_PILL) },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
    }
}

pub fn icon_tint(color: Color) -> impl Fn(&Theme, svg::Status) -> svg::Style {
    move |_, _| svg::Style { color: Some(color) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rapporto di contrasto WCAG 2.x tra due colori opachi.
    fn contrast(a: Color, b: Color) -> f32 {
        fn channel(c: f32) -> f32 {
            if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        }
        fn luminance(c: Color) -> f32 {
            0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
        }
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn every_text_tier_passes_wcag_aa_on_every_surface() {
        for text in [TEXT, MUTED, FAINT, NAV_IDLE] {
            for surface in [GROUND, PANEL, SURFACE, HOVER, RAISED] {
                assert!(contrast(text, surface) >= 4.5, "{text:?} on {surface:?}");
            }
        }
    }

    #[test]
    fn primary_label_passes_wcag_aa() {
        assert!(contrast(ON_PRIMARY, TEXT) >= 4.5);
    }

    #[test]
    fn the_play_button_label_passes_wcag_aa_on_the_accent() {
        assert!(contrast(ON_PRIMARY, ACCENT) >= 4.5 && contrast(ON_PRIMARY, ACCENT_HOVER) >= 4.5);
    }

    #[test]
    fn accent_text_passes_wcag_aa_on_the_panel() {
        assert!(contrast(ACCENT, PANEL) >= 4.5 && contrast(ACCENT, SURFACE) >= 4.5);
    }

    #[test]
    fn a_count_on_the_selected_pill_passes_wcag_aa() {
        assert!(contrast(CONTROL, TEXT) >= 4.5);
    }

    #[test]
    fn negative_text_passes_wcag_aa_on_the_panel_and_surface() {
        assert!(contrast(NEGATIVE, PANEL) >= 4.5 && contrast(NEGATIVE, SURFACE) >= 4.5);
    }

    #[test]
    fn white_text_on_the_close_button_hover_passes_wcag_aa() {
        assert!(contrast(TEXT, hex(0xc42b3b)) >= 4.5);
    }
}
