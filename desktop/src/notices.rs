//! The notices the core reports to the window: one system for all the domains.
//!
//! Each domain with events has a ring (`events`: the latest, from the oldest, each with a `seq` that
//! grows), and playback has only one (`PlaybackView::event`, the last of the turn). The window
//! remembers the last `seq` shown for each ring ([`Seen`]) and at each publication takes only the
//! new ones, in order. At start-up it begins from the most recent of each ring ([`Seen::at_start`]): what the
//! core said before the window existed is not repeated.
//!
//! The texts are the core's when the core gives them (`message()` and `is_error()` of external sources, social and
//! jam); those of the downloads and of playback are here, the same as in the earlier client. The notices
//! shown are a short queue ([`Toasts`]): one at a time for `TOAST_HOLD_MS`, at most
//! [`MAX_PENDING`] waiting (the oldest falls off), two equal in a row count as one. The timer is kept by
//! whoever shows, and it exists only while a notice is on the display: with an empty queue nothing runs.
//!
//! Settings have no ring: their notice is the write error (`SettingsView::save_error`),
//! shown once for each new error.

use std::collections::VecDeque;

use gpui::SharedString;
use psytify_core::api::Downloaded;
use psytify_core::error::AppError;
use psytify_core::snapshot::{CatalogEvent, DownloadsEvent, PlaybackEvent, Refusal, Snapshot};

use crate::core_model::Changed;

/// Notices waiting behind the one shown: beyond that, the oldest falls off.
pub const MAX_PENDING: usize = 3;

/// A notice: the text and whether it is an error (the `ERROR` bar on the toast's left).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: SharedString,
    pub error: bool,
}

impl Notice {
    pub fn info(text: impl Into<SharedString>) -> Self {
        Self { text: text.into(), error: false }
    }

    pub fn error(text: impl Into<SharedString>) -> Self {
        Self { text: text.into(), error: true }
    }

    fn tinted(text: String, error: bool) -> Self {
        Self { text: text.into(), error }
    }
}

/// What the window has to do after reading the new events.
#[derive(Debug, Default, PartialEq)]
pub struct Heard {
    /// The new notices, in order: first playback, then catalogue, external sources, downloads,
    /// social and jam, and within a domain by `seq`.
    pub notices: Vec<Notice>,
    /// The system asked to bring the window to the front (it is not a notice).
    pub raise: bool,
    /// The official catalogue's artist now asked for has a page on the server: it is opened (the last, if
    /// there are more than one).
    pub open_artist: Option<uuid::Uuid>,
}

/// The last `seq` shown of each event ring.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    playback: u64,
    catalog: u64,
    external: u64,
    downloads: u64,
    social: u64,
    jam: u64,
    /// The last settings write error already shown.
    save_error: Option<AppError>,
}

impl Seen {
    /// Starts from the most recent of each ring: nothing of what is already there repeats.
    pub fn at_start(snapshot: &Snapshot) -> Self {
        Self {
            playback: snapshot.playback.event.as_ref().map_or(0, |raised| raised.seq),
            catalog: snapshot.catalog.events.back().map_or(0, |raised| raised.seq),
            external: snapshot.external.events.back().map_or(0, |raised| raised.seq),
            downloads: snapshot.downloads.events.back().map_or(0, |raised| raised.seq),
            social: snapshot.social.events.back().map_or(0, |raised| raised.seq),
            jam: snapshot.jam.events.back().map_or(0, |raised| raised.seq),
            save_error: snapshot.settings.save_error.clone(),
        }
    }

