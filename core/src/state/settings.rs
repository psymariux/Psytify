//! Impostazioni e salvataggio di `config.json` (passo 4.1). Il core è l'unico a scrivere il file: i
//! frontend mandano le modifiche, il dominio le tiene nella sua vista e le scrive al più tardi dopo
//! [`SAVE_DELAY`], una scrittura alla volta e sempre con lo stato più recente. Alla chiusura aspetta
//! la scrittura in corso, scrive subito ciò che manca e solo allora lascia fermare il core.
//!
//! I campi dei domini ancora nel frontend arrivano tutti insieme con [`SettingsCmd::Store`]; un
//! dominio che passa nel core li consegna da sé (la libreria, con [`Settings::store_library`], la
//! riproduzione con [`Settings::store_playback`], il catalogo con [`Settings::store_query`], i
//! download con [`Settings::store_downloads`]) e il frontend smette di mandarli: `Store` non tocca
//! più i campi che un dominio del core possiede.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::config::{self, Config, CoreConfig, DownloadsFields, LibraryFields, PlaybackFields};
use crate::effect::{self, Effect, TimerKey};
use crate::error::AppError;
use crate::system_motion::SystemMotion;

/// Quanto aspetta una modifica prima di finire sul disco: quelle che arrivano nel frattempo vanno
/// nella stessa scrittura. Una modifica nuova non rinvia la scrittura già in attesa.
pub(crate) const SAVE_DELAY: Duration = Duration::from_millis(500);

/// I comandi del dominio.
#[derive(Debug, Clone)]
pub enum SettingsCmd {
    /// I campi del core come li tiene il frontend, finché i loro domini non passano nel core
    /// (passi 4.3-4.9). Si scrivono solo se sono cambiati. I campi della libreria (passo 4.2) li
    /// tiene il suo dominio: quelli di questo valore si ignorano.
    Store(Box<CoreConfig>),
    /// Le preferenze di un frontend: ogni chiave sostituisce quella salvata, `null` la toglie, le
    /// altre restano ([`config::merge_ui`]).
    SaveUiPrefs(Map<String, Value>),
    /// Ascolta (o smette di ascoltare) che cosa il sistema chiede del movimento: il frontend lo
    /// accende quando la sua preferenza segue il sistema. La lettura arriva in
    /// [`SettingsView::system_motion`]; spento, l'host chiude il collegamento e la vista torna a
    /// «nessuna preferenza».
    WatchSystemMotion(bool),
    /// Scrive subito ciò che aspetta [`SAVE_DELAY`], senza fermare il core: il frontend lo manda
    /// quando l'app va in background (Android la può chiudere in ogni momento, BACKLOG 12). Senza
    /// modifiche non fa niente; con una scrittura in corso, la prossima parte appena finisce.
    SaveNow,
}

/// Ciò che i frontend vedono del dominio. Si pubblica solo quando un valore cambia davvero.
#[derive(Debug, Clone, Default)]
pub struct SettingsView {
    /// I campi del core in `config.json`: quelli letti all'avvio, poi quelli dell'ultimo
    /// [`SettingsCmd::Store`]. I domini ancora nel frontend partono da qui.
    pub config: Arc<CoreConfig>,
    /// Le altre chiavi del file: le preferenze dei frontend e quelle che questa versione non conosce.
    pub ui: Arc<Map<String, Value>>,
    /// Perché l'ultima scrittura non è riuscita; `None` dopo una riuscita. Intanto il file resta
    /// quello di prima, intero, e si riprova alla prossima modifica e alla chiusura.
    pub save_error: Option<AppError>,
    /// Che cosa il sistema chiede del movimento (accessibilità, risparmio energetico), finché un
    /// frontend lo ascolta con [`SettingsCmd::WatchSystemMotion`]. Non si salva: è del sistema.
    pub system_motion: SystemMotion,
}

/// I timer del dominio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {
    /// È ora di scrivere le modifiche raccolte.
    Save,
}

/// I lavori asincroni del dominio.
#[derive(Debug)]
pub(crate) enum Job {
    /// Scrive `config.json` in `path` ([`config::write`]), direttamente dalla vista: niente copie.
    Write { path: PathBuf, core: Arc<CoreConfig>, ui: Arc<Map<String, Value>> },
}

