//! Il posto dell'ultimo fotogramma: fra il thread di GStreamer che decodifica e la UI che disegna c'è un
//! posto solo, non una coda. Un fotogramma nuovo prende il posto di quello che la UI non ha ancora preso
//! (che si scarta e torna come buffer da riusare), così una UI in ritardo salta fotogrammi e non ne
//! accumula mai: la memoria resta quella di due fotogrammi al massimo, qualunque cosa succeda.
//!
//! La sveglia della UI parte solo quando il posto passa da vuoto a pieno: fra due disegni, per quanti
//! fotogrammi arrivino, la UI si sveglia una volta sola.

use std::sync::{Mutex, PoisonError};

/// Un fotogramma pronto da disegnare: `width` x `height` pixel BGRA (l'ordine che vuole l'atlante di GPUI),
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
    /// Il posto era vuoto: la UI va svegliata (una volta).
    pub wake: bool,
    /// Il fotogramma non ancora preso, scartato: il suo buffer si riusa per il prossimo.
    pub spare: Option<Vec<u8>>,
}

/// Il posto, condiviso fra il thread che decodifica e la UI.
#[derive(Debug, Default)]
pub struct FrameSlot {
    latest: Mutex<Option<Frame>>,
}

impl FrameSlot {
    /// Mette il fotogramma nuovo al posto di quello che c'era.
    pub fn put(&self, frame: Frame) -> Put {
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        let old = latest.replace(frame);
        Put { wake: old.is_none(), spare: old.map(|frame| frame.bgra) }
    }

    /// Prende il fotogramma più recente, se ce n'è uno che la UI non ha ancora preso.
    pub fn take(&self) -> Option<Frame> {
        self.latest.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// C'è un fotogramma da prendere.
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
        // Il primo fotogramma sveglia la UI; quelli che arrivano prima che la UI lo prenda no, e ognuno
        // restituisce il buffer di quello che scavalca.
        assert_eq!(slot.put(frame(1)), Put { wake: true, spare: None });
        for n in 2..=200 {
            let put = slot.put(frame(n));
            assert!(!put.wake, "un posto pieno non sveglia di nuovo");
            assert_eq!(put.spare, Some(vec![n - 1; 8]), "il fotogramma scavalcato torna come buffer");
        }
        // La UI, in ritardo, trova solo l'ultimo: niente arretrato.
        assert_eq!(slot.take(), Some(frame(200)));
        assert_eq!(slot.take(), None);
        assert!(!slot.is_full());
        // Preso il fotogramma, il prossimo sveglia di nuovo.
        assert!(slot.put(frame(7)).wake);
        assert!(slot.is_full());
    }

    #[test]
    fn a_frame_has_four_bytes_a_pixel() {
        assert_eq!(Frame::len_of(1280, 720), 3_686_400);
        assert_eq!(Frame::len_of(0, 720), 0);
    }
}
