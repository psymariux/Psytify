//! Impostazioni, come la pagina del client Iced (`client/src/ui/pages/settings.rs`) con le schede della
//! direzione artistica: Riproduzione, Suono, Download, Account, Informazioni. Gruppi di righe con il nome e
//! la spiegazione a sinistra e il controllo a destra (`SettingRow`), un separatore fra un gruppo e l'altro.
//!
//! Ogni scelta è un comando al core che esisteva già per Iced: qualità e riproduzione automatica, timer di
//! spegnimento ed equalizzatore sono `PlaybackCmd`; qualità e cartella dei download e aggiornamenti
//! `DownloadsCmd`; l'uscita dall'account `SocialCmd::SignOut`; Discord e il server stanno fra i campi del core
//! che il frontend scrive con `SettingsCmd::Store` (come fa Iced); il movimento è una preferenza di questa
//! interfaccia (`SettingsCmd::SaveUiPrefs`, chiave [`MOTION_KEY`]: segui il sistema, ridotto o completo) e si
//! applica subito al deposito degli sprite; seguendo il sistema, il core ascolta che cosa chiede
//! (`SettingsCmd::WatchSystemMotion`, `SettingsView::system_motion`) e [`follow_motion`] lo applica dal vivo. Ciò che il desktop non sa ancora fare non c'è (né spento né grigio): il
//! videoclip sulla scheda video, l'elenco Iced delle scorciatoie. Nella scheda Account, con l'accesso fatto, c'è
//! il profilo come nel client Iced: nome utente e nome (`SocialCmd::SaveProfile`; un nome già preso o troppo
//! corto lo dice l'avviso del core), «Condividi cosa ascolti» e «Il tuo profilo»; senza accesso, «Accedi».
//!
//! La pagina tiene lo specchio dei valori che mostra ([`Mirror`]) e i testi già pronti: li rifà solo quando
//! un valore cambia davvero, e solo allora si ridisegna. La lista è la `list` di GPUI (`page_list`): la
//! testa con titolo e schede è la voce 0, le righe del piano ([`plan`]) vengono dopo.

use std::sync::Arc;

use gpui::{
    AnyElement, App, AppContext, BorrowAppContext, ClickEvent, Context, Entity, EventEmitter, IntoElement,
    ListState, ParentElement, Render, SharedString, Styled, Subscription, Window, div, list,
};
use psytify_core::api::{DownloadQuality, Quality, base_host};
use psytify_core::command::{DownloadsCmd, PlaybackCmd, SettingsCmd, SleepChoice, SocialCmd, UpdateCheck};
use psytify_core::config::{self, CoreConfig, DEFAULT_DISCORD_APP_ID};
use psytify_core::player::eq::{self, PRESETS};
use psytify_core::snapshot::{Account, Snapshot, SocialView, UpdateView};
use psytify_core::system_motion::{Reason, SystemMotion};
use psytify_design::sprite;
use psytify_design::tokens::color::{SURFACE_CANVAS, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY};
use psytify_design::tokens::size::{CONTROL_H, CONTROL_H_LARGE, MENU_W};
use psytify_design::tokens::space::{SPACE_3, SPACE_4, SPACE_5, SPACE_6, SPACE_7};
use psytify_design::tokens::type_::{TYPE_BODY, TYPE_META};
use serde_json::{Map, Value};

use super::page_frame::PageFrame;
use super::page_list::{new_list, set_rows, to_top};
use super::update_banner::{Place, UpdateControl, update_line};
use super::{Page, PageEvent};
use crate::components::button::Button;
use crate::components::frame::Frame;
use crate::components::indicators::DownloadSteps;
use crate::components::navigation::{Tab as TabItem, Tabs};
use crate::components::search_field::SearchField;
use crate::components::section::{PageTitle, section_heading_at};
use crate::components::select::Select;
use crate::components::setting_row::{SettingRow, setting_gap, shortcut_row};
use crate::components::slider::SliderBank;
use crate::components::toggle::Toggle;
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::notices::Notice;
use crate::pixel::{PixelSprite, SpriteStore};
use crate::theme::{TypeStyle, col, dp};
use crate::zoom::ZoomPref;

/// La chiave della preferenza «Riduci il movimento» in `config.json` (fra quelle dei frontend):
/// `"system"`, `"reduce"` o `"full"`.
pub const MOTION_KEY: &str = "motion";

/// La chiave del passo 6.5 (un interruttore): `true` vale «Ridotto». Si legge ancora e la prima scelta
/// nuova la toglie.
pub const LEGACY_REDUCE_MOTION_KEY: &str = "reduce_motion";

/// «Riduci il movimento»: segue il sistema (il predefinito), sempre ridotto o sempre completo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MotionPref {
    #[default]
    System,
    Reduce,
    Full,
}

impl MotionPref {
    /// Nell'ordine del menu.
    const ALL: [Self; 3] = [Self::System, Self::Reduce, Self::Full];

    fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Reduce => "reduce",
            Self::Full => "full",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::System => "Segui il sistema",
            Self::Reduce => "Ridotto",
            Self::Full => "Completo",
        }
    }

    /// La preferenza salvata, letta dalle chiavi dei frontend: la chiave nuova se ha un valore noto,
    /// altrimenti la vecchia (`reduce_motion: true` è «Ridotto»), altrimenti «Segui il sistema».
    pub fn read(ui: &Map<String, Value>) -> Self {
        let chosen = ui
            .get(MOTION_KEY)
            .and_then(Value::as_str)
            .and_then(|key| Self::ALL.into_iter().find(|pref| pref.key() == key));
        chosen.unwrap_or(if ui.get(LEGACY_REDUCE_MOTION_KEY).and_then(Value::as_bool) == Some(true) {
            Self::Reduce
        } else {
            Self::System
        })
    }

    /// Le chiavi da salvare per questa scelta: la nuova, e la vecchia tolta.
    fn prefs(self) -> Map<String, Value> {
        let mut prefs = Map::new();
        prefs.insert(MOTION_KEY.to_owned(), Value::String(self.key().to_owned()));
        prefs.insert(LEGACY_REDUCE_MOTION_KEY.to_owned(), Value::Null);
        prefs
    }
}

/// Il movimento ridotto voluto dalla preferenza e dal sistema (senza l'ambiente, che `SpriteStore` aggiunge
/// da sé): «Ridotto» sempre, «Completo» mai, «Segui il sistema» quando il sistema lo chiede.
pub fn motion_reduced(pref: MotionPref, system: SystemMotion) -> bool {
    match pref {
        MotionPref::System => system.reduce(),
        MotionPref::Reduce => true,
        MotionPref::Full => false,
    }
}

/// La spiegazione della riga: seguendo il sistema dice che cosa il sistema chiede adesso.
fn motion_detail(pref: MotionPref, system: SystemMotion) -> &'static str {
    if pref != MotionPref::System {
        return "Gli sprite restano fermi, il testo salta alla riga senza scorrere e il cursore non lampeggia.";
    }
    match system.reason() {
        None => "Ora il sistema chiede movimento pieno.",
        Some(Reason::ReducedMotion) => "Ora il sistema chiede movimento ridotto (animazioni ridotte).",
        Some(Reason::PowerSaver) => "Ora il sistema chiede movimento ridotto (risparmio energetico).",
        Some(Reason::Both) => {
            "Ora il sistema chiede movimento ridotto (animazioni ridotte e risparmio energetico)."
        }
    }
}

