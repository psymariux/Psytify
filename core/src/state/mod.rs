//! Lo stato del core: un riduttore senza IO. `apply` riceve un ingresso e l'istante corrente e
//! restituisce gli effetti da eseguire; non apre la rete, non legge l'orologio, non tocca i file.
//! Così i test lo guidano con un tempo finto e controllano gli effetti senza eseguirli.
//!
//! Ogni dominio sta nel suo modulo e cambia la sua vista con `Arc::make_mut`: finché un frontend
//! tiene l'istantanea precedente la vista si copia, e `publish` riconosce dal puntatore cambiato
//! quali domini pubblicare.

pub mod catalog;
pub mod downloads;
pub mod external;
pub mod jam;
pub mod library;
pub mod playback;
pub mod settings;
pub mod social;
pub mod system;
#[cfg(feature = "video")]
pub mod video;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::command::Command;
use crate::config::Config;
use crate::effect::{Done, Effect, TimerKey};
use crate::snapshot::{DomainVersions, Snapshot};

/// Ciò che fa muovere lo stato.
#[derive(Debug)]
pub(crate) enum Input {
    /// Un comando di un frontend.
    Command(Command),
    /// Un timer armato con [`Effect::Timer`] è scaduto.
    Timer(TimerKey),
    /// Un lavoro chiesto con [`Effect::Run`] è finito.
    Done(Done),
    /// L'uscita audio ha risposto a un'[`Effect::Audio`].
    Audio(playback::AudioFact),
    /// Un comando dal sistema (tasti multimediali, widget del desktop, cuffie) o il collegamento
    /// multimediale pronto.
    Media(crate::media::Event),
    /// Qualcosa dalla stanza della jam (ingresso, membri, lo stato degli altri, la chiusura): l'host
    /// aspetta il collegamento insieme a tutto il resto, e una jam ferma non ha bisogno di un battito.
    Jam(crate::jam::Event),
    /// Che cosa il sistema chiede del movimento, a ogni cambio vero (dopo `Effect::WatchMotion`).
    SystemMotion(crate::system_motion::SystemMotion),
    /// Il core è appena partito: i domini si passano ciò che hanno letto da `config.json` (Discord,
    /// se l'utente lo vuole, parte da qui, prima di ogni comando).
    Start,
    /// Il frontend chiude: i domini finiscono di scrivere, poi [`Effect::Flushed`].
    Shutdown,
}

/// Lo stato di tutti i domini.
#[derive(Debug, Default)]
pub(crate) struct CoreState {
    pub(crate) settings: settings::Settings,
    pub(crate) library: library::Library,
    pub(crate) playback: playback::Playback,
    pub(crate) catalog: catalog::Catalog,
    pub(crate) external: external::External,
    pub(crate) downloads: downloads::Downloads,
    pub(crate) social: social::Social,
    pub(crate) jam: jam::Jam,
    #[cfg(feature = "video")]
    pub(crate) video: video::Video,
}

impl CoreState {
    /// Lo stato all'avvio, con la configurazione già letta da `path` (l'host la legge prima di
    /// partire: il riduttore non tocca i file). Senza `path` non si scrive niente. `seed` tiene
    /// fermi i mix della Home per la sessione (lo sceglie l'host: il riduttore non ha orologio).
    pub(crate) fn new(config: Config, path: Option<PathBuf>, seed: u64) -> Self {
        let library = library::Library::new(config.core.library(), seed);
        let playback = playback::Playback::new(
            config.core.playback(),
            config.core.server.clone(),
            seed,
            crate::config::taste::now_secs(),
        );
        let catalog = catalog::Catalog::new(config.core.server.clone());
        let external = external::External::new(config.core.server.clone());
        let downloads = downloads::Downloads::new(
            &config.core.downloads(),
            config.core.server.clone(),
            downloads::Dirs::default(),
        );
        let social = social::Social::new(config.core.clerk_app(), config.core.server.clone());
        let jam = jam::Jam::new(config.core.server.clone());
        #[cfg(feature = "video")]
        let video = video::Video::new(config.core.server.clone());
        Self {
            settings: settings::Settings::new(config, path),
            library,
            playback,
            catalog,
            external,
            downloads,
            social,
            jam,
            #[cfg(feature = "video")]
            video,
        }
    }

    /// Le cartelle dove scrivono i download: i brani e i pacchetti degli aggiornamenti. Le sceglie
    /// l'host (il riduttore non conosce il sistema).
    #[must_use]
    pub(crate) fn with_dirs(mut self, downloads: PathBuf, updates: PathBuf) -> Self {
        self.downloads.set_dirs(downloads::Dirs { downloads, updates });
        self
    }

    /// Il login: dove sta `session.json` (nella cartella dei dati di questo core) e se l'host l'ha
    /// trovato sul disco. Con una sessione salvata il dominio parte già su «accesso in corso» e la
    /// riprende al primo `Input::Start`.
    #[must_use]
    pub(crate) fn with_session(mut self, file: Option<PathBuf>, stored: bool) -> Self {
        self.social = std::mem::take(&mut self.social).with_session(file, stored);
        self
    }

    /// L'uscita audio non si è aperta: la riproduzione lo sa dall'avvio.
    pub(crate) fn with_device_error(mut self, error: Option<crate::error::AppError>) -> Self {
        self.playback.set_device_error(error);
        self
    }

