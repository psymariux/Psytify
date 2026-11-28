//! Tracks kept "warm": the head of what will probably be played soon (the track the mouse stops on,
//! the next ones in the queue, the first of an album). A click on Play then finds the first
//! seconds already in memory and the track starts without waiting for the network.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::stream::StreamBuffer;
use crate::api::Quality;

/// Warm tracks kept at most: each weighs about `WARM_BYTES` until it plays.
const MAX_WARM: usize = 5;
/// Past this age the signed URL and the connection are considered expired.
const MAX_AGE: Duration = Duration::from_secs(4 * 60);

#[derive(Debug)]
struct Entry {
    id: Uuid,
    quality: Quality,
    stream: StreamBuffer,
    since: Instant,
}

impl Entry {
    fn is_usable(&self) -> bool {
        !self.stream.is_cancelled() && !self.stream.is_failed() && self.since.elapsed() < MAX_AGE
    }
}

#[derive(Debug, Default)]
pub struct WarmCache {
    entries: Vec<Entry>,
    /// Tracks being opened: one is not requested twice.
    pending: HashSet<(Uuid, Quality)>,
}

impl WarmCache {
    /// The track is neither warm nor being opened: it is worth opening.
    pub fn wants(&self, id: Uuid, quality: Quality) -> bool {
        !self.pending.contains(&(id, quality))
            && !self.entries.iter().any(|e| e.id == id && e.quality == quality && e.is_usable())
    }

    /// Marks the track as being opened. `false` if it does not need opening.
    pub fn begin(&mut self, id: Uuid, quality: Quality) -> bool {
        if !self.wants(id, quality) {
            return false;
        }
        self.entries.retain(|e| !(e.id == id && e.quality == quality));
        self.pending.insert((id, quality))
    }

    /// The track has arrived (or the opening failed: it will be retried next time).
    pub fn finish(&mut self, id: Uuid, quality: Quality, stream: Option<StreamBuffer>) {
        self.pending.remove(&(id, quality));
        let Some(stream) = stream else { return };
        self.entries.push(Entry { id, quality, stream, since: Instant::now() });
        self.entries.retain(|e| e.is_usable() || e.stream.is_cancelled());
        while self.entries.len() > MAX_WARM {
            self.entries.remove(0).stream.cancel();
        }
    }

    /// The track's buffer, if it is warm and still good. It leaves the cache: from here its player owns it.
    pub fn take(&mut self, id: Uuid, quality: Quality) -> Option<StreamBuffer> {
        let at = self.entries.iter().position(|e| e.id == id && e.quality == quality)?;
        let entry = self.entries.remove(at);
        entry.is_usable().then(|| {
            entry.stream.release();
            entry.stream
        })
    }

    /// Lets the download of a warm track continue to the end (the next in the queue, close
    /// to the end of the track playing). `true` if the track was warm.
    pub fn release(&self, id: Uuid, quality: Quality) -> bool {
        self.entries.iter().find(|e| e.id == id && e.quality == quality && e.is_usable()).is_some_and(|e| {
            e.stream.release();
            true
        })
    }

    /// Stops everything: no connection stays open for tracks that are no longer needed.
    pub fn clear(&mut self) {
        for entry in self.entries.drain(..) {
            entry.stream.cancel();
        }
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q: Quality = Quality::Lossless;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn a_track_is_requested_once() {
        let mut cache = WarmCache::default();
        assert!(cache.begin(id(1), Q));
        assert!(!cache.begin(id(1), Q), "already opening");
        cache.finish(id(1), Q, Some(StreamBuffer::new()));
        assert!(!cache.begin(id(1), Q), "already warm");
    }

    #[test]
    fn a_failed_opening_can_be_retried() {
        let mut cache = WarmCache::default();
        assert!(cache.begin(id(1), Q));
        cache.finish(id(1), Q, None);
        assert!(cache.begin(id(1), Q));
    }

    #[test]
    fn taking_a_warm_track_hands_it_over_and_empties_the_slot() {
        let mut cache = WarmCache::default();
        cache.begin(id(1), Q);
        cache.finish(id(1), Q, Some(StreamBuffer::new()));
        assert!(cache.take(id(1), Q).is_some());
        assert!(cache.take(id(1), Q).is_none());
    }

    #[test]
    fn a_cancelled_or_failed_buffer_is_not_handed_over() {
        let mut cache = WarmCache::default();
        let stream = StreamBuffer::new();
        stream.fail();
        cache.begin(id(1), Q);
        cache.finish(id(1), Q, Some(stream));
        assert!(cache.take(id(1), Q).is_none());
    }

    #[test]
    fn the_oldest_track_is_cancelled_beyond_the_limit() {
        let mut cache = WarmCache::default();
        let first = StreamBuffer::new();
        cache.begin(id(0), Q);
        cache.finish(id(0), Q, Some(first.clone()));
        for n in 1..=MAX_WARM as u128 {
            cache.begin(id(n), Q);
            cache.finish(id(n), Q, Some(StreamBuffer::new()));
        }
        assert!(first.is_cancelled());
        assert!(cache.take(id(0), Q).is_none());
        assert!(cache.take(id(MAX_WARM as u128), Q).is_some());
    }

    #[test]
    fn another_quality_is_another_track() {
        let mut cache = WarmCache::default();
        cache.begin(id(1), Q);
        cache.finish(id(1), Q, Some(StreamBuffer::new()));
        assert!(cache.wants(id(1), Quality::Standard));
    }

    #[test]
    fn clearing_cancels_every_download() {
        let mut cache = WarmCache::default();
        let stream = StreamBuffer::new();
        cache.begin(id(1), Q);
        cache.finish(id(1), Q, Some(stream.clone()));
        cache.clear();
        assert!(stream.is_cancelled() && cache.wants(id(1), Q));
    }
}
