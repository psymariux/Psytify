//! Stato di riproduzione. Ogni transizione avviene in `App::update` in risposta a un `Message`.

use std::time::Duration;

use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Loading {
        track_id: Uuid,
    },
    Playing,
    Paused,
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Playback {
    pub state: PlaybackState,
    pub position: Duration,
    /// Valore dello slider durante il trascinamento: il seek parte al rilascio.
    pub scrubbing: Option<f32>,
    pub volume: f32,
    /// Aumenta a ogni caricamento: le risposte di caricamenti superati vengono ignorate.
    pub generation: u64,
}

impl Playback {
    pub fn new(volume: f32) -> Self {
        Self {
            state: PlaybackState::Stopped,
            position: Duration::ZERO,
            scrubbing: None,
            volume: volume.clamp(0.0, 1.0),
            generation: 0,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.state == PlaybackState::Playing
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, PlaybackState::Playing | PlaybackState::Paused)
    }

    /// Inizia un nuovo caricamento e ne restituisce il numero.
    pub fn begin_loading(&mut self, track_id: Uuid) -> u64 {
        self.generation += 1;
        self.state = PlaybackState::Loading { track_id };
        self.position = Duration::ZERO;
        self.scrubbing = None;
        self.generation
    }

    pub fn is_current(&self, generation: u64) -> bool {
        generation == self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_load_is_not_current() {
        let mut playback = Playback::new(0.8);
        let first = playback.begin_loading(Uuid::nil());
        playback.begin_loading(Uuid::nil());
        assert!(!playback.is_current(first));
    }

    #[test]
    fn volume_is_clamped() {
        assert_eq!(Playback::new(3.0).volume, 1.0);
    }
}
