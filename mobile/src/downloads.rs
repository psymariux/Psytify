//! What the app shows of the downloads outside the «Scaricati» page: the mark of a track row, the
//! foreground notification while downloads run, and the notices of the core's download events (the
//! desktop's texts, `desktop/src/notices.rs`). The queue, the files and the index are the core's
//! (`DownloadsView`); nothing here touches the disk except the free space of the folder.

use std::collections::HashMap;
use std::path::Path;

use psytify_core::api::{DownloadQuality, Downloaded};
use psytify_core::error::AppError;
use psytify_core::offline::{Lost, SavedTrack};
use psytify_core::shared::Shared;
use psytify_core::snapshot::{Batch, DownloadsEvent, DownloadsView, Item, ItemState, Refusal};
use uuid::Uuid;

use crate::format;

/// The quality of a download, as the app names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Quality {
    Lossless,
    High,
    Medium,
    Low,
}

impl Quality {
    pub(crate) fn of(quality: DownloadQuality) -> Self {
        match quality {
            DownloadQuality::Lossless => Self::Lossless,
            DownloadQuality::High => Self::High,
            DownloadQuality::Medium => Self::Medium,
            DownloadQuality::Low => Self::Low,
        }
    }

    pub(crate) fn core(self) -> DownloadQuality {
        match self {
            Self::Lossless => DownloadQuality::Lossless,
            Self::High => DownloadQuality::High,
            Self::Medium => DownloadQuality::Medium,
            Self::Low => DownloadQuality::Low,
        }
    }
}

/// How a track row shows the track's download. A phone row has no room for the button: downloads
/// start from the «Altro» sheet or «Scarica tutto», and the mark only says where the track is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Enum)]
pub enum DownloadMark {
    /// Not on the device and not in the queue.
    #[default]
    None,
    /// Waiting for its turn in «Scarica tutto».
    Queued,
    /// Being prepared by the server or arriving.
    Downloading,
    /// On the device, its file in place: it plays without the network.
    Saved,
    /// In the index, but its file is missing or changed (error tint): it plays from the network.
    Lost,
    /// The download failed («Riprova» in «Scaricati»).
    Failed,
}

/// The marks of many rows at once: the queue indexed once, not scanned for every row.
pub(crate) struct Marks<'a> {
    view: &'a DownloadsView,
    items: HashMap<Uuid, &'a Item>,
}

impl<'a> Marks<'a> {
    pub(crate) fn of(view: &'a DownloadsView) -> Self {
        Self { view, items: view.items.iter().map(|item| (item.track.id, item)).collect() }
    }

    /// The queue first (a track saved and downloading again shows the download), then the index;
    /// the order of the desktop's cell (`track_list::download_state`).
    pub(crate) fn mark(&self, id: Uuid) -> DownloadMark {
        if let Some(item) = self.items.get(&id) {
            return match item.state {
                ItemState::Queued => DownloadMark::Queued,
                ItemState::Preparing | ItemState::Downloading { .. } => DownloadMark::Downloading,
                ItemState::Failed(_) => DownloadMark::Failed,
            };
        }
        if !self.view.is_saved(id) {
            DownloadMark::None
        } else if self.view.lost_reason(id).is_some() {
            DownloadMark::Lost
        } else {
            DownloadMark::Saved
        }
    }

    /// The track is in the queue, waiting or running.
    pub(crate) fn busy(&self, id: Uuid) -> bool {
        self.items.get(&id).is_some_and(|item| item.is_busy())
    }
}

/// What the pages read of the downloads, without the progress: the pages other than «Scaricati»
/// are rebuilt only when this changes, not at every percent.
#[derive(Debug, Default)]
pub(crate) struct Shape {
    items: Vec<(Uuid, u8)>,
    saved: Shared<SavedTrack>,
    lost: Shared<Lost>,
    batch: Option<Batch>,
    quality: Option<DownloadQuality>,
}

impl Shape {
    pub(crate) fn of(view: &DownloadsView) -> Self {
        let kind = |item: &Item| match item.state {
            ItemState::Queued => 0,
            ItemState::Preparing | ItemState::Downloading { .. } => 1,
            ItemState::Failed(_) => 2,
        };
        Self {
            items: view.items.iter().map(|item| (item.track.id, kind(item))).collect(),
            saved: view.saved.clone(),
            lost: view.lost.clone(),
            batch: view.batch,
            quality: Some(view.quality),
        }
    }

