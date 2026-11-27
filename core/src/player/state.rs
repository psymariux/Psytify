//! Stato di riproduzione. Lo tiene il dominio `state/playback.rs` del core e lo pubblica nella sua
//! vista; ogni transizione avviene in risposta a un comando, all'esito di un lavoro o a un battito.

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

    /// C'è un brano che suona o è in pausa.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Playing | Self::Paused)
    }
}
