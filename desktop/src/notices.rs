//! Gli avvisi che il core racconta alla finestra: un solo sistema per tutti i domini.
//!
//! Ogni dominio con eventi ha un anello (`events`: gli ultimi, dal più vecchio, ognuno con un `seq` che
//! cresce) e la riproduzione ne ha uno solo (`PlaybackView::event`, l'ultimo del turno). La finestra
//! ricorda l'ultimo `seq` mostrato per ogni anello ([`Seen`]) e a ogni pubblicazione prende solo i
//! nuovi, in ordine. All'avvio si parte dal più recente di ogni anello ([`Seen::at_start`]): ciò che il
//! core ha detto prima che la finestra esistesse non si ripete.
//!
//! I testi sono del core quando il core li dà (`message()` e `is_error()` di fonti esterne, social e
//! jam); quelli dei download e della riproduzione stanno qui, gli stessi del client Iced. Gli avvisi
//! mostrati sono una coda corta ([`Toasts`]): uno alla volta per `TOAST_HOLD_MS`, al massimo
//! [`MAX_PENDING`] in attesa (il più vecchio cade), due uguali di fila contano per uno. Il timer lo
//! tiene chi mostra, e c'è solo mentre un avviso è sul display: a coda vuota non c'è niente che giri.
//!
//! Le impostazioni non hanno un anello: il loro avviso è l'errore di scrittura (`SettingsView::save_error`),
//! mostrato una volta per ogni errore nuovo, come nel client Iced.

use std::collections::VecDeque;

use gpui::SharedString;
use psytify_core::api::Downloaded;
use psytify_core::error::AppError;
use psytify_core::snapshot::{CatalogEvent, DownloadsEvent, PlaybackEvent, Refusal, Snapshot};

use crate::core_model::Changed;

/// Avvisi in attesa dietro quello mostrato: oltre, cade il più vecchio.
pub const MAX_PENDING: usize = 3;

/// Un avviso: il testo e se è un errore (la barra `ERROR` a sinistra del toast).
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

/// Cosa la finestra ha da fare dopo aver letto i nuovi eventi.
#[derive(Debug, Default, PartialEq)]
pub struct Heard {
    /// Gli avvisi nuovi, in ordine: prima la riproduzione, poi catalogo, fonti esterne, download,
    /// social e jam, e dentro un dominio per `seq`.
    pub notices: Vec<Notice>,
    /// Il sistema ha chiesto di portare la finestra in primo piano (non è un avviso).
    pub raise: bool,
    /// L'artista del catalogo ufficiale chiesto ora ha una pagina sul server: la si apre (l'ultimo, se
    /// sono più d'uno).
    pub open_artist: Option<uuid::Uuid>,
}

/// L'ultimo `seq` mostrato di ogni anello di eventi.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    playback: u64,
    catalog: u64,
    external: u64,
    downloads: u64,
    social: u64,
    jam: u64,
    /// L'ultimo errore di scrittura delle impostazioni già mostrato.
    save_error: Option<AppError>,
}

impl Seen {
    /// Parte dal più recente di ogni anello: niente di ciò che c'è già si ripete.
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

    /// Gli eventi nuovi dei domini cambiati. Un dominio che non è cambiato non si guarda (non ha
    /// eventi nuovi: un evento nasce con una pubblicazione del suo dominio).
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
        // Come il client Iced (`client/src/app/playback.rs`).
        PlaybackEvent::RadioStarted(track) => Some(Notice::info(format!("Radio di «{}»", track.title))),
        PlaybackEvent::Raise | PlaybackEvent::AlbumStarted => None,
    }
}

/// Del catalogo si dicono solo i guasti: aprire un artista e le copertine pronte sono lavoro delle
/// pagine, non avvisi.
fn catalog_notice(event: &CatalogEvent) -> Option<Notice> {
    match event {
        CatalogEvent::MetaArtistFailed(error) | CatalogEvent::LibraryPageFailed(error) => {
            Some(Notice::error(error.to_string()))
        }
        CatalogEvent::MetaArtistOpened(_) | CatalogEvent::Replaced(_) | CatalogEvent::Prefetched(_) => None,
    }
}

/// Gli stessi testi del client Iced (`client/src/app/downloads.rs`).
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

/// Dove è finito il file e, se la qualità non è quella chiesta, quale è arrivata.
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

/// Perché un download non è partito.
fn refusal_message(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::NotDownloadable => "Questo brano non si può scaricare: la licenza non lo permette",
        Refusal::NoneDownloadable => "Nessuno di questi brani si può scaricare: la licenza non lo permette",
        Refusal::AlreadyQueued => "Niente da scaricare: i brani sono già in corso",
        Refusal::QueueFull => "Troppi brani in coda: riprova quando ne sono scesi",
        Refusal::AlreadySaved => "Questo brano è già sul dispositivo",
    }
}

/// Cosa è successo spingendo un avviso nella coda.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pushed {
    /// Non c'era niente sul display: ora è questo, e chi mostra deve armare il timer.
    Shown,
    /// Aspetta il suo turno (o è uguale a quello che c'è già e non conta).
    Waiting,
}

/// L'avviso sul display e quelli in attesa. Nessun tempo qui: chi mostra arma il timer quando
/// `push` dice `Shown` e chiama `expire` alla scadenza.
#[derive(Debug, Default)]
pub struct Toasts {
    shown: Option<Notice>,
    waiting: VecDeque<Notice>,
}

impl Toasts {
    /// L'avviso sul display, se c'è.
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
        // Lo stesso avviso due volte di fila (un errore che si ripete) conta per uno.
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

    /// Il tempo dell'avviso mostrato è finito: passa al prossimo. `true` se ce n'è uno nuovo sul
    /// display (il timer va armato di nuovo), `false` se non resta niente.
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
        // Un evento nuovo si sente una volta sola.
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
        // Dopo la prima lettura ognuno ricorda il suo.
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
        // Con il display vuoto il prossimo parte subito.
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
        // Quello mostrato non cade; fra gli altri restano i più recenti.
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
        // Lo stesso testo con un altro tono è un altro avviso.
        toasts.push(Notice::error("altro"));
        assert_eq!(toasts.waiting(), 2);
    }
}
