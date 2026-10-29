//! Coda di riproduzione: brani, ordine di ascolto, shuffle e repeat.
//!
//! Lo shuffle cambia solo `order`, una permutazione degli indici di `tracks`: disattivarlo
//! ripristina l'ordine originale senza perdere il brano corrente.

use serde::{Deserialize, Serialize};

use crate::api::Track;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

/// Perché si passa al brano successivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// Il brano è finito da solo: `Repeat::One` lo ripete.
    Finished,
    /// L'utente ha premuto "successivo": si va avanti anche con `Repeat::One`.
    Skip,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Queue {
    tracks: Vec<Track>,
    order: Vec<usize>,
    position: Option<usize>,
    shuffle: bool,
    repeat: Repeat,
}

impl Queue {
    /// Sostituisce la coda e parte dal brano `start` (indice in `tracks`).
    pub fn replace(&mut self, tracks: Vec<Track>, start: usize, seed: u64) {
        self.tracks = tracks;
        if self.tracks.is_empty() {
            self.order.clear();
            self.position = None;
            return;
        }
        let start = start.min(self.tracks.len() - 1);
        self.order = (0..self.tracks.len()).collect();
        if self.shuffle {
            self.shuffle_around(start, seed);
        } else {
            self.position = Some(start);
        }
    }

    pub fn current(&self) -> Option<&Track> {
        let index = *self.order.get(self.position?)?;
        self.tracks.get(index)
    }