    pub(crate) fn same(&self, other: &Self) -> bool {
        self.items == other.items
            && self.saved.same(&other.saved)
            && self.lost.same(&other.lost)
            && self.batch == other.batch
            && self.quality == other.quality
    }
}

/// The downloads as the system shows them while they run: a foreground notification, so Android
/// does not freeze or kill the process under a long transfer.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct DownloadsSnapshot {
    /// Something waits or arrives: the notification is up.
    pub running: bool,
    pub title: String,
    pub text: String,
    /// 0 to 100; `None` while nothing says how far (preparing, a length the server did not give).
    pub percent: Option<u8>,
}

/// The status line of one download, as on the desktop and in «Scaricati».
pub(crate) fn status(item: &Item) -> String {
    match &item.state {
        ItemState::Queued => "In coda".to_owned(),
        ItemState::Preparing => "Preparazione…".to_owned(),
        ItemState::Downloading { .. } => {
            item.percent().map_or_else(|| "Scaricamento…".to_owned(), |percent| format!("{percent}%"))
        }
        ItemState::Failed(error) => format!("Non riuscito. {error}"),
    }
}

impl DownloadsSnapshot {
    pub(crate) fn of(view: &DownloadsView) -> Self {
        let busy: Vec<&Item> = view.items.iter().filter(|item| item.is_busy()).collect();
        let Some(first) = busy.first() else { return Self::default() };
        let running = busy.iter().find(|item| matches!(item.state, ItemState::Downloading { .. }));
        let shown = running.unwrap_or(first);
        match view.batch {
            Some(batch) if batch.total > 1 => {
                let finished = batch.done + batch.failed;
                Self {
                    running: true,
                    title: format!("Scarico {}", format::tracks(batch.total)),
                    text: format!("{finished} di {} · «{}»", batch.total, shown.track.title),
                    percent: Some(percent(finished, batch.total)),
                }
            }
            _ if busy.len() > 1 => Self {
                running: true,
                title: format!("Scarico {}", format::tracks(busy.len())),
                text: format!("«{}» · {}", shown.track.title, status(shown)),
                percent: shown.percent(),
            },
            _ => Self {
                running: true,
                title: format!("Scarico «{}»", shown.track.title),
                text: status(shown),
                percent: shown.percent(),
            },
        }
    }
}

fn percent(part: usize, whole: usize) -> u8 {
    (part.min(whole) * 100).checked_div(whole).map_or(0, |done| u8::try_from(done).unwrap_or(100))
}

/// The notice of a download event, with the desktop's words; `None` for what the app does not
/// have yet (the app update, stage 9 of the plan) or cannot do (open the folder).
pub(crate) fn notice(event: &DownloadsEvent) -> Option<(String, bool)> {
    let info = |text: String| Some((text, false));
    let error = |text: String| Some((text, true));
    match event {
        DownloadsEvent::Saved(done) => info(saved_message(done)),
        DownloadsEvent::Failed { error: why, .. } => error(format!("Download non riuscito. {why}")),
        DownloadsEvent::BatchFinished { done, failed, last_error } => {
            Some(batch_summary(*done, *failed, last_error.as_ref()))
        }
        DownloadsEvent::Refused(refusal) => info(refusal_message(*refusal).to_owned()),
        DownloadsEvent::Removed { title, kept_file: true } => info(format!(
            "«{title}» non è più nei download. Il file è cambiato fuori dall'app: l'ho lasciato dov'è"
        )),
        DownloadsEvent::Removed { title, kept_file: false } => {
            info(format!("«{title}» tolto dal dispositivo"))
        }
        DownloadsEvent::RemoveFailed { title, error: why } => {
            error(format!("«{title}» non è più nei download, ma il file non si è cancellato. {why}"))
        }
        DownloadsEvent::Verified { checked, lost: 0 } => {
            info(format!("Controllati {checked} brani: i file sono tutti a posto"))
        }
        DownloadsEvent::Verified { checked, lost } => error(format!(
            "Controllati {checked} brani: {lost} non hanno più il loro file. Suonano dalla rete finché non li scarichi di nuovo"
        )),
        // The update's, as the phone app said them.
        DownloadsEvent::UpdateOffered { tag, .. } => info(format!("Nuova versione {tag} disponibile")),
        DownloadsEvent::UpToDate => info("Hai già l'ultima versione".to_owned()),
        DownloadsEvent::UpdateCheckFailed(why) => error(format!("Controllo non riuscito: {why}")),
        DownloadsEvent::Unreachable => error("Il server non è raggiungibile".to_owned()),
        DownloadsEvent::UpdateSaved(_) => {
            info("Scaricato. Android ti chiede di confermare l'installazione.".to_owned())
        }
        DownloadsEvent::UpdateFailed(why) => error(format!("Download non riuscito: {why}")),
        // A phone has no file manager to open: the core never raises these here.
        DownloadsEvent::FolderNotOpened | DownloadsEvent::UpdateFolderNotOpened => None,
    }
}

