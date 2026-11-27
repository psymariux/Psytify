//! «Scaricati», as the desktop page (`desktop/src/app/downloads_page.rs`) and the Iced one: the
//! summary and the space, Riproduci, Casuale and «Controlla i file»; «In corso» (status, the bar of
//! ten steps, Annulla; Riprova and «Togli dall'elenco» for a failed one), then «Sul dispositivo»
//! (format and size, or «File mancante»/«File cambiato» in the error tint with «Scarica di nuovo»;
//! «Rimuovi dal dispositivo»; a tap plays the tracks on the device from there). Everything comes
//! from `DownloadsView`; every button is a `DownloadsCmd`.
//!
//! This page is rebuilt at each percent of a download (the other pages only when the downloads
//! change shape): the queue of saved tracks it plays is kept while the index stays the same.

use std::sync::Arc;

use psytify_core::Command;
use psytify_core::Snapshot;
use psytify_core::api::Track;
use psytify_core::command::{DownloadsCmd, PlaybackCmd};
use psytify_core::offline::{LostReason, SavedTrack};
use psytify_core::shared::Shared;
use psytify_core::snapshot::{DownloadsView, Item};

use super::album::button;
use super::{Art, Built, Header, Mark, PageAction, PageStatus, Row, RowAction, RowState, Target};
use crate::downloads::{free_bytes, status};
use crate::format;

/// The saved tracks as a queue to play, kept while the index is the same one.
#[derive(Debug, Default)]
pub(crate) struct Memo {
    saved: Option<(Shared<SavedTrack>, Arc<[Track]>)>,
}

impl Memo {
    fn tracks(&mut self, saved: &Shared<SavedTrack>) -> Arc<[Track]> {
        match &self.saved {
            Some((known, tracks)) if known.same(saved) => Arc::clone(tracks),
            _ => {
                let tracks: Arc<[Track]> = saved.iter().map(|entry| entry.track.clone()).collect();
                self.saved = Some((saved.clone(), Arc::clone(&tracks)));
                tracks
            }
        }
    }
}

/// The line under the title: how many tracks and how much they weigh, or that the check runs.
fn summary(view: &DownloadsView) -> String {
    if view.verifying {
        "Controllo dei file…".to_owned()
    } else if view.saved.is_empty() {
        "Nessun brano sul dispositivo".to_owned()
    } else {
        format!("{} · {}", format::tracks(view.saved.len()), format::size(view.used_bytes))
    }
}

/// The note under the buttons: the space left, the quality, and where the files live.
fn storage(view: &DownloadsView, free: Option<u64>) -> String {
    let mut note = match free {
        Some(free) => format!("{} liberi sul telefono", format::size(free)),
        None => "Spazio libero sconosciuto".to_owned(),
    };
    note.push_str(&format!(" · qualità dei download {}.", view.quality.label()));
    note.push_str(" I brani stanno nella memoria dell'app: se la disinstalli, si cancellano.");
    note
}

pub(super) fn build(snapshot: &Snapshot, memo: &mut Memo) -> Built {
    let view = &snapshot.downloads;
    let mut page = Built::titled("Scaricati");
    let tracks = memo.tracks(&view.saved);
    let mut buttons = Vec::new();
    if !tracks.is_empty() {
        buttons.push(button(PageAction::Play, "Riproduci", true, false));
        buttons.push(button(PageAction::Shuffle, "Casuale", false, false));
        page.action(PageAction::Play, Target::Play(Arc::clone(&tracks), 0));
        let shuffle = Command::Playback(PlaybackCmd::ShufflePlay(Arc::clone(&tracks)));
        page.action(PageAction::Shuffle, Target::Send(shuffle));
        if view.verifying {
            buttons.push(button(PageAction::Verify, "Controllo in corso…", false, true));
        } else {
            buttons.push(button(PageAction::Verify, "Controlla i file", false, false));
            page.action(PageAction::Verify, Target::Send(Command::Downloads(DownloadsCmd::Verify)));
        }
    }
    page.header = Some(Header {
        kicker: "Sul dispositivo".into(),
        title: "Scaricati".into(),
        subtitle: None,
        subtitle_opens: false,
        meta: summary(view),
        art: Art::Downloads,
        buttons,
        note: Some(storage(view, free_bytes(&view.folder))),
    });
    page.status = if view.items.is_empty() && view.saved.is_empty() {
        PageStatus::Empty {
            message:
                "I brani che scarichi stanno qui e suonano anche senza rete. Scaricali dal menu «Altro» \
                      di un brano o con «Scarica tutto», se la licenza lo permette."
                    .into(),
        }
    } else {
        PageStatus::Ready
    };

    if !view.items.is_empty() {
        page.heading("progress", "In corso", format::tracks(view.items.len()));
        for item in view.items.iter() {
            in_progress(&mut page, item);
        }
    }
    if view.saved.is_empty() {
        if !view.items.is_empty() {
            let text = "Niente sul dispositivo, per ora: i brani arrivano qui appena scaricati.";
            page.note("nothing", text.to_owned(), super::Tone::Plain);
        }
        return page;
    }
    page.heading("saved", "Sul dispositivo", format::tracks(view.saved.len()));
    for (index, entry) in view.saved.iter().enumerate() {
        on_device(&mut page, view, entry, &tracks, index);
    }
    page
}