    pub fn position(&self) -> Option<usize> {
        self.position
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    /// Passa al brano successivo. `None` vuol dire fine della coda.
    pub fn advance(&mut self, reason: Advance) -> Option<&Track> {
        let position = self.position?;
        if reason == Advance::Finished && self.repeat == Repeat::One {
            return self.current();
        }
        if position + 1 < self.order.len() {
            self.position = Some(position + 1);
        } else if self.repeat != Repeat::Off {
            self.position = Some(0);
        } else {
            return None;
        }
        self.current()
    }

    /// Il brano che partirà alla fine di quello corrente, senza spostarsi: lo stesso di
    /// `advance(Advance::Finished)`.
    pub fn peek_next(&self) -> Option<&Track> {
        let position = self.position?;
        let next = if self.repeat == Repeat::One {
            position
        } else if position + 1 < self.order.len() {
            position + 1
        } else if self.repeat != Repeat::Off {
            0
        } else {
            return None;
        };
        self.tracks.get(*self.order.get(next)?)
    }

    /// Torna al brano precedente; sul primo brano lo riparte (o va all'ultimo con repeat).
    pub fn previous(&mut self) -> Option<&Track> {
        let position = self.position?;
        self.position = Some(match position {
            0 if self.repeat == Repeat::All => self.order.len() - 1,
            0 => 0,
            p => p - 1,
        });
        self.current()
    }

    /// Salta a una posizione dell'ordine di ascolto.
    pub fn jump(&mut self, order_position: usize) -> Option<&Track> {
        if order_position < self.order.len() {
            self.position = Some(order_position);
        }
        self.current()
    }

    pub fn enqueue(&mut self, track: Track) {
        self.order.push(self.tracks.len());
        self.tracks.push(track);
        if self.position.is_none() {
            self.position = Some(self.order.len() - 1);
        }
    }

    /// Mette il brano subito dopo quello corrente ("Riproduci dopo"). Con la coda vuota diventa il corrente.
    pub fn enqueue_next(&mut self, track: Track) {
        let Some(position) = self.position else {
            self.enqueue(track);
            return;
        };
        self.order.insert(position + 1, self.tracks.len());
        self.tracks.push(track);
    }

    /// Toglie il brano in quella posizione dell'ordine. Restituisce `true` se era il corrente.
    pub fn remove(&mut self, order_position: usize) -> bool {
        let Some(&index) = self.order.get(order_position) else {
            return false;
        };
        self.tracks.remove(index);
        self.order.remove(order_position);
        for i in &mut self.order {
            if *i > index {
                *i -= 1;
            }
        }
        let Some(position) = self.position else {
            return false;
        };
        if self.order.is_empty() {
            self.position = None;
            return position == order_position;
        }
        match order_position.cmp(&position) {
            std::cmp::Ordering::Less => {
                self.position = Some(position - 1);
                false
            }
            std::cmp::Ordering::Equal => {
                self.position = Some(position.min(self.order.len() - 1));
                true
            }
            std::cmp::Ordering::Greater => false,
        }
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.order.clear();
        self.position = None;
    }

    pub fn set_shuffle(&mut self, on: bool, seed: u64) {
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        let current = self.position.and_then(|p| self.order.get(p).copied());
        self.order = (0..self.tracks.len()).collect();
        match current {
            Some(index) if on => self.shuffle_around(index, seed),
            Some(index) => self.position = Some(index),
            None if on => shuffle_slice(&mut self.order, seed),
            None => {}
        }
    }

    pub fn cycle_repeat(&mut self) {
        self.repeat = self.repeat.cycle();
    }

    /// Brani nell'ordine di ascolto, con la loro posizione.
    pub fn entries(&self) -> impl Iterator<Item = (usize, &Track)> {
        self.order.iter().enumerate().filter_map(|(pos, &i)| self.tracks.get(i).map(|t| (pos, t)))
    }

    /// Brani dopo quello corrente, nell'ordine di ascolto.
    pub fn upcoming(&self) -> impl Iterator<Item = (usize, &Track)> {
        let from = self.position.map_or(0, |p| p + 1);
        self.entries().skip(from)
    }

    /// Applica `change` a ogni brano; `true` se almeno una chiamata ha cambiato qualcosa.
    pub fn update_tracks(&mut self, mut change: impl FnMut(&mut Track) -> bool) -> bool {
        let mut changed = false;
        for track in &mut self.tracks {
            changed |= change(track);
        }
        changed
    }

    pub fn tracks_in_order(&self) -> Vec<Track> {
        self.entries().map(|(_, t)| t.clone()).collect()
    }

    pub fn total_duration_ms(&self) -> u64 {
        self.tracks.iter().map(|t| t.duration_ms).sum()
    }

    /// Ripara una coda letta da disco: ordine non valido o posizione fuori intervallo.
    pub fn sanitize(&mut self) {
        let mut sorted = self.order.clone();
        sorted.sort_unstable();
        if sorted != (0..self.tracks.len()).collect::<Vec<_>>() {
            self.order = (0..self.tracks.len()).collect();
            self.position = None;
        }
        if self.position.is_some_and(|p| p >= self.order.len()) {
            self.position = None;
        }
        if self.position.is_none() && !self.order.is_empty() {
            self.position = Some(0);
        }
    }

    /// Mette `first` in testa e mescola il resto.
    fn shuffle_around(&mut self, first: usize, seed: u64) {
        self.order.retain(|&i| i != first);
        shuffle_slice(&mut self.order, seed);
        self.order.insert(0, first);
        self.position = Some(0);
    }
}

/// Fisher-Yates con SplitMix64: deterministico dato il seme, nessuna dipendenza esterna.
fn shuffle_slice<T>(items: &mut [T], seed: u64) {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for i in (1..items.len()).rev() {
        let j = usize::try_from(next() % (i as u64 + 1)).unwrap_or(0);
        items.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures::track;

    fn queue_of(n: u128, start: usize) -> Queue {
        let mut queue = Queue::default();
        queue.replace((1..=n).map(track).collect(), start, 42);
        queue
    }

    fn current_id(queue: &Queue) -> Option<u128> {
        queue.current().map(|t| t.id.as_u128())
    }

    #[test]
    fn updating_tracks_reports_whether_anything_changed() {
        let mut queue = queue_of(3, 0);
        assert!(!queue.update_tracks(|_| false));
        assert!(queue.update_tracks(|t| {
            t.cover_url = Some("x".into());
            true
        }));
        assert!(queue.entries().all(|(_, t)| t.cover_url.as_deref() == Some("x")));
    }

    #[test]
    fn replace_starts_at_requested_track() {
        assert_eq!(current_id(&queue_of(5, 2)), Some(3));
    }

    #[test]
    fn enqueue_next_plays_right_after_the_current_track() {
        let mut queue = queue_of(3, 0);
        queue.enqueue_next(track(9));
        let order: Vec<u128> = queue.entries().map(|(_, t)| t.id.as_u128()).collect();
        assert_eq!((order, current_id(&queue)), (vec![1, 9, 2, 3], Some(1)));
    }

    #[test]
    fn enqueue_next_on_an_empty_queue_becomes_the_current_track() {
        let mut queue = Queue::default();
        queue.enqueue_next(track(4));
        assert_eq!(current_id(&queue), Some(4));
    }

    #[test]
    fn replace_with_empty_list_clears_position() {
        let mut queue = queue_of(3, 0);
        queue.replace(Vec::new(), 0, 1);
        assert_eq!(queue.current(), None);
    }

    #[test]
    fn advance_moves_to_next_track() {
        let mut queue = queue_of(3, 0);
        queue.advance(Advance::Skip);
        assert_eq!(current_id(&queue), Some(2));
    }

    #[test]
    fn peek_next_agrees_with_advance_in_every_repeat_mode() {
        for (repeats, start) in [(0, 0), (0, 2), (1, 2), (2, 1)] {
            let mut queue = queue_of(3, start);
            for _ in 0..repeats {
                queue.cycle_repeat();
            }
            let peeked = queue.peek_next().map(|t| t.id.as_u128());
            assert_eq!(peeked, queue.advance(Advance::Finished).map(|t| t.id.as_u128()));
        }
    }

    #[test]
    fn advance_at_end_without_repeat_ends_the_queue() {
        let mut queue = queue_of(3, 2);
        assert!(queue.advance(Advance::Finished).is_none());
    }

    #[test]
    fn repeat_all_wraps_to_first_track() {
        let mut queue = queue_of(3, 2);
        queue.cycle_repeat();
        queue.advance(Advance::Finished);
        assert_eq!(current_id(&queue), Some(1));
    }

    #[test]
    fn repeat_one_replays_when_track_finishes() {
        let mut queue = queue_of(3, 1);
        queue.cycle_repeat();
        queue.cycle_repeat();
        queue.advance(Advance::Finished);
        assert_eq!(current_id(&queue), Some(2));
    }

    #[test]
    fn repeat_one_still_skips_on_user_next() {
        let mut queue = queue_of(3, 1);
        queue.cycle_repeat();
        queue.cycle_repeat();
        queue.advance(Advance::Skip);
        assert_eq!(current_id(&queue), Some(3));
    }

    #[test]
    fn previous_on_first_track_restarts_it() {
        let mut queue = queue_of(3, 0);
        queue.previous();
        assert_eq!(current_id(&queue), Some(1));
    }

    #[test]
    fn previous_with_repeat_all_goes_to_last() {
        let mut queue = queue_of(3, 0);
        queue.cycle_repeat();
        queue.previous();
        assert_eq!(current_id(&queue), Some(3));
    }

    #[test]
    fn shuffle_keeps_current_track_first() {
        let mut queue = queue_of(10, 4);
        queue.set_shuffle(true, 7);
        assert_eq!((queue.position(), current_id(&queue)), (Some(0), Some(5)));
    }

    #[test]
    fn shuffle_keeps_every_track_once() {
        let mut queue = queue_of(10, 4);
        queue.set_shuffle(true, 7);
        let mut ids: Vec<u128> = queue.entries().map(|(_, t)| t.id.as_u128()).collect();
        ids.sort_unstable();
        assert_eq!(ids, (1..=10).collect::<Vec<_>>());
    }

    #[test]
    fn shuffle_changes_the_order() {
        let mut queue = queue_of(10, 0);
        queue.set_shuffle(true, 7);
        let ids: Vec<u128> = queue.entries().map(|(_, t)| t.id.as_u128()).collect();
        assert_ne!(ids, (1..=10).collect::<Vec<_>>());
    }

    #[test]
    fn shuffle_off_restores_order_and_keeps_current() {
        let mut queue = queue_of(10, 4);
        queue.set_shuffle(true, 7);
        queue.advance(Advance::Skip);
        let playing = current_id(&queue);
        queue.set_shuffle(false, 0);
        let ids: Vec<u128> = queue.entries().map(|(_, t)| t.id.as_u128()).collect();
        assert_eq!((ids, current_id(&queue)), ((1..=10).collect::<Vec<_>>(), playing));
    }

    #[test]
    fn remove_before_current_keeps_current() {
        let mut queue = queue_of(4, 2);
        assert!(!queue.remove(0));
        assert_eq!(current_id(&queue), Some(3));
    }

    #[test]
    fn remove_current_moves_to_the_next_track() {
        let mut queue = queue_of(4, 1);
        assert!(queue.remove(1));
        assert_eq!(current_id(&queue), Some(3));
    }

    #[test]
    fn remove_current_last_track_moves_back() {
        let mut queue = queue_of(3, 2);
        queue.remove(2);
        assert_eq!(current_id(&queue), Some(2));
    }

    #[test]
    fn remove_only_track_empties_the_queue() {
        let mut queue = queue_of(1, 0);
        queue.remove(0);
        assert_eq!((queue.len(), queue.current()), (0, None));
    }

    #[test]
    fn remove_after_shuffle_keeps_order_consistent() {
        let mut queue = queue_of(6, 0);
        queue.set_shuffle(true, 3);
        queue.remove(3);
        let mut indices = queue.order.clone();
        indices.sort_unstable();
        assert_eq!(indices, (0..5).collect::<Vec<_>>());
    }

    #[test]
    fn enqueue_on_empty_queue_sets_current() {
        let mut queue = Queue::default();
        queue.enqueue(track(9));
        assert_eq!(current_id(&queue), Some(9));
    }

    #[test]
    fn upcoming_lists_tracks_after_current() {
        let queue = queue_of(4, 1);
        let ids: Vec<u128> = queue.upcoming().map(|(_, t)| t.id.as_u128()).collect();
        assert_eq!(ids, vec![3, 4]);
    }

    #[test]
    fn sanitize_repairs_a_corrupted_order() {
        let mut queue = queue_of(3, 1);
        queue.order = vec![0, 0, 7];
        queue.position = Some(9);
        queue.sanitize();
        assert_eq!((queue.order.clone(), queue.position()), (vec![0, 1, 2], Some(0)));
    }

    #[test]
    fn queue_round_trips_through_json() {
        let mut queue = queue_of(3, 1);
        queue.set_shuffle(true, 5);
        let json = serde_json::to_string(&queue).unwrap();
        assert_eq!(serde_json::from_str::<Queue>(&json).unwrap(), queue);
    }
}
