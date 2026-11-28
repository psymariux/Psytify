//! The video's GStreamer pipeline: video only (the audio stays the core's, which the video
//! follows), reduced to the pixels of the box and to 30 frames per second, in BGRA in the place of the last
//! fotogramma (`slot`).
//!
//! `uridecodebin ! videorate (max 30 fps, drops only) ! videoscale ! videoconvert ! capsfilter (BGRA,
//! misura) ! appsink (un buffer, scarta)`: the same family of pipelines as the earlier client,
//! but the conversion to BGRA is done by the CPU, because GPUI on Linux and Windows does not
//! accept YUV textures (`05-video-spike.md`, option A). Reducing before converting costs less: the
//! mini player converts a tenth of the pixels.
//!
//! The threads are GStreamer's (network, demuxer, decoders): they exist as long as the pipeline exists.
//! Opening (up to 5 seconds, the network) and closing (`Null` can wait for the network) block, so they are done
//! on another thread; pause, seek and position are fast and the UI asks for them. GStreamer's errors
//! may contain the signed address: they are never kept, they become only "did not open" or
//! "stopped".

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use psytify_core::player::Clock;
use psytify_core::video::{DRIFT, Measure, SETTLE};
use tokio::sync::Notify;

use super::plan::Controls;
use super::size::Size;
use super::slot::{Frame, FrameSlot};

/// Frames per second at most: a 60 fps video is halved in the pipeline (60 costs twice as much,
/// `05-video-spike.md` §4.1).
pub const MAX_FPS: u32 = 30;
/// How long it waits for the first frame to be ready (as the earlier player did).
const PREROLL: gst::ClockTime = gst::ClockTime::from_seconds(5);

/// The whole pipeline; `src`, `size` and `frames` are the names of the elements that are touched. The address
/// is not here: it is a property of `src`.
fn description() -> String {
    format!(
        "uridecodebin name=src ! videorate drop-only=true max-rate={MAX_FPS} ! videoscale ! videoconvert \
         ! capsfilter name=size caps=video/x-raw,format=BGRA,pixel-aspect-ratio=1/1 \
         ! appsink name=frames max-buffers=1 drop=true enable-last-sample=false"
    )
}

/// With this environment variable, at the close of each pipeline one line on stderr: frames copied
/// and shown, timings, duration (for the measures in `docs/PERFORMANCE.md`). Never the address.
const STATS_VAR: &str = "PSYTIFY_VIDEO_STATS";

/// NVIDIA decoders (NVDEC via CUDA): as before, rank zero, so a hybrid laptop
/// does not wake the dedicated card (and ~130 MB of VRAM) for a 360p or 720p video.
const NVIDIA_DECODERS: &str = "nvcodec";

/// What GStreamer's threads and the UI say to each other: the frame's place, the wake-up and two signals.
#[derive(Debug, Default)]
pub struct Shared {
    pub slot: FrameSlot,
    /// A wake-up for the UI: a new frame in the empty place, an error or the end. It holds at most
    /// one permit (`Notify`): the wake-ups do not pile up.
    pub wake: Notify,
    failed: AtomicBool,
    ended: AtomicBool,
    frames: AtomicU64,
    shown: AtomicU64,
}

impl Shared {
    /// The stream stopped (network or decoder error).
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    /// The video reached the end.
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    /// The UI took a frame from the place (for the `PSYTIFY_VIDEO_STATS` line).
    pub fn mark_shown(&self) {
        self.shown.fetch_add(1, Ordering::Relaxed);
    }
}

/// Copies a sample into the place, reusing the frame's buffer that the UI did not take.
struct Copier {
    shared: Arc<Shared>,
    spare: Mutex<Option<Vec<u8>>>,
}

impl Copier {
    fn copy(&self, sample: &gst::Sample) -> Result<gst::FlowSuccess, gst::FlowError> {
        let structure = sample.caps().and_then(|caps| caps.structure(0)).ok_or(gst::FlowError::Error)?;
        let width = structure.get::<i32>("width").map_err(|_| gst::FlowError::Error)?;
        let height = structure.get::<i32>("height").map_err(|_| gst::FlowError::Error)?;
        let (width, height) = (
            u32::try_from(width).map_err(|_| gst::FlowError::Error)?,
            u32::try_from(height).map_err(|_| gst::FlowError::Error)?,
        );
        let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
        let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
        let bytes = map.as_slice();
        let row = width as usize * 4;
        let rows = height as usize;
        // Usually the rows are contiguous; if an allocator aligns them, it copies row by row.
        let stride = bytes.len().checked_div(rows).unwrap_or(0);
        if row == 0 || stride < row {
            // A frame that cannot be understood is skipped (it does not stop the video).
            return Ok(gst::FlowSuccess::Ok);
        }
        let mut bgra = self.spare.lock().unwrap_or_else(PoisonError::into_inner).take().unwrap_or_default();
        bgra.clear();
        bgra.reserve(Frame::len_of(width, height));
        if stride == row {
            bgra.extend_from_slice(&bytes[..row * rows]);
        } else {
            for line in bytes.chunks_exact(stride).take(rows) {
                bgra.extend_from_slice(&line[..row]);
            }
        }
        drop(map);
        let put = self.shared.slot.put(Frame { width, height, bgra });
        self.shared.frames.fetch_add(1, Ordering::Relaxed);
        if let Some(spare) = put.spare {
            *self.spare.lock().unwrap_or_else(PoisonError::into_inner) = Some(spare);
        }
        if put.wake {
            self.shared.wake.notify_one();
        }
        Ok(gst::FlowSuccess::Ok)
    }
}

