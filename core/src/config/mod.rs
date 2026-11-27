//! Configurazione dell'utente: indirizzo del backend, volume, qualità d'ascolto, login Clerk,
//! ultima coda e playlist locali. Vive in `<config_dir>/psytify/config.json` (crate `dirs`). Non
//! contiene segreti: il refresh token del login sta in `session.json`.
//!
//! Il file ha due parti nello stesso oggetto JSON: i campi del core ([`CoreConfig`]) e tutte le
//! altre chiavi ([`Config::ui`]), cioè le preferenze dei frontend (l'ultima schermata, il pannello
//! aperto) e ciò che ha scritto una versione più nuova. Il core non le interpreta: le rilegge e le
//! riscrive tali e quali, così un downgrade non cancella nulla.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::api::{DownloadQuality, Quality, SourceItem, Track};
use crate::auth::ClerkApp;
use crate::error::AppError;
use crate::offline::SavedTrack;
use crate::player::Queue;
use crate::player::eq;
use crate::shared::Shared;

pub mod taste;
pub use taste::{FollowedArtist, PlayStat, SavedAlbum};

/// Il backend pubblico: quello che le build di distribuzione (desktop, APK, pacchetto AUR) usano di
/// default. Non è un segreto: è l'indirizzo che l'app contatta comunque.
pub const DISTRIBUTION_API_URL: &str = "http://127.0.0.1:8080";
/// Il backend di una configurazione nuova. Le build di debug usano un server sul proprio PC, le
/// altre il backend pubblico. `PSYTIFY_DEFAULT_API_URL` alla compilazione lo sostituisce.
pub const DEFAULT_API_URL: &str = match option_env!("PSYTIFY_DEFAULT_API_URL") {
    // Una variabile definita ma vuota vale come assente.
    Some(url) if !url.is_empty() => url,
    _ if cfg!(debug_assertions) => "http://127.0.0.1:8080",
    _ => DISTRIBUTION_API_URL,
};
/// L'app Discord della build (Rich Presence): l'Application ID non è un segreto. Vuoto
/// (`PSYTIFY_DEFAULT_DISCORD_APP_ID=""`), niente Discord finché `config.json` non ne indica uno.
pub const DEFAULT_DISCORD_APP_ID: &str = match option_env!("PSYTIFY_DEFAULT_DISCORD_APP_ID") {
    Some(id) => id,
    None => "1565369762429804665",
};
/// Il login con Clerk della build: la Frontend API dell'istanza. Non è un segreto. Vale quando
/// `config.json` non ne sceglie un'altra; vuota (`PSYTIFY_DEFAULT_CLERK_ISSUER=""`), il login resta
/// spento.
const DEFAULT_CLERK_ISSUER: &str = match option_env!("PSYTIFY_DEFAULT_CLERK_ISSUER") {
    Some(issuer) => issuer,
    None => "https://needed-grouse-4672.clerk.accounts.dev",
};
/// Indirizzi predefiniti delle build di sviluppo (desktop ed emulatore Android). Le versioni
/// precedenti li salvavano nel file anche senza una scelta dell'utente: letti lì, non valgono come
/// scelta.
const BUILTIN_API_URLS: [&str; 2] = ["http://127.0.0.1:8080", "http://10.0.2.2:8080"];
/// Variabile d'ambiente che sostituisce il backend senza toccare il file.
pub const API_URL_ENV: &str = "PSYTIFY_API_URL";
/// Variabili d'ambiente che sostituiscono `clerk_issuer` e `clerk_client_id`.
pub const CLERK_ISSUER_ENV: &str = "PSYTIFY_CLERK_ISSUER";
pub const CLERK_CLIENT_ID_ENV: &str = "PSYTIFY_CLERK_CLIENT_ID";
/// Porta di loopback per il ritorno dal browser (GitHub, Google, Discord).
pub const DEFAULT_OAUTH_PORT: u16 = 53682;
pub const MAX_PLAYLIST_NAME: usize = 80;
/// Brani ascoltati di recente che l'app ricorda.
pub const MAX_RECENT: usize = 30;
/// Playlist locali al massimo: oltre, la barra laterale non serve più a niente.
pub const MAX_PLAYLISTS: usize = 200;

/// Playlist locale: il backend non ha playlist, quindi l'app salva una copia dei brani.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: Uuid,
    pub name: String,
    pub tracks: Vec<Track>,
    /// Brani di una playlist esterna salvata, ancora da importare: entrano in `tracks` uno alla
    /// volta, anche dopo un riavvio.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<SourceItem>,
    /// Visibile sul proprio profilo (a chi è collegato), con i brani del catalogo.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub public: bool,
}

impl Playlist {
    pub fn new(name: &str, tracks: Vec<Track>) -> Self {
        Self { id: Uuid::new_v4(), name: clean_name(name), tracks, pending: Vec::new(), public: false }
    }