/// L'esito di un [`Job`].
#[derive(Debug)]
pub(crate) enum Done {
    Written(Result<(), AppError>),
}

/// A che punto è la chiusura, per il salvataggio.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Closing {
    /// Il core è in funzione.
    #[default]
    No,
    /// Il core si ferma: si aspetta l'esito della scrittura partita prima.
    Requested,
    /// La scrittura in corso è l'ultima: se fallisce, il core si ferma lo stesso e lo dice.
    LastWrite,
}

/// Lo stato del dominio: la vista pubblicata e ciò che resta dentro il core.
#[derive(Debug, Default)]
pub(crate) struct Settings {
    pub(crate) view: Arc<SettingsView>,
    /// Dove sta `config.json`. Senza, la configurazione resta in memoria e non si scrive mai (i
    /// test, un frontend senza cartella dei dati).
    path: Option<PathBuf>,
    /// La vista ha modifiche che il file non ha.
    dirty: bool,
    /// Il timer [`Timer::Save`] è armato.
    scheduled: bool,
    /// Una scrittura è in corso. La prossima parte solo dopo il suo esito: due scritture non si
    /// sovrappongono mai, e mai due usano insieme `config.json.tmp`.
    writing: bool,
    closing: Closing,
    /// Un frontend ascolta il movimento chiesto dal sistema: l'host tiene il collegamento.
    watching_motion: bool,
    /// [`SettingsCmd::SaveNow`] è arrivato durante una scrittura: ciò che resta si scrive al suo
    /// esito, senza aspettare il timer.
    hurry: bool,
}

impl Settings {
    /// Il dominio con la configurazione letta all'avvio, da riscrivere in `path`.
    pub(crate) fn new(config: Config, path: Option<PathBuf>) -> Self {
        let view = SettingsView {
            config: Arc::new(config.core),
            ui: Arc::new(config.ui),
            save_error: None,
            system_motion: SystemMotion::default(),
        };
        Self { view: Arc::new(view), path, ..Self::default() }
    }

    pub(crate) fn apply(&mut self, command: SettingsCmd, _now: Instant, effects: &mut Vec<Effect>) {
        match command {
            SettingsCmd::Store(core) => {
                let mut core = *core;
                core.set_library(self.view.config.library());
                core.set_playback(self.view.config.playback());
                core.set_downloads(self.view.config.downloads());
                core.last_query.clone_from(&self.view.config.last_query);
                self.store(core, effects);
            }
            SettingsCmd::SaveUiPrefs(prefs) => {
                if let Some(ui) = config::merge_ui(&self.view.ui, prefs) {
                    Arc::make_mut(&mut self.view).ui = Arc::new(ui);
                    self.changed(effects);
                }
            }
            SettingsCmd::SaveNow => self.save_now(effects),
            SettingsCmd::WatchSystemMotion(on) => {
                if on == self.watching_motion {
                    return;
                }
                self.watching_motion = on;
                effects.push(Effect::WatchMotion(on));
                if !on && self.view.system_motion != SystemMotion::default() {
                    Arc::make_mut(&mut self.view).system_motion = SystemMotion::default();
                }
            }
        }
    }

    /// Una lettura del sistema. Si pubblica solo se cambia, e solo mentre qualcuno ascolta: una
    /// lettura arrivata dopo lo spegnimento si butta.
    pub(crate) fn system_motion(&mut self, motion: SystemMotion) {
        if self.watching_motion && self.view.system_motion != motion {
            Arc::make_mut(&mut self.view).system_motion = motion;
        }
    }

    /// Sostituisce i campi del core, se sono cambiati: uguali, non si pubblica e non si scrive.
    pub(crate) fn store(&mut self, core: CoreConfig, effects: &mut Vec<Effect>) {
        if *self.view.config != core {
            Arc::make_mut(&mut self.view).config = Arc::new(core);
            self.changed(effects);
        }
    }

    /// Sostituisce i campi della libreria, se sono cambiati. Il confronto guarda i puntatori delle
    /// liste (`Shared`), non gli elementi, e l'`Arc` della configurazione si copia solo allora.
    pub(crate) fn store_library(&mut self, fields: LibraryFields, effects: &mut Vec<Effect>) {
        if self.view.config.library() != fields {
            Arc::make_mut(&mut Arc::make_mut(&mut self.view).config).set_library(fields);
            self.changed(effects);
        }
    }

