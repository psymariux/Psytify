//! Il ponte fra il core e GPUI: un'entità che possiede il `Core`, tiene l'ultima istantanea e dice alle
//! viste quali domini sono cambiati.
//!
//! Il core gira sul suo thread, con un runtime proprio da due thread (`RuntimeChoice::Own`): GPUI ha il
//! suo executor e non usa tokio. Le istantanee arrivano su un `tokio::sync::watch`, che non dipende dal
//! runtime, quindi basta un task di GPUI che aspetta `changed()`: nessun ciclo di sondaggio, nessun
//! risveglio se il core non pubblica. Il core pubblica solo quando una vista cambia davvero, e
//! `CoreModel::apply` confronta le versioni dei domini: una vista viene notificata solo per il suo
//! dominio (l'istantanea contiene un `Arc` per dominio, la posizione del brano non c'è, vedi
//! `PlaybackView::position`).

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Task};
use psytify_core::artwork::CoverPixels;
use psytify_core::command::{
    CatalogCmd, DownloadsCmd, ExternalCmd, JamCmd, LibraryCmd, PlaybackCmd, SettingsCmd, SocialCmd,
};
use psytify_core::config;
use psytify_core::snapshot::{DomainVersions, JamView, SocialView, UpdateView};
use psytify_core::{
    AudioChoice, Command, Core, CoreError, CoreOptions, RuntimeChoice, Snapshot, SystemChoice,
};
use tokio::sync::oneshot;
use uuid::Uuid;

/// Thread del runtime proprio del core (rete, copertine, download): il piano ne prevede due.
const CORE_WORKERS: usize = 2;

/// Quali domini sono cambiati nell'ultima pubblicazione del core: uno per ogni versione di
/// `DomainVersions`, così una vista si sveglia solo per il dominio che legge. `video` c'è solo con la
/// feature `video` del desktop (il core la compila allora; senza, il dominio non esiste).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    pub settings: bool,
    pub library: bool,
    pub playback: bool,
    pub catalog: bool,
    pub external: bool,
    pub downloads: bool,
    pub social: bool,
    pub jam: bool,
    #[cfg(feature = "video")]
    pub video: bool,
}

impl Changed {
    /// Confronta le versioni di due istantanee: chi non è cambiato resta `false`.
    pub fn between(old: &DomainVersions, new: &DomainVersions) -> Self {
        Self {
            settings: old.settings != new.settings,
            library: old.library != new.library,
            playback: old.playback != new.playback,
            catalog: old.catalog != new.catalog,
            external: old.external != new.external,
            downloads: old.downloads != new.downloads,
            social: old.social != new.social,
            jam: old.jam != new.jam,
            #[cfg(feature = "video")]
            video: old.video != new.video,
        }
    }

    /// Almeno un dominio è cambiato.
    pub fn any(self) -> bool {
        let any = self.settings
            || self.library
            || self.playback
            || self.catalog
            || self.external
            || self.downloads
            || self.social
            || self.jam;
        #[cfg(feature = "video")]
        let any = any || self.video;
        any
    }
}

pub struct CoreModel {
    /// `None` se il core non è partito o si sta fermando: i comandi si perdono.
    core: Option<Core>,
    snapshot: Arc<Snapshot>,
    /// Gli id dei brani piaciuti, rifatti solo quando la libreria cambia (la view non li conta).
    liked: Arc<HashSet<Uuid>>,
    start_error: Option<String>,
    /// Con la feature `fixtures` e `--fixture`: la vista sociale di prova mostrata al posto di quella del
    /// core (account, amici, profili), per le schermate. I comandi sociali allora non partono.
    #[cfg(feature = "fixtures")]
    social_fixture: Option<Arc<SocialView>>,
    /// Con la feature `fixtures`: la vista della jam di prova (in connessione, in stanza…). I comandi
    /// della jam allora non partono: nessuna stanza vera si apre.
    #[cfg(feature = "fixtures")]
    jam_fixture: Option<Arc<JamView>>,
    /// Con la feature `fixtures`: l'aggiornamento di prova (offerto, in download, pronto, controllo non
    /// riuscito). I comandi dell'aggiornamento allora non partono: nessun pacchetto si scarica davvero.
    #[cfg(feature = "fixtures")]
    update_fixture: Option<UpdateView>,
    /// La vista del videoclip come la legge il desktop, rifatta solo quando il dominio cambia.
    #[cfg(feature = "video")]
    clip: Arc<crate::video::ClipView>,
    /// Con la feature `fixtures` e `--fixture video-*`: il videoclip di prova (in preparazione, pronto su un
    /// file locale, non riuscito). I comandi del videoclip allora non partono: niente si chiede al server.
    #[cfg(all(feature = "video", feature = "fixtures"))]
    clip_fixture: Option<Arc<crate::video::ClipView>>,
    /// Il task che aspetta le istantanee: finisce con il modello.
    _pump: Option<Task<()>>,
}