    pub fn contains(&self, track_id: Uuid) -> bool {
        self.tracks.iter().any(|t| t.id == track_id)
    }

    /// Aggiunge il brano in fondo. `false` se c'era già: una playlist non ripete un brano.
    pub fn add(&mut self, track: Track) -> bool {
        if self.contains(track.id) {
            return false;
        }
        self.tracks.push(track);
        true
    }

    /// Toglie il brano alla posizione indicata. `false` se la posizione non esiste.
    pub fn remove_at(&mut self, index: usize) -> bool {
        if index >= self.tracks.len() {
            return false;
        }
        self.tracks.remove(index);
        true
    }
}

/// Mette o toglie "mi piace". Il brano appena piaciuto va in testa. Restituisce `true` se ora piace.
pub fn toggle_like(liked: &mut Vec<Track>, track: Track) -> bool {
    match liked.iter().position(|t| t.id == track.id) {
        Some(at) => {
            liked.remove(at);
            false
        }
        None => {
            liked.insert(0, track);
            true
        }
    }
}

/// Registra un ascolto: il brano va in testa e non compare due volte.
pub fn remember_listen(recent: &mut Vec<Track>, track: &Track) {
    recent.retain(|t| t.id != track.id);
    recent.insert(0, track.clone());
    recent.truncate(MAX_RECENT);
}

/// Nome di playlist pulito: niente caratteri di controllo, spazi compattati, lunghezza limitata.
pub fn clean_name(raw: &str) -> String {
    let collapsed = raw
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let name: String = collapsed.chars().take(MAX_PLAYLIST_NAME).collect();
    if name.is_empty() { "Nuova playlist".into() } else { name }
}

/// Tutto `config.json`: i campi del core e, accanto, le chiavi che il core non conosce.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Config {
    #[serde(flatten)]
    pub core: CoreConfig,
    /// Ogni altra chiave del file: le preferenze dei frontend (`SettingsCmd::SaveUiPrefs`) e quelle
    /// di versioni più nuove. Si riscrivono tali e quali; nessuna ha il nome di un campo del core
    /// ([`merge_ui`] le scarta).
    #[serde(flatten)]
    pub ui: Map<String, Value>,
}

/// I campi di `config.json` che il core conosce. Le preferenze della sola interfaccia (l'ultima
/// schermata, il pannello aperto, il videoclip nel pannello, la scheda video dedicata) non sono
/// qui: le tiene ogni frontend in [`Config::ui`], con i nomi di sempre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreConfig {
    /// Backend scelto dall'utente nelle impostazioni; `None` = quello predefinito della build.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    /// Chiave delle versioni precedenti, che scrivevano sempre il backend in uso: un indirizzo
    /// vecchio sopravviveva così a ogni aggiornamento. Si legge soltanto, per la migrazione: chi
    /// aggiorna da una di quelle versioni la ha ancora nel file (`docs/transformation/BACKLOG.md`, 2).
    #[serde(rename = "api_base_url", skip_serializing)]
    pub legacy_api: Option<String>,
    pub volume: f32,
    pub queue: Arc<Queue>,
    /// Where the current queue entry was at the last save, in milliseconds: a restart resumes it
    /// from there.
    pub position_ms: u64,
    pub playlists: Shared<Playlist>,
    /// Brani a cui si è messo "mi piace", il più recente in testa. Il backend non ha utenti per
    /// questo: come le playlist, stanno solo su questo computer.
    pub liked: Shared<Track>,
    /// Ultimi brani ascoltati, il più recente in testa.
    pub recent: Shared<Track>,
    /// Ultima ricerca eseguita, ripetuta se l'ultima schermata era Cerca.
    pub last_query: String,
    pub quality: Quality,
    /// Qualità dei download (`Message::Download`); dal menu di un brano si può scegliere un'altra.
    pub download_quality: DownloadQuality,
    /// I brani scaricati sul dispositivo (l'indice di `offline.rs`), il più recente in testa: dove
    /// stanno i file e che file sono, per suonarli senza rete.
    pub downloaded: Shared<SavedTrack>,
    /// Frontend API dell'istanza Clerk (`https://...`); vuoto = login disattivato.
    pub clerk_issuer: String,
    /// Dell'applicazione OAuth del login nel browser delle versioni precedenti: il login dentro
    /// l'app non lo usa più, resta per non perdere il valore.
    pub clerk_client_id: String,
    pub oauth_port: u16,
    /// Mostra su Discord cosa si ascolta (Rich Presence), se Discord è aperto sullo stesso PC.
    pub discord_presence: bool,
    /// Application ID dell'app Discord «Psytify»; vuoto = quello della build.
    pub discord_app_id: String,
    /// Equalizzatore a 10 bande, acceso o spento con le sue regolazioni.
    pub equalizer: eq::Settings,
    /// Artisti seguiti: le loro uscite compaiono in «Novità».
    pub followed: Shared<FollowedArtist>,
    /// Album salvati nella libreria.
    pub saved_albums: Shared<SavedAlbum>,
    /// Ascolti per brano: i più ascoltati, i tuoi artisti e i mix si ricavano da qui.
    pub stats: Shared<PlayStat>,
    /// Tempo ascoltato in tutto, in millisecondi.
    pub listened_ms: u64,
    /// L'uscita più recente che si è vista in «Novità» (`created_at` del brano): ciò che viene dopo
    /// è nuovo.
    pub news_seen: String,
    /// A fine coda continua da solo con brani dello stesso artista.
    pub autoplay: bool,
    /// Ultime ricerche, la più recente in testa (`remember_search`).
    pub search_history: Shared<String>,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            server: None,
            legacy_api: None,
            volume: 0.8,
            queue: Arc::default(),
            position_ms: 0,
            playlists: Shared::default(),
            liked: Shared::default(),
            recent: Shared::default(),
            last_query: String::new(),
            quality: Quality::default(),
            download_quality: DownloadQuality::default(),
            downloaded: Shared::default(),
            clerk_issuer: String::new(),
            clerk_client_id: String::new(),
            oauth_port: DEFAULT_OAUTH_PORT,
            discord_presence: true,
            discord_app_id: String::new(),
            equalizer: eq::Settings::default(),
            followed: Shared::default(),
            saved_albums: Shared::default(),
            stats: Shared::default(),
            listened_ms: 0,
            news_seen: String::new(),
            autoplay: true,
            search_history: Shared::default(),
        }
    }
}

