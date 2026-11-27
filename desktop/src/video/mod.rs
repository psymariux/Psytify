//! Il videoclip nel desktop GPUI (feature `video`, solo Linux con GStreamer): la metà «di affari» è del
//! core (`VideoCmd`, `VideoView`: quale videoclip, quando chiederlo, il passaggio da 360p a 720p, i
//! guasti), qui c'è il resto, come da `docs/transformation/05-video-spike.md` §5 (opzione A).
//!
//! - `pipeline`: GStreamer, solo video, ridotto ai pixel del riquadro e a 30 fotogrammi al secondo, BGRA.
//! - `slot`: il posto dell'ultimo fotogramma fra GStreamer e la UI (mai una coda) e una sveglia per
//!   fotogramma nuovo.
//! - `size`: quanto grande decodificare (i pixel fisici del riquadro, mai oltre il videoclip né 720p).
//! - `plan`: le decisioni pure: quando una pipeline esiste, apri / affianca / chiudi, l'applicazione delle
//!   regole di sincronia del core (`psytify_core::video::Alignment`) e il ritmo del controllo dello scarto.
//! - `stage`: l'entità GPUI che tiene tutto insieme e disegna il fotogramma (`Arc<RenderImage>` nuova a
//!   ogni fotogramma, la precedente tolta dall'atlante nello stesso disegno).
//!
//! Una pipeline esiste solo per il brano in corso, mentre suona e mentre il pannello o la vista immersiva
//! mostrano il videoclip; altrimenti niente pipeline, niente thread, niente timer, niente fotogrammi.

pub mod pipeline;
pub mod plan;
pub mod size;
pub mod slot;
pub mod stage;

use psytify_core::snapshot::{ClipState, VideoView};
use uuid::Uuid;

/// Un indirizzo da aprire, con il suo numero d'ordine e l'altezza. L'indirizzo è firmato e a scadenza: va
/// solo alla pipeline, mai in un log (il `Debug` non lo stampa).
#[derive(Clone, PartialEq, Eq)]
pub struct Source {
    pub seq: u64,
    pub height: Option<u32>,
    url: String,
}

impl Source {
    /// Un indirizzo locale (`file://…`), per i dati di prova.
    #[cfg(feature = "fixtures")]
    pub fn local(seq: u64, url: String) -> Self {
        Self { seq, height: None, url }
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Source").field("seq", &self.seq).field("height", &self.height).finish_non_exhaustive()
    }
}

/// A che punto è il videoclip del brano di cui parla il core, per il desktop.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Clip {
    /// Nessuno lo mostra, o il brano non ha un videoclip.
    #[default]
    Idle,
    /// Il server lo prepara (la prima volta lo scarica).
    Loading,
    /// L'indirizzo da aprire.
    Ready(Source),
    /// Non si è aperto: il testo dell'errore, per «Riprova».
    Failed(String),
}

/// La vista del video come la legge il desktop: quella del core o, con `--fixture video-*`, una di prova.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipView {
    pub track: Option<Uuid>,
    pub clip: Clip,
    /// Cresce a ogni videoclip che si ferma: un avviso per ogni aumento.
    pub failures: u64,
}

impl ClipView {
    pub fn of(view: &VideoView) -> Self {
        let clip = match &view.state {
            ClipState::Idle => Clip::Idle,
            ClipState::Loading => Clip::Loading,
            ClipState::Ready(clip) => {
                Clip::Ready(Source { seq: clip.seq, height: clip.height, url: clip.url().to_owned() })
            }
            ClipState::Failed(error) => Clip::Failed(error.to_string()),
        };
        Self { track: view.track, clip, failures: view.failures }
    }

    /// Il videoclip pronto per `track`, se c'è.
    pub fn ready_for(&self, track: Uuid) -> Option<&Source> {
        match &self.clip {
            Clip::Ready(source) if self.track == Some(track) => Some(source),
            _ => None,
        }
    }

    /// Il videoclip di `track` non si è aperto: il testo dell'errore.
    pub fn failed_for(&self, track: Uuid) -> Option<&str> {
        match &self.clip {
            Clip::Failed(error) if self.track == Some(track) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_never_reaches_a_debug_line() {
        let source =
            Source { seq: 3, height: Some(360), url: "https://x.example/v1/media?t=SECRET".to_owned() };
        let line = format!("{:?}", ClipView { track: None, clip: Clip::Ready(source), failures: 0 });
        assert!(!line.contains("SECRET") && !line.contains("media"), "{line}");
        assert!(line.contains("seq: 3"));
    }

    #[test]
    fn a_clip_is_ready_or_failed_only_for_its_own_track() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let source = Source { seq: 1, height: None, url: String::new() };
        let ready = ClipView { track: Some(a), clip: Clip::Ready(source), failures: 0 };
        assert!(ready.ready_for(a).is_some() && ready.ready_for(b).is_none());
        let failed = ClipView { track: Some(a), clip: Clip::Failed("no".to_owned()), failures: 1 };
        assert_eq!(failed.failed_for(a), Some("no"));
        assert_eq!(failed.failed_for(b), None);
        assert!(failed.ready_for(a).is_none());
    }
}