fn in_progress(page: &mut Built, item: &Item) {
    let track = &item.track;
    let id = track.id;
    let failed = !item.is_busy();
    let row = Row::Download {
        key: format!("dl:{id}"),
        art: Art::Cover { url: track.cover_url.clone() },
        title: track.title.clone(),
        status: status(item),
        failed,
        steps: (!failed).then(|| item.percent().map_or(0, |percent| percent.min(100) / 10)),
    };
    let at = page.rows.len();
    page.push(row, Mark::None, Target::None);
    let cancel = Target::Send(Command::Downloads(DownloadsCmd::Cancel(id)));
    if failed {
        page.row_action(at, RowAction::Retry, Target::Send(Command::Downloads(DownloadsCmd::Retry(id))));
        page.row_action(at, RowAction::Dismiss, cancel);
    } else {
        page.row_action(at, RowAction::Cancel, cancel);
    }
}

fn on_device(
    page: &mut Built,
    view: &DownloadsView,
    entry: &SavedTrack,
    tracks: &Arc<[Track]>,
    index: usize,
) {
    let track = &entry.track;
    let lost = view.lost_reason(track.id);
    let detail = match lost {
        Some(LostReason::Missing) => format!("{} · File mancante", track.artist.name),
        Some(LostReason::Changed) => format!("{} · File cambiato", track.artist.name),
        None => {
            format!("{} · {} · {}", track.artist.name, entry.format.to_uppercase(), format::size(entry.size))
        }
    };
    let row = Row::Saved {
        key: format!("saved:{}", track.id),
        art: Art::Cover { url: track.cover_url.clone() },
        title: track.title.clone(),
        detail,
        lost: lost.is_some(),
        state: RowState::Idle,
    };
    let at = page.rows.len();
    page.push(row, Mark::Track(track.id), Target::Play(Arc::clone(tracks), index));
    if lost.is_some() {
        let again = DownloadsCmd::Download(Box::new(track.clone()));
        page.row_action(at, RowAction::DownloadAgain, Target::Send(Command::Downloads(again)));
    }
    page.row_action(at, RowAction::Remove, Target::Send(Command::Downloads(DownloadsCmd::Remove(track.id))));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_counts_and_weighs_or_says_the_check_runs() {
        let mut view = DownloadsView::default();
        assert_eq!(summary(&view), "Nessun brano sul dispositivo");
        view.used_bytes = 12_300_000;
        view.saved = vec![SavedTrack {
            track: crate::test_support::track(1),
            path: "/m/a.flac".into(),
            format: "flac".into(),
            quality: None,
            size: 12_300_000,
            sha256: String::new(),
        }]
        .into();
        assert_eq!(summary(&view), "1 brano · 12,3 MB");
        view.verifying = true;
        assert_eq!(summary(&view), "Controllo dei file…");
        assert!(storage(&view, Some(4_200_000_000)).starts_with("4,2 GB liberi sul telefono · qualità"));
    }
}