/// I campi di [`CoreConfig`] che possiede il dominio della libreria (`state/library.rs`): sono
/// `Arc` condivisi, quindi passarli da un dominio all'altro non copia le liste.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LibraryFields {
    pub(crate) playlists: Shared<Playlist>,
    pub(crate) liked: Shared<Track>,
    pub(crate) recent: Shared<Track>,
    pub(crate) followed: Shared<FollowedArtist>,
    pub(crate) saved_albums: Shared<SavedAlbum>,
    pub(crate) stats: Shared<PlayStat>,
    pub(crate) listened_ms: u64,
    pub(crate) news_seen: String,
    pub(crate) search_history: Shared<String>,
}

/// I campi di [`CoreConfig`] che possiede il dominio della riproduzione (`state/playback.rs`). La
/// coda è un `Arc`: passarla da un dominio all'altro non la copia.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlaybackFields {
    pub(crate) volume: f32,
    pub(crate) queue: Arc<Queue>,
    pub(crate) position_ms: u64,
    pub(crate) quality: Quality,
    pub(crate) equalizer: eq::Settings,
    pub(crate) autoplay: bool,
}

/// I campi di [`CoreConfig`] che possiede il dominio dei download (`state/downloads.rs`).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DownloadsFields {
    pub(crate) download_quality: DownloadQuality,
    pub(crate) downloaded: Shared<SavedTrack>,
}

impl CoreConfig {
    /// I campi dei download, per il suo dominio.
    pub(crate) fn downloads(&self) -> DownloadsFields {
        DownloadsFields { download_quality: self.download_quality, downloaded: self.downloaded.clone() }
    }

    /// Mette al loro posto i campi dei download.
    pub(crate) fn set_downloads(&mut self, fields: DownloadsFields) {
        self.download_quality = fields.download_quality;
        self.downloaded = fields.downloaded;
    }

    /// I campi della riproduzione, per il suo dominio.
    pub(crate) fn playback(&self) -> PlaybackFields {
        PlaybackFields {
            volume: self.volume,
            queue: Arc::clone(&self.queue),
            position_ms: self.position_ms,
            quality: self.quality,
            equalizer: self.equalizer,
            autoplay: self.autoplay,
        }
    }

    /// Mette al loro posto i campi della riproduzione.
    pub(crate) fn set_playback(&mut self, fields: PlaybackFields) {
        self.volume = fields.volume;
        self.queue = fields.queue;
        self.position_ms = fields.position_ms;
        self.quality = fields.quality;
        self.equalizer = fields.equalizer;
        self.autoplay = fields.autoplay;
    }

    /// I campi della libreria, per il suo dominio.
    pub(crate) fn library(&self) -> LibraryFields {
        LibraryFields {
            playlists: self.playlists.clone(),
            liked: self.liked.clone(),
            recent: self.recent.clone(),
            followed: self.followed.clone(),
            saved_albums: self.saved_albums.clone(),
            stats: self.stats.clone(),
            listened_ms: self.listened_ms,
            news_seen: self.news_seen.clone(),
            search_history: self.search_history.clone(),
        }
    }