/// Frequenze delle bande, come le legge chi regola un equalizzatore (le stesse del client Iced).
const BAND_LABELS: [&str; eq::BANDS] = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];

/// Le scorciatoie del desktop, nell'ordine in cui si imparano (`actions.rs`).
const SHORTCUTS: [(&str, &[&str]); 13] = [
    ("Riproduci o metti in pausa", &["Spazio"]),
    ("Brano successivo e precedente", &["Ctrl →", "Ctrl ←"]),
    ("Cerca", &["Ctrl K"]),
    ("Coda e testo nel pannello laterale", &["Q", "T"]),
    ("Vista immersiva", &["F"]),
    ("Indietro e avanti fra le pagine", &["Alt ←", "Alt →"]),
    ("Riproduzione casuale e ripetizione", &["Ctrl S", "Ctrl R"]),
    ("Impostazioni", &["Ctrl ,"]),
    ("Ingrandisci, riduci, dimensione automatica", &["Ctrl =", "Ctrl -", "Ctrl 0"]),
    ("Barra laterale aperta o compatta", &["Ctrl B"]),
    ("Passa da un controllo all'altro", &["Tab", "Maiusc Tab"]),
    ("Chiude menu, finestre e la vista immersiva", &["Esc"]),
    ("Esci da Psytify", &["Ctrl Q"]),
];

/// Crediti e diritti: titolo e testo.
const CREDITS: [(&str, &str); 3] = [
    ("Caratteri", "Inter e Departure Mono, con licenza SIL Open Font License 1.1."),
    ("Icone e Nastro", "Disegnati per Psytify sulla griglia dei pixel d'arte."),
    (
        "Diritti",
        "Ogni brano del catalogo ha fonte e licenza documentate dal server: si scarica solo se la licenza lo permette.",
    ),
];

/// Le schede, nell'ordine della direzione artistica.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Playback,
    Sound,
    Downloads,
    Appearance,
    Account,
    About,
}

impl Tab {
    const ALL: [Self; 6] =
        [Self::Playback, Self::Sound, Self::Downloads, Self::Appearance, Self::Account, Self::About];

    fn label(self) -> &'static str {
        match self {
            Self::Playback => "Riproduzione",
            Self::Sound => "Suono",
            Self::Downloads => "Download",
            Self::Appearance => "Aspetto",
            Self::Account => "Account",
            Self::About => "Informazioni",
        }
    }
}

/// Una riga della lista dopo la testa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Heading(&'static str),
    /// Il separatore fra due gruppi.
    Gap,
    Quality,
    Autoplay,
    Sleep,
    Zoom,
    ReduceMotion,
    EqEnabled,
    EqPreset,
    EqBands,
    DownloadQuality,
    Folder,
    Space,
    Account,
    /// «Il tuo profilo», con «Apri».
    OwnProfile,
    Handle,
    Name,
    SaveProfile,
    ShareActivity,
    Discord,
    About,
    Server,
    Update,
    Shortcut(usize),
    Credit(usize),
}

/// Le righe di una scheda, nell'ordine del client Iced. Pura.
fn plan(tab: Tab, discord: bool, profile: ProfileForm) -> Vec<Row> {
    match tab {
        Tab::Playback => vec![
            Row::Heading("Qualità"),
            Row::Quality,
            Row::Gap,
            Row::Heading("Riproduzione"),
            Row::Autoplay,
            Row::Sleep,
        ],
        Tab::Appearance => {
            vec![
                Row::Heading("Dimensioni"),
                Row::Zoom,
                Row::Gap,
                Row::Heading("Movimento"),
                Row::ReduceMotion,
            ]
        }
        Tab::Sound => vec![Row::Heading("Equalizzatore"), Row::EqEnabled, Row::EqPreset, Row::EqBands],
        Tab::Downloads => {
            vec![Row::Heading("Download"), Row::DownloadQuality, Row::Folder, Row::Space]
        }
        Tab::Account => {
            let mut rows = vec![Row::Heading("Account"), Row::Account];
            if profile != ProfileForm::None {
                if profile == ProfileForm::Saved {
                    rows.push(Row::OwnProfile);
                }
                rows.extend([Row::Gap, Row::Heading("Profilo"), Row::Handle, Row::Name, Row::SaveProfile]);
                if profile == ProfileForm::Saved {
                    rows.push(Row::ShareActivity);
                }
            }
            if discord {
                rows.extend([Row::Gap, Row::Heading("Discord"), Row::Discord]);
            }
            rows
        }
        Tab::About => {
            let mut rows =
                vec![Row::About, Row::Heading("Server e aggiornamenti"), Row::Server, Row::Update, Row::Gap];
            rows.push(Row::Heading("Scorciatoie"));
            rows.extend((0..SHORTCUTS.len()).map(Row::Shortcut));
            rows.extend([Row::Gap, Row::Heading("Crediti")]);
            rows.extend((0..CREDITS.len()).map(Row::Credit));
            rows
        }
    }
}

/// Il modulo del profilo nella scheda Account.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ProfileForm {
    /// Senza accesso: niente modulo.
    #[default]
    None,
    /// Accesso fatto, nome utente ancora da scegliere: solo i campi e «Salva».
    New,
    /// Con il profilo: anche «Il tuo profilo» e «Condividi cosa ascolti».
    Saved,
}

/// Il profilo salvato, come lo riempie il modulo.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SavedProfile {
    handle: String,
    name: String,
    share: bool,
}

/// Lo stato dell'account come lo mostra la riga.
#[derive(Clone, Debug, PartialEq, Eq)]
enum AccountShown {
    Unavailable,
    SignedOut,
    Working,
    /// «Nome · @utente · amministratore».
    SignedIn(String),
}

impl AccountShown {
    fn of(account: &Account) -> Self {
        match account {
            Account::Unavailable => Self::Unavailable,
            Account::SignedOut => Self::SignedOut,
            Account::Working => Self::Working,
            Account::SignedIn(me) => {
                let role = if me.admin { " · amministratore" } else { "" };
                Self::SignedIn(match &me.profile {
                    Some(profile) => format!("{} · @{}{role}", profile.display_name, profile.handle),
                    None => format!("Nessun nome utente ancora{role}"),
                })
            }
        }
    }
}

/// I valori che la pagina mostra, presi dall'istantanea: se non cambiano, la pagina non si ridisegna.
#[derive(Clone, Debug, PartialEq)]
struct Mirror {
    quality: Quality,
    autoplay: bool,
    sleep: Option<SleepChoice>,
    eq: eq::Settings,
    download_quality: DownloadQuality,
    folder: std::path::PathBuf,
    saved: usize,
    used_bytes: u64,
    update: UpdateView,
    account: AccountShown,
    profile_form: ProfileForm,
    profile: Option<SavedProfile>,
    profile_saving: bool,
    server: Option<String>,
    discord: bool,
    discord_available: bool,
    motion: MotionPref,
    system_motion: SystemMotion,
    zoom: ZoomPref,
}