/// The desktop names the folder; on the phone it is the app's own, so only the quality is said.
fn saved_message(done: &Downloaded) -> String {
    match done.delivered {
        Some(got) if done.is_lower_than_asked() => format!(
            "Salvato sul dispositivo · qualità {}, la migliore che il brano ha",
            got.label().to_lowercase()
        ),
        Some(got) => format!("Salvato sul dispositivo · qualità {}", got.label().to_lowercase()),
        None => "Salvato sul dispositivo".to_owned(),
    }
}

fn batch_summary(done: usize, failed: usize, last_error: Option<&AppError>) -> (String, bool) {
    match (failed, last_error) {
        (0, _) => (format!("Scaricati {done} brani"), false),
        (_, Some(error)) => (format!("Scaricati {done} brani su {}. {error}", done + failed), true),
        _ => (format!("Scaricati {done} brani su {}", done + failed), true),
    }
}

fn refusal_message(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::NotDownloadable => "Questo brano non si può scaricare: la licenza non lo permette",
        Refusal::NoneDownloadable => "Nessuno di questi brani si può scaricare: la licenza non lo permette",
        Refusal::AlreadyQueued => "Niente da scaricare: i brani sono già in corso",
        Refusal::QueueFull => "Troppi brani in coda: riprova quando ne sono scesi",
        Refusal::AlreadySaved => "Questo brano è già sul dispositivo",
    }
}

/// Bytes free for the app where `dir` is, or on the nearest folder above it that exists (the
/// downloads folder appears with the first download).
pub(crate) fn free_bytes(dir: &Path) -> Option<u64> {
    let existing = dir.ancestors().find(|path| path.exists())?;
    available(existing)
}

#[cfg(unix)]
fn available(path: &Path) -> Option<u64> {
    let stats = rustix::fs::statvfs(path).ok()?;
    Some(stats.f_bavail.saturating_mul(stats.f_frsize))
}