    /// Passa l'ingresso al dominio che lo riguarda e raccoglie gli effetti che chiede.
    pub(crate) fn apply(&mut self, input: Input, now: Instant) -> Vec<Effect> {
        let mut effects = Vec::new();
        match input {
            Input::Command(command) => match command {
                Command::Settings(command) => self.settings.apply(command, now, &mut effects),
                Command::Library(command) => self.library.apply(*command, now, &mut effects),
                Command::Playback(command) => self.playback.apply(command, now, &mut effects),
                Command::Catalog(command) => self.catalog.apply(command, now, &mut effects),
                Command::External(command) => self.external.apply(command, now, &mut effects),
                Command::Downloads(command) => self.downloads.apply(command, now, &mut effects),
                Command::Social(command) => self.social.apply(command, now, &mut effects),
                Command::Jam(command) => self.jam.apply(command, now, &mut effects),
                #[cfg(feature = "video")]
                Command::Video(command) => self.video.apply(command, now, &mut effects),
            },
            Input::Timer(key) => match key {
                TimerKey::Settings(timer) => self.settings.fire(timer, now, &mut effects),
                TimerKey::Library(timer) => self.library.fire(timer, now, &mut effects),
                TimerKey::Playback(timer) => self.playback.fire(timer, now, &mut effects),
                TimerKey::Catalog(timer) => self.catalog.fire(timer, now, &mut effects),
                TimerKey::External(timer) => self.external.fire(timer, now, &mut effects),
                TimerKey::Downloads(timer) => self.downloads.fire(timer, now, &mut effects),
                TimerKey::Social(timer) => self.social.fire(timer, now, &mut effects),
                #[cfg(feature = "video")]
                TimerKey::Video(timer) => self.video.fire(timer, now, &mut effects),
            },
            Input::Audio(fact) => self.playback.audio(fact, now, &mut effects),
            Input::Media(event) => self.playback.media(event, now, &mut effects),
            Input::Jam(event) => self.jam.event(event),
            Input::SystemMotion(motion) => self.settings.system_motion(motion),
            Input::Done(done) => match done {
                Done::Settings(done) => self.settings.done(done, now, &mut effects),
                Done::Library(done) => self.library.done(done, now, &mut effects),
                Done::Playback(done) => self.playback.done(done, now, &mut effects),
                Done::Catalog(done) => self.catalog.done(done, now, &mut effects),
                Done::External(done) => self.external.done(done, now, &mut effects),
                Done::Downloads(done) => self.downloads.done(done, now, &mut effects),
                Done::Social(done) => self.social.done(done, now, &mut effects),
                Done::Jam(done) => self.jam.done(done, now, &mut effects),
                #[cfg(feature = "video")]
                Done::Video(done) => self.video.done(done, now, &mut effects),
            },
            // Il passaggio fra i domini avviene dopo ogni ingresso, qui sotto: il catalogo rilegge
            // i brani salvati senza copertina.
            Input::Start => {
                self.catalog.start();
                self.external.start();
                self.downloads.start(&mut effects);
                self.social.start(&mut effects);
            }
            // Ogni dominio salva attraverso le impostazioni: è lì che la chiusura aspetta il disco.
            Input::Shutdown => {
                // Prima della riproduzione: fermarsi alla chiusura non va detto alla stanza.
                self.jam.close();
                self.playback.close(now, &mut effects);
                self.handoff(now, &mut effects);
                self.settings.close(&mut effects);
            }
        }
        self.handoff(now, &mut effects);
        effects
    }

    /// Ciò che un dominio passa agli altri dopo un ingresso: il serbatoio dei brani recenti e il
    /// backend a chi ne ha bisogno, ciò che la riproduzione chiede alla libreria (ascolti, tempo
    /// ascoltato), e i campi che un dominio possiede alle impostazioni, che scrivono il file.
    fn handoff(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        // Le fonti esterne, per prime: leggono la libreria, il catalogo e la riproduzione, e ciò che
        // chiedono (un brano importato, un `Play`, «il catalogo è cambiato») lo applicano gli altri
        // domini nello stesso giro, qui e nei passaggi che seguono. La prossima importazione parte
        // solo dopo, con la libreria già aggiornata.
        self.external.set_server(&self.settings.view.config.server);
        self.external.settle(&self.library.view, &self.catalog.view, &self.playback.view, effects);
        for command in self.external.take_library() {
            self.library.apply(command, now, effects);
        }
        for command in self.external.take_playback() {
            self.playback.apply(command, now, effects);
        }
        for command in self.external.take_catalog() {
            self.catalog.apply(command, now, effects);
        }
        self.external.pace(&self.library.view, effects);
        // Il catalogo: ritrova i brani che la riproduzione dice spariti, rilegge quello appena
        // partito, e consegna alla libreria e alla riproduzione ciò che ha chiesto di cambiare (i
        // brani recenti da cui nasce il feed, le copertine corrette, i brani ritrovati).
        for track in self.playback.take_heal() {
            self.catalog.heal(track, effects);
        }
        self.catalog.set_server(&self.settings.view.config.server, effects);
        let current = self.playback.view.queue.current().map(|track| track.id);
        self.catalog.track_started(self.playback.view.started, current, effects);
        for command in self.catalog.take_library() {
            self.library.apply(command, now, effects);
        }
        for command in self.catalog.take_playback() {
            self.playback.apply(command, now, effects);
        }
        self.catalog.after_library(&self.library.view, effects);
        if let Some(query) = self.catalog.take_query() {
            self.settings.store_query(query, effects);
        }
        self.downloads.set_server(&self.settings.view.config.server);
        self.playback.set_pool(self.library.view.feed_source.as_ref());
        self.playback.set_server(&self.settings.view.config.server);
        self.playback.set_feed(&self.library.view.feed);
        let config = &self.settings.view.config;
        self.playback.set_discord(config.discord_presence, &config.discord_app_id, now, effects);
        // La jam: legge la riproduzione e il profilo. Ciò che chiede alla riproduzione (il gruppo,
        // l'allineamento) si applica subito, e solo dopo la stanza riceve lo stato locale, già
        // allineato. Il frontend non fa da tramite.
        self.jam.set_server(&self.settings.view.config.server);
        self.jam.set_profile(self.social.view.account.profile().map(|profile| profile.display_name.as_str()));
        self.jam.settle(&self.playback.view, now, effects);
        for command in self.jam.take_playback() {
            self.playback.apply(command, now, effects);
        }
        self.jam.publish(&self.playback.view, now, effects);
        for command in self.playback.take_library() {
            self.library.apply(command, now, effects);
        }
        // Il profilo: gli ascolti appena contati, e poi ciò che è cambiato nel brano in ascolto e
        // nelle playlist pubbliche, a cose già assestate. Un ascolto senza accesso si butta.
        self.social.set_server(&self.settings.view.config.server);
        self.social.set_jam(self.jam.code());
        for track in self.playback.take_listens() {
            self.social.listen(track, effects);
        }
        self.social.sync(&self.playback.view, &self.library.view, now, effects);
        if self.library.take_persist() {
            self.settings.store_library(self.library.fields(), effects);
        }
        if self.playback.take_persist() {
            self.settings.store_playback(self.playback.fields(), effects);
        }
        // I file scaricati: la riproduzione dice quelli che non ha potuto aprire (i download li
        // segnano persi), e riceve la mappa di quelli da cui suonare.
        let lost = self.playback.take_lost_files();
        if !lost.is_empty() {
            self.downloads.files_lost(lost);
        }
        self.playback.set_local(self.downloads.local());
        if self.downloads.take_persist() {
            self.settings.store_downloads(self.downloads.fields(), effects);
        }
        // Il videoclip, per ultimo: legge la riproduzione già assestata (il brano in corso, la coda,
        // il brano che è appena partito) e non chiede niente a nessun altro dominio.
        #[cfg(feature = "video")]
        {
            self.video.set_server(&self.settings.view.config.server);
            self.video.settle(&self.playback.view, effects);
        }
    }