/// Una pipeline aperta.
pub struct Pipeline {
    pipeline: gst::Pipeline,
    size: gst::Element,
    shared: Arc<Shared>,
    /// La misura del videoclip e quella chiesta a GStreamer.
    native: Size,
    target: Size,
    duration: Duration,
    paused: bool,
    opened_at: Instant,
}

impl std::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The (signed) address is never printed.
        f.debug_struct("Pipeline")
            .field("native", &self.native)
            .field("target", &self.target)
            .field("paused", &self.paused)
            .finish_non_exhaustive()
    }
}

/// The NVIDIA decoders at rank zero (see `NVIDIA_DECODERS`). Blocking the first time: it reads the registry.
fn demote_nvidia() {
    for feature in gst::Registry::get().features_by_plugin(NVIDIA_DECODERS) {
        feature.set_rank(gst::Rank::NONE);
    }
}

/// The measure negotiated at the pipeline's output (after the first frame).
fn negotiated(sink: &gst_app::AppSink) -> Option<Size> {
    let caps = sink.static_pad("sink")?.current_caps()?;
    let structure = caps.structure(0)?;
    let width = u32::try_from(structure.get::<i32>("width").ok()?).ok()?;
    let height = u32::try_from(structure.get::<i32>("height").ok()?).ok()?;
    Some(Size::new(width, height))
}

fn caps_for(size: Option<Size>) -> gst::Caps {
    let builder = gst::Caps::builder("video/x-raw")
        .field("format", "BGRA")
        .field("pixel-aspect-ratio", gst::Fraction::new(1, 1));
    match size {
        Some(size) => builder
            .field("width", i32::try_from(size.width).unwrap_or(i32::MAX))
            .field("height", i32::try_from(size.height).unwrap_or(i32::MAX))
            .build(),
        None => builder.build(),
    }
}

