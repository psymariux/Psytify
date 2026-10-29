//! Configurazione dell'utente: indirizzo del backend, volume, qualità d'ascolto, login Clerk,
//! ultima coda, ultima schermata e playlist locali. Vive in `<config_dir>/psytify/config.json`
//! (crate `dirs`). Non contiene segreti: il refresh token del login sta in `session.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::{DownloadQuality, Quality, SourceItem, Track};
use crate::auth::ClerkApp;
use crate::error::AppError;
use crate::player::Queue;
use crate::player::eq;

pub mod taste;
pub use taste::{FollowedArtist, PlayStat, SavedAlbum};

/// Il backend di una configurazione nuova: `PSYTIFY_DEFAULT_API_URL` alla compilazione (la build di
/// distribuzione ci mette il proprio backend), altrimenti un server sul proprio PC.
pub const DEFAULT_API_URL: &str = match option_env!("PSYTIFY_DEFAULT_API_URL") {
    // Una variabile definita ma vuota (CI senza backend di distribuzione) vale come assente.
    Some(url) if !url.is_empty() => url,
    _ => "http://127.0.0.1:8080",
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
/// Porta di loopback del login: va registrata come redirect URI su Clerk.
pub const DEFAULT_OAUTH_PORT: u16 = 53682;
pub const MAX_PLAYLIST_NAME: usize = 80;
/// Brani ascoltati di recente che l'app ricorda.
pub const MAX_RECENT: usize = 30;
/// Playlist locali al massimo: oltre, la barra laterale non serve più a niente.
pub const MAX_PLAYLISTS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum SavedScreen {
    #[default]
    Home,
    Search,
    Library,
    Album(Uuid),
    Playlist(Uuid),
    Artist(Uuid),
    Settings,
    Jam,
    /// I brani a cui si è messo "mi piace".
    Liked,
    /// Le uscite degli artisti seguiti.
    News,
    /// La libreria personale (telefono).
    Collection,
}

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
}