#[cfg(not(unix))]
fn available(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use psytify_core::offline::LostReason;

    use super::*;
    use crate::test_support::track;

    fn item(n: u128, state: ItemState) -> Item {
        Item { track: Arc::new(track(n)), quality: DownloadQuality::High, state, batched: false }
    }

    fn saved(n: u128) -> SavedTrack {
        SavedTrack {
            track: track(n),
            path: format!("/music/{n}.flac").into(),
            format: "flac".into(),
            quality: Some(DownloadQuality::Lossless),
            size: 10,
            sha256: String::new(),
        }
    }

    #[test]
    fn a_mark_follows_the_queue_then_the_index() {
        let mut view = DownloadsView {
            saved: vec![saved(1), saved(2)].into(),
            saved_ids: Arc::new([1, 2].map(Uuid::from_u128).into_iter().collect()),
            lost: vec![Lost { id: Uuid::from_u128(2), reason: LostReason::Missing }].into(),
            ..DownloadsView::default()
        };
        let id = Uuid::from_u128;
        let marks = Marks::of(&view);
        assert_eq!(
            [marks.mark(id(1)), marks.mark(id(2)), marks.mark(id(3))],
            [DownloadMark::Saved, DownloadMark::Lost, DownloadMark::None]
        );
        view.items = vec![
            item(1, ItemState::Downloading { received: 1, total: None }),
            item(3, ItemState::Queued),
            item(4, ItemState::Failed(AppError::Network)),
        ]
        .into();
        let marks = Marks::of(&view);
        assert_eq!(marks.mark(id(1)), DownloadMark::Downloading, "downloading again shows the download");
        assert_eq!([marks.mark(id(3)), marks.mark(id(4))], [DownloadMark::Queued, DownloadMark::Failed]);
        assert!(marks.busy(id(3)) && !marks.busy(id(4)));
    }

    #[test]
    fn the_shape_ignores_the_progress() {
        let mut view = DownloadsView {
            items: vec![item(1, ItemState::Downloading { received: 1, total: Some(100) })].into(),
            ..DownloadsView::default()
        };
        let before = Shape::of(&view);
        view.items = vec![item(1, ItemState::Downloading { received: 50, total: Some(100) })].into();
        assert!(before.same(&Shape::of(&view)));
        view.items = vec![item(1, ItemState::Failed(AppError::Network))].into();
        assert!(!before.same(&Shape::of(&view)));
    }

    #[test]
    fn the_notification_names_the_track_or_counts_the_batch() {
        assert!(!DownloadsSnapshot::of(&DownloadsView::default()).running);
        let one = DownloadsView {
            items: vec![item(1, ItemState::Downloading { received: 45, total: Some(100) })].into(),
            ..DownloadsView::default()
        };
        let shown = DownloadsSnapshot::of(&one);
        assert_eq!((shown.title.as_str(), shown.text.as_str()), ("Scarico «Track 1»", "45%"));
        assert_eq!(shown.percent, Some(45));

        let batch = DownloadsView {
            items: vec![
                item(1, ItemState::Queued),
                item(2, ItemState::Downloading { received: 1, total: None }),
                item(3, ItemState::Failed(AppError::Network)),
            ]
            .into(),
            batch: Some(Batch { total: 4, done: 1, failed: 1 }),
            ..DownloadsView::default()
        };
        let shown = DownloadsSnapshot::of(&batch);
        assert_eq!(shown.title, "Scarico 4 brani");
        assert_eq!(shown.text, "2 di 4 · «Track 2»");
        assert_eq!(shown.percent, Some(50));

        let failed = DownloadsView {
            items: vec![item(3, ItemState::Failed(AppError::Network))].into(),
            ..DownloadsView::default()
        };
        assert!(!DownloadsSnapshot::of(&failed).running, "a failed download does not hold the service");
    }

    #[test]
    fn notices_use_the_desktop_words_and_the_iced_update_lines() {
        let done = Downloaded {
            path: "/music/a.m4a".into(),
            size: 1,
            sha256: String::new(),
            format: "m4a".into(),
            delivered: Some(DownloadQuality::High),
            asked: DownloadQuality::Lossless,
        };
        assert_eq!(
            notice(&DownloadsEvent::Saved(done)),
            Some(("Salvato sul dispositivo · qualità alta, la migliore che il brano ha".into(), false))
        );
        assert_eq!(
            notice(&DownloadsEvent::BatchFinished { done: 3, failed: 0, last_error: None }),
            Some(("Scaricati 3 brani".into(), false))
        );
        assert!(
            notice(&DownloadsEvent::BatchFinished { done: 2, failed: 1, last_error: None })
                .is_some_and(|n| n.1)
        );
        assert_eq!(notice(&DownloadsEvent::UpToDate), Some(("Hai già l'ultima versione".into(), false)));
        assert!(notice(&DownloadsEvent::UpdateSaved("/data/updates/p.apk".into())).is_some_and(|n| !n.1));
        assert_eq!(notice(&DownloadsEvent::UpdateFolderNotOpened), None);
    }

    #[test]
    fn free_space_reads_the_nearest_existing_folder() {
        let dir = std::env::temp_dir().join("psytify-mobile-no-such-folder").join("Music");
        assert!(free_bytes(&dir).is_some_and(|free| free > 0));
    }
}