    /// The new events of the changed domains. A domain that has not changed is not looked at (it has no
    /// new events: an event is born with a publication of its domain).
    pub fn collect(&mut self, snapshot: &Snapshot, changed: Changed) -> Heard {
        let mut heard = Heard::default();
        if changed.settings && snapshot.settings.save_error != self.save_error {
            self.save_error = snapshot.settings.save_error.clone();
            if let Some(error) = &self.save_error {
                heard.notices.push(Notice::error(format!("Impostazioni non salvate. {error}")));
            }
        }
        if changed.playback
            && let Some(raised) = &snapshot.playback.event
            && raised.seq > self.playback
        {
            self.playback = raised.seq;
            match playback_notice(&raised.event) {
                Some(notice) => heard.notices.push(notice),
                None => heard.raise |= matches!(raised.event, PlaybackEvent::Raise),
            }
        }
        if changed.catalog {
            let from = self.catalog;
            for raised in snapshot.catalog.events.iter().filter(|raised| raised.seq > from) {
                self.catalog = self.catalog.max(raised.seq);
                heard.notices.extend(catalog_notice(&raised.event));
                if let CatalogEvent::MetaArtistOpened(id) = raised.event {
                    heard.open_artist = Some(id);
                }
            }
        }
        if changed.external {
            let from = self.external;
            for raised in snapshot.external.events.iter().filter(|raised| raised.seq > from) {
                self.external = self.external.max(raised.seq);
                heard
                    .notices
                    .extend(raised.event.message().map(|text| Notice::tinted(text, raised.event.is_error())));
            }
        }
        if changed.downloads {
            let from = self.downloads;
            for raised in snapshot.downloads.events.iter().filter(|raised| raised.seq > from) {
                self.downloads = self.downloads.max(raised.seq);
                heard.notices.push(downloads_notice(&raised.event));
            }
        }
        if changed.social {
            let from = self.social;
            for raised in snapshot.social.events.iter().filter(|raised| raised.seq > from) {
                self.social = self.social.max(raised.seq);
                heard.notices.push(Notice::tinted(raised.event.message(), raised.event.is_error()));
            }
        }
        if changed.jam {
            let from = self.jam;
            for raised in snapshot.jam.events.iter().filter(|raised| raised.seq > from) {
                self.jam = self.jam.max(raised.seq);
                heard.notices.push(Notice::tinted(raised.event.message(), raised.event.is_error()));
            }
        }
        heard
    }
}

fn playback_notice(event: &PlaybackEvent) -> Option<Notice> {
    match event {
        PlaybackEvent::SleepStopped => Some(Notice::info("Il timer di spegnimento ha fermato la musica.")),
        PlaybackEvent::AutoplayAdded => Some(Notice::info("Ho aggiunto altri brani alla coda.")),
        PlaybackEvent::SeekFailed => Some(Notice::error("Impossibile spostarsi in questo punto del brano.")),
        PlaybackEvent::PlayFailed(error) => Some(Notice::error(error.to_string())),
        // As before the migration.
        PlaybackEvent::RadioStarted(track) => Some(Notice::info(format!("Radio di «{}»", track.title))),
        PlaybackEvent::Raise | PlaybackEvent::AlbumStarted => None,
    }
}

/// Of the catalogue only the failures are reported: opening an artist and the ready covers are work for the
/// pages, not notices.
fn catalog_notice(event: &CatalogEvent) -> Option<Notice> {
    match event {
        CatalogEvent::MetaArtistFailed(error) | CatalogEvent::LibraryPageFailed(error) => {
            Some(Notice::error(error.to_string()))
        }
        CatalogEvent::MetaArtistOpened(_) | CatalogEvent::Replaced(_) | CatalogEvent::Prefetched(_) => None,
    }
}