impl Mirror {
    fn of(snapshot: &Snapshot, social: &SocialView, update: &UpdateView) -> Self {
        let config = &snapshot.settings.config;
        let profile = social.account.profile().map(|profile| SavedProfile {
            handle: profile.handle.clone(),
            name: profile.display_name.clone(),
            share: profile.share_activity,
        });
        let profile_form = match (&social.account, &profile) {
            (Account::SignedIn(_), Some(_)) => ProfileForm::Saved,
            (Account::SignedIn(_), None) => ProfileForm::New,
            _ => ProfileForm::None,
        };
        Self {
            quality: snapshot.playback.quality,
            autoplay: snapshot.playback.autoplay,
            sleep: snapshot.playback.sleep.map(|timer| timer.choice),
            eq: snapshot.playback.equalizer,
            download_quality: snapshot.downloads.quality,
            folder: snapshot.downloads.folder.clone(),
            saved: snapshot.downloads.saved.len(),
            used_bytes: snapshot.downloads.used_bytes,
            update: update.clone(),
            account: AccountShown::of(&social.account),
            profile_form,
            profile,
            profile_saving: social.profile_saving,
            server: config.server.clone(),
            discord: config.discord_presence,
            discord_available: discord_available(config),
            motion: MotionPref::read(&snapshot.settings.ui),
            system_motion: snapshot.settings.system_motion,
            zoom: ZoomPref::read(&snapshot.settings.ui),
        }
    }
}

/// Come nel client Iced: l'interruttore c'è solo se la build o la configurazione hanno un'app Discord.
fn discord_available(config: &CoreConfig) -> bool {
    let chosen = config.discord_app_id.trim();
    !(if chosen.is_empty() { DEFAULT_DISCORD_APP_ID } else { chosen }).is_empty()
}

/// Le scelte del timer di spegnimento: «Spento» e poi quelle del core.
fn sleep_options() -> Vec<SharedString> {
    std::iter::once(SharedString::from("Spento"))
        .chain(SleepChoice::ALL.iter().map(|choice| SharedString::from(choice.label())))
        .collect()
}

fn sleep_index(choice: Option<SleepChoice>) -> usize {
    choice
        .and_then(|choice| SleepChoice::ALL.iter().position(|known| *known == choice))
        .map_or(0, |ix| ix + 1)
}

/// L'avviso del timer, come nel client Iced.
fn sleep_notice(choice: Option<SleepChoice>) -> String {
    match choice {
        Some(choice) => format!("Timer di spegnimento: {}", choice.label().to_lowercase()),
        None => "Timer di spegnimento disattivato".to_owned(),
    }
}

/// La spiegazione della qualità d'ascolto (i testi del client Iced).
fn quality_detail(quality: Quality) -> &'static str {
    match quality {
        Quality::Lossless => {
            "FLAC o WAV senza perdita da Internet Archive. Se un brano non ce l'ha, suona la versione standard."
        }
        Quality::Standard => {
            "Versione compressa (m4a) da YouTube, più leggera. Se un brano non ce l'ha, suona la lossless."
        }
    }
}

/// «+3», «−6» o «0»: il guadagno di una banda in dB (come il client Iced).
fn gain_label(gain: f32) -> String {
    if gain.abs() < 0.5 { "0".into() } else { format!("{gain:+.0}").replace('-', "−") }
}

/// Il guadagno sotto il puntatore: la frazione dello slider (1 in alto), a passi di 0,5 dB.
fn band_gain(fraction: f32) -> f32 {
    let gain = -eq::MAX_GAIN_DB + fraction * 2.0 * eq::MAX_GAIN_DB;
    (gain * 2.0).round() / 2.0
}

/// La posizione dello slider per un guadagno.
fn band_fraction(gain: f32) -> f32 {
    ((gain + eq::MAX_GAIN_DB) / (2.0 * eq::MAX_GAIN_DB)).clamp(0.0, 1.0)
}

/// Il preset che corrisponde alle bande, se c'è.
fn preset_index(gains: &[f32; eq::BANDS]) -> Option<usize> {
    PRESETS.iter().position(|(_, preset)| preset == gains)
}

/// Dove arriva il catalogo (`host`, quello in uso), e se è il server predefinito o uno scelto qui (i
/// testi del client Iced).
fn server_detail(host: &str, chosen: bool) -> String {
    if chosen {
        format!("In uso: {host}, scelto da te. Vuoto torna al predefinito.")
    } else {
        format!("In uso: {host}, il predefinito. L'audio passa sempre da qui.")
    }
}

/// L'host in uso: quello dell'ambiente (`PSYTIFY_API_URL`), della scelta o del predefinito.
fn host_in_use(server: Option<&str>) -> String {
    let url = config::api_url(server);
    base_host(&url).unwrap_or(url)
}

/// I testi già pronti per il disegno.
#[derive(Clone, Debug, Default)]
struct Texts {
    download_detail: SharedString,
    folder: SharedString,
    space: SharedString,
    update_detail: SharedString,
    server_detail: SharedString,
    motion_detail: SharedString,
    band_values: Vec<f32>,
    band_texts: Vec<SharedString>,
}

impl Texts {
    fn of(mirror: &Mirror) -> Self {
        let update = update_line(&mirror.update, Place::Settings).map(|line| line.text).unwrap_or_default();
        Self {
            download_detail: format!(
                "{}. Dal menu «Altro» di un brano («Scarica in…») si sceglie caso per caso. Un file con perdita \
                 non si migliora: se il brano non ha la qualità chiesta arriva quella più vicina.",
                mirror.download_quality.description()
            )
            .into(),
            folder: format!("I brani scaricati arrivano in {}.", mirror.folder.display()).into(),
            space: if mirror.saved == 0 {
                "Nessun brano sul dispositivo.".into()
            } else {
                format!("{} · {}, suonano anche senza rete.", format::tracks(mirror.saved), format::size(mirror.used_bytes))
                    .into()
            },
            update_detail: update.into(),
            server_detail: server_detail(&host_in_use(mirror.server.as_deref()), mirror.server.is_some()).into(),
            motion_detail: motion_detail(mirror.motion, mirror.system_motion).into(),
            band_values: mirror.eq.gains.iter().copied().map(band_fraction).collect(),
            band_texts: mirror.eq.gains.iter().map(|gain| SharedString::from(gain_label(*gain))).collect(),
        }
    }
}

