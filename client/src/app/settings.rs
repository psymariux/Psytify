//! Le impostazioni, smistate da `App::handle`, e le preferenze di questa interfaccia ([`UiPrefs`]),
//! che `config.json` tiene accanto ai campi del core. Il file lo scrive solo il core
//! (`core/src/state/settings.rs`): l'app gli manda lo stato con [`App::persist`] e mostra un avviso
//! se il salvataggio non riesce.

use iced::Task;
use psytify_core::Command;
use psytify_core::command::SettingsCmd;
use psytify_core::snapshot::SettingsView;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use super::{App, Message, Notice, Screen};
use crate::config::{self, CoreConfig};

/// La schermata riaperta all'avvio.
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
    /// I brani scaricati sul dispositivo.
    Downloads,
    /// Le uscite degli artisti seguiti.
    News,
    /// La libreria personale (telefono).
    Collection,
}

impl From<SavedScreen> for Screen {
    fn from(saved: SavedScreen) -> Self {
        match saved {
            SavedScreen::Home => Self::Home,
            SavedScreen::Search => Self::Search,
            SavedScreen::Library => Self::Library,
            SavedScreen::Album(id) => Self::Album(id),
            SavedScreen::Playlist(id) => Self::Playlist(id),
            SavedScreen::Artist(id) => Self::Artist(id),
            SavedScreen::Settings => Self::Settings,
            SavedScreen::Jam => Self::Jam,
            SavedScreen::Liked => Self::Liked,
            SavedScreen::Downloads => Self::Downloads,
            SavedScreen::News => Self::News,
            SavedScreen::Collection => Self::Collection,
        }
    }
}

impl From<Screen> for SavedScreen {
    fn from(screen: Screen) -> Self {
        match screen {
            Screen::Home => Self::Home,
            // La playlist esterna non si salva: alla riapertura si torna alla ricerca.
            Screen::Search | Screen::SourcePlaylist => Self::Search,
            Screen::Library => Self::Library,
            Screen::Album(id) => Self::Album(id),
            Screen::Playlist(id) => Self::Playlist(id),
            Screen::Artist(id) => Self::Artist(id),
            Screen::Settings => Self::Settings,
            Screen::Jam => Self::Jam,
            Screen::Liked => Self::Liked,
            Screen::Downloads => Self::Downloads,
            Screen::News => Self::News,
            Screen::Collection => Self::Collection,
            // Profili e accesso dipendono dalla rete: alla riapertura si parte dalla Home.
            Screen::SignIn | Screen::Friends | Screen::Profile | Screen::FriendPlaylist => Self::Home,
        }
    }
}

/// Le preferenze di questa interfaccia in `config.json`. Il core le conserva come chiavi qualsiasi,
/// accanto ai suoi campi e con i nomi di sempre, così il file non cambia formato.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct UiPrefs {
    /// La schermata aperta alla chiusura.
    pub last_screen: SavedScreen,
    /// Il pannello destro aperto alla chiusura.
    pub panel_open: bool,
    /// Il pannello «In riproduzione» mostra il videoclip al posto della copertina, se c'è.
    pub panel_video: bool,
    /// I videoclip li decodifica la scheda video dedicata (NVIDIA), se c'è: più potente, ma la tiene
    /// sveglia. Di norma decodifica la GPU integrata, che consuma meno.
    pub dedicated_video: bool,
}

/// Il nome di `panel_open` nelle versioni precedenti.
const OLD_PANEL_OPEN: &str = "queue_open";

impl Default for UiPrefs {
    fn default() -> Self {
        Self { last_screen: SavedScreen::Home, panel_open: false, panel_video: true, dedicated_video: false }
    }
}

impl UiPrefs {
    /// Le preferenze lette dalle chiavi di `config.json` che il core non conosce. Una chiave che
    /// manca, o con un valore che questa versione non capisce (una schermata di una versione più
    /// nuova), prende il suo default senza toccare le altre.
    pub fn from_map(map: &Map<String, Value>) -> Self {
        fn read<T: serde::de::DeserializeOwned>(map: &Map<String, Value>, key: &str) -> Option<T> {
            map.get(key).and_then(|value| T::deserialize(value).ok())
        }
        let default = Self::default();
        Self {
            last_screen: read(map, "last_screen").unwrap_or(default.last_screen),
            panel_open: read(map, "panel_open")
                .or_else(|| read(map, OLD_PANEL_OPEN))
                .unwrap_or(default.panel_open),
            panel_video: read(map, "panel_video").unwrap_or(default.panel_video),
            dedicated_video: read(map, "dedicated_video").unwrap_or(default.dedicated_video),
        }
    }

    /// Le chiavi da salvare (`config::merge_ui`). Il nome vecchio di `panel_open` si toglie: con
    /// tutti e due nel file, una versione precedente non lo rileggerebbe più.
    pub fn to_map(self) -> Map<String, Value> {
        let mut map = match serde_json::to_value(self) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        };
        map.insert(OLD_PANEL_OPEN.into(), Value::Null);
        map
    }
}

