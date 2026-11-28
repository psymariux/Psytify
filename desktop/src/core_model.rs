//! The bridge between the core and GPUI: an entity that owns the `Core`, keeps the last snapshot and tells the
//! views which domains have changed.
//!
//! The core runs on its own thread, with a runtime of its own of two threads (`RuntimeChoice::Own`): GPUI has
//! its own executor, not tokio. The snapshots arrive on a `tokio::sync::watch`, which does not depend on the
//! runtime, so a GPUI task waiting on `changed()` is enough: no polling loop, no
//! wake-up if the core does not publish. The core publishes only when a view really changes, and
//! `CoreModel::apply` compares the domains' versions: a view is notified only for its own
//! domain (the snapshot holds an `Arc` per domain; the track's position is not there, see
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

/// The core's own runtime thread (network, covers, downloads): the plan foresees two.
const CORE_WORKERS: usize = 2;

/// Which domains changed in the core's last publication: one for each version of
/// `DomainVersions`, so a view wakes only for the domain it reads. `video` exists only with the
/// desktop's `video` feature (the core compiles it then; without it, the domain does not exist).
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
    /// Compares the versions of two snapshots: whoever has not changed stays `false`.
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

    /// At least one domain has changed.
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
    /// `None` if the core has not started or is stopping: the commands are lost.
    core: Option<Core>,
    snapshot: Arc<Snapshot>,
    /// The liked tracks' ids, rebuilt only when the library changes (the view does not count them).
    liked: Arc<HashSet<Uuid>>,
    start_error: Option<String>,
    /// Con la feature `fixtures` e `--fixture`: la vista sociale di prova mostrata al posto di quella del
    /// core (account, friends, profiles), for the screens. The social commands do not start then.
    #[cfg(feature = "fixtures")]
    social_fixture: Option<Arc<SocialView>>,
    /// With the `fixtures` feature: the test jam view (connecting, in a room…). The jam
    /// commands do not start then: no real room is opened.
    #[cfg(feature = "fixtures")]
    jam_fixture: Option<Arc<JamView>>,
    /// With the `fixtures` feature: the test update (offered, downloading, ready, check
    /// failed). The update commands do not start then: no package is really downloaded.
    #[cfg(feature = "fixtures")]
    update_fixture: Option<UpdateView>,
    /// The video view as the desktop reads it, rebuilt only when the domain changes.
    #[cfg(feature = "video")]
    clip: Arc<crate::video::ClipView>,
    /// With the `fixtures` feature and `--fixture video-*`: the test video (preparing, ready on a
    /// local file, failed). The video commands do not start then: nothing is asked of the server.
    #[cfg(all(feature = "video", feature = "fixtures"))]
    clip_fixture: Option<Arc<crate::video::ClipView>>,
    /// The task that waits for the snapshots: it ends with the model.
    _pump: Option<Task<()>>,
}

impl EventEmitter<Changed> for CoreModel {}

impl CoreModel {
    /// Starts the core (same data folder and same configuration as the earlier client: the only one that
    /// writes `config.json` is the core, see `core/src/state/settings.rs`) and listens to its snapshots.
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

    /// Takes the new snapshot and says which domains have changed. An older one never replaces
    /// a newer one.
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

    /// The last snapshot (a copy of an `Arc`).
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

    /// Shows a test social view in place of the core's (only for the screens).
    #[cfg(feature = "fixtures")]
    pub fn set_social_fixture(&mut self, view: Arc<SocialView>) {
        self.social_fixture = Some(view);
    }

    /// The jam view: the snapshot's; with the `--fixture` test data, the test one. The
    /// viste la leggono da qui, mai dall'istantanea.
    pub fn jam_view(&self) -> Arc<JamView> {
        #[cfg(feature = "fixtures")]
        if let Some(fixture) = &self.jam_fixture {
            return Arc::clone(fixture);
        }
        Arc::clone(&self.snapshot.jam)
    }

    /// Shows a test jam view in place of the core's (only for the screens).
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

    /// Shows a test update in place of the core's (only for the screens).
    #[cfg(feature = "fixtures")]
    pub fn set_update_fixture(&mut self, view: UpdateView) {
        self.update_fixture = Some(view);
    }

    /// The video of the track playing (what to open): the core's; with the `--fixture` test data
    /// video-*`, quello di prova. Il palco del video lo legge da qui, mai dall'istantanea.
    #[cfg(feature = "video")]
    pub fn clip_view(&self) -> Arc<crate::video::ClipView> {
        #[cfg(feature = "fixtures")]
        if let Some(fixture) = &self.clip_fixture {
            return Arc::clone(fixture);
        }
        Arc::clone(&self.clip)
    }

    /// Shows a test video in place of the core's (only for the screens).
    #[cfg(all(feature = "video", feature = "fixtures"))]
    pub fn set_clip_fixture(&mut self, view: crate::video::ClipView) {
        self.clip_fixture = Some(Arc::new(view));
    }

    /// A command to the video domain. With a test video nothing starts.
    #[cfg(feature = "video")]
    pub fn video(&self, command: psytify_core::command::VideoCmd) {
        #[cfg(feature = "fixtures")]
        if self.clip_fixture.is_some() {
            return;
        }
        self.send(Command::Video(command));
    }

    /// The liked tracks' ids.
    pub fn liked(&self) -> Arc<HashSet<Uuid>> {
        Arc::clone(&self.liked)
    }

    /// Why the core has not started, if it has not.
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
        // package downloaded. The banner's closing is done by `dismiss_update`.
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

    /// With a test update: closing the banner removes the offer or the ready file, as the
    /// core does, and says so to the views as a downloads change. Without test data it sends the command.
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

    /// Asks the core for a cover (downloaded and decoded on its runtime). `None` if the core
    /// is no longer there.
    pub fn artwork(&self, url: String) -> Option<oneshot::Receiver<Option<CoverPixels>>> {
        self.core.as_ref().map(|core| core.artwork(url))
    }

    /// [`CoreModel::artwork`] at most `side` pixels wide (the larger tiers of `covers.rs`).
    pub fn artwork_side(&self, url: String, side: u32) -> Option<oneshot::Receiver<Option<CoverPixels>>> {
        self.core.as_ref().map(|core| core.artwork_side(url, side))
    }

    /// Stops the core after it has written `config.json`: the receiver resolves when the work is done. From here
    /// on the commands are lost. `None` if there was no core or it has already been stopped.
    pub fn begin_shutdown(&mut self) -> Option<oneshot::Receiver<Result<(), CoreError>>> {
        self._pump = None;
        self.core.take().map(Core::shutdown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One version per domain, each with a different number: a field swapped for another shows.
    /// The core's `video` field exists even when another crate of the workspace switches on the
    /// core's feature without the desktop switching on its own: `..default()` fills it in that case.
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