pub struct SettingsPage {
    core: Entity<CoreModel>,
    tab: Tab,
    mirror: Mirror,
    texts: Texts,
    rows: Vec<Row>,
    list: ListState,
    frame: PageFrame,
    /// Il campo dell'indirizzo del server.
    server: Entity<SearchField>,
    /// I campi del profilo: nome utente e nome.
    handle: Entity<SearchField>,
    name: Entity<SearchField>,
    quality_options: Vec<SharedString>,
    download_options: Vec<SharedString>,
    sleep_options: Vec<SharedString>,
    preset_options: Vec<SharedString>,
    motion_options: Vec<SharedString>,
    zoom_options: Vec<SharedString>,
    version: SharedString,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for SettingsPage {}

impl SettingsPage {
    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let changes =
            cx.subscribe(&core, |this, core, changed: &Changed, cx| this.refresh(&core, *changed, cx));
        let snapshot = core.read(cx).snapshot();
        let mirror = Mirror::of(&snapshot, &core.read(cx).social_view(), core.read(cx).update_view());
        let page = cx.entity().downgrade();
        let submit = page.clone();
        let server = cx.new(|cx| {
            SearchField::new("settings-server", config::default_api_url(), cx)
                .plain()
                .with_text(mirror.server.clone().unwrap_or_default())
                .on_submit(move |text, _, cx| {
                    let text = text.to_owned();
                    submit.update(cx, |page, cx| page.apply_server(&text, cx)).ok();
                })
        });
        let saved = mirror.profile.clone().unwrap_or_default();
        let (save_handle, save_name) = (page.clone(), page.clone());
        let handle = cx.new(|cx| {
            SearchField::new("settings-handle", "nome_utente", cx)
                .plain()
                .large()
                .sanitize(super::people::handle_chars)
                .max_chars(super::people::HANDLE_MAX)
                .with_text(saved.handle.clone())
                .on_submit(move |_, _, cx| {
                    let page = save_handle.clone();
                    cx.defer(move |cx| {
                        page.update(cx, |page, cx| page.save_profile(None, cx)).ok();
                    });
                })
        });
        let name = cx.new(|cx| {
            SearchField::new("settings-name", "Il tuo nome", cx)
                .plain()
                .large()
                .sanitize(super::people::line_chars)
                .max_chars(super::people::NAME_MAX)
                .with_text(saved.name.clone())
                .on_submit(move |_, _, cx| {
                    let page = save_name.clone();
                    cx.defer(move |cx| {
                        page.update(cx, |page, cx| page.save_profile(None, cx)).ok();
                    });
                })
        });
        let rows = plan(Tab::default(), mirror.discord_available, mirror.profile_form);
        let list = new_list(rows.len());
        let texts = Texts::of(&mirror);
        let _ = page;
        Self {
            core,
            tab: Tab::default(),
            texts,
            mirror,
            list,
            frame: PageFrame::default(),
            rows,
            server,
            handle,
            name,
            quality_options: vec!["Lossless".into(), "Standard".into()],
            download_options: DownloadQuality::ALL
                .iter()
                .map(|quality| SharedString::from(format!("{} · {}", quality.label(), quality.detail())))
                .collect(),
            sleep_options: sleep_options(),
            preset_options: PRESETS.iter().map(|(name, _)| SharedString::from(*name)).collect(),
            motion_options: MotionPref::ALL.iter().map(|pref| SharedString::from(pref.label())).collect(),
            zoom_options: ZoomPref::options()
                .into_iter()
                .map(|pref| SharedString::from(pref.label()))
                .collect(),
            version: format!(
                "Versione {}",
                psytify_core::api::update::BUILD_TAG.unwrap_or(env!("CARGO_PKG_VERSION"))
            )
            .into(),
            _changes: changes,
        }
    }

    fn refresh(&mut self, core: &Entity<CoreModel>, changed: Changed, cx: &mut Context<Self>) {
        if !(changed.playback || changed.downloads || changed.settings || changed.social) {
            return;
        }
        let snapshot = core.read(cx).snapshot();
        let mirror = Mirror::of(&snapshot, &core.read(cx).social_view(), core.read(cx).update_view());
        self.take(mirror, cx);
    }

    /// Prende i valori nuovi; se qualcosa è cambiato rifà i testi e le righe e si ridisegna.
    fn take(&mut self, mirror: Mirror, cx: &mut Context<Self>) {
        if mirror == self.mirror {
            return;
        }
        if mirror.server != self.mirror.server {
            let text = mirror.server.clone().unwrap_or_default();
            self.server.update(cx, |field, cx| field.set_text(text, cx));
        }
        if mirror.profile != self.mirror.profile {
            // Il profilo salvato (o arrivato con l'accesso) riempie di nuovo i campi, come nel client Iced.
            let saved = mirror.profile.clone().unwrap_or_default();
            self.handle.update(cx, |field, cx| field.set_text(saved.handle, cx));
            self.name.update(cx, |field, cx| field.set_text(saved.name, cx));
        }
        self.texts = Texts::of(&mirror);
        self.mirror = mirror;
        self.rows = plan(self.tab, self.mirror.discord_available, self.mirror.profile_form);
        // Le righe si rimisurano tutte (sono poche): una spiegazione cambiata può cambiare altezza.
        set_rows(&self.list, self.rows.len());
        cx.notify();
    }

    /// Apre la scheda Account (da «Modifica profilo» o dagli Amici senza nome utente).
    pub fn show_account(&mut self, cx: &mut Context<Self>) {
        self.select_tab(Tab::Account, cx);
    }

    /// «Salva il profilo» o Invio in un campo; con `share` cambia anche la condivisione dell'attività (che usa
    /// il profilo salvato, non i campi a metà).
    fn save_profile(&mut self, share: Option<bool>, cx: &mut Context<Self>) {
        let (handle, display_name) = match (share, &self.mirror.profile) {
            (Some(_), Some(saved)) => (saved.handle.clone(), saved.name.clone()),
            (Some(_), None) => return,
            (None, _) => {
                (self.handle.read(cx).text().to_owned(), self.name.read(cx).text().trim().to_owned())
            }
        };
        self.core.read(cx).social(SocialCmd::SaveProfile { handle, display_name, share_activity: share });
    }