impl App {
    /// I messaggi di questo dominio, smistati da `App::handle`.
    pub(super) fn handle_settings(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Backgrounded => {
                if let Some(core) = &self.core {
                    core.send(Command::Settings(SettingsCmd::SaveNow));
                }
                Task::none()
            }
            Message::ServerDraft(url) => {
                self.server_draft = url;
                Task::none()
            }
            Message::ServerApply => {
                let server = config::normalize_server(&self.server_draft);
                self.set_server(server)
            }
            Message::ServerReset => self.set_server(None),
            Message::SetDiscord(on) => {
                self.discord_presence = on;
                self.persist()
            }
            // `handle` manda qui solo i messaggi di questo dominio.
            _ => Task::none(),
        }
    }

    /// Manda al core lo stato da salvare: i campi del core che l'app tiene ancora e le preferenze di
    /// questa interfaccia. Il core li confronta con i suoi, raccoglie le modifiche e scrive lui
    /// (al più ogni mezzo secondo, una scrittura alla volta); se nulla è cambiato, non succede niente.
    /// Senza core (i test sincroni, un core che non è partito) non si salva. Il `Task` è sempre
    /// vuoto: resta perché i chiamanti lo mettono nei loro batch, finché i loro domini non passano
    /// nel core (passi 4.3-4.9).
    pub(super) fn persist(&self) -> Task<Message> {
        if let Some(core) = &self.core {
            core.send(Command::Settings(SettingsCmd::Store(Box::new(self.stored_config()))));
            core.send(Command::Settings(SettingsCmd::SaveUiPrefs(self.ui_prefs().to_map())));
        }
        Task::none()
    }

    /// Le impostazioni appena pubblicate dal core, al posto di `previous`: un salvataggio fallito
    /// si mostra come avviso, una volta per ogni errore nuovo. Intanto il file resta quello di prima.
    pub(super) fn settings_published(&mut self, previous: &SettingsView) {
        if let Some(err) = &self.core_snapshot.settings.save_error
            && previous.save_error.as_ref() != Some(err)
        {
            self.notice = Some(Notice::Error(format!("Impostazioni non salvate. {err}")));
        }
    }

    /// I campi del core che l'app tiene ancora. Quelli della libreria (playlist, brani piaciuti,
    /// ascolti, …), della riproduzione (volume, coda, qualità, equalizzatore, riproduzione
    /// automatica), della ricerca (l'ultimo testo cercato) e dei download (la loro qualità) no: dai
    /// passi 4.2-4.6 li possiede il core, che ignora i loro valori in `Store`.
    pub(super) fn stored_config(&self) -> CoreConfig {
        CoreConfig {
            server: self.server.clone(),
            legacy_api: None,
            clerk_issuer: self.clerk_issuer.clone(),
            clerk_client_id: self.clerk_client_id.clone(),
            discord_presence: self.discord_presence,
            discord_app_id: self.discord_app_id.clone(),
            oauth_port: self.oauth_port,
            ..CoreConfig::default()
        }
    }

    /// Le preferenze di questa interfaccia come stanno adesso.
    pub(super) fn ui_prefs(&self) -> UiPrefs {
        UiPrefs {
            last_screen: self.screen.into(),
            panel_open: self.panel_open,
            panel_video: self.panel_video,
            dedicated_video: self.dedicated_video,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            other => panic!("not an object: {other}"),
        }
    }

    #[test]
    fn ui_prefs_round_trip_through_their_keys() {
        let prefs = UiPrefs {
            last_screen: SavedScreen::Album(Uuid::from_u128(7)),
            panel_open: true,
            panel_video: false,
            dedicated_video: true,
        };
        assert_eq!(UiPrefs::from_map(&prefs.to_map()), prefs);
        assert_eq!(UiPrefs::from_map(&Map::new()), UiPrefs::default());
    }

    #[test]
    fn the_old_queue_open_key_still_loads_and_is_removed_on_save() {
        assert!(UiPrefs::from_map(&map(serde_json::json!({"queue_open": true}))).panel_open);
        let both = map(serde_json::json!({"queue_open": true, "panel_open": false}));
        assert!(!UiPrefs::from_map(&both).panel_open, "the new name wins");
        let saved = config::merge_ui(&both, UiPrefs::default().to_map()).unwrap();
        assert!(!saved.contains_key("queue_open"));
    }

    #[test]
    fn a_value_from_a_newer_version_falls_back_alone() {
        let prefs = UiPrefs::from_map(&map(serde_json::json!({
            "last_screen": {"kind": "podcasts"},
            "panel_open": true,
        })));
        assert_eq!((prefs.last_screen, prefs.panel_open), (SavedScreen::Home, true));
    }
}
