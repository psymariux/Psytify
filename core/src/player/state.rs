//! Playback state. The `state/playback.rs` domain of the core holds it and publishes it in its
//! view; each transition happens in response to a command, to the outcome of a job, or to a tick.

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

impl PlaybackState {
    pub fn is_playing(&self) -> bool {
        *self == Self::Playing
    }

    /// There is a track playing or paused.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Playing | Self::Paused)
    }
}