    /// La prima istantanea, prima di ogni ingresso: versione zero per tutti.
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            version: 0,
            versions: DomainVersions::default(),
            settings: Arc::clone(&self.settings.view),
            library: Arc::clone(&self.library.view),
            playback: Arc::clone(&self.playback.view),
            catalog: Arc::clone(&self.catalog.view),
            external: Arc::clone(&self.external.view),
            downloads: Arc::clone(&self.downloads.view),
            social: Arc::clone(&self.social.view),
            jam: Arc::clone(&self.jam.view),
            #[cfg(feature = "video")]
            video: Arc::clone(&self.video.view),
        }
    }

    /// L'istantanea che segue `previous`, se almeno un dominio è cambiato; `None` altrimenti, così
    /// un core fermo non sveglia nessun frontend. La versione di un dominio cresce quando la sua
    /// vista non è più quella pubblicata.
    pub(crate) fn publish(&self, previous: &Snapshot) -> Option<Snapshot> {
        let mut versions = previous.versions;
        let mut changed = false;
        changed |= bump(&previous.settings, &self.settings.view, &mut versions.settings);
        changed |= bump(&previous.library, &self.library.view, &mut versions.library);
        changed |= bump(&previous.playback, &self.playback.view, &mut versions.playback);
        changed |= bump(&previous.catalog, &self.catalog.view, &mut versions.catalog);
        changed |= bump(&previous.external, &self.external.view, &mut versions.external);
        changed |= bump(&previous.downloads, &self.downloads.view, &mut versions.downloads);
        changed |= bump(&previous.social, &self.social.view, &mut versions.social);
        changed |= bump(&previous.jam, &self.jam.view, &mut versions.jam);
        #[cfg(feature = "video")]
        {
            changed |= bump(&previous.video, &self.video.view, &mut versions.video);
        }
        changed.then(|| Snapshot { version: previous.version + 1, versions, ..self.snapshot() })
    }
}