/// The same texts as the earlier client.
fn downloads_notice(event: &DownloadsEvent) -> Notice {
    match event {
        DownloadsEvent::Saved(done) => Notice::info(saved_message(done)),
        DownloadsEvent::Failed { error, .. } => Notice::error(format!("Download non riuscito. {error}")),
        DownloadsEvent::BatchFinished { done, failed, last_error } => {
            batch_summary(*done, *failed, last_error.as_ref())
        }
        DownloadsEvent::Refused(refusal) => Notice::info(refusal_message(*refusal)),
        DownloadsEvent::UpdateOffered { tag, .. } => {
            Notice::info(format!("Nuova versione {tag} disponibile"))
        }
        DownloadsEvent::UpToDate => Notice::info("Hai già l'ultima versione"),
        DownloadsEvent::UpdateCheckFailed(error) => Notice::error(format!("Controllo non riuscito: {error}")),
        DownloadsEvent::Unreachable => Notice::error("Il server non è raggiungibile"),
        DownloadsEvent::UpdateSaved(path) => {
            Notice::info(format!("Scaricato in {}. Installalo a mano.", path.display()))
        }
        DownloadsEvent::UpdateFailed(error) => Notice::error(format!("Download non riuscito: {error}")),
        DownloadsEvent::Removed { title, kept_file: true } => Notice::info(format!(
            "«{title}» non è più nei download. Il file è cambiato fuori dall'app: l'ho lasciato dov'è"
        )),
        DownloadsEvent::Removed { title, kept_file: false } => {
            Notice::info(format!("«{title}» tolto dal dispositivo"))
        }
        DownloadsEvent::RemoveFailed { title, error } => Notice::error(format!(
            "«{title}» non è più nei download, ma il file non si è cancellato. {error}"
        )),
        DownloadsEvent::Verified { checked, lost: 0 } => {
            Notice::info(format!("Controllati {checked} brani: i file sono tutti a posto"))
        }
        DownloadsEvent::Verified { checked, lost } => Notice::error(format!(
            "Controllati {checked} brani: {lost} non hanno più il loro file. Suonano dalla rete finché non li scarichi di nuovo"
        )),
        DownloadsEvent::FolderNotOpened => Notice::error("Non riesco ad aprire la cartella dei download"),
        DownloadsEvent::UpdateFolderNotOpened => {
            Notice::error("Non riesco ad aprire la cartella dell'aggiornamento")
        }
    }
}

/// Where the file ended up and, if the quality is not the one asked for, which one arrived.
fn saved_message(done: &Downloaded) -> String {
    let place = done.path.display();
    match done.delivered {
        Some(got) if done.is_lower_than_asked() => {
            format!(
                "Salvato in {place} · qualità {}, la migliore che il brano ha",
                got.label().to_lowercase()
            )
        }
        Some(got) => format!("Salvato in {place} · qualità {}", got.label().to_lowercase()),
        None => format!("Salvato in {place}"),
    }
}

fn batch_summary(done: usize, failed: usize, last_error: Option<&AppError>) -> Notice {
    match (failed, last_error) {
        (0, _) => Notice::info(format!("Scaricati {done} brani")),
        (_, Some(error)) => Notice::error(format!("Scaricati {done} brani su {}. {error}", done + failed)),
        _ => Notice::error(format!("Scaricati {done} brani su {}", done + failed)),
    }
}

/// Why a download did not start.
fn refusal_message(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::NotDownloadable => "Questo brano non si può scaricare: la licenza non lo permette",
        Refusal::NoneDownloadable => "Nessuno di questi brani si può scaricare: la licenza non lo permette",
        Refusal::AlreadyQueued => "Niente da scaricare: i brani sono già in corso",
        Refusal::QueueFull => "Troppi brani in coda: riprova quando ne sono scesi",
        Refusal::AlreadySaved => "Questo brano è già sul dispositivo",
    }
}

/// What happened pushing a notice into the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pushed {
    /// There was nothing on the display: now this is it, and whoever shows must arm the timer.
    Shown,
    /// Waits for its turn (or it is equal to the one already there and does not count).
    Waiting,
}

/// The notice on the display and the ones waiting. No time here: whoever shows arms the timer when
/// `push` says `Shown` and calls `expire` at the deadline.
#[derive(Debug, Default)]
pub struct Toasts {
    shown: Option<Notice>,
    waiting: VecDeque<Notice>,
}

impl Toasts {
    /// The notice on the display, if there is one.
    pub fn current(&self) -> Option<&Notice> {
        self.shown.as_ref()
    }