impl EventEmitter<Changed> for CoreModel {}

impl CoreModel {
    /// Avvia il core (stessa cartella dei dati e stessa configurazione del client Iced: l'unico che
    /// scrive `config.json` è il core, vedi `core/src/state/settings.rs`) e ne ascolta le istantanee.
    pub fn start(cx: &mut App) -> Entity<Self> {
        let options = CoreOptions {
            runtime: RuntimeChoice::Own { workers: CORE_WORKERS },
            audio: AudioChoice::System,
            system: SystemChoice::Desktop,
            data_dir: config::dir(),
            downloads_dir: Some(config::downloads_dir()),
        };
        let started = Core::start(options);
        cx.new(|cx: &mut Context<Self>| match started {
            Ok(core) => {
                let snapshot = core.snapshot();
                let mut changes = core.changes();
                let pump = cx.spawn(async move |this, cx| {
                    while changes.changed().await.is_ok() {
                        let next = Arc::clone(&changes.borrow_and_update());
                        if this.update(cx, |model, cx| model.apply(next, cx)).is_err() {
                            break;
                        }
                    }
                });
                let liked = Arc::new(snapshot.library.liked.iter().map(|track| track.id).collect());
                Self {
                    core: Some(core),
                    #[cfg(feature = "video")]
                    clip: Arc::new(crate::video::ClipView::of(&snapshot.video)),
                    #[cfg(all(feature = "video", feature = "fixtures"))]
                    clip_fixture: None,
                    snapshot,
                    liked,
                    start_error: None,
                    #[cfg(feature = "fixtures")]
                    social_fixture: None,
                    #[cfg(feature = "fixtures")]
                    jam_fixture: None,
                    #[cfg(feature = "fixtures")]
                    update_fixture: None,
                    _pump: Some(pump),
                }
            }
            Err(err) => Self {
                core: None,
                #[cfg(feature = "video")]
                clip: Arc::default(),
                #[cfg(all(feature = "video", feature = "fixtures"))]
                clip_fixture: None,
                snapshot: Arc::default(),
                liked: Arc::default(),
                start_error: Some(err.to_string()),
                #[cfg(feature = "fixtures")]
                social_fixture: None,
                #[cfg(feature = "fixtures")]
                jam_fixture: None,
                #[cfg(feature = "fixtures")]
                update_fixture: None,
                _pump: None,
            },
        })
    }

    /// Prende l'istantanea nuova e dice quali domini sono cambiati. Una più vecchia non sostituisce
    /// mai una più nuova.
    fn apply(&mut self, next: Arc<Snapshot>, cx: &mut Context<Self>) {
        if next.version <= self.snapshot.version {
            return;
        }
        let changed = Changed::between(&self.snapshot.versions, &next.versions);
        if changed.library {
            self.liked = Arc::new(next.library.liked.iter().map(|track| track.id).collect());
        }
        #[cfg(feature = "video")]
        if changed.video {
            self.clip = Arc::new(crate::video::ClipView::of(&next.video));
        }
        self.snapshot = next;
        if changed.any() {
            cx.emit(changed);
        }
    }