    fn select_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if tab != self.tab {
            self.tab = tab;
            self.rows = plan(tab, self.mirror.discord_available, self.mirror.profile_form);
            set_rows(&self.list, self.rows.len());
            to_top(&self.list);
            cx.notify();
        }
    }

    fn notice(&mut self, notice: Notice, cx: &mut Context<Self>) {
        cx.emit(PageEvent::Notice(notice));
    }

    fn playback(&self, command: PlaybackCmd, cx: &App) {
        self.core.read(cx).playback(command);
    }

    /// Un campo del core che il frontend scrive (Discord, il server): la configurazione di adesso con quel
    /// campo cambiato. Il core ignora i campi che possiedono i suoi domini.
    fn store(&self, change: impl FnOnce(&mut CoreConfig), cx: &App) {
        let core = self.core.read(cx);
        let mut config = (*core.snapshot().settings.config).clone();
        change(&mut config);
        core.settings(SettingsCmd::Store(Box::new(config)));
    }

    fn set_quality(&mut self, quality: Quality, cx: &mut Context<Self>) {
        if quality != self.mirror.quality {
            self.notice(Notice::info(format!("Qualità {}: vale dal prossimo brano", quality.label())), cx);
        }
        self.playback(PlaybackCmd::Quality(quality), cx);
    }

    fn set_sleep(&mut self, choice: Option<SleepChoice>, cx: &mut Context<Self>) {
        self.notice(Notice::info(sleep_notice(choice)), cx);
        self.playback(PlaybackCmd::Sleep(choice), cx);
    }

    /// «Riduci il movimento»: si salva fra le preferenze di questa interfaccia e vale subito. Il core ascolta
    /// il sistema solo mentre la scelta lo segue.
    fn set_motion(&mut self, pref: MotionPref, cx: &mut Context<Self>) {
        if pref == self.mirror.motion {
            return;
        }
        let core = self.core.read(cx);
        core.settings(SettingsCmd::SaveUiPrefs(pref.prefs()));
        core.settings(SettingsCmd::WatchSystemMotion(pref == MotionPref::System));
        apply_reduce_motion(motion_reduced(pref, self.mirror.system_motion), cx);
        let mut mirror = self.mirror.clone();
        mirror.motion = pref;
        self.take(mirror, cx);
    }

    /// The UI zoom: saved with the frontend preferences; the window applies it when the core publishes.
    fn set_zoom(&mut self, pref: ZoomPref, cx: &mut Context<Self>) {
        if pref != self.mirror.zoom {
            self.core.read(cx).settings(SettingsCmd::SaveUiPrefs(pref.prefs()));
        }
    }

    /// «Usa» o Invio nel campo del server: come il client Iced, un indirizzo vuoto o uguale al predefinito
    /// torna al predefinito; uno non valido dà un avviso e non cambia niente.
    fn apply_server(&mut self, text: &str, cx: &mut Context<Self>) {
        let server = config::normalize_server(text);
        match base_host(server.as_deref().unwrap_or(config::default_api_url())) {
            Err(error) => self.notice(Notice::error(error.to_string()), cx),
            Ok(host) => {
                if server == self.mirror.server {
                    return;
                }
                self.notice(Notice::info(format!("Server: {host}")), cx);
                self.store(move |config| config.server = server, cx);
            }
        }
    }

    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = Tab::ALL.iter().position(|tab| *tab == self.tab).unwrap_or(0);
        let tabs = Tabs::new(
            Tab::ALL
                .iter()
                .enumerate()
                .map(|(ix, tab)| TabItem::new(("settings-tab", ix), tab.label()))
                .collect(),
            selected,
        )
        .on_select({
            let pick = cx.listener(|this, ix: &usize, _, cx| {
                if let Some(tab) = Tab::ALL.get(*ix) {
                    this.select_tab(*tab, cx);
                }
            });
            move |ix, window, cx| pick(&ix, window, cx)
        });
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(PageTitle::screen("Impostazioni").margin(self.frame.margin()))
            .child(div().px(dp(self.frame.margin())).pt(dp(SPACE_3)).child(tabs))
            .into_any_element()
    }

    /// The page comes into view (`Shell::page_shown`).
    pub fn enter(&mut self, rise: bool, cx: &mut Context<Self>) {
        self.frame.enter(rise, cx);
        cx.notify();
    }

    fn set_frame(&mut self, frame: Frame, cx: &mut Context<Self>) {
        if frame != self.frame.frame {
            self.frame.frame = frame;
            cx.notify();
        }
    }

    fn item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if ix == 0 {
            let header = self.header(cx);
            return self.frame.wrap(header, 0, window);
        }
        let element = match self.rows.get(ix - 1).copied() {
            // Rows carry `SPACE_6` each side: the rest of the frame margin goes around them.
            Some(row @ Row::Heading(_)) => self.row(row, window, cx),
            Some(row) => div()
                .w_full()
                .px(dp((self.frame.margin() - SPACE_6).max(0.0)))
                .child(self.row(row, window, cx))
                .into_any_element(),
            None => div().into_any_element(),
        };
        let section = 1 + self.rows.iter().take(ix).filter(|row| matches!(row, Row::Heading(_))).count();
        self.frame.wrap(element, section, window)
    }

    fn row(&mut self, row: Row, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let margin = self.frame.margin();
        let m = &self.mirror;
        match row {
            Row::Heading(title) => {
                section_heading_at(title.into(), SharedString::default(), margin).into_any_element()
            }
            Row::Gap => setting_gap().into_any_element(),
            Row::Quality => {
                let current = usize::from(m.quality == Quality::Standard);
                SettingRow::new("Qualità d'ascolto")
                    .description(quality_detail(m.quality))
                    .control(
                        Select::new("set-quality", self.quality_options.clone(), Some(current))
                            .min_width(MENU_W / 2.0)
                            .on_select(self.select(|this, ix, cx| {
                                let quality = if ix == 0 { Quality::Lossless } else { Quality::Standard };
                                this.set_quality(quality, cx);
                            }, cx)),
                    )
                    .into_any_element()
            }
            Row::Autoplay => SettingRow::new("Riproduzione automatica")
                .description(if m.autoplay {
                    "Finita la coda, continua da solo con altri brani dello stesso artista."
                } else {
                    "Finita la coda, la musica si ferma."
                })
                .control(Toggle::new("set-autoplay", m.autoplay).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        let on = !this.mirror.autoplay;
                        this.playback(PlaybackCmd::Autoplay(on), cx);
                    },
                )))
                .into_any_element(),
            Row::Sleep => SettingRow::new("Timer di spegnimento")
                .description("Ferma la musica dopo un tempo o alla fine del brano.")
                .control(
                    Select::new("set-sleep", self.sleep_options.clone(), Some(sleep_index(m.sleep)))
                        .min_width(MENU_W / 2.0)
                        .on_select(self.select(|this, ix, cx| {
                            let choice = ix.checked_sub(1).and_then(|ix| SleepChoice::ALL.get(ix).copied());
                            this.set_sleep(choice, cx);
                        }, cx)),
                )
                .into_any_element(),
            Row::Zoom => {
                let current = ZoomPref::options().iter().position(|pref| *pref == m.zoom);
                SettingRow::new("Dimensione dell'interfaccia")
                    .description(
                        "Automatica segue la larghezza della finestra. Ctrl = e Ctrl - cambiano, Ctrl 0 torna ad automatica.",
                    )
                    .control(
                        Select::new("set-zoom", self.zoom_options.clone(), current)
                            .min_width(MENU_W / 2.0)
                            .on_select(self.select(|this, ix, cx| {
                                if let Some(pref) = ZoomPref::options().get(ix) {
                                    this.set_zoom(*pref, cx);
                                }
                            }, cx)),
                    )
                    .into_any_element()
            }
            Row::ReduceMotion => {
                let current = MotionPref::ALL.iter().position(|pref| *pref == m.motion);
                SettingRow::new("Riduci il movimento")
                    .description(self.texts.motion_detail.clone())
                    .control(
                        Select::new("set-motion", self.motion_options.clone(), current)
                            .min_width(MENU_W / 2.0)
                            .on_select(self.select(|this, ix, cx| {
                                if let Some(pref) = MotionPref::ALL.get(ix) {
                                    this.set_motion(*pref, cx);
                                }
                            }, cx)),
                    )
                    .into_any_element()
            }
            Row::EqEnabled => SettingRow::new("Equalizzatore a 10 bande")
                .description("Dai più corpo ai bassi o alla voce. Si sente subito.")
                .control(Toggle::new("set-eq", m.eq.enabled).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        let on = !this.mirror.eq.enabled;
                        this.playback(PlaybackCmd::EqualizerEnabled(on), cx);
                    },
                )))
                .into_any_element(),
            Row::EqPreset => SettingRow::new("Preset")
                .description("Regolazioni pronte. Muovere una banda le personalizza.")
                .control(
                    Select::new("set-preset", self.preset_options.clone(), preset_index(&m.eq.gains))
                        .placeholder("Personalizzato")
                        .min_width(MENU_W / 2.0)
                        .on_select(self.select(|this, ix, cx| {
                            if let Some((_, gains)) = PRESETS.get(ix) {
                                this.playback(PlaybackCmd::EqualizerGains(*gains), cx);
                            }
                        }, cx)),
                )
                .into_any_element(),
            Row::EqBands => self.bands(window, cx),
            Row::DownloadQuality => {
                let current = DownloadQuality::ALL.iter().position(|quality| *quality == m.download_quality);
                SettingRow::new("Qualità dei download")
                    .description(self.texts.download_detail.clone())
                    .control(
                        Select::new("set-download", self.download_options.clone(), current)
                            .min_width(MENU_W * 0.75)
                            .on_select(self.select(|this, ix, cx| {
                                if let Some(quality) = DownloadQuality::ALL.get(ix) {
                                    this.core.read(cx).downloads(DownloadsCmd::SetQuality(*quality));
                                }
                            }, cx)),
                    )
                    .into_any_element()
            }
            Row::Folder => SettingRow::new("Cartella dei download")
                .description(self.texts.folder.clone())
                .control(Button::new("set-folder", "Apri cartella").secondary().on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| this.core.read(cx).downloads(DownloadsCmd::OpenFolder),
                )))
                .into_any_element(),
            Row::Space => SettingRow::new("Sul dispositivo")
                .description(self.texts.space.clone())
                .control(Button::new("set-space", "Vai a Scaricati").secondary().on_click(cx.listener(
                    |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::Downloads)),
                )))
                .into_any_element(),
            Row::Account => self.account(cx),
            Row::OwnProfile => {
                let handle = self.mirror.profile.as_ref().map(|p| p.handle.clone()).unwrap_or_default();
                SettingRow::new("Il tuo profilo")
                    .description("Come ti vedono gli amici: ascolti, artisti e playlist pubbliche.")
                    .control(Button::new("set-own-profile", "Apri").secondary().on_click(cx.listener(
                        move |_, _: &ClickEvent, _, cx| cx.emit(PageEvent::OpenProfile(handle.clone())),
                    )))
                    .into_any_element()
            }
            Row::Handle => SettingRow::new("Nome utente")
                .description("Gli amici ti trovano così: da 3 a 30 fra lettere, cifre e _.")
                .control(div().w(dp(MENU_W)).h(dp(CONTROL_H_LARGE)).child(self.handle.clone()))
                .into_any_element(),
            Row::Name => SettingRow::new("Nome")
                .description("Il nome che vedono gli altri.")
                .control(div().w(dp(MENU_W)).h(dp(CONTROL_H_LARGE)).child(self.name.clone()))
                .into_any_element(),
            Row::SaveProfile => {
                let saving = m.profile_saving;
                SettingRow::new("")
                    .control(
                        Button::new("set-save-profile", if saving { "Salvataggio…" } else { "Salva il profilo" })
                            .primary()
                            .loading(saving)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.save_profile(None, cx))),
                    )
                    .into_any_element()
            }
            Row::ShareActivity => {
                let on = m.profile.as_ref().is_some_and(|p| p.share);
                SettingRow::new("Condividi cosa ascolti")
                    .description(if on {
                        "Gli amici vedono il brano che stai ascoltando e i tuoi ascolti recenti."
                    } else {
                        "Nessuno vede cosa ascolti: gli amici vedono solo il tuo nome."
                    })
                    .control(Toggle::new("set-share", on).disabled(m.profile_saving).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| this.save_profile(Some(!on), cx),
                    )))
                    .into_any_element()
            }
            Row::Discord => SettingRow::new("Mostra su Discord cosa ascolti")
                .description(
                    "Con Discord aperto su questo computer, il tuo profilo dice «Sta ascoltando Psytify» con il brano.",
                )
                .control(Toggle::new("set-discord", m.discord).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        let on = !this.mirror.discord;
                        this.store(move |config| config.discord_presence = on, cx);
                    },
                )))
                .into_any_element(),
            Row::About => self.about(window),
            Row::Server => self.server_row(cx),
            Row::Update => self.update(cx),
            Row::Shortcut(ix) => match SHORTCUTS.get(ix) {
                Some((what, keys)) => shortcut_row(*what, keys).into_any_element(),
                None => div().into_any_element(),
            },
            Row::Credit(ix) => match CREDITS.get(ix) {
                Some((title, text)) => SettingRow::new(*title).description(*text).into_any_element(),
                None => div().into_any_element(),
            },
        }
    }

    /// Adatta una scelta di un `Select` (l'indice) a un metodo della pagina.
    fn select(
        &self,
        f: impl Fn(&mut Self, usize, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl Fn(usize, &mut Window, &mut App) + 'static {
        let pick = cx.listener(move |this, ix: &usize, _, cx| f(this, *ix, cx));
        move |ix, window, cx| pick(&ix, window, cx)
    }

    /// Le dieci bande: ogni movimento si sente subito (`EqualizerBand`), il rilascio salva (`Save`).
    fn bands(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let change = cx.listener(|this, (band, fraction): &(usize, f32), _, cx| {
            this.playback(PlaybackCmd::EqualizerBand { band: *band, gain: band_gain(*fraction) }, cx);
        });
        let commit = cx.listener(|this, _: &(), _, cx| this.playback(PlaybackCmd::Save, cx));
        let bank = SliderBank::new("set-bands", self.texts.band_values.clone())
            .values_text(self.texts.band_texts.clone())
            .names(BAND_LABELS.iter().map(|label| SharedString::from(*label)).collect())
            .tick(0.5)
            .step(0.5 / (2.0 * eq::MAX_GAIN_DB))
            .on_change(move |band, fraction, window, cx| change(&(band, fraction), window, cx))
            .on_commit(move |_, _, window, cx| commit(&(), window, cx));
        div()
            .w_full()
            .flex()
            .flex_col()
            .items_start()
            .gap(dp(SPACE_4))
            .px(dp(SPACE_6))
            .py(dp(SPACE_5))
            .child(bank)
            .child(
                div()
                    .type_role(TYPE_META)
                    .text_color(col(TEXT_TERTIARY))
                    .child("Guadagno in dB per banda, da −12 a +12. Vale anche mentre un brano suona."),
            )
            .into_any_element()
    }

    fn account(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.mirror.account {
            AccountShown::Unavailable => SettingRow::new("Accesso non configurato")
                .description("Imposta clerk_issuer in config.json per attivarlo.")
                .into_any_element(),
            AccountShown::SignedOut => SettingRow::new("Accedi a Psytify")
                .description(
                    "Facoltativo: catalogo e ascolto restano liberi. Con l'account trovi gli amici e vedi cosa ascoltano.",
                )
                .control(Button::new("set-signin", "Accedi").secondary().on_click(
                    cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PageEvent::Open(Page::SignIn))),
                ))
                .into_any_element(),
            AccountShown::Working => SettingRow::new("Accesso in corso")
                .description("Un momento…")
                .control(PixelSprite::new(sprite::LOADER))
                .into_any_element(),
            AccountShown::SignedIn(who) => SettingRow::new("Collegato")
                .description(who.clone())
                .control(Button::new("set-signout", "Esci").secondary().on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| this.core.read(cx).social(SocialCmd::SignOut),
                )))
                .into_any_element(),
        }
    }

    /// Nastro, il nome in tipo pixel e la versione.
    fn about(&self, _window: &mut Window) -> AnyElement {
        div()
            .w_full()
            .flex()
            .items_center()
            .gap(dp(SPACE_6))
            .px(dp(SPACE_6))
            .pt(dp(SPACE_5))
            .pb(dp(SPACE_3))
            .child(PixelSprite::new(sprite::NASTRO_IDLE))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(dp(SPACE_3))
                    .child(div().text_color(col(TEXT_PRIMARY)).child("Psytify").pixel_type(false))
                    .child(div().text_color(col(TEXT_SECONDARY)).child(self.version.clone()).pixel_type(true))
                    .child(
                        div()
                            .type_role(TYPE_BODY)
                            .text_color(col(TEXT_SECONDARY))
                            .child("Player desktop per il catalogo Psytify."),
                    ),
            )
            .into_any_element()
    }

    fn server_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut actions = div()
            .flex()
            .items_center()
            .gap(dp(SPACE_3))
            .child(div().w(dp(MENU_W)).h(dp(CONTROL_H)).child(self.server.clone()))
            .child(Button::new("set-server-use", "Usa").secondary().on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    let text = this.server.read(cx).text().to_owned();
                    this.apply_server(&text, cx);
                },
            )));
        if self.mirror.server.is_some() {
            actions = actions.child(
                Button::new("set-server-reset", "Predefinito")
                    .secondary()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.apply_server("", cx))),
            );
        }
        SettingRow::new("Server")
            .description(self.texts.server_detail.clone())
            .control(actions)
            .into_any_element()
    }

    fn update(&self, cx: &mut Context<Self>) -> AnyElement {
        let control = update_line(&self.mirror.update, Place::Settings).and_then(|line| line.control);
        let control: AnyElement = match control.unwrap_or(UpdateControl::Check { checking: false }) {
            UpdateControl::Download => Button::new("set-update", "Scarica")
                .primary()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.core.read(cx).downloads(DownloadsCmd::DownloadUpdate)
                }))
                .into_any_element(),
            // La barra a dieci passi della fascia; senza la lunghezza (il loader nella fascia) basta il testo.
            UpdateControl::Steps(Some(percent)) => {
                DownloadSteps::new(f32::from(percent) / 100.0).into_any_element()
            }
            UpdateControl::Steps(None) => {
                div().type_role(TYPE_META).text_color(col(TEXT_TERTIARY)).child("In corso").into_any_element()
            }
            UpdateControl::Reveal => Button::new("set-update", "Apri cartella")
                .secondary()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.core.read(cx).downloads(DownloadsCmd::RevealUpdate)
                }))
                .into_any_element(),
            UpdateControl::Check { checking } => Button::new("set-update", "Controlla ora")
                .secondary()
                .loading(checking)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.core.read(cx).downloads(DownloadsCmd::CheckUpdate(UpdateCheck::Manual))
                }))
                .into_any_element(),
        };
        SettingRow::new("Aggiornamento")
            .description(self.texts.update_detail.clone())
            .control(control)
            .into_any_element()
    }
}