    /// Sostituisce i campi dei download, se sono cambiati.
    pub(crate) fn store_downloads(&mut self, fields: DownloadsFields, effects: &mut Vec<Effect>) {
        if self.view.config.downloads() != fields {
            Arc::make_mut(&mut Arc::make_mut(&mut self.view).config).set_downloads(fields);
            self.changed(effects);
        }
    }

    /// Sostituisce il testo cercato per ultimo, se è cambiato.
    pub(crate) fn store_query(&mut self, query: String, effects: &mut Vec<Effect>) {
        if self.view.config.last_query != query {
            Arc::make_mut(&mut Arc::make_mut(&mut self.view).config).last_query = query;
            self.changed(effects);
        }
    }

    /// Sostituisce i campi della riproduzione, se sono cambiati (la coda si confronta per puntatore).
    pub(crate) fn store_playback(&mut self, fields: PlaybackFields, effects: &mut Vec<Effect>) {
        if self.view.config.playback() != fields {
            Arc::make_mut(&mut Arc::make_mut(&mut self.view).config).set_playback(fields);
            self.changed(effects);
        }
    }

    pub(crate) fn fire(&mut self, timer: Timer, _now: Instant, effects: &mut Vec<Effect>) {
        match timer {
            Timer::Save => {
                self.scheduled = false;
                self.write(effects);
            }
        }
    }

    pub(crate) fn done(&mut self, done: Done, _now: Instant, effects: &mut Vec<Effect>) {
        match done {
            Done::Written(result) => {
                self.writing = false;
                if result.is_err() {
                    // Il file non ha queste modifiche: si riprova alla prossima o alla chiusura, mai
                    // da soli a vuoto mentre l'app è ferma.
                    self.dirty = true;
                }
                let error = result.as_ref().err();
                if self.view.save_error.as_ref() != error {
                    Arc::make_mut(&mut self.view).save_error = error.cloned();
                }
                match self.closing {
                    // Chiesta subito durante la scrittura: si scrive ora ciò che è cambiato intanto.
                    Closing::No if std::mem::take(&mut self.hurry) && self.dirty && result.is_ok() => {
                        self.write(effects);
                    }
                    Closing::No if self.dirty && result.is_ok() => self.schedule(effects),
                    Closing::No => {}
                    Closing::LastWrite if result.is_err() => effects.push(Effect::Flushed(result)),
                    Closing::Requested | Closing::LastWrite => self.flush(effects),
                }
            }
        }
    }

    /// Il core si ferma: niente più attese. Si scrive subito ciò che manca, dopo l'esito della
    /// scrittura in corso se ce n'è una; poi [`Effect::Flushed`] lascia fermare il core.
    pub(crate) fn close(&mut self, effects: &mut Vec<Effect>) {
        if std::mem::take(&mut self.scheduled) {
            effects.push(Effect::CancelTimer(TimerKey::Settings(Timer::Save)));
        }
        self.closing = Closing::Requested;
        if !self.writing {
            self.flush(effects);
        }
    }

    /// Scrive ora ciò che aspetta il timer: il timer non serve più. Con una scrittura in corso, lo fa
    /// [`Settings::done`] al suo esito. Durante la chiusura ci pensa già [`Settings::close`].
    fn save_now(&mut self, effects: &mut Vec<Effect>) {
        if self.closing != Closing::No || !self.dirty {
            return;
        }
        if std::mem::take(&mut self.scheduled) {
            effects.push(Effect::CancelTimer(TimerKey::Settings(Timer::Save)));
        }
        if self.writing {
            self.hurry = true;
        } else {
            self.write(effects);
        }
    }

    /// Alla chiusura, senza scritture in corso: l'ultima scrittura, se manca qualcosa, o la fine.
    fn flush(&mut self, effects: &mut Vec<Effect>) {
        if self.write(effects) {
            self.closing = Closing::LastWrite;
        } else {
            effects.push(Effect::Flushed(Ok(())));
        }
    }

    /// La vista è cambiata: il file va riscritto.
    fn changed(&mut self, effects: &mut Vec<Effect>) {
        if self.path.is_some() {
            self.dirty = true;
            self.schedule(effects);
        }
    }