    /// L'ultima istantanea (una copia di un `Arc`).
    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.snapshot)
    }

    /// La vista sociale (account, accesso, amici, profili): quella dell'istantanea; con i dati di prova di
    /// `--fixture` (feature `fixtures`), quella di prova. Le viste la leggono da qui, mai dall'istantanea.
    pub fn social_view(&self) -> Arc<SocialView> {
        #[cfg(feature = "fixtures")]
        if let Some(fixture) = &self.social_fixture {
            return Arc::clone(fixture);
        }
        Arc::clone(&self.snapshot.social)
    }

    /// Mostra una vista sociale di prova al posto di quella del core (solo per le schermate).
    #[cfg(feature = "fixtures")]
    pub fn set_social_fixture(&mut self, view: Arc<SocialView>) {
        self.social_fixture = Some(view);
    }

    /// La vista della jam: quella dell'istantanea; con i dati di prova di `--fixture`, quella di prova. Le
    /// viste la leggono da qui, mai dall'istantanea.
    pub fn jam_view(&self) -> Arc<JamView> {
        #[cfg(feature = "fixtures")]
        if let Some(fixture) = &self.jam_fixture {
            return Arc::clone(fixture);
        }
        Arc::clone(&self.snapshot.jam)
    }

    /// Mostra una vista della jam di prova al posto di quella del core (solo per le schermate).
    #[cfg(feature = "fixtures")]
    pub fn set_jam_fixture(&mut self, view: Arc<JamView>) {
        self.jam_fixture = Some(view);
    }

    /// L'aggiornamento dell'app: quello dell'istantanea; con i dati di prova di `--fixture`, quello di prova.
    /// La fascia e le Impostazioni lo leggono da qui, mai dall'istantanea.
    pub fn update_view(&self) -> &UpdateView {
        #[cfg(feature = "fixtures")]
        if let Some(fixture) = &self.update_fixture {
            return fixture;
        }
        &self.snapshot.downloads.update
    }

    /// Mostra un aggiornamento di prova al posto di quello del core (solo per le schermate).
    #[cfg(feature = "fixtures")]
    pub fn set_update_fixture(&mut self, view: UpdateView) {
        self.update_fixture = Some(view);
    }

    /// Il videoclip del brano in corso (che cosa aprire): quello del core; con i dati di prova di `--fixture
    /// video-*`, quello di prova. Il palco del video lo legge da qui, mai dall'istantanea.
    #[cfg(feature = "video")]
    pub fn clip_view(&self) -> Arc<crate::video::ClipView> {
        #[cfg(feature = "fixtures")]
        if let Some(fixture) = &self.clip_fixture {
            return Arc::clone(fixture);
        }
        Arc::clone(&self.clip)
    }

    /// Mostra un videoclip di prova al posto di quello del core (solo per le schermate).
    #[cfg(all(feature = "video", feature = "fixtures"))]
    pub fn set_clip_fixture(&mut self, view: crate::video::ClipView) {
        self.clip_fixture = Some(Arc::new(view));
    }

    /// Un comando al dominio del videoclip. Con un videoclip di prova non parte niente.
    #[cfg(feature = "video")]
    pub fn video(&self, command: psytify_core::command::VideoCmd) {
        #[cfg(feature = "fixtures")]
        if self.clip_fixture.is_some() {
            return;
        }
        self.send(Command::Video(command));
    }

    /// Gli id dei brani piaciuti.
    pub fn liked(&self) -> Arc<HashSet<Uuid>> {
        Arc::clone(&self.liked)
    }

    /// Perché il core non è partito, se non è partito.
    pub fn start_error(&self) -> Option<&str> {
        self.start_error.as_deref()
    }

    pub fn send(&self, command: Command) {
        if let Some(core) = &self.core {
            core.send(command);
        }
    }

    pub fn playback(&self, command: PlaybackCmd) {
        self.send(Command::Playback(command));
    }

    pub fn library(&self, command: LibraryCmd) {
        self.send(Command::library(command));
    }

    pub fn catalog(&self, command: CatalogCmd) {
        self.send(Command::Catalog(command));
    }

    pub fn downloads(&self, command: DownloadsCmd) {
        // Con un aggiornamento di prova niente dell'aggiornamento va al core: nessun controllo, nessun
        // pacchetto scaricato. La chiusura della fascia la fa `dismiss_update`.
        #[cfg(feature = "fixtures")]
        if self.update_fixture.is_some()
            && matches!(
                command,
                DownloadsCmd::CheckUpdate(_)
                    | DownloadsCmd::DownloadUpdate
                    | DownloadsCmd::DismissUpdate
                    | DownloadsCmd::RevealUpdate
            )
        {
            return;
        }
        self.send(Command::Downloads(command));
    }

    /// Con un aggiornamento di prova: la chiusura della fascia toglie l'offerta o il file pronto, come fa il
    /// core, e lo dice alle viste come un cambio dei download. Senza dati di prova manda il comando.
    pub fn dismiss_update(&mut self, cx: &mut Context<Self>) {
        #[cfg(feature = "fixtures")]
        if let Some(fixture) = &mut self.update_fixture {
            fixture.state = None;
            cx.emit(Changed { downloads: true, ..Changed::default() });
            return;
        }
        let _ = cx;
        self.downloads(DownloadsCmd::DismissUpdate);
    }

    pub fn external(&self, command: ExternalCmd) {
        self.send(Command::External(command));
    }

    pub fn settings(&self, command: SettingsCmd) {
        self.send(Command::Settings(command));
    }

    pub fn social(&self, command: SocialCmd) {
        // Con i dati di prova niente va al core: nessun accesso, nessuna richiesta a nome di nessuno.
        #[cfg(feature = "fixtures")]
        if self.social_fixture.is_some() {
            return;
        }
        self.send(Command::Social(command));
    }

    pub fn jam(&self, command: JamCmd) {
        // Con i dati di prova niente va al core: nessuna stanza si apre, nessuno entra in quella di altri.
        #[cfg(feature = "fixtures")]
        if self.jam_fixture.is_some() || self.social_fixture.is_some() {
            return;
        }
        self.send(Command::Jam(command));
    }

    /// Chiede al core una copertina (scaricata e decodificata sul suo runtime). `None` se il core non
    /// c'è più.
    pub fn artwork(&self, url: String) -> Option<oneshot::Receiver<Option<CoverPixels>>> {
        self.core.as_ref().map(|core| core.artwork(url))
    }

    /// Ferma il core dopo che ha scritto `config.json`: il ricevitore si risolve a fine lavoro. Da qui
    /// in poi i comandi si perdono. `None` se il core non c'era o è già stato fermato.
    pub fn begin_shutdown(&mut self) -> Option<oneshot::Receiver<Result<(), CoreError>>> {
        self._pump = None;
        self.core.take().map(Core::shutdown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una versione per dominio, ognuna con un numero diverso: un campo scambiato per un altro si vede.
    /// Il campo `video` del core esiste anche quando un'altra crate del workspace accende la feature
    /// del core senza che il desktop accenda la sua: `..default()` lo riempie in quel caso.
    #[allow(clippy::needless_update)]
    fn versions() -> DomainVersions {
        DomainVersions {
            settings: 1,
            library: 2,
            playback: 3,
            catalog: 4,
            external: 5,
            downloads: 6,
            social: 7,
            jam: 8,
            #[cfg(feature = "video")]
            video: 9,
            ..DomainVersions::default()
        }
    }

    #[test]
    fn nothing_changed_between_equal_versions() {
        let same = versions();
        let changed = Changed::between(&same, &same);
        assert_eq!(changed, Changed::default());
        assert!(!changed.any());
    }

    #[test]
    fn each_domain_is_reported_alone_and_wakes_the_models_listeners() {
        let old = versions();
        let cases: [(DomainVersions, Changed); 8] = [
            (DomainVersions { settings: 10, ..old }, Changed { settings: true, ..Changed::default() }),
            (DomainVersions { library: 10, ..old }, Changed { library: true, ..Changed::default() }),
            (DomainVersions { playback: 10, ..old }, Changed { playback: true, ..Changed::default() }),
            (DomainVersions { catalog: 10, ..old }, Changed { catalog: true, ..Changed::default() }),
            (DomainVersions { external: 10, ..old }, Changed { external: true, ..Changed::default() }),
            (DomainVersions { downloads: 10, ..old }, Changed { downloads: true, ..Changed::default() }),
            (DomainVersions { social: 10, ..old }, Changed { social: true, ..Changed::default() }),
            (DomainVersions { jam: 10, ..old }, Changed { jam: true, ..Changed::default() }),
        ];
        for (new, expected) in cases {
            let changed = Changed::between(&old, &new);
            assert_eq!(changed, expected);
            assert!(changed.any(), "{expected:?} deve svegliare chi ascolta");
        }
    }

    #[test]
    fn several_domains_in_one_publication_are_all_reported() {
        let old = versions();
        let new = DomainVersions { external: 6, social: 8, jam: 9, ..old };
        let changed = Changed::between(&old, &new);
        assert!(changed.external && changed.social && changed.jam);
        assert!(!changed.settings && !changed.library && !changed.playback);
        assert!(!changed.catalog && !changed.downloads);
    }

    #[cfg(feature = "video")]
    #[test]
    fn the_video_domain_is_reported_with_the_video_feature() {
        let old = versions();
        let changed = Changed::between(&old, &DomainVersions { video: 10, ..old });
        assert_eq!(changed, Changed { video: true, ..Changed::default() });
        assert!(changed.any());
    }
}