impl Playlist {
    pub fn new(name: &str, tracks: Vec<Track>) -> Self {
        Self { id: Uuid::new_v4(), name: clean_name(name), tracks, pending: Vec::new() }
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Backend scelto dall'utente nelle impostazioni; `None` = quello predefinito della build.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    /// Chiave delle versioni precedenti, che scrivevano sempre il backend in uso: un indirizzo
    /// vecchio sopravviveva così a ogni aggiornamento. Si legge soltanto, per la migrazione.
    #[serde(rename = "api_base_url", skip_serializing)]
    pub(crate) legacy_api: Option<String>,
    pub volume: f32,
    pub last_screen: SavedScreen,
    pub queue: Queue,
    pub playlists: Vec<Playlist>,
    /// Pannello destro aperto alla chiusura.
    #[serde(alias = "queue_open")]
    pub panel_open: bool,
    /// Brani a cui si è messo "mi piace", il più recente in testa. Il backend non ha utenti per
    /// questo: come le playlist, stanno solo su questo computer.
    pub liked: Vec<Track>,
    /// Ultimi brani ascoltati, il più recente in testa.
    pub recent: Vec<Track>,
    /// Ultima ricerca eseguita, ripetuta se l'ultima schermata era Cerca.
    pub last_query: String,
    pub quality: Quality,
    /// Qualità dei download (`Message::Download`); dal menu di un brano si può scegliere un'altra.
    pub download_quality: DownloadQuality,
    /// Frontend API dell'istanza Clerk (`https://...`); vuoto = login disattivato.
    pub clerk_issuer: String,
    pub clerk_client_id: String,
    pub oauth_port: u16,
    /// Equalizzatore a 10 bande, acceso o spento con le sue regolazioni.
    pub equalizer: eq::Settings,
    /// Artisti seguiti: le loro uscite compaiono in «Novità».
    pub followed: Vec<FollowedArtist>,
    /// Album salvati nella libreria.
    pub saved_albums: Vec<SavedAlbum>,
    /// Ascolti per brano: i più ascoltati, i tuoi artisti e i mix si ricavano da qui.
    pub stats: Vec<PlayStat>,
    /// Tempo ascoltato in tutto, in millisecondi.
    pub listened_ms: u64,
    /// L'uscita più recente che si è vista in «Novità» (`created_at` del brano): ciò che viene dopo
    /// è nuovo.
    pub news_seen: String,
    /// A fine coda continua da solo con brani dello stesso artista.
    pub autoplay: bool,
    /// Il pannello «In riproduzione» mostra il videoclip al posto della copertina, se c'è.
    pub panel_video: bool,
    /// Ultime ricerche, la più recente in testa (`remember_search`).
    pub search_history: Vec<String>,
    /// I videoclip li decodifica la scheda video dedicata (NVIDIA), se c'è: più potente, ma la tiene
    /// sveglia. Di norma decodifica la GPU integrata, che consuma meno.
    pub dedicated_video: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: None,
            legacy_api: None,
            volume: 0.8,
            last_screen: SavedScreen::Home,
            queue: Queue::default(),
            playlists: Vec::new(),
            panel_open: false,
            liked: Vec::new(),
            recent: Vec::new(),
            last_query: String::new(),
            quality: Quality::default(),
            download_quality: DownloadQuality::default(),
            clerk_issuer: String::new(),
            clerk_client_id: String::new(),
            oauth_port: DEFAULT_OAUTH_PORT,
            equalizer: eq::Settings::default(),
            followed: Vec::new(),
            saved_albums: Vec::new(),
            stats: Vec::new(),
            listened_ms: 0,
            news_seen: String::new(),
            autoplay: true,
            panel_video: true,
            search_history: Vec::new(),
            dedicated_video: false,
        }
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

impl Config {
    /// L'applicazione OAuth di Clerk, se configurata con un issuer https e un client ID.
    pub fn clerk_app(&self) -> Option<ClerkApp> {
        let issuer = self.clerk_issuer.trim().trim_end_matches('/');
        let client_id = self.clerk_client_id.trim();
        let valid = issuer.starts_with("https://")
            && issuer.len() > "https://".len()
            && !issuer.contains(['?', '#', ' '])
            && !client_id.is_empty()
            && client_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
        valid.then(|| ClerkApp {
            issuer: issuer.to_owned(),
            client_id: client_id.to_owned(),
            port: self.oauth_port,
        })
    }