    /// Mette al loro posto i campi della libreria.
    pub(crate) fn set_library(&mut self, fields: LibraryFields) {
        self.playlists = fields.playlists;
        self.liked = fields.liked;
        self.recent = fields.recent;
        self.followed = fields.followed;
        self.saved_albums = fields.saved_albums;
        self.stats = fields.stats;
        self.listened_ms = fields.listened_ms;
        self.news_seen = fields.news_seen;
        self.search_history = fields.search_history;
    }
}

/// Ricerche ricordate al massimo.
pub const SEARCH_HISTORY: usize = 8;

/// Mette `query` in testa alle ricerche recenti. Mentre si scrive la ricerca parte più volte
/// («emi», «emin», «eminem»): una ricerca che ne allunga o ne accorcia un'altra la sostituisce.
/// Una ricerca da tenere: almeno una lettera o una cifra («,» o «...» no).
pub fn worth_remembering(query: &str) -> bool {
    query.chars().any(char::is_alphanumeric)
}

pub fn remember_search(history: &mut Vec<String>, query: &str) {
    let query = query.trim();
    if !worth_remembering(query) {
        return;
    }
    let lower = query.to_lowercase();
    history.retain(|old| {
        let old = old.to_lowercase();
        !(old.starts_with(&lower) || lower.starts_with(&old))
    });
    history.insert(0, query.to_owned());
    history.truncate(SEARCH_HISTORY);
}

impl CoreConfig {
    /// L'istanza Clerk del login, se configurata con un issuer https.
    pub fn clerk_app(&self) -> Option<ClerkApp> {
        clerk_app_from(or_default(&self.clerk_issuer, DEFAULT_CLERK_ISSUER), self.oauth_port)
    }

    /// Ripara valori fuori intervallo letti da disco.
    fn sanitize(mut self) -> Self {
        if !self.volume.is_finite() {
            self.volume = Self::default().volume;
        }
        self.volume = self.volume.clamp(0.0, 1.0);
        Arc::make_mut(&mut self.queue).sanitize();
        // Il backend scritto dalle versioni precedenti resta solo se era una scelta: sul telefono
        // non c'era modo di cambiarlo, quindi era sempre il predefinito di una build passata.
        if let Some(legacy) = self.legacy_api.take()
            && self.server.is_none()
            && !cfg!(target_os = "android")
            && !BUILTIN_API_URLS.contains(&legacy.trim().trim_end_matches('/'))
        {
            self.server = normalize_server(&legacy);
        }
        // Le porte sotto 1024 richiedono privilegi: si torna al default.
        if self.oauth_port < 1024 {
            self.oauth_port = DEFAULT_OAUTH_PORT;
        }
        let playlists = self.playlists.make_mut();
        for playlist in playlists.iter_mut() {
            playlist.name = clean_name(&playlist.name);
        }
        playlists.truncate(MAX_PLAYLISTS);
        self.recent.make_mut().truncate(MAX_RECENT);
        self.followed.make_mut().truncate(taste::MAX_FOLLOWED);
        self.saved_albums.make_mut().truncate(taste::MAX_SAVED_ALBUMS);
        self.stats.make_mut().truncate(taste::MAX_STATS);
        self
    }
}

/// Applica a `ui` le preferenze `patch` di un frontend: ogni chiave sostituisce quella salvata,
/// `null` la toglie, le chiavi che `patch` non nomina restano (anche quelle di un altro frontend o
/// di una versione più nuova). Una chiave con il nome di un campo del core si scarta: scritta due
/// volte, il file non si rileggerebbe più. Restituisce la mappa nuova solo se qualcosa cambia, così
/// chi la tiene in un `Arc` la copia solo allora.
pub fn merge_ui(ui: &Map<String, Value>, patch: Map<String, Value>) -> Option<Map<String, Value>> {
    let mut merged: Option<Map<String, Value>> = None;
    for (key, value) in patch {
        let current = merged.as_ref().unwrap_or(ui).get(&key);
        let changes = if value.is_null() { current.is_some() } else { current != Some(&value) };
        // Una chiave già nella mappa non è del core: o veniva dal file, dove serde ha già preso i
        // campi suoi, o è passata da qui.
        if !changes || (current.is_none() && is_core_key(&key)) {
            continue;
        }
        let map = merged.get_or_insert_with(|| ui.clone());
        if value.is_null() {
            map.remove(&key);
        } else {
            map.insert(key, value);
        }
    }
    merged
}

/// `key` è il nome di un campo di [`CoreConfig`] (o di una chiave che il core legge soltanto, come
/// `api_base_url`). Lo dice serde stesso: letto un file con quella sola chiave, il core la prende
/// per sé (o rifiuta il valore) invece di lasciarla fra le altre. Così l'elenco non va tenuto a mano.
pub(crate) fn is_core_key(key: &str) -> bool {
    let mut probe = Map::new();
    probe.insert(key.to_owned(), Value::Null);
    match serde_json::from_value::<Config>(Value::Object(probe)) {
        Ok(config) => !config.ui.contains_key(key),
        Err(_) => true,
    }
}

