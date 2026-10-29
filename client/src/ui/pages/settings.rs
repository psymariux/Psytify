//! Impostazioni: una barra di schede in alto (come Apple Music) e, sotto, gruppi di righe con il
//! nome e la spiegazione a sinistra e il controllo a destra (come Spotify). Interruttori per ciò
//! che si accende e si spegne, scelte a segmenti per il resto.
//!
//! L'impaginazione dipende dalla larghezza della pagina, non dal dispositivo: anche sul desktop la
//! colonna centrale fra la barra laterale e il pannello destro può essere stretta come un telefono.
//! Stretta, le scelte a segmenti scendono sotto il testo a tutta riga; interruttori e pulsanti
//! restano a destra, come nelle impostazioni di un telefono.

use iced::widget::{Column, Space, button, column, container, row, text, toggler, vertical_slider};
use iced::{Alignment, Element, Length, Theme, alignment};

use crate::api::{DownloadQuality, Quality};
use crate::app::{Account, App, MenuTarget, Message, SettingsTab, is_phone};
use crate::player::eq::{self, PRESETS};
use crate::ui::components::{
    MUTED_LINE, h1, h2, line_aligned, muted, outlined_button, outlined_button_owned, primary_button, small,
};
use crate::ui::icons::Icon;
use crate::ui::theme;

/// Frequenze delle bande, come le legge chi regola un equalizzatore.
const BAND_LABELS: [&str; eq::BANDS] = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
/// Altezza degli slider verticali dell'equalizzatore.
const BAND_HEIGHT: f32 = 150.0;
/// Sotto questa larghezza della pagina l'impaginazione si stringe: con quattro segmenti affiancati
/// la spiegazione resterebbe una colonna di poche parole.
const COMPACT_BELOW: f32 = 720.0;
/// Larghezza dell'indirizzo del server quando sta a destra del testo.
const HOST_WIDTH: f32 = 260.0;
/// Spazio fra il binario di una scelta a segmenti e le sue opzioni.
const SEGMENT_INSET: f32 = 3.0;

/// Le schede, col nome lungo e quello corto per quando si dividono una riga stretta.
const TABS: [(SettingsTab, &str, &str); 4] = [
    (SettingsTab::Playback, "Riproduzione", "Ascolto"),
    (SettingsTab::Sound, "Suono", "Suono"),
    (SettingsTab::Account, "Account", "Account"),
    (SettingsTab::About, "Informazioni", "Info"),
];

/// Come impaginare, deciso una volta per vista dalla larghezza della pagina.
#[derive(Clone, Copy)]
struct Fit {
    /// Schede e scelte a tutta riga, scelte sotto il testo.
    compact: bool,
    /// Margine orizzontale dentro una card.
    pad: f32,
    /// Spazio utile dentro una card.
    inner: f32,
}

impl Fit {
    fn of(width: f32) -> Self {
        let compact = is_phone() || width < COMPACT_BELOW;
        let pad = if compact { theme::PHONE_GUTTER } else { theme::PAD };
        Self { compact, pad, inner: (width - 2.0 * pad).max(0.0) }
    }
}

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let fit = Fit::of(width);
    let body = match app.settings_tab {
        SettingsTab::Playback => playback(app, fit),
        SettingsTab::Sound => sound(app, fit),
        SettingsTab::Account => account(app, fit),
        SettingsTab::About => about(app, fit),
    };
    column![h1("Impostazioni"), tabs(app.settings_tab, fit), body].spacing(28).width(Length::Fill).into()
}

/// Le schede: una scelta a segmenti sul pannello. Stretta, si dividono la riga in parti uguali
/// con i nomi corti, così ci stanno tutte senza scorrere.
fn tabs<'a>(current: SettingsTab, fit: Fit) -> Element<'a, Message> {
    let options = TABS.map(|(tab, long, short)| (tab, if fit.compact { short } else { long }));
    segments(&options, current, Message::SettingsTab, fit.compact, theme::segment_track)
}