    /// Arma il timer, se non è già armato e nessuna scrittura è in corso: in quel caso lo riarma
    /// [`Settings::done`], se nel frattempo è cambiato qualcosa. Durante la chiusura non serve.
    fn schedule(&mut self, effects: &mut Vec<Effect>) {
        if self.scheduled || self.writing || self.closing != Closing::No {
            return;
        }
        self.scheduled = true;
        effects.push(Effect::Timer { key: TimerKey::Settings(Timer::Save), after: SAVE_DELAY });
    }

    /// Scrive ora, se c'è qualcosa da scrivere e nessuna scrittura è in corso. Dice se è partita.
    fn write(&mut self, effects: &mut Vec<Effect>) -> bool {
        let Some(path) = &self.path else { return false };
        if self.writing || !self.dirty {
            return false;
        }
        self.dirty = false;
        self.writing = true;
        let job = Job::Write {
            path: path.clone(),
            core: Arc::clone(&self.view.config),
            ui: Arc::clone(&self.view.ui),
        };
        effects.push(Effect::Run(effect::Job::Settings(job)));
        true
    }
}

impl Job {
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Write { path, core, ui } => Done::Written(config::write(&path, &core, &ui).await),
        }
    }

    /// Se il lavoro va perso, il file non è stato scritto.
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Write { path, .. } => Done::Written(Err(AppError::Io(path.display().to_string()))),
        }
    }
}