/// «Riduci il movimento» per tutta l'app, prima del primo disegno: legge la preferenza, accende l'ascolto
/// del sistema se la preferenza lo segue e applica la scelta; poi la rifà a ogni pubblicazione delle
/// impostazioni (la preferenza salvata, o il sistema che cambia: risparmio energetico, animazioni). Il
/// ridisegno avviene solo quando il movimento in vigore cambia davvero.
pub fn follow_motion(core: &Entity<CoreModel>, cx: &mut App) {
    let model = core.read(cx);
    let settings = Arc::clone(&model.snapshot().settings);
    let pref = MotionPref::read(&settings.ui);
    model.settings(SettingsCmd::WatchSystemMotion(pref == MotionPref::System));
    apply_reduce_motion(motion_reduced(pref, settings.system_motion), cx);
    cx.subscribe(core, |core, changed: &Changed, cx| {
        if changed.settings {
            let settings = Arc::clone(&core.read(cx).snapshot().settings);
            apply_reduce_motion(motion_reduced(MotionPref::read(&settings.ui), settings.system_motion), cx);
        }
    })
    .detach();
}

/// Applica la preferenza al deposito degli sprite e ridisegna le finestre, così ogni animazione segue la
/// scelta subito (anche le viste in cache).
/// Anche il segnale di GPUI (`App::set_reduce_motion`, letto dai suoi elementi animati) segue la scelta.
pub fn apply_reduce_motion(on: bool, cx: &mut App) {
    if cx.try_global::<SpriteStore>().is_none() {
        return;
    }
    let changed = cx.update_global::<SpriteStore, _>(|store, _| store.set_reduce_motion_pref(on));
    let effective = cx.global::<SpriteStore>().reduce_motion();
    cx.set_reduce_motion(effective);
    if changed {
        cx.refresh_windows();
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame.begin(window, Some(&self.list));
        div()
            .relative()
            .size_full()
            .bg(col(SURFACE_CANVAS))
            .pb(dp(SPACE_7))
            .child(self.frame.measure(cx.entity().downgrade(), Self::set_frame))
            .child(
                list(
                    self.list.clone(),
                    cx.processor(|this: &mut Self, ix: usize, window, cx| this.item(ix, window, cx)),
                )
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_tab_lists_its_groups_in_the_iced_order() {
        assert_eq!(
            plan(Tab::Playback, false, ProfileForm::None),
            vec![
                Row::Heading("Qualità"),
                Row::Quality,
                Row::Gap,
                Row::Heading("Riproduzione"),
                Row::Autoplay,
                Row::Sleep,
            ]
        );
        assert_eq!(
            plan(Tab::Appearance, false, ProfileForm::None),
            vec![
                Row::Heading("Dimensioni"),
                Row::Zoom,
                Row::Gap,
                Row::Heading("Movimento"),
                Row::ReduceMotion
            ]
        );
        assert_eq!(plan(Tab::Sound, false, ProfileForm::None).last(), Some(&Row::EqBands));
        assert_eq!(plan(Tab::Downloads, false, ProfileForm::None)[1], Row::DownloadQuality);
        let about = plan(Tab::About, false, ProfileForm::None);
        assert_eq!(
            &about[..4],
            &[Row::About, Row::Heading("Server e aggiornamenti"), Row::Server, Row::Update]
        );
        assert_eq!(about.iter().filter(|row| matches!(row, Row::Shortcut(_))).count(), SHORTCUTS.len());
        assert_eq!(about.last(), Some(&Row::Credit(CREDITS.len() - 1)));
    }

    #[test]
    fn discord_is_there_only_when_the_build_or_the_config_has_an_app() {
        assert!(!plan(Tab::Account, false, ProfileForm::None).contains(&Row::Discord));
        assert_eq!(
            plan(Tab::Account, true, ProfileForm::None)[2..],
            [Row::Gap, Row::Heading("Discord"), Row::Discord]
        );
        let mut config = CoreConfig { discord_app_id: "123".into(), ..CoreConfig::default() };
        assert!(discord_available(&config));
        config.discord_app_id = "  ".into();
        assert_eq!(discord_available(&config), !DEFAULT_DISCORD_APP_ID.is_empty());
    }

    #[test]
    fn band_positions_map_to_half_decibel_steps_and_back() {
        assert_eq!(band_gain(1.0), eq::MAX_GAIN_DB);
        assert_eq!(band_gain(0.0), -eq::MAX_GAIN_DB);
        assert_eq!(band_gain(0.5), 0.0);
        assert_eq!(band_gain(0.75), 6.0);
        assert_eq!(band_gain(0.52), 0.5);
        for gain in [-12.0, -6.5, 0.0, 3.0, 12.0] {
            assert_eq!(band_gain(band_fraction(gain)), gain);
        }
        assert_eq!(gain_label(0.2), "0");
        assert_eq!(gain_label(3.0), "+3");
        assert_eq!(gain_label(-6.0), "−6");
    }

    #[test]
    fn the_preset_is_found_only_when_every_band_matches() {
        assert_eq!(preset_index(&[0.0; eq::BANDS]), Some(0));
        let mut bass = PRESETS[1].1;
        assert_eq!(preset_index(&bass), Some(1));
        bass[9] = 1.0;
        assert_eq!(preset_index(&bass), None);
    }

    #[test]
    fn the_sleep_choices_start_with_off_and_say_what_they_do() {
        let options = sleep_options();
        assert_eq!(options.len(), SleepChoice::ALL.len() + 1);
        assert_eq!(options[0].as_ref(), "Spento");
        assert_eq!(sleep_index(None), 0);
        assert_eq!(sleep_index(Some(SleepChoice::EndOfTrack)), SleepChoice::ALL.len());
        assert_eq!(sleep_notice(Some(SleepChoice::Minutes(15))), "Timer di spegnimento: 15 minuti");
        assert_eq!(sleep_notice(None), "Timer di spegnimento disattivato");
    }

    #[test]
    fn the_profile_form_comes_with_the_sign_in() {
        let account = plan(Tab::Account, true, ProfileForm::Saved);
        assert_eq!(
            account,
            vec![
                Row::Heading("Account"),
                Row::Account,
                Row::OwnProfile,
                Row::Gap,
                Row::Heading("Profilo"),
                Row::Handle,
                Row::Name,
                Row::SaveProfile,
                Row::ShareActivity,
                Row::Gap,
                Row::Heading("Discord"),
                Row::Discord,
            ]
        );
        // Senza nome utente: solo i campi per sceglierlo.
        let new = plan(Tab::Account, false, ProfileForm::New);
        assert!(
            new.contains(&Row::Handle)
                && !new.contains(&Row::OwnProfile)
                && !new.contains(&Row::ShareActivity)
        );
    }

    #[test]
    fn the_account_line_says_who_is_signed_in() {
        assert_eq!(AccountShown::of(&Account::SignedOut), AccountShown::SignedOut);
        let me = psytify_core::api::Me {
            user_id: "u".into(),
            admin: true,
            profile: Some(psytify_core::api::social::MyProfile {
                handle: "marea".into(),
                display_name: "Marea".into(),
                avatar_url: None,
                share_activity: false,
            }),
        };
        assert_eq!(
            AccountShown::of(&Account::SignedIn(me)),
            AccountShown::SignedIn("Marea · @marea · amministratore".into())
        );
    }

    #[test]
    fn the_motion_preference_reads_the_new_key_and_migrates_the_old_one() {
        let mut ui = Map::new();
        assert_eq!(MotionPref::read(&ui), MotionPref::System, "il predefinito segue il sistema");
        ui.insert(LEGACY_REDUCE_MOTION_KEY.into(), Value::Bool(false));
        assert_eq!(MotionPref::read(&ui), MotionPref::System);
        ui.insert(LEGACY_REDUCE_MOTION_KEY.into(), Value::Bool(true));
        assert_eq!(MotionPref::read(&ui), MotionPref::Reduce, "l'interruttore acceso del passo 6.5");
        ui.insert(LEGACY_REDUCE_MOTION_KEY.into(), Value::String("sì".into()));
        assert_eq!(MotionPref::read(&ui), MotionPref::System, "un valore che non è un booleano");
        ui.insert(LEGACY_REDUCE_MOTION_KEY.into(), Value::Bool(true));
        for pref in MotionPref::ALL {
            let saved = pref.prefs();
            assert_eq!(
                saved.get(LEGACY_REDUCE_MOTION_KEY),
                Some(&Value::Null),
                "la vecchia chiave si toglie"
            );
            let mut next = ui.clone();
            next.extend(saved.into_iter().filter(|(_, value)| !value.is_null()));
            assert_eq!(MotionPref::read(&next), pref, "la chiave nuova vince sulla vecchia");
        }
        ui.insert(MOTION_KEY.into(), Value::String("lento".into()));
        assert_eq!(
            MotionPref::read(&ui),
            MotionPref::Reduce,
            "un valore sconosciuto: vale la vecchia chiave"
        );
    }

    #[test]
    fn following_the_system_reduces_only_when_the_system_asks() {
        let full = SystemMotion { reduced_motion: Some(false), power_saver: Some(false) };
        let saver = SystemMotion { power_saver: Some(true), ..full };
        let reduced = SystemMotion { reduced_motion: Some(true), ..full };
        assert!(!motion_reduced(MotionPref::System, full));
        assert!(!motion_reduced(MotionPref::System, SystemMotion::default()), "nessuna risposta: pieno");
        assert!(motion_reduced(MotionPref::System, saver));
        assert!(motion_reduced(MotionPref::System, reduced));
        assert!(motion_reduced(MotionPref::Reduce, full));
        assert!(!motion_reduced(MotionPref::Full, saver), "«Completo» non guarda il sistema");
        assert_eq!(motion_detail(MotionPref::System, full), "Ora il sistema chiede movimento pieno.");
        assert_eq!(
            motion_detail(MotionPref::System, saver),
            "Ora il sistema chiede movimento ridotto (risparmio energetico)."
        );
        assert!(
            !motion_detail(MotionPref::Full, saver).starts_with("Ora il sistema"),
            "solo seguendo il sistema"
        );
    }

    #[test]
    fn the_server_line_names_the_host_in_use() {
        assert_eq!(
            server_detail("10.0.0.2:8080", true),
            "In uso: 10.0.0.2:8080, scelto da te. Vuoto torna al predefinito."
        );
        assert!(server_detail("music.example.org", false).contains("il predefinito"));
    }
}
