//! The queue for the app, a page at a time: a queue can hold thousands of tracks, and the player's
//! list only needs the rows on screen. The bridge keeps the queue it last published and a version
//! that grows when the core publishes a different one, so the app knows when its pages are stale.

use std::sync::Arc;

use psytify_core::player::Queue;

use crate::format;
use crate::playback::{TrackItem, count};

/// At most this many entries in one page.
pub const MAX_PAGE: u32 = 100;

/// An entry of the queue and its position in listening order (what `PlayAt`, `Remove` and `Move`
/// take).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QueueEntry {
    pub position: u32,
    pub track: TrackItem,
}

/// Entries `offset..offset + entries.len()` of the queue in listening order, as of `version`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QueuePage {
    /// The queue version this page was read from (`PlaybackSnapshot::queue_version`).
    pub version: u64,
    pub total: u32,
    /// Position of the current entry.
    pub current: Option<u32>,
    pub offset: u32,
    pub entries: Vec<QueueEntry>,
    /// «N brani · durata», for the head of the list.
    pub summary: String,
}

#[derive(Default)]
pub(crate) struct QueueModel {
    queue: Arc<Queue>,
    version: u64,
}

impl QueueModel {
    /// The queue the core just published, and its version: the same object keeps its version.
    pub(crate) fn observe(&mut self, queue: &Arc<Queue>) -> u64 {
        if !Arc::ptr_eq(&self.queue, queue) {
            self.queue = Arc::clone(queue);
            self.version += 1;
        }
        self.version
    }

    /// The track at `position` of the listening order, if the queue is still the one of `version`.
    pub(crate) fn track(&self, version: u64, position: usize) -> Option<psytify_core::api::Track> {
        if version != self.version {
            return None;
        }
        self.queue.entries().nth(position).map(|(_, track)| track.clone())
    }

    pub(crate) fn page(&self, offset: u32, limit: u32) -> QueuePage {
        let queue = &self.queue;
        let entries = queue
            .entries()
            .skip(offset as usize)
            .take(limit.min(MAX_PAGE) as usize)
            .map(|(position, track)| QueueEntry { position: count(position), track: TrackItem::of(track) })
            .collect();
        QueuePage {
            version: self.version,
            total: count(queue.len()),
            current: queue.position().map(count),
            offset,
            entries,
            summary: format!(
                "{} · {}",
                format::tracks(queue.len()),
                format::total(queue.total_duration_ms())
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::track;

    fn published(n: u128, start: usize) -> Arc<Queue> {
        let mut queue = Queue::default();
        queue.replace((0..n).map(track).collect(), start, 7);
        Arc::new(queue)
    }

    #[test]
    fn the_version_grows_only_when_the_core_publishes_another_queue() {
        let mut model = QueueModel::default();
        let first = published(3, 0);
        assert_eq!(model.observe(&first), 1);
        assert_eq!(model.observe(&Arc::clone(&first)), 1, "the same queue republished");
        assert_eq!(model.observe(&published(3, 0)), 2);
    }

    #[test]
    fn a_page_is_a_window_of_the_listening_order_with_the_current_entry() {
        let mut model = QueueModel::default();
        model.observe(&published(250, 4));
        let page = model.page(2, 3);
        assert_eq!((page.total, page.current, page.offset), (250, Some(4), 2));
        let positions: Vec<u32> = page.entries.iter().map(|entry| entry.position).collect();
        assert_eq!(positions, [2, 3, 4]);
        assert_eq!(page.entries[0].track, TrackItem::of(&track(2)));
        assert_eq!(model.page(0, 1000).entries.len(), MAX_PAGE as usize, "pages are bounded");
        assert!(model.page(250, 10).entries.is_empty(), "past the end");
        assert!(page.summary.starts_with("250 brani · "), "{}", page.summary);
    }

    #[test]
    fn an_empty_queue_has_no_current_entry() {
        let page = QueueModel::default().page(0, 10);
        assert_eq!((page.version, page.total, page.current), (0, 0, None));
        assert_eq!(page.summary, "0 brani · 0 min");
    }
}