/// Un campo della configurazione, o il predefinito della build se è vuoto.
fn or_default<'a>(value: &'a str, default: &'a str) -> &'a str {
    if value.trim().is_empty() { default } else { value }
}

/// L'istanza Clerk, se l'issuer è un indirizzo `https://` valido.
fn clerk_app_from(issuer: &str, port: u16) -> Option<ClerkApp> {
    let issuer = issuer.trim().trim_end_matches('/');
    let valid = issuer.starts_with("https://")
        && issuer.len() > "https://".len()
        && !issuer.contains(['?', '#', ' ']);
    valid.then(|| ClerkApp { issuer: issuer.to_owned(), port, session_file: None })
}

/// Backend predefinito scelto da chi avvia l'app (Android: fissato alla compilazione, perché sul
/// telefono il backend non è su `127.0.0.1`). Vale finché l'utente non ne sceglie un altro.
static DEFAULT_API: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn set_default_api_url(url: String) {
    let _ = DEFAULT_API.set(url);
}

/// Il backend predefinito di questa build.
pub fn default_api_url() -> &'static str {
    DEFAULT_API.get().map_or(DEFAULT_API_URL, String::as_str)
}

/// Il backend da usare: la variabile d'ambiente, poi la scelta dell'utente, poi il predefinito.
pub fn api_url(server: Option<&str>) -> String {
    std::env::var(API_URL_ENV)
        .ok()
        .filter(|url| !url.trim().is_empty())
        .or_else(|| server.map(str::to_owned))
        .unwrap_or_else(|| default_api_url().to_owned())
}

/// Ripulisce l'indirizzo scritto dall'utente; vuoto o uguale al predefinito vuol dire nessuna scelta.
pub fn normalize_server(input: &str) -> Option<String> {
    let url = input.trim().trim_end_matches('/');
    (!url.is_empty() && url != default_api_url()).then(|| url.to_owned())
}

/// Cartella dei dati scelta da chi avvia l'app (Android: la cartella privata dell'app), al posto di
/// quelle del sistema che `dirs` conosce solo su desktop. Dal passo 4.7 la leggono solo i
/// frontend, per riempire `CoreOptions::data_dir` e `downloads_dir`: il core non la guarda più,
/// e ogni core ha la sua cartella (`config.json`, `session.json`, download e aggiornamenti).
static DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Usa `dir` per configurazione, sessione e download. Va chiamata prima di `run`; la seconda volta
/// non cambia niente.
pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

/// La cartella di configurazione di Psytify.
pub fn dir() -> Option<PathBuf> {
    match DATA_DIR.get() {
        Some(dir) => Some(dir.clone()),
        None => dirs::config_dir().map(|dir| dir.join("psytify")),
    }
}

/// Dove finiscono i brani scaricati: la cartella Download dell'utente, o quella dei dati dell'app.
pub fn downloads_dir() -> PathBuf {
    match DATA_DIR.get() {
        Some(dir) => dir.join("Download"),
        None => {
            dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(std::env::temp_dir).join("Psytify")
        }
    }
}

/// `config.json` nella cartella dei dati `dir`.
pub fn file_in(dir: &Path) -> PathBuf {
    dir.join("config.json")
}

/// `config.json` nella cartella di [`dir`].
pub fn path() -> Option<PathBuf> {
    dir().map(|dir| file_in(&dir))
}

/// Legge `config.json` da `path`; senza percorso, i default. Un file assente dà i default; un file
/// illeggibile viene messo da parte come `config.json.broken` e si riparte dai default, senza
/// perdere i dati dell'utente. Lettura sincrona: avviene una volta sola, all'avvio.
pub fn load(path: Option<&Path>) -> Config {
    let mut config = path.map_or_else(Config::default, load_from);
    for (var, field) in [
        (CLERK_ISSUER_ENV, &mut config.core.clerk_issuer),
        (CLERK_CLIENT_ID_ENV, &mut config.core.clerk_client_id),
    ] {
        if let Ok(value) = std::env::var(var)
            && !value.trim().is_empty()
        {
            *field = value;
        }
    }
    config
}

fn load_from(path: &Path) -> Config {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Config::default();
    };
    match serde_json::from_str::<Config>(&text) {
        Ok(config) => Config { core: config.core.sanitize(), ui: config.ui },
        Err(_) => {
            let _ = std::fs::rename(path, path.with_extension("json.broken"));
            Config::default()
        }
    }
}

/// Scrive `config.json` in modo atomico: un file temporaneo accanto, poi la rinomina. Una
/// scrittura interrotta lascia intatto il file di prima. Solo il core la chiama
/// (`state::settings`), una scrittura alla volta: è l'unico a scrivere il file.
pub(crate) async fn write(path: &Path, core: &CoreConfig, ui: &Map<String, Value>) -> Result<(), AppError> {
    let io_error = |_| AppError::Io(path.display().to_string());
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io_error)?;
    }
    let json = serde_json::to_vec_pretty(&Document { core, ui })
        .map_err(|_| AppError::Io(path.display().to_string()))?;
    let temp = path.with_extension("json.tmp");
    tokio::fs::write(&temp, json).await.map_err(io_error)?;
    tokio::fs::rename(&temp, path).await.map_err(io_error)
}