    /// Quanti aspettano dietro quello mostrato.
    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    pub fn push(&mut self, notice: Notice) -> Pushed {
        if self.shown.is_none() {
            self.shown = Some(notice);
            return Pushed::Shown;
        }
        // The same notice twice in a row (an error that repeats) counts as one.
        let last = self.waiting.back().or(self.shown.as_ref());
        if last == Some(&notice) {
            return Pushed::Waiting;
        }
        if self.waiting.len() == MAX_PENDING {
            self.waiting.pop_front();
        }
        self.waiting.push_back(notice);
        Pushed::Waiting
    }

    /// The time of the notice shown is up: it moves to the next. `true` if there is a new one on the
    /// display (the timer must be armed again), `false` if nothing is left.
    pub fn expire(&mut self) -> bool {
        self.shown = self.waiting.pop_front();
        self.shown.is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use psytify_core::snapshot::{
        CatalogRaised, DownloadsRaised, DownloadsView, ExternalEvent, ExternalRaised, ExternalView, JamEvent,
        JamRaised, JamView, Raised, SocialEvent, SocialRaised, SocialView,
    };

    use super::*;

    fn all() -> Changed {
        Changed {
            settings: true,
            library: true,
            playback: true,
            catalog: true,
            external: true,
            downloads: true,
            social: true,
            jam: true,
            #[cfg(feature = "video")]
            video: true,
        }
    }

    fn external(events: &[(u64, ExternalEvent)]) -> Arc<ExternalView> {
        let ring: VecDeque<_> =
            events.iter().map(|(seq, event)| ExternalRaised { seq: *seq, event: event.clone() }).collect();
        Arc::new(ExternalView { events: Arc::new(ring), ..ExternalView::default() })
    }

    fn with_external(events: &[(u64, ExternalEvent)]) -> Snapshot {
        Snapshot { external: external(events), ..Snapshot::default() }
    }

    #[test]
    fn what_the_core_said_before_the_window_existed_is_not_replayed() {
        let old = with_external(&[(1, ExternalEvent::NothingToPlay), (2, ExternalEvent::NothingToPlay)]);
        let mut seen = Seen::at_start(&old);
        assert!(seen.collect(&old, all()).notices.is_empty(), "niente di ciò che c'era all'avvio");
        // A new event is heard only once.
        let next = with_external(&[
            (1, ExternalEvent::NothingToPlay),
            (2, ExternalEvent::NothingToPlay),
            (3, ExternalEvent::PlayFailed(AppError::Network)),
        ]);
        let heard = seen.collect(&next, all());
        assert_eq!(heard.notices, vec![Notice::error(AppError::Network.to_string())]);
        assert!(seen.collect(&next, all()).notices.is_empty(), "letto una volta, non si ripete");
    }

    #[test]
    fn a_burst_is_shown_in_order_with_the_tint_the_core_gives() {
        let mut seen = Seen::default();
        let snapshot = with_external(&[
            (1, ExternalEvent::PlaylistSaved { name: "Sera".into(), count: 2 }),
            (2, ExternalEvent::NothingToPlay),
            (3, ExternalEvent::PlaylistReady { name: "Sera".into(), count: 2, failed: 0 }),
        ]);
        let heard = seen.collect(&snapshot, all());
        let tints: Vec<bool> = heard.notices.iter().map(|notice| notice.error).collect();
        assert_eq!(tints, [false, true, false], "l'errore lo decide `is_error()` del core");
        assert_eq!(
            heard.notices[0].text,
            SharedString::from(
                ExternalEvent::PlaylistSaved { name: "Sera".into(), count: 2 }.message().unwrap_or_default()
            )
        );
        assert_eq!(seen.external, 3);
    }

    #[test]
    fn imported_tracks_carry_data_and_say_nothing() {
        let mut seen = Seen::default();
        let snapshot = with_external(&[(1, ExternalEvent::Imported(Arc::from(Vec::new())))]);
        assert!(seen.collect(&snapshot, all()).notices.is_empty());
        assert_eq!(seen.external, 1, "visto comunque: non torna alla prossima pubblicazione");
    }

    #[test]
    fn a_settings_save_error_is_said_once_for_each_new_error() {
        let mut seen = Seen::default();
        let failed = |error: Option<AppError>| Snapshot {
            settings: Arc::new(psytify_core::snapshot::SettingsView {
                save_error: error,
                ..Default::default()
            }),
            ..Snapshot::default()
        };
        let changed = Changed { settings: true, ..Changed::default() };
        let broken = failed(Some(AppError::Timeout));
        let heard = seen.collect(&broken, changed);
        assert_eq!(
            heard.notices,
            vec![Notice::error(format!("Impostazioni non salvate. {}", AppError::Timeout))]
        );
        assert!(seen.collect(&broken, changed).notices.is_empty(), "lo stesso errore non si ripete");
        assert!(seen.collect(&failed(None), changed).notices.is_empty(), "riuscito: niente avviso");
        assert_eq!(seen.collect(&broken, changed).notices.len(), 1, "un errore che torna si dice di nuovo");
    }

    #[test]
    fn a_domain_that_did_not_change_is_not_read() {
        let mut seen = Seen::default();
        let snapshot = with_external(&[(1, ExternalEvent::NothingToPlay)]);
        let quiet = Changed { catalog: true, ..Changed::default() };
        assert!(seen.collect(&snapshot, quiet).notices.is_empty());
        assert_eq!(seen.external, 0);
        let loud = Changed { external: true, ..Changed::default() };
        assert_eq!(seen.collect(&snapshot, loud).notices.len(), 1, "alla sua pubblicazione si sente");
    }

    #[test]
    fn every_domain_has_its_own_counter() {
        let mut snapshot = Snapshot::default();
        snapshot.playback = Arc::new(psytify_core::snapshot::PlaybackView {
            event: Some(Raised { seq: 7, event: PlaybackEvent::SeekFailed }),
            ..Default::default()
        });
        snapshot.catalog = Arc::new(psytify_core::snapshot::CatalogView {
            events: Arc::new(VecDeque::from([CatalogRaised {
                seq: 1,
                event: CatalogEvent::LibraryPageFailed(AppError::Timeout),
            }])),
            ..Default::default()
        });
        snapshot.downloads = Arc::new(DownloadsView {
            events: Arc::new(VecDeque::from([DownloadsRaised { seq: 1, event: DownloadsEvent::UpToDate }])),
            ..Default::default()
        });
        snapshot.social = Arc::new(SocialView {
            events: Arc::new(VecDeque::from([SocialRaised { seq: 1, event: SocialEvent::ProfileSaved }])),
            ..Default::default()
        });
        snapshot.jam = Arc::new(JamView {
            events: Arc::new(VecDeque::from([JamRaised { seq: 1, event: JamEvent::BadCode }])),
            ..Default::default()
        });
        let mut seen = Seen::default();
        let heard = seen.collect(&snapshot, all());
        assert_eq!(heard.notices.len(), 5, "un avviso per dominio, nonostante i `seq` uguali a 1");
        assert!(heard.notices[0].error && heard.notices[1].error, "riproduzione e catalogo sono errori");
        assert!(!heard.notices[2].error && !heard.notices[3].error, "download e social sono informazioni");
        assert!(heard.notices[4].error, "un codice sbagliato è un errore");
        // After the first read each one remembers its own.
        assert_eq!(seen, Seen::at_start(&snapshot));
        assert!(seen.collect(&snapshot, all()).notices.is_empty());
    }

    #[test]
    fn raising_the_window_is_not_a_notice() {
        let mut snapshot = Snapshot::default();
        snapshot.playback = Arc::new(psytify_core::snapshot::PlaybackView {
            event: Some(Raised { seq: 1, event: PlaybackEvent::Raise }),
            ..Default::default()
        });
        let heard = Seen::default().collect(&snapshot, all());
        assert!(heard.raise);
        assert!(heard.notices.is_empty());
    }

    #[test]
    fn an_official_artist_brought_to_the_server_is_opened_not_announced() {
        let id = uuid::Uuid::from_u128(7);
        let ring: VecDeque<_> = [CatalogRaised { seq: 1, event: CatalogEvent::MetaArtistOpened(id) }].into();
        let snapshot = Snapshot {
            catalog: Arc::new(psytify_core::snapshot::CatalogView {
                events: Arc::new(ring),
                ..Default::default()
            }),
            ..Snapshot::default()
        };
        let heard = Seen::default().collect(&snapshot, all());
        assert_eq!(heard.open_artist, Some(id));
        assert!(heard.notices.is_empty());
    }

    #[test]
    fn a_radio_says_where_it_started() {
        let track = psytify_core::api::models::fixtures::track(4);
        let mut snapshot = Snapshot::default();
        snapshot.playback = Arc::new(psytify_core::snapshot::PlaybackView {
            event: Some(Raised { seq: 1, event: PlaybackEvent::RadioStarted(Arc::new(track)) }),
            ..Default::default()
        });
        let heard = Seen::default().collect(&snapshot, all());
        assert_eq!(heard.notices, vec![Notice::info("Radio di «Brano 4»")]);
    }

    #[test]
    fn the_download_texts_match_the_iced_client() {
        assert_eq!(batch_summary(3, 0, None), Notice::info("Scaricati 3 brani"));
        assert_eq!(
            batch_summary(2, 1, Some(&AppError::Network)),
            Notice::error(format!("Scaricati 2 brani su 3. {}", AppError::Network))
        );
        assert_eq!(batch_summary(2, 1, None), Notice::error("Scaricati 2 brani su 3"));
        assert_eq!(
            downloads_notice(&DownloadsEvent::Removed { title: "Sera".into(), kept_file: false }),
            Notice::info("«Sera» tolto dal dispositivo")
        );
        assert!(downloads_notice(&DownloadsEvent::Verified { checked: 4, lost: 1 }).error);
        assert!(!downloads_notice(&DownloadsEvent::Verified { checked: 4, lost: 0 }).error);
    }

    #[test]
    fn the_toast_queue_shows_one_at_a_time_in_order() {
        let mut toasts = Toasts::default();
        assert_eq!(toasts.push(Notice::info("uno")), Pushed::Shown);
        assert_eq!(toasts.push(Notice::info("due")), Pushed::Waiting);
        assert_eq!(toasts.push(Notice::error("tre")), Pushed::Waiting);
        assert_eq!(toasts.current(), Some(&Notice::info("uno")));
        assert!(toasts.expire());
        assert_eq!(toasts.current(), Some(&Notice::info("due")));
        assert!(toasts.expire());
        assert_eq!(toasts.current(), Some(&Notice::error("tre")));
        assert!(!toasts.expire(), "coda finita: niente più timer");
        assert_eq!(toasts.current(), None);
        assert_eq!(toasts.waiting(), 0);
        // With the display empty the next one starts at once.
        assert_eq!(toasts.push(Notice::info("quattro")), Pushed::Shown);
    }

    #[test]
    fn the_toast_queue_is_bounded_and_drops_the_oldest_waiting() {
        let mut toasts = Toasts::default();
        toasts.push(Notice::info("0"));
        for n in 1..=(MAX_PENDING + 2) {
            toasts.push(Notice::info(n.to_string()));
        }
        assert_eq!(toasts.waiting(), MAX_PENDING);
        // The one shown does not fall; among the others the most recent remain.
        assert_eq!(toasts.current(), Some(&Notice::info("0")));
        assert!(toasts.expire());
        assert_eq!(toasts.current(), Some(&Notice::info("3")));
    }

    #[test]
    fn the_same_notice_twice_in_a_row_counts_once() {
        let mut toasts = Toasts::default();
        toasts.push(Notice::error("Server non raggiungibile."));
        toasts.push(Notice::error("Server non raggiungibile."));
        assert_eq!(toasts.waiting(), 0, "uguale a quello mostrato");
        toasts.push(Notice::info("altro"));
        toasts.push(Notice::info("altro"));
        assert_eq!(toasts.waiting(), 1, "uguale all'ultimo in attesa");
        // The same text with another tone is another notice.
        toasts.push(Notice::error("altro"));
        assert_eq!(toasts.waiting(), 2);
    }
}