/// Un gruppo: titolo e, nella card sotto, le righe separate da una linea sottile.
fn group<'a>(title: &'a str, rows: Vec<Element<'a, Message>>, fit: Fit) -> Element<'a, Message> {
    let mut list = column![].spacing(0);
    let last = rows.len().saturating_sub(1);
    for (index, item) in rows.into_iter().enumerate() {
        list = list.push(item);
        if index != last {
            list = list.push(container(Space::new(Length::Fill, 1.0)).style(theme::bar(theme::HOVER, 0.0)));
        }
    }
    column![h2(title), container(list).padding([6.0, fit.pad]).width(Length::Fill).style(theme::card)]
        .spacing(12)
        .into()
}

/// Nome e spiegazione di una riga; prende lo spazio che il controllo lascia.
fn label<'a>(title: &'a str, detail: impl text::IntoFragment<'a>) -> Column<'a, Message> {
    column![text(title).size(theme::FS_BODY + 1.0).font(theme::FONT_MEDIUM), small(detail)]
        .spacing(4)
        .width(Length::Fill)
}

/// Una riga con una scelta: a destra del testo se c'è posto, sotto e a tutta riga se no.
fn setting<'a>(
    title: &'a str,
    detail: impl text::IntoFragment<'a>,
    control: Element<'a, Message>,
    fit: Fit,
) -> Element<'a, Message> {
    if fit.compact {
        return column![label(title, detail), control]
            .spacing(12)
            .padding([16, 0])
            .width(Length::Fill)
            .into();
    }
    row![label(title, detail), control].spacing(24).padding([16, 0]).align_y(Alignment::Center).into()
}

/// Una riga con un controllo piccolo (interruttore, pulsante, valore): sempre a destra.
fn inline<'a>(
    title: &'a str,
    detail: impl text::IntoFragment<'a>,
    control: Element<'a, Message>,
) -> Element<'a, Message> {
    row![label(title, detail), control].spacing(16).padding([16, 0]).align_y(Alignment::Center).into()
}

/// Una riga di solo testo.
fn note<'a>(title: &'a str, detail: impl text::IntoFragment<'a>) -> Element<'a, Message> {
    container(label(title, detail)).padding([16, 0]).into()
}

/// Scelta a segmenti: un binario con le opzioni affiancate, quella attiva piena. `fill` la stende
/// sulla riga in parti uguali. I nomi non vanno mai a capo: una lettera per riga non si legge.
fn segments<'a, T: Copy + PartialEq + 'a>(
    options: &[(T, &'a str)],
    current: T,
    message: impl Fn(T) -> Message,
    fill: bool,
    track: fn(&Theme) -> container::Style,
) -> Element<'a, Message> {
    let height = if is_phone() { theme::TOUCH_TARGET } else { theme::CONTROL_HEIGHT };
    let width = if fill { Length::Fill } else { Length::Shrink };
    let buttons = options.iter().map(|&(value, name)| {
        let selected = value == current;
        let name = text(name)
            .size(theme::FS_BODY)
            .font(if selected { theme::FONT_SEMIBOLD } else { theme::FONT_MEDIUM })
            .wrapping(text::Wrapping::None);
        // A tutta riga le parti uguali bastano a separarle: il margine toglierebbe spazio al nome.
        let content = if fill {
            container(name).center(Length::Fill)
        } else {
            container(name).center_y(Length::Fill).padding([0, 14])
        };
        button(content)
            .width(width)
            .height(height - 2.0 * SEGMENT_INSET)
            .padding(0)
            .style(theme::segment(selected))
            .on_press(message(value))
            .into()
    });
    container(row(buttons).spacing(2).width(width)).padding(SEGMENT_INSET).width(width).style(track).into()
}

fn switch<'a>(on: bool, message: impl Fn(bool) -> Message + 'a) -> Element<'a, Message> {
    toggler(on).on_toggle(message).size(24.0).style(theme::switch).into()
}

// ---------- Riproduzione ----------