/// `config.json` da scrivere, in prestito: i campi del core e, nello stesso oggetto, le altre chiavi.
#[derive(Serialize)]
struct Document<'a> {
    #[serde(flatten)]
    core: &'a CoreConfig,
    #[serde(flatten)]
    ui: &'a Map<String, Value>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn typing_a_search_keeps_only_the_final_query() {
        let mut history = vec!["Daft Punk".to_string()];
        for typed in ["emi", "emin", "Eminem", "Eminem lose"] {
            super::remember_search(&mut history, typed);
        }
        assert_eq!(history, ["Eminem lose", "Daft Punk"]);
        super::remember_search(&mut history, "daft punk");
        assert_eq!(history, ["daft punk", "Eminem lose"], "the same search moves to the top");
        super::remember_search(&mut history, ",");
        assert_eq!(history, ["daft punk", "Eminem lose"], "punctuation alone is not a search");
        for n in 0..20 {
            super::remember_search(&mut history, &format!("q{n:02}x"));
        }
        assert_eq!(history.len(), super::SEARCH_HISTORY);
    }

    use super::*;
    use crate::api::models::fixtures::track;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("psytify-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Le chiavi di un oggetto JSON scritto a mano.
    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            other => panic!("not an object: {other}"),
        }
    }

    #[tokio::test]
    async fn config_round_trips_through_disk() {
        let path = temp_dir().join("config.json");
        let mut core = CoreConfig { volume: 0.35, ..CoreConfig::default() };
        Arc::make_mut(&mut core.queue).replace(vec![track(1), track(2)], 1, 3);
        core.playlists.make_mut().push(Playlist::new("Sera", vec![track(3)]));
        let ui = object(serde_json::json!({"last_screen": {"kind": "library"}}));

        write(&path, &core, &ui).await.unwrap();

        assert_eq!(load_from(&path), Config { core, ui });
    }

    /// Una configurazione con ogni campo diverso dal default, per il test di `config.json`.
    fn full_config() -> Config {
        let mut core = CoreConfig {
            server: Some("https://backend.example.test".into()),
            legacy_api: None,
            volume: 0.5,
            position_ms: 42_000,
            last_query: "daft punk".into(),
            quality: Quality::Standard,
            download_quality: DownloadQuality::High,
            downloaded: vec![SavedTrack {
                track: Track { downloadable: true, ..track(6) },
                path: PathBuf::from("/home/utente/Download/Artista 0 - Brano 6.flac"),
                format: "flac".into(),
                quality: Some(DownloadQuality::Lossless),
                size: 31_457_280,
                sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
            }]
            .into(),
            clerk_issuer: "https://clerk.example.test".into(),
            clerk_client_id: "client-id-fittizio".into(),
            oauth_port: 54000,
            discord_presence: false,
            discord_app_id: "123456789".into(),
            equalizer: eq::Settings {
                enabled: true,
                gains: [1.0, 2.0, 3.0, 4.0, 5.0, -1.0, -2.0, -3.0, -4.0, -5.0],
            },
            followed: vec![FollowedArtist {
                id: Uuid::from_u128(10),
                name: "Artista Uno".into(),
                cover_url: Some("https://cdn.example.test/a.jpg".into()),
            }]
            .into(),
            saved_albums: vec![SavedAlbum {
                id: Uuid::from_u128(11),
                title: "Album Uno".into(),
                artist: track(1).artist,
                cover_url: None,
            }]
            .into(),
            stats: vec![PlayStat { track: track(2), plays: 3, last_played: 1_700_000_000 }].into(),
            listened_ms: 123_456,
            news_seen: "2026-10-01T09:30:00Z".into(),
            autoplay: false,
            search_history: vec!["daft punk".to_owned(), "eminem".to_owned()].into(),
            ..CoreConfig::default()
        };
        let queue = Arc::make_mut(&mut core.queue);
        queue.replace(vec![track(1), track(2), track(3)], 1, 5);
        queue.set_shuffle(true, 9);
        queue.cycle_repeat();
        let mut playlist = Playlist::new("Sera", vec![track(3)]);
        playlist.id = Uuid::from_u128(12);
        playlist.public = true;
        playlist.pending.push(SourceItem {
            source: crate::api::SourceKind::YouTube,
            id: "id-fittizio".into(),
            title: "Da importare".into(),
            artist: "Qualcuno".into(),
            duration_ms: Some(1000),
            cover_url: None,
        });
        core.playlists.make_mut().push(playlist);
        core.liked.make_mut().push(track(4));
        core.recent.make_mut().push(track(5));
        // Le preferenze dell'interfaccia Iced e una chiave di una versione futura: per il core
        // sono chiavi qualsiasi.
        let ui = object(serde_json::json!({
            "last_screen": {"kind": "album", "id": "00000000-0000-0000-0000-000000000007"},
            "panel_open": true,
            "panel_video": false,
            "dedicated_video": true,
            "unknown_future_key": {"a": 1},
        }));
        Config { core, ui }
    }

    /// Il file golden di `config.json`, nel formato delle release: ogni campo valorizzato, senza
    /// token e con id fittizi. Il confronto è sui valori JSON (non sui byte): l'ordine dei campi e
    /// gli spazi non sono un contratto. Il file porta anche `api_base_url`, letta per la migrazione
    /// ma mai riscritta, e `unknown_future_key`, che resta com'è.
    #[tokio::test]
    async fn golden_config_json_round_trips() {
        let text = include_str!("../../tests/fixtures/config.golden.json");
        let dir = temp_dir();
        let path = dir.join("config.json");
        std::fs::write(&path, text).unwrap();

        let loaded = load_from(&path);
        assert_eq!(loaded, full_config(), "the fixture sets every field");

        write(&path, &loaded.core, &loaded.ui).await.unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let mut expected: Value = serde_json::from_str(text).unwrap();
        expected.as_object_mut().unwrap().remove("api_base_url");
        assert_eq!(saved, expected);
    }

    #[tokio::test]
    async fn unknown_keys_survive_a_save() {
        let path = temp_dir().join("config.json");
        std::fs::write(&path, r#"{"volume": 0.4, "unknown_future_key": {"a": 1}}"#).unwrap();

        let loaded = load_from(&path);
        write(&path, &loaded.core, &loaded.ui).await.unwrap();

        // Un downgrade non cancella i campi scritti da una versione più nuova
        // (`docs/transformation/BACKLOG.md`, 1).
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["unknown_future_key"], serde_json::json!({"a": 1}));
        assert_eq!(saved["volume"], 0.4_f32);
    }

    #[test]
    fn a_ui_value_from_a_newer_version_does_not_break_the_core_fields() {
        // Una schermata che questa versione non conosce: prima rendeva illeggibile tutto il file.
        let json = r#"{"volume": 0.3, "last_screen": {"kind": "podcasts"}}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.core.volume, 0.3);
        assert_eq!(config.ui["last_screen"], serde_json::json!({"kind": "podcasts"}));
    }

    #[test]
    fn ui_prefs_replace_their_keys_null_removes_and_the_rest_stays() {
        let ui = object(serde_json::json!({"panel_open": false, "gpui_theme": "dark", "queue_open": true}));
        let patch = object(serde_json::json!({"panel_open": true, "queue_open": null, "last_screen": "x"}));
        let merged = merge_ui(&ui, patch).expect("something changed");
        assert_eq!(
            Value::Object(merged.clone()),
            serde_json::json!({"panel_open": true, "gpui_theme": "dark", "last_screen": "x"}),
            "the other frontend's key survives"
        );
        let same = object(serde_json::json!({"panel_open": true, "queue_open": null}));
        assert_eq!(merge_ui(&merged, same), None, "nothing changes, no copy");
    }

    #[test]
    fn a_ui_pref_never_takes_the_name_of_a_core_field() {
        let patch = object(serde_json::json!({"volume": 3, "server": null, "api_base_url": "x", "own": 1}));
        let merged = merge_ui(&Map::new(), patch).expect("the frontend's own key goes in");
        assert_eq!(Value::Object(merged), serde_json::json!({"own": 1}));
        for key in ["volume", "server", "api_base_url", "queue", "search_history"] {
            assert!(is_core_key(key), "{key}");
        }
        for key in ["last_screen", "panel_open", "queue_open", "unknown_future_key"] {
            assert!(!is_core_key(key), "{key}");
        }
    }

    #[test]
    fn a_playlist_never_repeats_a_track() {
        let mut playlist = Playlist::new("Sera", Vec::new());
        assert!(playlist.add(track(1)));
        assert!(!playlist.add(track(1)));
        assert_eq!(playlist.tracks.len(), 1);
    }

    #[test]
    fn removing_from_a_playlist_checks_the_position() {
        let mut playlist = Playlist::new("Sera", vec![track(1), track(2)]);
        assert!(!playlist.remove_at(5));
        assert!(playlist.remove_at(0));
        assert_eq!(playlist.tracks[0].id, track(2).id);
    }

    #[test]
    fn liking_twice_unlikes_and_the_newest_like_comes_first() {
        let mut liked = Vec::new();
        assert!(toggle_like(&mut liked, track(1)));
        assert!(toggle_like(&mut liked, track(2)));
        assert_eq!(liked[0].id, track(2).id);
        assert!(!toggle_like(&mut liked, track(2)));
        assert_eq!(liked.len(), 1);
    }

    #[test]
    fn a_listen_moves_the_track_to_the_front_without_repeats_and_is_capped() {
        let mut recent = Vec::new();
        for n in 1..=(MAX_RECENT as u128 + 5) {
            remember_listen(&mut recent, &track(n));
        }
        remember_listen(&mut recent, &track(40));
        assert_eq!((recent.len(), recent[0].id), (MAX_RECENT, track(40).id));
        assert_eq!(recent.iter().filter(|t| t.id == track(40).id).count(), 1);
    }

    #[test]
    fn missing_file_gives_defaults() {
        assert_eq!(load_from(&temp_dir().join("absent.json")), Config::default());
    }

    #[test]
    fn corrupted_file_is_kept_aside() {
        let dir = temp_dir();
        let path = dir.join("config.json");
        std::fs::write(&path, "{not json").unwrap();

        assert_eq!(load_from(&path), Config::default());
        assert!(dir.join("config.json.broken").exists());
    }

    #[test]
    fn out_of_range_volume_is_clamped() {
        let config = CoreConfig { volume: 4.0, ..CoreConfig::default() }.sanitize();
        assert_eq!(config.volume, 1.0);
    }

    #[test]
    fn unknown_fields_and_missing_fields_are_tolerated() {
        let config: Config = serde_json::from_str(r#"{"volume": 0.5, "future": true}"#).unwrap();
        assert_eq!((config.core.volume, config.core.server), (0.5, None));
        assert_eq!(Value::Object(config.ui), serde_json::json!({"future": true}));
    }

    #[test]
    fn a_legacy_default_address_is_not_a_choice() {
        for url in BUILTIN_API_URLS {
            let json = format!(r#"{{"api_base_url": "{url}/"}}"#);
            let config = serde_json::from_str::<CoreConfig>(&json).unwrap().sanitize();
            assert_eq!(config.server, None, "{url}");
        }
    }

    #[tokio::test]
    async fn a_legacy_custom_address_becomes_the_user_choice() {
        let path = temp_dir().join("config.json");
        std::fs::write(&path, r#"{"api_base_url": "https://music.example.org/"}"#).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.core.server.as_deref(), Some("https://music.example.org"));
        assert!(loaded.ui.is_empty(), "the old key is the core's, not a ui key");
        // Riscritto, il file non ha più la chiave vecchia: la scelta sta in `server`.
        write(&path, &loaded.core, &loaded.ui).await.unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved.get("api_base_url"), None);
        assert_eq!(saved["server"], "https://music.example.org");
    }

    #[test]
    fn no_choice_means_the_build_default() {
        let saved = serde_json::to_value(CoreConfig::default()).unwrap();
        assert_eq!(saved.get("server"), None);
        assert_eq!(normalize_server("  "), None);
        assert_eq!(normalize_server(&format!("{}/", default_api_url())), None);
        assert_eq!(normalize_server(" http://10.0.0.5:8080/ ").as_deref(), Some("http://10.0.0.5:8080"));
    }

    #[test]
    fn clerk_login_is_off_without_issuer() {
        // Un issuer vuoto spegne il login; `CoreConfig::default()` invece prende quello della build.
        assert_eq!(clerk_app_from("", DEFAULT_OAUTH_PORT), None);
    }

    #[test]
    fn clerk_app_requires_https_issuer() {
        let config = CoreConfig {
            clerk_issuer: "http://x.clerk.accounts.dev".into(),
            clerk_client_id: "abc".into(),
            ..CoreConfig::default()
        };
        assert_eq!(config.clerk_app(), None);
    }

    #[test]
    fn clerk_app_trims_the_issuer() {
        let config = CoreConfig {
            clerk_issuer: " https://x.clerk.accounts.dev/ ".into(),
            clerk_client_id: "abc_1".into(),
            ..CoreConfig::default()
        };
        assert_eq!(config.clerk_app().map(|a| a.issuer), Some("https://x.clerk.accounts.dev".into()));
    }

    #[test]
    fn an_empty_clerk_field_takes_the_build_default() {
        assert_eq!(or_default("  ", "https://b.clerk.accounts.dev"), "https://b.clerk.accounts.dev");
        assert_eq!(or_default("https://a.clerk.accounts.dev", "https://b"), "https://a.clerk.accounts.dev");
    }

    #[test]
    fn privileged_oauth_port_falls_back_to_the_default() {
        let config = CoreConfig { oauth_port: 80, ..CoreConfig::default() };
        assert_eq!(config.sanitize().oauth_port, DEFAULT_OAUTH_PORT);
    }

    #[test]
    fn playlist_name_is_cleaned() {
        assert_eq!(clean_name("  Notte \n\t lunga  "), "Notte lunga");
    }

    #[test]
    fn empty_playlist_name_gets_a_default() {
        assert_eq!(clean_name("   "), "Nuova playlist");
    }
}
