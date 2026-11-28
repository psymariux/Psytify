//! Playback queue: tracks, listening order, shuffle and repeat.
//!
//! Shuffle only changes `order`, a permutation of the indices of `tracks`: turning it off
//! restores the original order without losing the current track.

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

/// Why we move to the next track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// The track ended by itself: `Repeat::One` repeats it.
    Finished,
    /// The user pressed "successivo": we go forward even with `Repeat::One`.
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
    /// Replaces the queue and starts from the track `start` (index in `tracks`).
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

    /// Moves to the next track. `None` means the end of the queue.
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

    /// The track that will play at the end of the current one, without moving: the same as
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

    /// Goes back to the previous track; on the first track it restarts it (or goes to the last with repeat).
    pub fn previous(&mut self) -> Option<&Track> {
        let position = self.position?;
        self.position = Some(match position {
            0 if self.repeat == Repeat::All => self.order.len() - 1,
            0 => 0,
            p => p - 1,
        });
        self.current()
    }

    /// Jumps to a position in the listening order.
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

    /// Puts the track right after the current one ("Riproduci dopo"). With an empty queue it becomes the current.
    pub fn enqueue_next(&mut self, track: Track) {
        let Some(position) = self.position else {
            self.enqueue(track);
            return;
        };
        self.order.insert(position + 1, self.tracks.len());
        self.tracks.push(track);
    }

    /// Removes the track at that position of the order. Returns `true` if it was the current one.
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

    /// Moves the entry at `from` to `to` in the listening order (drag reorder). The current track
    /// stays the same, wherever it ends up. `false` if a position does not exist or there is
    /// nothing to move.
    pub fn move_entry(&mut self, from: usize, to: usize) -> bool {
        let len = self.order.len();
        if from >= len || to >= len || from == to {
            return false;
        }
        let item = self.order.remove(from);
        self.order.insert(to, item);
        if let Some(position) = self.position {
            self.position = Some(if position == from {
                to
            } else if from < position && position <= to {
                position - 1
            } else if to <= position && position < from {
                position + 1
            } else {
                position
            });
        }
        true
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

    /// Tracks in listening order, with their position.
    pub fn entries(&self) -> impl Iterator<Item = (usize, &Track)> {
        self.order.iter().enumerate().filter_map(|(pos, &i)| self.tracks.get(i).map(|t| (pos, t)))
    }

    /// Tracks after the current one, in listening order.
    pub fn upcoming(&self) -> impl Iterator<Item = (usize, &Track)> {
        let from = self.position.map_or(0, |p| p + 1);
        self.entries().skip(from)
    }

    /// Applies `change` to each track; `true` if at least one call changed something.
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

    /// Repairs a queue read from disk: invalid order or position out of range.
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

    /// Puts `first` at the top and shuffles the rest.
    fn shuffle_around(&mut self, first: usize, seed: u64) {
        self.order.retain(|&i| i != first);
        shuffle_slice(&mut self.order, seed);
        self.order.insert(0, first);
        self.position = Some(0);
    }
}

/// Fisher-Yates with SplitMix64: deterministic given the seed, no external dependency.
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

    fn order_ids(queue: &Queue) -> Vec<u128> {
        queue.entries().map(|(_, t)| t.id.as_u128()).collect()
    }

    #[test]
    fn moving_an_upcoming_entry_reorders_without_losing_the_current_track() {
        let mut queue = queue_of(5, 1);
        assert!(queue.move_entry(4, 2));
        assert_eq!(order_ids(&queue), vec![1, 2, 5, 3, 4]);
        assert_eq!(current_id(&queue), Some(2));
        assert!(queue.move_entry(2, 4));
        assert_eq!(order_ids(&queue), vec![1, 2, 3, 4, 5]);
        assert_eq!(current_id(&queue), Some(2));
    }

    #[test]
    fn moving_across_the_current_track_keeps_it_current() {
        let mut queue = queue_of(5, 2);
        // A track that comes after moves ahead of the current one: the current's position goes down.
        assert!(queue.move_entry(4, 0));
        assert_eq!(order_ids(&queue), vec![5, 1, 2, 3, 4]);
        assert_eq!(current_id(&queue), Some(3));
        assert_eq!(queue.position(), Some(3));
        // And the other way round.
        assert!(queue.move_entry(0, 4));
        assert_eq!(current_id(&queue), Some(3));
        assert_eq!(queue.position(), Some(2));
    }

    #[test]
    fn moving_the_current_track_moves_the_position_with_it() {
        let mut queue = queue_of(4, 1);
        assert!(queue.move_entry(1, 3));
        assert_eq!(order_ids(&queue), vec![1, 3, 4, 2]);
        assert_eq!(current_id(&queue), Some(2));
        assert_eq!(queue.position(), Some(3));
    }

    #[test]
    fn moving_with_bad_positions_does_nothing() {
        let mut queue = queue_of(3, 0);
        assert!(!queue.move_entry(1, 1));
        assert!(!queue.move_entry(0, 3));
        assert!(!queue.move_entry(7, 0));
        assert_eq!(order_ids(&queue), vec![1, 2, 3]);
        assert!(!Queue::default().move_entry(0, 0));
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