fn playback(app: &App, fit: Fit) -> Element<'_, Message> {
    let quality_detail = match app.quality {
        Quality::Lossless => {
            "FLAC o WAV senza perdita da Internet Archive. Se un brano non ce l'ha, suona la versione standard."
        }
        Quality::Standard => {
            "Versione compressa (m4a) da YouTube, più leggera. Se un brano non ce l'ha, suona la lossless."
        }
    };
    let mut groups = column![
        group(
            "Qualità",
            vec![
                setting(
                    "Qualità d'ascolto",
                    quality_detail,
                    segments(
                        &[(Quality::Lossless, "Lossless"), (Quality::Standard, "Standard")],
                        app.quality,
                        Message::SetQuality,
                        fit.compact,
                        theme::segment_well,
                    ),
                    fit,
                ),
                setting(
                    "Qualità dei download",
                    format!(
                        "{}. Dal menu di un brano ({}, «Scarica in…») si sceglie caso per caso. \
                         Un file con perdita non si migliora: se il brano non ha la qualità chiesta arriva quella più vicina.",
                        app.download_quality.description(),
                        if is_phone() { "pressione lunga" } else { "tasto destro" },
                    ),
                    segments(
                        &DownloadQuality::ALL.map(|quality| (quality, quality.label())),
                        app.download_quality,
                        Message::SetDownloadQuality,
                        fit.compact,
                        theme::segment_well,
                    ),
                    fit,
                ),
            ],
            fit,
        ),
        group(
            "Riproduzione",
            vec![
                inline(
                    "Riproduzione automatica",
                    if app.autoplay {
                        "Finita la coda, continua da solo con altri brani dello stesso artista."
                    } else {
                        "Finita la coda, la musica si ferma."
                    },
                    switch(app.autoplay, Message::SetAutoplay),
                ),
                inline(
                    "Timer di spegnimento",
                    if is_phone() {
                        "Ferma la musica dopo un tempo o alla fine del brano."
                    } else {
                        "Ferma la musica dopo un tempo o alla fine del brano. Si imposta anche dal pulsante con i tre puntini in basso a destra."
                    },
                    sleep_control(app),
                ),
            ],
            fit,
        ),
    ]
    .spacing(28);
    // Sul telefono non c'è una tastiera da scorciatoie.
    if !is_phone() {
        groups = groups.push(group(
            "Tastiera",
            vec![inline(
                "Scorciatoie da tastiera",
                "Play, salti, volume, pannelli e navigazione senza mouse. L'elenco si apre anche con ?.",
                outlined_button("Mostra", Some(Icon::Keyboard), Some(Message::ToggleShortcuts)),
            )],
            fit,
        ));
    }
    // Solo se c'è una scheda video dedicata che sappia decodificare i videoclip.
    if let Some(gpu) = &app.dedicated_gpu {
        let detail = if app.dedicated_video {
            format!("I videoclip li decodifica la {gpu}: resta accesa mentre guardi e consuma più batteria.")
        } else {
            format!(
                "I videoclip li decodifica la GPU integrata, che consuma meno. Attivalo per usare la {gpu}."
            )
        };
        groups = groups.push(group(
            "Video",
            vec![inline(
                "Videoclip sulla scheda video",
                detail,
                switch(app.dedicated_video, Message::SetDedicatedVideo),
            )],
            fit,
        ));
    }
    groups.into()
}

// ---------- Suono ----------

/// Equalizzatore a 10 bande, tutto in una card: interruttore, preset di fabbrica (quello che
/// corrisponde alle bande è pieno) e uno slider per banda. Ogni movimento si sente subito; la
/// configurazione si salva al rilascio.
fn sound(app: &App, fit: Fit) -> Element<'_, Message> {
    let settings = app.equalizer;
    let active = PRESETS.iter().position(|(_, gains)| *gains == settings.gains);
    let presets = row(PRESETS.iter().enumerate().map(|(index, (name, _))| {
        let on = active == Some(index);
        button(text(*name).size(theme::FS_BODY).font(theme::FONT_MEDIUM).wrapping(text::Wrapping::None))
            .padding([8, 16])
            .style(
                move |theme_: &Theme, status| {
                    if on { theme::pill(true)(theme_, status) } else { theme::outlined(theme_, status) }
                },
            )
            .on_press(Message::EqualizerPreset(index))
            .into()
    }))
    .spacing(8)
    .wrap();
    let bands = row(settings.gains.iter().enumerate().map(|(band, &gain)| {
        column![
            small(gain_label(gain)),
            vertical_slider(-eq::MAX_GAIN_DB..=eq::MAX_GAIN_DB, gain, move |value| {
                Message::EqualizerBand(band, value)
            })
            .step(0.5_f32)
            .height(BAND_HEIGHT)
            .on_release(Message::EqualizerReleased)
            .style(theme::volume_slider),
            small(BAND_LABELS[band]),
        ]
        .spacing(8)
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .into()
    }))
    .spacing(4)
    .align_y(Alignment::End);
    let tuning = column![
        presets,
        bands,
        small("Guadagno in dB per banda, da −12 a +12. Vale anche mentre un brano suona."),
    ]
    .spacing(18)
    .padding([16, 0]);
    group(
        "Equalizzatore",
        vec![
            inline(
                "Equalizzatore a 10 bande",
                "Dai più corpo ai bassi o alla voce. Si sente subito.",
                switch(settings.enabled, Message::EqualizerToggle),
            ),
            tuning.into(),
        ],
        fit,
    )
}