/// I passi 0 di questo dominio (`saves_do_not_overlap_and_the_last_state_wins`,
/// `close_flushes_before_exit`), portati sul riduttore: il tempo è finto, i timer scadono quando lo
/// dice il test e le scritture finiscono quando il test consegna il loro esito.
#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings::new(Config::default(), Some(PathBuf::from("/dati/psytify/config.json")))
    }

    fn query(text: &str) -> SettingsCmd {
        SettingsCmd::Store(Box::new(CoreConfig { clerk_client_id: text.to_owned(), ..CoreConfig::default() }))
    }

    fn prefs(value: Value) -> SettingsCmd {
        match value {
            Value::Object(map) => SettingsCmd::SaveUiPrefs(map),
            other => panic!("not an object: {other}"),
        }
    }

    fn send(settings: &mut Settings, commands: impl IntoIterator<Item = SettingsCmd>) -> Vec<Effect> {
        let mut effects = Vec::new();
        for command in commands {
            settings.apply(command, Instant::now(), &mut effects);
        }
        effects
    }

    /// Il movimento del sistema: l'ascolto si accende e si spegne una volta sola, una lettura cambia
    /// la vista solo se è diversa e solo mentre si ascolta, e niente di tutto questo si salva.
    #[test]
    fn the_system_motion_is_published_only_on_a_real_change_while_watched() {
        let reduced = SystemMotion { reduced_motion: Some(false), power_saver: Some(true) };
        let mut settings = settings();
        let start = Arc::clone(&settings.view);

        settings.system_motion(reduced);
        assert!(Arc::ptr_eq(&start, &settings.view), "nessuno ascolta: la lettura si butta");

        let effects =
            send(&mut settings, [SettingsCmd::WatchSystemMotion(true), SettingsCmd::WatchSystemMotion(true)]);
        assert!(matches!(effects[..], [Effect::WatchMotion(true)]), "{effects:?}");
        assert!(Arc::ptr_eq(&start, &settings.view), "accendere non cambia la vista");

        settings.system_motion(reduced);
        assert_eq!(settings.view.system_motion, reduced);
        let published = Arc::clone(&settings.view);
        settings.system_motion(reduced);
        assert!(Arc::ptr_eq(&published, &settings.view), "la stessa lettura non si ripubblica");
        assert!(!settings.dirty && !settings.scheduled, "il movimento del sistema non si salva");

        let effects = send(&mut settings, [SettingsCmd::WatchSystemMotion(false)]);
        assert!(matches!(effects[..], [Effect::WatchMotion(false)]), "{effects:?}");
        assert_eq!(settings.view.system_motion, SystemMotion::default(), "spento: nessuna preferenza");
        settings.system_motion(reduced);
        assert_eq!(settings.view.system_motion, SystemMotion::default(), "una lettura in ritardo");
    }

    fn fire(settings: &mut Settings) -> Vec<Effect> {
        let mut effects = Vec::new();
        settings.fire(Timer::Save, Instant::now(), &mut effects);
        effects
    }

    fn written(settings: &mut Settings, result: Result<(), AppError>) -> Vec<Effect> {
        let mut effects = Vec::new();
        settings.done(Done::Written(result), Instant::now(), &mut effects);
        effects
    }

    fn close(settings: &mut Settings) -> Vec<Effect> {
        let mut effects = Vec::new();
        settings.close(&mut effects);
        effects
    }

    fn io_error() -> AppError {
        AppError::Io("/dati/psytify/config.json".into())
    }

    /// Le ultime ricerche che gli effetti scrivono su disco, nell'ordine.
    fn writes(effects: &[Effect]) -> Vec<String> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Run(effect::Job::Settings(Job::Write { core, .. })) => {
                    Some(core.clerk_client_id.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn arms_the_timer(effects: &[Effect]) -> bool {
        matches!(effects, [Effect::Timer { key: TimerKey::Settings(Timer::Save), after }] if *after == SAVE_DELAY)
    }

    #[test]
    fn changes_are_written_once_after_the_delay_with_the_last_state() {
        let mut settings = settings();
        assert!(arms_the_timer(&send(&mut settings, [query("a")])));
        // Altre modifiche prima della scadenza: nessun timer nuovo, la scrittura non si rinvia.
        assert!(send(&mut settings, [query("b"), query("c")]).is_empty());
        assert_eq!(writes(&fire(&mut settings)), ["c"]);
        assert!(written(&mut settings, Ok(())).is_empty(), "nothing left to write");
        assert_eq!(settings.view.config.clerk_client_id, "c");
    }

    #[test]
    fn an_unchanged_value_publishes_and_writes_nothing() {
        let mut settings = settings();
        let published = Arc::clone(&settings.view);
        let same = [SettingsCmd::Store(Box::default()), prefs(serde_json::json!({}))];
        assert!(send(&mut settings, same).is_empty());
        assert!(Arc::ptr_eq(&published, &settings.view), "same pointer: nothing to publish");

        assert!(arms_the_timer(&send(&mut settings, [prefs(serde_json::json!({"panel_open": true}))])));
        let published = Arc::clone(&settings.view);
        assert!(
            send(&mut settings, [prefs(serde_json::json!({"panel_open": true, "queue_open": null}))])
                .is_empty()
        );
        assert!(Arc::ptr_eq(&published, &settings.view));
    }

    #[test]
    fn a_change_during_a_write_waits_for_it_and_the_last_state_wins() {
        let mut settings = settings();
        send(&mut settings, [query("a")]);
        assert_eq!(writes(&fire(&mut settings)), ["a"]);
        // La scrittura è in volo: niente timer e niente seconda scrittura accanto alla prima.
        assert!(send(&mut settings, [query("b"), query("c")]).is_empty());
        assert!(fire(&mut settings).is_empty(), "a stray timer starts no second write");
        // Arrivato il suo esito, si riparte con lo stato di adesso.
        assert!(arms_the_timer(&written(&mut settings, Ok(()))));
        assert_eq!(writes(&fire(&mut settings)), ["c"]);
        assert!(written(&mut settings, Ok(())).is_empty());
    }

    /// BACKLOG 12: l'app va in background e Android la può chiudere. `SaveNow` scrive subito ciò che
    /// aspetta il timer (e lo spegne), non fa niente senza modifiche, e durante una scrittura fa
    /// partire la prossima al suo esito, senza il timer.
    #[test]
    fn save_now_writes_what_waits_without_the_delay() {
        let mut settings = settings();
        assert!(send(&mut settings, [SettingsCmd::SaveNow]).is_empty(), "niente da scrivere");

        send(&mut settings, [query("a")]);
        let effects = send(&mut settings, [SettingsCmd::SaveNow]);
        assert!(
            matches!(effects.first(), Some(Effect::CancelTimer(TimerKey::Settings(Timer::Save)))),
            "{effects:?}"
        );
        assert_eq!(writes(&effects), ["a"]);
        assert!(fire(&mut settings).is_empty(), "il timer spento non scrive di nuovo");

        // Una modifica e `SaveNow` mentre la scrittura è in volo: la prossima parte al suo esito.
        assert!(send(&mut settings, [query("b"), SettingsCmd::SaveNow]).is_empty());
        assert_eq!(writes(&written(&mut settings, Ok(()))), ["b"], "subito, non dopo il timer");
        assert!(written(&mut settings, Ok(())).is_empty());
        assert!(send(&mut settings, [SettingsCmd::SaveNow]).is_empty(), "tutto scritto");

        // Dopo, le modifiche tornano ad aspettare il timer.
        assert!(arms_the_timer(&send(&mut settings, [query("c")])));
    }

    #[test]
    fn a_failed_write_is_shown_and_retried_with_the_next_change() {
        let mut settings = settings();
        send(&mut settings, [query("a")]);
        fire(&mut settings);
        assert!(written(&mut settings, Err(io_error())).is_empty(), "no retry loop while idle");
        assert_eq!(settings.view.save_error, Some(io_error()));

        assert!(arms_the_timer(&send(&mut settings, [query("b")])));
        assert_eq!(writes(&fire(&mut settings)), ["b"]);
        written(&mut settings, Ok(()));
        assert_eq!(settings.view.save_error, None);
    }

    #[test]
    fn ui_prefs_merge_into_the_view_and_go_with_the_core_fields() {
        let mut config = Config::default();
        config.ui.insert("gpui_theme".into(), "dark".into());
        let mut settings = Settings::new(config, Some(PathBuf::from("/dati/psytify/config.json")));
        send(
            &mut settings,
            [prefs(serde_json::json!({"panel_open": true, "clerk_client_id": "z"})), query("d")],
        );
        let effects = fire(&mut settings);
        let [Effect::Run(effect::Job::Settings(Job::Write { core, ui, .. }))] = effects.as_slice() else {
            panic!("one write: {effects:?}");
        };
        assert_eq!(
            Value::Object((**ui).clone()),
            serde_json::json!({"gpui_theme": "dark", "panel_open": true})
        );
        assert_eq!(core.clerk_client_id, "d", "a ui key never overrides a core field");
    }

    #[test]
    fn close_cancels_the_wait_and_writes_at_once() {
        let mut settings = settings();
        send(&mut settings, [query("a")]);
        let effects = close(&mut settings);
        assert!(matches!(effects.first(), Some(Effect::CancelTimer(TimerKey::Settings(Timer::Save)))));
        assert_eq!(writes(&effects), ["a"]);
        assert!(matches!(written(&mut settings, Ok(())).as_slice(), [Effect::Flushed(Ok(()))]));
    }

    /// `docs/transformation/BACKLOG.md`, 7: la chiusura non scrive accanto a una scrittura in corso.
    #[test]
    fn close_waits_for_the_write_in_flight_then_writes_the_rest() {
        let mut settings = settings();
        send(&mut settings, [query("a")]);
        fire(&mut settings);
        send(&mut settings, [query("b")]);
        assert!(close(&mut settings).is_empty(), "the first write is still in flight");
        assert_eq!(writes(&written(&mut settings, Ok(()))), ["b"]);
        assert!(matches!(written(&mut settings, Ok(())).as_slice(), [Effect::Flushed(Ok(()))]));
    }

    #[test]
    fn close_with_nothing_to_write_stops_at_once() {
        let mut settings = settings();
        assert!(matches!(close(&mut settings).as_slice(), [Effect::Flushed(Ok(()))]));
    }

    #[test]
    fn a_failed_last_write_is_reported_and_the_core_stops_anyway() {
        let mut settings = settings();
        send(&mut settings, [query("a")]);
        fire(&mut settings);
        close(&mut settings);
        // Fallisce la scrittura partita prima della chiusura: se ne tenta un'ultima.
        assert_eq!(writes(&written(&mut settings, Err(io_error()))), ["a"]);
        // Fallisce anche l'ultima: il core si ferma e dice perché.
        let effects = written(&mut settings, Err(io_error()));
        assert!(
            matches!(effects.as_slice(), [Effect::Flushed(Err(err))] if *err == io_error()),
            "{effects:?}"
        );
    }

    #[test]
    fn without_a_path_nothing_is_ever_written() {
        let mut settings = Settings::new(Config::default(), None);
        assert!(send(&mut settings, [query("a")]).is_empty());
        assert_eq!(settings.view.config.clerk_client_id, "a", "the view still changes");
        assert!(matches!(close(&mut settings).as_slice(), [Effect::Flushed(Ok(()))]));
    }
}