/// Fa crescere `version` se la vista corrente non è quella pubblicata. Dice se è cambiata.
fn bump<T>(published: &Arc<T>, current: &Arc<T>, version: &mut u64) -> bool {
    if Arc::ptr_eq(published, current) {
        return false;
    }
    *version += 1;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures::track;
    use crate::command::{LibraryCmd, PlaybackCmd, SettingsCmd};
    use crate::config::CoreConfig;
    use crate::effect;
    use crate::snapshot::PlaybackView;

    fn command(state: &mut CoreState, command: Command) -> Vec<Effect> {
        state.apply(Input::Command(command), Instant::now())
    }

    fn like(n: u128) -> Command {
        Command::library(LibraryCmd::ToggleLike(track(n)))
    }

    /// Un core con un file da scrivere, così i cambi della libreria armano il salvataggio.
    fn state_with_a_file() -> CoreState {
        CoreState::new(Config::default(), Some(PathBuf::from("/dati/psytify/config.json")), 1)
    }

    #[test]
    fn an_untouched_state_publishes_nothing() {
        let state = CoreState::default();
        assert!(state.publish(&state.snapshot()).is_none());
    }

    #[test]
    fn a_changed_domain_bumps_only_its_own_version() {
        let mut state = CoreState::default();
        let first = state.snapshot();
        state.playback.view = Arc::new(PlaybackView::default());
        let next = state.publish(&first).unwrap();
        assert_eq!(next.version, 1);
        assert_eq!(next.versions, DomainVersions { playback: 1, ..DomainVersions::default() });
        assert!(Arc::ptr_eq(&next.settings, &first.settings), "gli altri domini restano gli stessi");
        assert!(!Arc::ptr_eq(&next.playback, &first.playback));
        assert!(state.publish(&next).is_none(), "pubblicata una volta, non si ripubblica");
    }

    /// Prestazioni (passo 4.2): fermo, o con comandi che non cambiano niente, il core non pubblica;
    /// un cambio vero è una pubblicazione sola, con la sola versione dei domini toccati.
    #[test]
    fn idle_and_unrelated_commands_publish_only_real_changes() {
        let mut state = CoreState::default();
        let start = state.snapshot();
        assert!(state.publish(&start).is_none(), "idle");

        // Comandi di altri domini e comandi della libreria senza effetto: nessuna pubblicazione.
        let nothing = [
            Command::Settings(SettingsCmd::Store(Box::default())),
            Command::library(LibraryCmd::Listen(0)),
            Command::library(LibraryCmd::ForgetSearch(None)),
            Command::library(LibraryCmd::RefreshFeed),
            Command::library(LibraryCmd::MarkNewsSeen),
            Command::library(LibraryCmd::DeletePlaylist(uuid::Uuid::from_u128(1))),
            Command::library(LibraryCmd::HomeTracks(None)),
        ];
        for each in nothing {
            command(&mut state, each);
        }
        assert!(state.publish(&start).is_none(), "commands that change nothing");

        // Un cambio vero: una pubblicazione, con la libreria e le impostazioni (il file da scrivere).
        command(&mut state, like(1));
        let next = state.publish(&start).expect("a like is a change");
        assert_eq!(next.version, 1);
        assert_eq!(next.versions.library, 1);
        assert_eq!(next.versions.settings, 1, "the file has to follow");
        assert_eq!(next.versions.playback, 0);
        assert!(Arc::ptr_eq(&next.playback, &start.playback));
        assert!(state.publish(&next).is_none(), "published once");

        // Lo stesso brano in testa agli ascolti due volte: la seconda non cambia niente.
        command(&mut state, Command::library(LibraryCmd::RememberListen(track(1))));
        let heard = state.publish(&next).expect("first listen");
        command(&mut state, Command::library(LibraryCmd::RememberListen(track(1))));
        assert!(state.publish(&heard).is_none());
    }

    /// Una raffica di cambi della libreria è un giro solo: i campi finiscono insieme in `config.json`.
    #[test]
    fn library_changes_reach_the_settings_and_arm_one_save() {
        let mut state = state_with_a_file();
        let start = state.apply(Input::Start, Instant::now());
        assert!(
            start.iter().all(|e| matches!(e, Effect::Discord(_))),
            "il core appena partito accende solo Discord, se c'è: {start:?}"
        );
        let effects = command(&mut state, like(1));
        assert!(matches!(effects.as_slice(), [Effect::Timer { .. }]), "one save armed: {effects:?}");
        assert!(command(&mut state, like(2)).is_empty(), "the second change joins the same save");
        let config = &state.settings.view.config;
        assert_eq!(config.liked.iter().map(|t| t.id).collect::<Vec<_>>(), [track(2).id, track(1).id]);
        assert!(state.library.view.liked.same(&config.liked), "the view and the file share the list");
    }

    /// `SettingsCmd::Store` del frontend non cancella ciò che la libreria ha: i suoi campi si
    /// ignorano, gli altri passano.
    #[test]
    fn the_frontends_stored_config_never_overwrites_the_library() {
        let mut state = state_with_a_file();
        command(&mut state, like(1));
        let stale = CoreConfig { clerk_client_id: "rock".into(), ..CoreConfig::default() };
        command(&mut state, Command::Settings(SettingsCmd::Store(Box::new(stale))));
        let config = &state.settings.view.config;
        assert_eq!(config.clerk_client_id, "rock", "the other fields pass");
        assert_eq!(config.liked.len(), 1, "the library fields stay");
    }

    #[test]
    fn the_library_starts_from_the_saved_file() {
        let mut config = Config::default();
        config.core.liked = vec![track(3)].into();
        config.core.news_seen = "2026-01-01".into();
        let state = CoreState::new(config, None, 1);
        let snapshot = state.snapshot();
        assert_eq!(snapshot.library.liked.len(), 1);
        assert_eq!(snapshot.library.news_seen, "2026-01-01");
        assert!(snapshot.library.liked.same(&snapshot.settings.config.liked), "the same list, not a copy");
    }

    /// Passo 4.3b: l'interruttore di Discord e l'app (impostazioni) arrivano alla riproduzione con il
    /// passaggio fra i domini: attivarlo avvia il collegamento, cambiare l'app lo riavvia,
    /// spegnerlo lo ferma; un `Store` uguale non fa niente.
    #[test]
    fn the_discord_setting_reaches_playback_through_the_handoff() {
        use crate::state::system::DiscordOp;

        let mut state = CoreState::new(
            Config {
                core: CoreConfig { discord_presence: false, ..CoreConfig::default() },
                ..Config::default()
            },
            None,
            1,
        );
        let discord = |effects: Vec<Effect>| -> Vec<String> {
            effects
                .into_iter()
                .filter_map(|effect| match effect {
                    Effect::Discord(DiscordOp::Start(id)) => Some(format!("start {id}")),
                    Effect::Discord(DiscordOp::Stop) => Some("stop".into()),
                    Effect::Discord(DiscordOp::Set(_)) => Some("set".into()),
                    _ => None,
                })
                .collect()
        };
        let store = |presence: bool, app: &str| {
            Command::Settings(SettingsCmd::Store(Box::new(CoreConfig {
                discord_presence: presence,
                discord_app_id: app.into(),
                ..CoreConfig::default()
            })))
        };
        assert!(discord(command(&mut state, store(false, "99"))).is_empty(), "spento");
        assert_eq!(discord(command(&mut state, store(true, "99"))), vec!["start 99", "set"]);
        assert!(discord(command(&mut state, store(true, "99"))).is_empty(), "uguale: niente");
        assert_eq!(discord(command(&mut state, store(true, "100"))), vec!["stop", "start 100", "set"]);
        assert_eq!(discord(command(&mut state, store(false, "100"))), vec!["stop"]);
    }

    /// Prestazioni (passo 4.3): fermo, o in pausa, il core non pubblica e non arma il battito; una
    /// transizione vera è una pubblicazione sola, e la posizione (che avanza) non è nell'istantanea.
    #[test]
    fn idle_and_paused_playback_publish_nothing_and_a_transition_publishes_once() {
        use std::time::Duration;

        use crate::api::Track;
        use crate::player::stream::StreamBuffer;
        use crate::state::playback::{AudioFact, AudioOp, Done as PlaybackDone};

        let mut state = CoreState::default();
        let mut snapshot = state.snapshot();
        let run = |state: &mut CoreState, input: Input| -> Vec<Effect> { state.apply(input, Instant::now()) };
        let ticks = |effects: &[Effect]| effects.iter().filter(|e| matches!(e, Effect::Timer { .. })).count();

        // Fermo: niente comandi, niente istantanee, niente battito.
        assert!(state.publish(&snapshot).is_none());
        let effects = run(&mut state, Input::Command(Command::Playback(PlaybackCmd::Toggle)));
        assert!(
            ticks(&effects) == 0 && state.publish(&snapshot).is_none(),
            "play con la coda vuota non fa niente"
        );

        // Play: Loading, una pubblicazione sola e solo del dominio della riproduzione.
        let tracks: Arc<[Track]> = vec![track(1), track(2)].into();
        let effects =
            run(&mut state, Input::Command(Command::Playback(PlaybackCmd::Play { tracks, start: 0 })));
        let generation = state.playback.view.generation;
        assert_eq!(ticks(&effects), 0, "in caricamento il battito non gira");
        let next = state.publish(&snapshot).expect("play is a transition");
        assert_eq!((next.versions.playback, next.versions.library), (1, 0));
        snapshot = next;

        // Il brano arriva e parte: il battito si arma, una pubblicazione.
        run(
            &mut state,
            Input::Done(Done::Playback(PlaybackDone::Loaded {
                generation,
                result: Ok(StreamBuffer::new()),
                lost: None,
            })),
        );
        let effects = run(&mut state, Input::Audio(AudioFact::Started(Ok(()))));
        assert_eq!(ticks(&effects), 1);
        assert!(effects.iter().all(|e| !matches!(e, Effect::Audio(AudioOp::Play { .. }))));
        let next = state.publish(&snapshot).expect("playing");
        assert!(next.playback.is_playing());
        snapshot = next;

        // Un battito che trova l'audio dov'è previsto non cambia la vista: niente pubblicazione.
        let effects = run(&mut state, Input::Timer(TimerKey::Playback(playback::Timer::Tick)));
        assert_eq!(ticks(&effects), 1, "il battito si riarma finché suona");
        run(
            &mut state,
            Input::Audio(AudioFact::Polled { position: Duration::ZERO, finished: false, seek_failed: false }),
        );
        assert!(state.publish(&snapshot).is_none(), "la posizione non è nell'istantanea");

        // Pausa: una pubblicazione, il battito si ferma; poi il core è di nuovo fermo del tutto.
        let effects = run(&mut state, Input::Command(Command::Playback(PlaybackCmd::Toggle)));
        assert!(effects.iter().any(|e| matches!(e, Effect::CancelTimer(TimerKey::Playback(_)))));
        snapshot = state.publish(&snapshot).expect("paused");
        assert_eq!(snapshot.playback.state, crate::player::PlaybackState::Paused);
        let same_quality = PlaybackCmd::Quality(snapshot.playback.quality);
        let effects = run(&mut state, Input::Command(Command::Playback(same_quality)));
        assert!(effects.is_empty(), "in pausa nessun timer, e un comando senza effetto non fa lavoro");
        assert!(state.publish(&snapshot).is_none(), "in pausa il core non pubblica");
    }

    /// Prestazioni (passo 4.4): fermo, il catalogo non pubblica niente; scrivere lo stesso testo di
    /// nuovo non cambia niente; la ricerca vera è una pubblicazione, ogni risposta una; una risposta
    /// superata nessuna.
    #[test]
    fn catalogue_idle_and_repeated_identical_queries_publish_nothing_and_an_answer_publishes_once() {
        use crate::api::SearchResults;
        use crate::command::CatalogCmd;
        use crate::state::catalog::{self, Done as CatalogDone, Job as CatalogJob};

        let mut state = CoreState::default();
        let mut snapshot = state.snapshot();
        let run = |state: &mut CoreState, input: Input| state.apply(input, Instant::now());
        let typed = |text: &str| Input::Command(Command::Catalog(CatalogCmd::SearchInput(text.into())));
        assert!(state.publish(&snapshot).is_none(), "idle");

        // Scrivere arma l'attesa, ma non cambia la vista; lo stesso testo di nuovo, nemmeno l'attesa.
        let effects = run(&mut state, typed("low"));
        assert!(effects.iter().any(|e| matches!(e, Effect::Timer { key: TimerKey::Catalog(_), .. })));
        assert!(state.publish(&snapshot).is_none(), "typing alone publishes nothing");
        assert!(run(&mut state, typed("low")).is_empty() && state.publish(&snapshot).is_none(), "same text");

        // L'attesa scade: una pubblicazione, del catalogo e di ciò che ne segue (la cronologia).
        let effects = run(&mut state, Input::Timer(TimerKey::Catalog(catalog::Timer::Search)));
        let ticket = effects
            .iter()
            .find_map(|e| match e {
                Effect::Run(effect::Job::Catalog(CatalogJob::Search { ticket, .. })) => Some(*ticket),
                _ => None,
            })
            .expect("the search starts");
        let next = state.publish(&snapshot).expect("the search starts");
        assert_eq!((next.versions.catalog, next.versions.library), (1, 1));
        assert_eq!(next.library.search_history.first().map(String::as_str), Some("low"));
        assert_eq!(next.settings.config.last_query, "low", "the saved query follows through the settings");
        assert!(Arc::ptr_eq(&next.playback, &snapshot.playback));
        snapshot = next;

        // Una risposta vera: una pubblicazione, e solo del catalogo. Una superata: nessuna.
        let answer = |ticket| {
            Input::Done(Done::Catalog(CatalogDone::Search {
                ticket,
                result: Ok(Box::<SearchResults>::default()),
            }))
        };
        run(&mut state, answer(ticket - 1));
        assert!(state.publish(&snapshot).is_none(), "a late answer is dropped");
        run(&mut state, answer(ticket));
        let next = state.publish(&snapshot).expect("an answer is a change");
        assert_eq!((next.versions.catalog, next.versions.library), (2, 1));
        assert!(matches!(next.catalog.search.results, crate::catalog::Load::Ready(_)));
        snapshot = next;

        // Lo stesso testo di nuovo e il core resta fermo del tutto.
        assert!(run(&mut state, typed("low")).is_empty());
        assert!(state.publish(&snapshot).is_none());
    }

    /// I brani recenti della Home arrivano alla libreria con il passaggio, e la Home li mostra
    /// insieme al loro feed nella stessa istantanea; gli artisti in vista si caricano in anticipo.
    #[test]
    fn the_home_and_its_feed_arrive_in_one_snapshot() {
        use crate::api::{Page, Track};
        use crate::command::CatalogCmd;
        use crate::state::catalog::{Done as CatalogDone, Job as CatalogJob, Resource, Section};

        let mut state = CoreState::default();
        let snapshot = state.snapshot();
        let effects = command(&mut state, Command::Catalog(CatalogCmd::Ensure(Resource::Home)));
        let epoch = effects
            .iter()
            .find_map(|e| match e {
                Effect::Run(effect::Job::Catalog(CatalogJob::Home { epoch, .. })) => Some(*epoch),
                _ => None,
            })
            .expect("the home loads");
        let loading = state.publish(&snapshot).expect("loading");
        assert!(loading.catalog.home.recent.is_loading());

        let recent: Vec<Track> = (1..=6).map(track).collect();
        let page = Page { items: recent, limit: 100, offset: 0, next_offset: None };
        let effects = state.apply(
            Input::Done(Done::Catalog(CatalogDone::Home {
                epoch,
                section: Section::Recent,
                result: Ok(page),
            })),
            Instant::now(),
        );
        let next = state.publish(&loading).expect("the tracks arrived");
        let crate::catalog::Load::Ready(shown) = &next.catalog.home.recent else { panic!("not ready") };
        assert!(
            next.library.feed_source.as_ref().is_some_and(|source| Arc::ptr_eq(source, shown)),
            "the feed is built from the very tracks the home shows"
        );
        assert!(!next.library.feed.releases.is_empty());
        let prefetched = effects
            .iter()
            .filter(|e| matches!(e, Effect::Run(effect::Job::Catalog(CatalogJob::Artist { .. }))))
            .count();
        assert!(prefetched > 0, "the artists on the home are loaded ahead: {effects:?}");

        // Il catalogo cambia: la Home torna da caricare e la libreria butta il feed.
        command(&mut state, Command::Catalog(CatalogCmd::Changed));
        let after = state.publish(&next).expect("changed");
        assert!(after.catalog.home.recent.needs_load());
        assert!(after.library.feed_source.is_none() && after.library.feed.releases.is_empty());
    }

    /// Un brano che il server non ha più: la riproduzione lo dice, il catalogo lo ritrova, e il nuovo
    /// prende il posto del vecchio in coda e fra i salvati, senza che il frontend faccia niente.
    #[test]
    fn a_missing_track_is_healed_inside_the_core() {
        use std::sync::Arc;

        use crate::api::Track;
        use crate::command::PlaybackCmd;
        use crate::error::AppError;
        use crate::state::catalog::{Done as CatalogDone, Job as CatalogJob};
        use crate::state::playback::Done as PlaybackDone;

        let mut state = CoreState::default();
        let missing = track(1);
        command(&mut state, like(1));
        let tracks: Arc<[Track]> = vec![missing.clone(), track(2)].into();
        command(&mut state, Command::Playback(PlaybackCmd::Play { tracks, start: 0 }));
        let generation = state.playback.view.generation;
        let not_found =
            AppError::Api { status: 404, code: "not_found".into(), message: "Brano non trovato".into() };
        let effects = state.apply(
            Input::Done(Done::Playback(PlaybackDone::Loaded {
                generation,
                result: Err(not_found),
                lost: None,
            })),
            Instant::now(),
        );
        assert!(
            effects.iter().any(|e| matches!(e, Effect::Run(effect::Job::Catalog(CatalogJob::Heal { track, .. })) if track.id == missing.id)),
            "the catalogue looks for it: {effects:?}"
        );

        let found = track(9);
        let effects = state.apply(
            Input::Done(Done::Catalog(CatalogDone::Healed {
                old: missing.id,
                result: Ok(Box::new(found.clone())),
            })),
            Instant::now(),
        );
        assert_eq!(state.playback.view.queue.current().map(|t| t.id), Some(found.id));
        assert_eq!(state.library.view.liked.first().map(|t| t.id), Some(found.id));
        assert!(
            effects.iter().any(|e| matches!(e, Effect::Run(effect::Job::Playback(playback::Job::Load { track_id, .. })) if *track_id == found.id)),
            "and it starts again: {effects:?}"
        );

        // Se non lo ritrova, la riproduzione resta in errore.
        let mut state = CoreState::default();
        let tracks: Arc<[Track]> = vec![missing.clone()].into();
        command(&mut state, Command::Playback(PlaybackCmd::Play { tracks, start: 0 }));
        let generation = state.playback.view.generation;
        let not_found =
            AppError::Api { status: 404, code: "not_found".into(), message: "Brano non trovato".into() };
        state.apply(
            Input::Done(Done::Playback(PlaybackDone::Loaded {
                generation,
                result: Err(not_found),
                lost: None,
            })),
            Instant::now(),
        );
        state.apply(
            Input::Done(Done::Catalog(CatalogDone::Healed {
                old: missing.id,
                result: Err(AppError::InvalidResponse),
            })),
            Instant::now(),
        );
        assert!(matches!(state.playback.view.state, crate::player::PlaybackState::Error(_)));
    }

    /// I lavori sociali che gli effetti chiedono, in ordine.
    fn social_work(effects: &[Effect]) -> Vec<crate::testing::SocialWork> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Run(effect::Job::Social(job)) => Some(crate::testing::SocialWork::of(job)),
                _ => None,
            })
            .collect()
    }

    fn adopt(state: &mut CoreState) -> Vec<Effect> {
        let clerk = state.social.clerk_app().expect("la configurazione di default ha un Clerk");
        let me = crate::api::Me {
            user_id: "user_prova".into(),
            admin: false,
            profile: Some(crate::api::social::MyProfile {
                handle: "prova".into(),
                display_name: "Prova".into(),
                avatar_url: None,
                share_activity: true,
            }),
        };
        command(
            state,
            Command::Social(crate::command::SocialCmd::Adopt {
                session: crate::auth::Session::for_tests(clerk),
                me,
            }),
        )
    }

    /// Passo 4.7, versioni dell'istantanea: un core senza accesso, anche con la riproduzione in
    /// moto, non pubblica mai il dominio sociale; un cambio sociale vero (l'accesso che parte)
    /// alza solo la sua versione, e la riproduzione non alza la sua.
    #[test]
    fn the_social_domain_bumps_its_own_version_only_on_real_changes() {
        let mut state = CoreState::new(Config::default(), None, 1);
        let start = state.snapshot();
        let effects = state.apply(Input::Start, Instant::now());
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::Run(_) | Effect::Timer { .. } | Effect::OpenUrl(_))),
            "un core senza accesso non lancia lavori né arma timer: {effects:?}"
        );
        assert!(state.publish(&start).is_none());

        // La riproduzione cambia, il sociale no: senza accesso `sync` non fa niente.
        command(&mut state, Command::Playback(PlaybackCmd::Play { tracks: vec![track(1)].into(), start: 0 }));
        let playing = state.publish(&start).expect("la riproduzione è partita");
        assert_eq!(playing.versions.social, 0);
        assert!(Arc::ptr_eq(&playing.social, &start.social));

        // Un comando sociale che non cambia niente non pubblica.
        command(&mut state, Command::Social(crate::command::SocialCmd::DismissError));
        command(&mut state, Command::Social(crate::command::SocialCmd::RefreshFriends));
        assert!(state.publish(&playing).is_none());

        // L'accesso che parte è un cambio vero: «accesso in corso», e basta.
        let effects = command(
            &mut state,
            Command::Social(crate::command::SocialCmd::SignIn {
                identifier: "psy".into(),
                password: crate::auth::Secret::new("una-password"),
            }),
        );
        assert_eq!(social_work(&effects), [crate::testing::SocialWork::SignIn]);
        let working = state.publish(&playing).expect("accesso in corso");
        assert_eq!(working.versions.social, 1);
        assert_eq!(working.versions.playback, playing.versions.playback);
        assert_eq!(working.social.account, social::Account::Working);
        assert!(state.publish(&working).is_none(), "pubblicato una volta");
    }

    /// Il passaggio fra i domini porta la riproduzione e la libreria al profilo: mentre un brano si
    /// carica non parte nessuna presenza (BACKLOG 9), e una playlist resa pubblica parte da sola.
    #[test]
    fn the_handoff_feeds_the_profile_from_playback_and_library() {
        let mut state = CoreState::new(Config::default(), None, 1);
        adopt(&mut state);

        let effects = command(
            &mut state,
            Command::Playback(PlaybackCmd::Play { tracks: vec![track(1)].into(), start: 0 }),
        );
        assert!(matches!(state.playback.view.state, crate::player::PlaybackState::Loading { .. }));
        assert!(
            !social_work(&effects).iter().any(|w| matches!(w, crate::testing::SocialWork::Presence { .. })),
            "mentre si carica non si dice niente: {effects:?}"
        );

        let playlist = crate::config::Playlist::new("Sera", vec![track(2)]);
        let id = playlist.id;
        command(
            &mut state,
            Command::library(LibraryCmd::CreatePlaylist {
                id,
                name: playlist.name,
                tracks: playlist.tracks,
                pending: Vec::new(),
            }),
        );
        let effects =
            command(&mut state, Command::library(LibraryCmd::SetPlaylistPublic { id, public: true }));
        assert_eq!(
            social_work(&effects),
            [crate::testing::SocialWork::PublishPlaylist {
                id,
                name: "Sera".into(),
                tracks: vec![track(2).id]
            }]
        );
        // Un comando che non c'entra non rimanda niente.
        let effects = command(&mut state, Command::library(LibraryCmd::RefreshFeed));
        assert!(social_work(&effects).is_empty());
    }

    /// Passo 4.8, versioni dell'istantanea: una jam ferma non pubblica niente e non arma nessun timer
    /// (il core non ha un timer per lei: è la stanza a svegliarlo); ogni cambio vero è una
    /// pubblicazione sola, con la versione dei soli domini toccati; ripetere ciò che c'è già non
    /// cambia nulla.
    #[test]
    fn an_idle_jam_arms_nothing_and_publishes_nothing_and_a_remote_change_publishes_once() {
        use crate::command::JamCmd;
        use crate::jam::{Event as RoomEvent, Member, Remote, Snapshot as Shared};

        let mut state = CoreState::new(Config::default(), None, 1);
        let start = state.snapshot();
        let effects = state.apply(Input::Start, Instant::now());
        assert!(effects.iter().all(|e| !matches!(e, Effect::Jam(_))));
        assert!(state.publish(&start).is_none(), "idle");
        let timers =
            |effects: &[Effect]| effects.iter().filter(|e| matches!(e, Effect::Timer { .. })).count();
        let run = |state: &mut CoreState, input: Input| state.apply(input, Instant::now());

        // Senza jam: uscire, o un evento di una stanza che non c'è, non cambiano niente.
        let effects = run(&mut state, Input::Command(Command::Jam(JamCmd::Leave)));
        assert!(effects.is_empty());
        let effects = run(&mut state, Input::Jam(RoomEvent::Closed("Jam chiusa".into())));
        assert!(effects.is_empty() && state.publish(&start).is_none());

        // Creare la stanza: una pubblicazione, della sola jam; il collegamento parte, nessun timer.
        let effects = run(&mut state, Input::Command(Command::Jam(JamCmd::Create { name: "Luca".into() })));
        assert!(matches!(effects.as_slice(), [Effect::Jam(crate::state::jam::JamOp::Start { .. })]));
        let connecting = state.publish(&start).expect("la jam parte");
        assert_eq!(connecting.versions, DomainVersions { jam: 1, ..DomainVersions::default() });
        assert!(Arc::ptr_eq(&connecting.playback, &start.playback), "il gruppo non è nella vista");

        // La stanza risponde senza stato: una pubblicazione, della sola jam; nessun invio, nessun timer.
        let members = vec![Member { id: uuid::Uuid::from_u128(77), name: "Luca".into() }];
        let joined = RoomEvent::Joined {
            code: "K7QXM3".into(),
            you: uuid::Uuid::from_u128(77),
            members: members.clone(),
            state: None,
        };
        let effects = run(&mut state, Input::Jam(joined));
        assert!(effects.is_empty(), "una jam ferma non ha niente da fare: {effects:?}");
        let open = state.publish(&connecting).expect("in stanza");
        assert_eq!((open.versions.jam, open.versions.playback), (2, 0));
        assert_eq!(open.jam.code.as_deref(), Some("K7QXM3"));

        // Gli stessi membri di nuovo, uno stato uguale a niente: nessuna pubblicazione e nessun effetto.
        let effects = run(&mut state, Input::Jam(RoomEvent::Members(members)));
        assert!(effects.is_empty() && state.publish(&open).is_none(), "la stessa lista non è un cambio");
        assert_eq!(timers(&effects), 0);

        // Uno stato degli altri con la coda vuota: la coda è già vuota, niente da cambiare. Con brani
        // già in coda, invece, è un solo giro: la riproduzione si allinea (una pubblicazione, due
        // domini), e riceverlo di nuovo uguale non rimanda niente alla stanza.
        let empty = Remote {
            snapshot: Shared { queue: vec![], index: 0, position_ms: 0, playing: false },
            from: "Luca".into(),
        };
        run(&mut state, Input::Jam(RoomEvent::State(empty)));
        assert!(state.publish(&open).is_none(), "una coda vuota su una coda vuota");

        let known = [track(1), track(2)];
        command(&mut state, Command::Playback(PlaybackCmd::Enqueue(Box::new(known[0].clone()))));
        command(&mut state, Command::Playback(PlaybackCmd::Enqueue(Box::new(known[1].clone()))));
        let queued = state.publish(&open).expect("la coda è cambiata");
        let remote = Remote {
            snapshot: Shared {
                queue: vec![known[0].id, known[1].id],
                index: 1,
                position_ms: 9_000,
                playing: true,
            },
            from: "Luca".into(),
        };
        let effects = run(&mut state, Input::Jam(RoomEvent::State(remote.clone())));
        assert!(!effects.iter().any(|e| matches!(e, Effect::Jam(_))), "lo stato ricevuto non si rimanda");
        let aligned = state.publish(&queued).expect("la riproduzione si allinea in un giro solo");
        assert_eq!(aligned.versions.playback, queued.versions.playback + 1);
        assert_eq!(aligned.versions.jam, queued.versions.jam + 1, "l'avviso «ha cambiato brano»");
        assert_eq!(aligned.versions.library, queued.versions.library);
        assert_eq!(aligned.versions.social, queued.versions.social);
        assert!(state.publish(&aligned).is_none(), "pubblicato una volta");

        // Lo stesso stato di nuovo (il battito di chi suona): alla riproduzione già allineata non
        // cambia niente e la stanza non riceve niente; nessun timer.
        let effects = run(&mut state, Input::Jam(RoomEvent::State(remote)));
        assert!(!effects.iter().any(|e| matches!(e, Effect::Jam(_) | Effect::Timer { .. })), "{effects:?}");
        assert!(state.publish(&aligned).is_none());

        // Uscire: una pubblicazione, il collegamento si chiude.
        let effects = run(&mut state, Input::Command(Command::Jam(JamCmd::Leave)));
        assert!(matches!(effects.as_slice(), [Effect::Jam(crate::state::jam::JamOp::Leave)]));
        let left = state.publish(&aligned).expect("uscito");
        assert!(!left.jam.is_active());
        assert!(state.publish(&left).is_none());
    }

    /// Il passaggio fra i domini porta il profilo alla jam (il nome in stanza di chi non ne scrive
    /// uno) e la stanza al dominio sociale (gli inviti partono con il suo codice, e quello alla
    /// propria stanza non si annuncia): il frontend non fa da tramite.
    #[test]
    fn the_handoff_gives_the_jam_the_profile_name_and_the_social_domain_the_room() {
        use crate::command::{JamCmd, SocialCmd};
        use crate::jam::{Event as RoomEvent, Member, Target};
        use crate::state::jam::JamOp;

        let mut state = CoreState::new(Config::default(), None, 1);
        adopt(&mut state);

        // Inviti senza stanza: «aspetta che la jam sia pronta».
        let effects = command(&mut state, Command::Social(SocialCmd::InviteToJam(vec!["luca".into()])));
        assert!(social_work(&effects).is_empty());
        assert!(matches!(
            state.social.view.events.back().map(|raised| &raised.event),
            Some(social::SocialEvent::JamNotReady)
        ));

        // Senza un nome scritto, in stanza ci si presenta con il nome del profilo.
        let effects = command(&mut state, Command::Jam(JamCmd::Create { name: String::new() }));
        assert!(matches!(
            effects.as_slice(),
            [Effect::Jam(JamOp::Start { target: Target::Create, name, .. })] if name == "Prova"
        ));
        // Il codice non c'è ancora: gli inviti aspettano.
        let before = state.social.view.events.len();
        command(&mut state, Command::Social(SocialCmd::InviteToJam(vec!["luca".into()])));
        assert_eq!(state.social.view.events.len(), before + 1);

        // La stanza dice il suo codice: gli inviti partono con quello.
        let members = vec![Member { id: uuid::Uuid::from_u128(77), name: "Prova".into() }];
        let joined =
            RoomEvent::Joined { code: "K7QXM3".into(), you: uuid::Uuid::from_u128(77), members, state: None };
        state.apply(Input::Jam(joined), Instant::now());
        let effects = command(&mut state, Command::Social(SocialCmd::InviteToJam(vec!["luca".into()])));
        assert_eq!(
            social_work(&effects),
            [crate::testing::SocialWork::InviteToJam { code: "K7QXM3".into(), handles: vec!["luca".into()] }]
        );
        // L'invito alla propria stanza non si annuncia; uno a un'altra sì.
        let invite = |code: &str| crate::api::social::JamInvite {
            code: code.into(),
            from_handle: "luca".into(),
            from_name: "LUCA".into(),
        };
        let announced = state.social.view.events.len();
        state.apply(
            Input::Done(Done::Social(social::Done::Invites(Ok(vec![invite("K7QXM3")])))),
            Instant::now(),
        );
        assert_eq!(state.social.view.events.len(), announced, "è la mia stanza");
        state.apply(
            Input::Done(Done::Social(social::Done::Invites(Ok(vec![invite("K7QXM3"), invite("ZZZ999")])))),
            Instant::now(),
        );
        assert_eq!(state.social.view.events.len(), announced + 1);

        // Uscita: il dominio sociale dimentica la stanza (gli inviti tornano a dire «non pronta»).
        command(&mut state, Command::Jam(JamCmd::Leave));
        let before = state.social.view.events.len();
        command(&mut state, Command::Social(SocialCmd::InviteToJam(vec!["luca".into()])));
        assert_eq!(state.social.view.events.len(), before + 1);
        assert!(matches!(
            state.social.view.events.back().map(|raised| &raised.event),
            Some(social::SocialEvent::JamNotReady)
        ));
    }

    /// Il backend scelto nelle impostazioni arriva anche al dominio sociale.
    #[test]
    fn the_server_choice_reaches_the_social_domain() {
        let mut state = CoreState::new(Config::default(), None, 1);
        let before = state.social.api_base();
        let mut config = state.settings.view.config.as_ref().clone();
        config.server = Some("https://altro.example".into());
        command(&mut state, Command::Settings(SettingsCmd::Store(Box::new(config))));
        assert_ne!(state.social.api_base(), before);
    }
}