/// "+3", "−6" o "0": il guadagno di una banda in dB.
fn gain_label(gain: f32) -> String {
    if gain.abs() < 0.5 { "0".into() } else { format!("{gain:+.0}").replace('-', "−") }
}

// ---------- Account ----------

fn account(app: &App, fit: Fit) -> Element<'_, Message> {
    let body: Element<'_, Message> = match &app.account {
        Account::Unavailable => note(
            "Login non configurato",
            "Imposta clerk_issuer e clerk_client_id in config.json per attivarlo.",
        ),
        Account::SignedOut => inline(
            "Accedi con il browser",
            "Facoltativo: catalogo e ascolto restano liberi.",
            primary_button("Accedi", None, Message::SignIn),
        ),
        Account::Working => inline("Accesso in corso", "Completa l'accesso nel browser.", muted("…").into()),
        Account::SignedIn(me) => {
            let role = if me.admin { " · amministratore" } else { "" };
            inline(
                "Collegato",
                format!("{}{role}", me.user_id),
                outlined_button("Esci", None, Some(Message::SignOut)),
            )
        }
    };
    group("Account", vec![body], fit)
}

// ---------- Informazioni ----------

fn about(app: &App, fit: Fit) -> Element<'_, Message> {
    let keys = [
        ("Spazio", "Riproduci o metti in pausa"),
        ("← →", "Salta di qualche secondo"),
        ("Ctrl F", "Cerca"),
        ("F11", "Schermo intero"),
        ("Esc", "Chiude menu e finestre"),
    ];
    let shortcuts: Vec<Element<'_, Message>> = keys
        .iter()
        .map(|&(key, what)| {
            row![container(small(key).color(theme::TEXT)).width(90), muted(what)]
                .padding([10, 0])
                .align_y(Alignment::Center)
                .into()
        })
        .collect();
    // Affiancato al testo, l'indirizzo si allinea a destra come gli altri controlli.
    let (host_width, host_align) = if fit.compact {
        (fit.inner, alignment::Horizontal::Left)
    } else {
        (HOST_WIDTH.min(fit.inner), alignment::Horizontal::Right)
    };
    let mut groups = column![group(
        "Psytify",
        vec![
            inline(
                "Versione",
                if is_phone() {
                    "Player per il catalogo Psytify."
                } else {
                    "Player desktop per il catalogo Psytify."
                },
                muted(env!("CARGO_PKG_VERSION")).into(),
            ),
            setting(
                "Server",
                "Il catalogo arriva da qui; l'audio mai.",
                line_aligned(&app.api_host(), host_width, MUTED_LINE, host_align),
                fit,
            ),
        ],
        fit,
    )];
    if !is_phone() {
        groups = groups.push(group("Scorciatoie", shortcuts, fit));
    }
    groups
        .push(group(
            "Crediti",
            vec![note(
                "Icone",
                "Iconly (licenza MIT), con i controlli del player disegnati nello stesso tratto.",
            )],
            fit,
        ))
        .spacing(28)
        .into()
}

/// Il timer di spegnimento: il pulsante apre il menu del player con le durate.
fn sleep_control(app: &App) -> Element<'_, Message> {
    let label = match app.sleep {
        Some(timer) => format!("Attivo: {}", timer.remaining_label(std::time::Instant::now())),
        None => "Imposta…".to_string(),
    };
    outlined_button_owned(label, Some(Icon::Moon), Message::OpenMenu(MenuTarget::Player))
}
