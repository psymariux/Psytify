//! Brani tenuti "caldi": la testata di ciò che probabilmente si ascolterà tra poco (il brano su cui
//! si ferma il mouse, i prossimi della coda, il primo di un album). Il clic su Riproduci trova già i
//! primi secondi in memoria e il brano parte senza aspettare la rete.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::stream::StreamBuffer;
use crate::api::Quality;

/// Brani caldi tenuti al massimo: ognuno pesa circa `WARM_BYTES` finché non suona.
const MAX_WARM: usize = 5;
/// Oltre questa età l'URL firmato e la connessione sono da considerare scaduti.
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
    /// Brani in apertura: non se ne chiede uno due volte.
    pending: HashSet<(Uuid, Quality)>,
}

impl WarmCache {
    /// Il brano non è caldo né in apertura: conviene aprirlo.
    pub fn wants(&self, id: Uuid, quality: Quality) -> bool {
        !self.pending.contains(&(id, quality))
            && !self.entries.iter().any(|e| e.id == id && e.quality == quality && e.is_usable())
    }

    /// Segna il brano come in apertura. `false` se non serve aprirlo.
    pub fn begin(&mut self, id: Uuid, quality: Quality) -> bool {
        if !self.wants(id, quality) {
            return false;
        }
        self.entries.retain(|e| !(e.id == id && e.quality == quality));
        self.pending.insert((id, quality))
    }

    /// Il brano è arrivato (o l'apertura è fallita: si riproverà alla prossima occasione).
    pub fn finish(&mut self, id: Uuid, quality: Quality, stream: Option<StreamBuffer>) {
        self.pending.remove(&(id, quality));
        let Some(stream) = stream else { return };
        self.entries.push(Entry { id, quality, stream, since: Instant::now() });
        self.entries.retain(|e| e.is_usable() || e.stream.is_cancelled());
        while self.entries.len() > MAX_WARM {
            self.entries.remove(0).stream.cancel();
        }
    }

    /// Il buffer del brano, se è caldo e ancora buono. Esce dalla cache: da qui lo possiede chi lo suona.
    pub fn take(&mut self, id: Uuid, quality: Quality) -> Option<StreamBuffer> {
        let at = self.entries.iter().position(|e| e.id == id && e.quality == quality)?;
        let entry = self.entries.remove(at);
        entry.is_usable().then(|| {
            entry.stream.release();
            entry.stream
        })
    }

    /// Fa continuare il download di un brano caldo fino alla fine (il prossimo della coda, vicino
    /// alla fine del brano che suona). `true` se il brano era caldo.
    pub fn release(&self, id: Uuid, quality: Quality) -> bool {
        self.entries.iter().find(|e| e.id == id && e.quality == quality && e.is_usable()).is_some_and(|e| {
            e.stream.release();
            true
        })
    }

    /// Ferma tutto: nessuna connessione resta aperta per brani che non servono più.
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