    /// Ripara valori fuori intervallo letti da disco.
    fn sanitize(mut self) -> Self {
        if !self.volume.is_finite() {
            self.volume = Self::default().volume;
        }
        self.volume = self.volume.clamp(0.0, 1.0);
        self.queue.sanitize();
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
        for playlist in &mut self.playlists {
            playlist.name = clean_name(&playlist.name);
        }
        self.playlists.truncate(MAX_PLAYLISTS);
        self.recent.truncate(MAX_RECENT);
        self.followed.truncate(taste::MAX_FOLLOWED);
        self.saved_albums.truncate(taste::MAX_SAVED_ALBUMS);
        self.stats.truncate(taste::MAX_STATS);
        self
    }
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
/// quelle del sistema che `dirs` conosce solo su desktop.
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

pub fn path() -> Option<PathBuf> {
    dir().map(|dir| dir.join("config.json"))
}

/// Legge la configurazione. Un file assente dà i default; un file illeggibile viene messo da
/// parte come `config.json.broken` e si riparte dai default, senza perdere i dati dell'utente.
pub fn load() -> Config {
    let mut config = path().map_or_else(Config::default, |p| load_from(&p));
    for (var, field) in
        [(CLERK_ISSUER_ENV, &mut config.clerk_issuer), (CLERK_CLIENT_ID_ENV, &mut config.clerk_client_id)]
    {
        if let Ok(value) = std::env::var(var)
            && !value.trim().is_empty()
        {
            *field = value;
        }
    }
    config
}

/// Lettura sincrona: avviene una volta sola, prima che la finestra esista.
fn load_from(path: &Path) -> Config {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Config::default();
    };
    match serde_json::from_str::<Config>(&text) {
        Ok(config) => config.sanitize(),
        Err(_) => {
            let _ = std::fs::rename(path, path.with_extension("json.broken"));
            Config::default()
        }
    }
}

/// Scrive la configurazione in modo atomico: file temporaneo e poi rinomina.
pub async fn save(config: Config) -> Result<(), AppError> {
    let Some(path) = path() else { return Ok(()) };
    save_to(&path, &config).await
}

async fn save_to(path: &Path, config: &Config) -> Result<(), AppError> {
    let io_error = |_| AppError::Io(path.display().to_string());
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io_error)?;
    }
    let json = serde_json::to_vec_pretty(config).map_err(|_| AppError::Io(path.display().to_string()))?;
    let temp = path.with_extension("json.tmp");
    tokio::fs::write(&temp, json).await.map_err(io_error)?;
    tokio::fs::rename(&temp, path).await.map_err(io_error)
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

    #[tokio::test]
    async fn config_round_trips_through_disk() {
        let path = temp_dir().join("config.json");
        let mut config = Config { volume: 0.35, last_screen: SavedScreen::Library, ..Config::default() };
        config.queue.replace(vec![track(1), track(2)], 1, 3);
        config.playlists.push(Playlist::new("Sera", vec![track(3)]));

        save_to(&path, &config).await.unwrap();

        assert_eq!(load_from(&path), config);
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
    fn the_old_queue_open_key_still_loads() {
        let config: Config = serde_json::from_str(r#"{"queue_open": true}"#).unwrap();
        assert!(config.panel_open);
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
        let config = Config { volume: 4.0, ..Config::default() }.sanitize();
        assert_eq!(config.volume, 1.0);
    }

    #[test]
    fn unknown_fields_and_missing_fields_are_tolerated() {
        let config: Config = serde_json::from_str(r#"{"volume": 0.5, "future": true}"#).unwrap();
        assert_eq!((config.volume, config.server), (0.5, None));
    }

    #[test]
    fn a_legacy_default_address_is_not_a_choice() {
        for url in BUILTIN_API_URLS {
            let json = format!(r#"{{"api_base_url": "{url}/"}}"#);
            let config = serde_json::from_str::<Config>(&json).unwrap().sanitize();
            assert_eq!(config.server, None, "{url}");
        }
    }

    #[test]
    fn a_legacy_custom_address_becomes_the_user_choice() {
        let json = r#"{"api_base_url": "https://music.example.org/"}"#;
        let config = serde_json::from_str::<Config>(json).unwrap().sanitize();
        assert_eq!(config.server.as_deref(), Some("https://music.example.org"));
        // Riscritto, il file non ha più la chiave vecchia: la scelta sta in `server`.
        let saved = serde_json::to_value(&config).unwrap();
        assert_eq!(saved.get("api_base_url"), None);
        assert_eq!(saved["server"], "https://music.example.org");
    }

    #[test]
    fn no_choice_means_the_build_default() {
        let saved = serde_json::to_value(Config::default()).unwrap();
        assert_eq!(saved.get("server"), None);
        assert_eq!(normalize_server("  "), None);
        assert_eq!(normalize_server(&format!("{}/", default_api_url())), None);
        assert_eq!(normalize_server(" http://10.0.0.5:8080/ ").as_deref(), Some("http://10.0.0.5:8080"));
    }

    #[test]
    fn clerk_login_is_off_without_issuer() {
        assert_eq!(Config::default().clerk_app(), None);
    }

    #[test]
    fn clerk_app_requires_https_issuer() {
        let config = Config {
            clerk_issuer: "http://x.clerk.accounts.dev".into(),
            clerk_client_id: "abc".into(),
            ..Config::default()
        };
        assert_eq!(config.clerk_app(), None);
    }

    #[test]
    fn clerk_app_trims_the_issuer() {
        let config = Config {
            clerk_issuer: " https://x.clerk.accounts.dev/ ".into(),
            clerk_client_id: "abc_1".into(),
            ..Config::default()
        };
        assert_eq!(config.clerk_app().map(|a| a.issuer), Some("https://x.clerk.accounts.dev".into()));
    }

    #[test]
    fn privileged_oauth_port_falls_back_to_the_default() {
        assert_eq!(Config { oauth_port: 80, ..Config::default() }.sanitize().oauth_port, DEFAULT_OAUTH_PORT);
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