impl Pipeline {
    /// Opens `url` already at the audio's position (`clock`, projected forward while the video
    /// prepares), at the size that `surface` asks for. Blocking: it must be called from a thread of its own. Every
    /// error is only "did not open" (GStreamer's can contain the address).
    pub fn open(url: &str, clock: Clock, surface: Option<Size>, shared: Arc<Shared>) -> Result<Self, ()> {
        gst::init().map_err(|_| ())?;
        demote_nvidia();
        let pipeline = gst::parse::launch(&description())
            .map_err(|_| ())?
            .downcast::<gst::Pipeline>()
            .map_err(|_| ())?;
        let source = pipeline.by_name("src").ok_or(())?;
        source.set_property("uri", url);
        let size = pipeline.by_name("size").ok_or(())?;
        let sink = pipeline
            .by_name("frames")
            .and_then(|element| element.downcast::<gst_app::AppSink>().ok())
            .ok_or(())?;
        let copier = Arc::new(Copier { shared: Arc::clone(&shared), spare: Mutex::new(None) });
        let preroll = Arc::clone(&copier);
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_preroll(move |sink| {
                    let sample = sink.pull_preroll().map_err(|_| gst::FlowError::Eos)?;
                    preroll.copy(&sample)
                })
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    copier.copy(&sample)
                })
                .build(),
        );
        let bus = pipeline.bus().ok_or(())?;
        let signals = Arc::clone(&shared);
        // The messages are read on the thread that sends them and do not stay on the bus (nobody empties it).
        bus.set_sync_handler(move |_, message| {
            match message.view() {
                gst::MessageView::Error(_) => {
                    signals.failed.store(true, Ordering::Release);
                    signals.wake.notify_one();
                }
                gst::MessageView::Eos(_) => {
                    signals.ended.store(true, Ordering::Release);
                    signals.wake.notify_one();
                }
                _ => {}
            }
            gst::BusSyncReply::Drop
        });

        let mut opened = Self {
            pipeline,
            size,
            shared,
            native: Size::new(0, 0),
            target: Size::new(0, 0),
            duration: Duration::ZERO,
            paused: true,
            opened_at: Instant::now(),
        };
        // Paused until the first frame: then the video's size is known.
        opened.pipeline.set_state(gst::State::Paused).map_err(|_| ())?;
        let (prerolled, ..) = opened.pipeline.state(PREROLL);
        prerolled.map_err(|_| ())?;
        if opened.shared.failed() {
            return Err(());
        }
        opened.native = negotiated(&sink).ok_or(())?;
        opened.target = opened.native;
        opened.duration = opened
            .pipeline
            .query_duration::<gst::ClockTime>()
            .map_or(Duration::ZERO, |duration| Duration::from_nanos(duration.nseconds()));
        if let Some(surface) = surface {
            opened.fit_to(surface);
        }
        opened.seek(clock.at(Instant::now()));
        if clock.playing {
            opened.set_paused(false);
            // It waits a little for the initial seek to reach where the audio is, so the
            // first frame shown is already aligned.
            let deadline = Instant::now() + SETTLE;
            while Instant::now() < deadline && opened.position().abs_diff(clock.at(Instant::now())) > DRIFT {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        if opened.shared.failed() { Err(()) } else { Ok(opened) }
    }

    /// Asks GStreamer for the measure for a box of `surface` physical pixels, if it changes enough
    /// (`size::worth_switching`): the pipeline renegotiates it without stopping.
    pub fn fit_to(&mut self, surface: Size) {
        let Some(wanted) = super::size::fit(self.native, surface) else { return };
        if !super::size::worth_switching(self.target, wanted) {
            return;
        }
        self.target = wanted;
        self.size.set_property("caps", caps_for(Some(wanted)));
    }

    /// Where the video is.
    pub fn position(&self) -> Duration {
        self.pipeline
            .query_position::<gst::ClockTime>()
            .map_or(Duration::ZERO, |position| Duration::from_nanos(position.nseconds()))
    }

    /// What the player measures of itself, for the core's alignment rules.
    pub fn measure(&self) -> Measure {
        Measure {
            position: self.position(),
            duration: self.duration,
            paused: self.paused,
            ended: self.shared.ended(),
        }
    }

    /// I segnali condivisi con i thread di GStreamer.
    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// Closes the pipeline on a thread of its own: stopping it can wait for the network. Without a thread (resources
    /// exhausted) the closing happens here, when `spawn` leaves the function that did not run.
    pub fn discard(self) {
        let _ = std::thread::Builder::new().name("video-close".into()).spawn(move || drop(self));
    }
}

impl Controls for Pipeline {
    fn set_paused(&mut self, paused: bool) {
        let state = if paused { gst::State::Paused } else { gst::State::Playing };
        if self.pipeline.set_state(state).is_ok() {
            self.paused = paused;
        }
    }

    fn seek(&mut self, position: Duration) {
        let target = gst::ClockTime::from_nseconds(u64::try_from(position.as_nanos()).unwrap_or(u64::MAX));
        // A seek that fails leaves the video where it is: the next drift check tries again.
        let _ = self.pipeline.seek_simple(gst::SeekFlags::FLUSH, target);
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        if std::env::var_os(STATS_VAR).is_some() {
            let secs = self.opened_at.elapsed().as_secs_f64();
            let copied = self.shared.frames.load(Ordering::Relaxed);
            let shown = self.shared.shown.load(Ordering::Relaxed);
            eprintln!(
                "video: {copied} fotogrammi copiati, {shown} mostrati in {secs:.1} s ({:.1}/s), {}x{} -> {}x{}",
                shown as f64 / secs.max(0.001),
                self.native.width,
                self.native.height,
                self.target.width,
                self.target.height,
            );
        }
        // The callbacks hold the place and the wake-up: they detach with the pipeline.
        if let Some(bus) = self.pipeline.bus() {
            bus.unset_sync_handler();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipeline_keeps_one_frame_at_thirty_per_second_and_no_address() {
        let description = description();
        assert!(description.contains("max-rate=30") && description.contains("drop-only=true"));
        // One buffer only in the output, dropped if the UI did not take it: never a queue of frames.
        assert!(description.contains("max-buffers=1 drop=true"));
        assert!(description.contains("format=BGRA"));
        assert!(!description.contains("uri="), "l'indirizzo è una proprietà, mai nel testo");
    }

    #[test]
    fn the_caps_ask_for_the_chosen_size() {
        // Without GStreamer installed there is nothing to try.
        if gst::init().is_err() {
            return;
        }
        let caps = caps_for(Some(Size::new(288, 162)));
        let structure = caps.structure(0).map(ToOwned::to_owned);
        let size = structure.and_then(|s| Some((s.get::<i32>("width").ok()?, s.get::<i32>("height").ok()?)));
        assert_eq!(size, Some((288, 162)));
    }
}
