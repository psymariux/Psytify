//! The place of the last frame: between the GStreamer thread that decodes and the UI that draws there is
//! one place only, not a queue. A new frame takes the place of the one the UI has not taken yet
//! (which is dropped and returned as a buffer to reuse), so a late UI skips frames and does not accumulate
//! accumula mai: la memoria resta quella di due fotogrammi al massimo, qualunque cosa succeda.
//!
//! The UI's wake-up starts only when the place goes from empty to full: between two draws, however many
//! frames arrive, the UI wakes up only once.

use std::sync::{Mutex, PoisonError};

/// A ready frame to draw: `width` x `height` BGRA pixels (the order GPUI's atlas wants),
/// righe contigue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Frame {
    /// I byte di un fotogramma di quella misura.
    pub fn len_of(width: u32, height: u32) -> usize {
        width as usize * height as usize * 4
    }
}

/// Che cosa ha fatto `put`.
#[derive(Debug, PartialEq, Eq)]
pub struct Put {
    /// The place was empty: the UI must be woken (once).
    pub wake: bool,
    /// The frame not yet taken, dropped: its buffer is reused for the next one.
    pub spare: Option<Vec<u8>>,
}

/// The place, shared between the decoding thread and the UI.
#[derive(Debug, Default)]
pub struct FrameSlot {
    latest: Mutex<Option<Frame>>,
}

impl FrameSlot {
    /// Puts the new frame in place of the one there was.
    pub fn put(&self, frame: Frame) -> Put {
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        let old = latest.replace(frame);
        Put { wake: old.is_none(), spare: old.map(|frame| frame.bgra) }
    }

    /// Takes the most recent frame, if there is one that the UI has not yet taken.
    pub fn take(&self) -> Option<Frame> {
        self.latest.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// There is a frame to take.
    pub fn is_full(&self) -> bool {
        self.latest.lock().unwrap_or_else(PoisonError::into_inner).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(n: u8) -> Frame {
        Frame { width: 2, height: 1, bgra: vec![n; Frame::len_of(2, 1)] }
    }

    #[test]
    fn the_slot_keeps_only_the_latest_frame_and_wakes_once() {
        let slot = FrameSlot::default();
        assert_eq!(slot.take(), None);
        // The first frame wakes the UI; the ones that arrive before the UI takes it do not, and each
        // returns the buffer of the one it overtakes.
        assert_eq!(slot.put(frame(1)), Put { wake: true, spare: None });
        for n in 2..=200 {
            let put = slot.put(frame(n));
            assert!(!put.wake, "un posto pieno non sveglia di nuovo");
            assert_eq!(put.spare, Some(vec![n - 1; 8]), "il fotogramma scavalcato torna come buffer");
        }
        // The UI, late, finds only the last one: no backlog.
        assert_eq!(slot.take(), Some(frame(200)));
        assert_eq!(slot.take(), None);
        assert!(!slot.is_full());
        // Once the frame is taken, the next one wakes again.
        assert!(slot.put(frame(7)).wake);
        assert!(slot.is_full());
    }

    #[test]
    fn a_frame_has_four_bytes_a_pixel() {
        assert_eq!(Frame::len_of(1280, 720), 3_686_400);
        assert_eq!(Frame::len_of(0, 720), 0);
    }
}
