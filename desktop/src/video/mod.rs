//! The music video in the GPUI desktop (feature `video`, Linux only with GStreamer): the "business" half is in the
//! core (`VideoCmd`, `VideoView`: which video, when to ask for it, the switch from 360p to 720p, the
//! failures); here is the rest, as in `docs/transformation/05-video-spike.md` §5 (option A).
//!
//! - `pipeline`: GStreamer, video only, reduced to the pixels of the box and to 30 frames per second, BGRA.
//! - `slot`: the place of the last frame between GStreamer and the UI (never a queue) and a wake-up for
//!   new frame.
//! - `size`: how big to decode (the physical pixels of the box, never beyond the video nor 720p).
//! - `plan`: the pure decisions: when a pipeline exists, open / alongside / close, the application of the
//!   core's sync rules (`psytify_core::video::Alignment`) and the rhythm of the drift check.
//! - `stage`: the GPUI entity that holds everything together and draws the frame (a new `Arc<RenderImage>` per
//!   frame, the previous one removed from the atlas in the same draw).
//!
//! A pipeline exists only for the track playing, while it plays and while the panel or the immersive view
//! show the video; otherwise no pipeline, no thread, no timer, no frames.

pub mod pipeline;
pub mod plan;
pub mod size;
pub mod slot;
pub mod stage;

use psytify_core::snapshot::{ClipState, VideoView};
use uuid::Uuid;

/// An address to open, with its sequence number and height. The address is signed and expires: it goes
/// only to the pipeline, never into a log (the `Debug` does not print it).
#[derive(Clone, PartialEq, Eq)]
pub struct Source {
    pub seq: u64,
    pub height: Option<u32>,
    url: String,
}

impl Source {
    /// A local address (`file://…`), for the test data.
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

/// Where the track's video is, as the core describes it, for the desktop.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Clip {
    /// Nobody shows it, or the track has no video.
    #[default]
    Idle,
    /// The server is preparing it (the first time it downloads it).
    Loading,
    /// L'indirizzo da aprire.
    Ready(Source),
    /// It did not open: the error text, for «Riprova».
    Failed(String),
}

/// The video view as the desktop reads it: the core's or, with `--fixture video-*`, a test one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipView {
    pub track: Option<Uuid>,
    pub clip: Clip,
    /// Grows with every video that stops: a notice for each increase.
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

    /// The video ready for `track`, if there is one.
    pub fn ready_for(&self, track: Uuid) -> Option<&Source> {
        match &self.clip {
            Clip::Ready(source) if self.track == Some(track) => Some(source),
            _ => None,
        }
    }

    /// The video of `track` did not open: the error text.
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
