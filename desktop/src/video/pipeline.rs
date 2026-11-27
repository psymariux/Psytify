//! La pipeline di GStreamer del videoclip: solo il video (l'audio resta quello del core, che il video
//! segue), ridotto ai pixel del riquadro e a 30 fotogrammi al secondo, in BGRA nel posto dell'ultimo
//! fotogramma (`slot`).
//!
//! `uridecodebin ! videorate (max 30 fps, solo scarti) ! videoscale ! videoconvert ! capsfilter (BGRA,
//! misura) ! appsink (un buffer, scarta)`: la stessa famiglia di pipeline del client Iced
//! (`iced_video_player`), ma la conversione in BGRA la fa la CPU, perché GPUI su Linux e Windows non
//! accetta texture YUV (`05-video-spike.md`, opzione A). Ridurre prima di convertire costa meno: il
//! mini player converte un decimo dei pixel.
//!
//! I thread sono quelli di GStreamer (rete, demuxer, decoder): esistono finché esiste la pipeline.
//! Aprire (fino a 5 secondi, la rete) e chiudere (`Null` può aspettare la rete) bloccano, quindi si fanno
//! su un altro thread; pausa, salto e posizione sono veloci e li chiede la UI. Gli errori di GStreamer
//! possono contenere l'indirizzo firmato: non si conservano mai, diventano solo «non si è aperto» o
//! «si è fermato».

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

/// Fotogrammi al secondo al massimo: un videoclip a 60 si dimezza nella pipeline (60 costa il doppio,
/// `05-video-spike.md` §4.1).
pub const MAX_FPS: u32 = 30;
/// Quanto si aspetta che il primo fotogramma sia pronto (come `iced_video_player`).
const PREROLL: gst::ClockTime = gst::ClockTime::from_seconds(5);

/// La pipeline completa; `src`, `size` e `frames` sono i nomi degli elementi che si toccano. L'indirizzo
/// non sta qui: è una proprietà di `src`.
fn description() -> String {
    format!(
        "uridecodebin name=src ! videorate drop-only=true max-rate={MAX_FPS} ! videoscale ! videoconvert \
         ! capsfilter name=size caps=video/x-raw,format=BGRA,pixel-aspect-ratio=1/1 \
         ! appsink name=frames max-buffers=1 drop=true enable-last-sample=false"
    )
}

/// Con questa variabile d'ambiente, alla chiusura di ogni pipeline una riga su stderr: fotogrammi copiati
/// e mostrati, misure, durata (per le misure di `docs/PERFORMANCE.md`). Mai l'indirizzo.
const STATS_VAR: &str = "PSYTIFY_VIDEO_STATS";

/// Plugin dei decoder NVIDIA (NVDEC via CUDA): come nel client Iced, rango zero, così un portatile
/// ibrido non sveglia la scheda dedicata (e ~130 MB di VRAM) per un videoclip a 360p o 720p.
const NVIDIA_DECODERS: &str = "nvcodec";

/// Ciò che i thread di GStreamer e la UI si dicono: il posto del fotogramma, la sveglia e due segnali.
#[derive(Debug, Default)]
pub struct Shared {
    pub slot: FrameSlot,
    /// Una sveglia per la UI: un fotogramma nuovo nel posto vuoto, un errore o la fine. Tiene al massimo
    /// un permesso (`Notify`): le sveglie non si accumulano.
    pub wake: Notify,
    failed: AtomicBool,
    ended: AtomicBool,
    frames: AtomicU64,
    shown: AtomicU64,
}

impl Shared {
    /// Lo stream si è fermato (errore di rete o del decoder).
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    /// Il video è arrivato in fondo.
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    /// La UI ha preso un fotogramma dal posto (per la riga di `PSYTIFY_VIDEO_STATS`).
    pub fn mark_shown(&self) {
        self.shown.fetch_add(1, Ordering::Relaxed);
    }
}

/// Copia un campione nel posto, riusando il buffer del fotogramma che la UI non ha preso.
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
        // Di solito le righe sono contigue; se un allocatore le allinea si copia riga per riga.
        let stride = bytes.len().checked_div(rows).unwrap_or(0);
        if row == 0 || stride < row {
            // Un fotogramma che non si capisce si salta (non ferma il videoclip).
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
        // L'indirizzo (firmato) non si stampa mai.
        f.debug_struct("Pipeline")
            .field("native", &self.native)
            .field("target", &self.target)
            .field("paused", &self.paused)
            .finish_non_exhaustive()
    }
}

/// I decoder NVIDIA a rango zero (vedi `NVIDIA_DECODERS`). Bloccante la prima volta: legge il registro.
fn demote_nvidia() {
    for feature in gst::Registry::get().features_by_plugin(NVIDIA_DECODERS) {
        feature.set_rank(gst::Rank::NONE);
    }
}

/// La misura negoziata all'uscita della pipeline (dopo il primo fotogramma).
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
    /// Apre `url` già alla posizione dell'audio (`clock`, proiettato in avanti mentre il video si
    /// prepara), alla misura che `surface` chiede. Bloccante: va chiamata da un thread a parte. Ogni
    /// errore è solo «non si è aperto» (quello di GStreamer può contenere l'indirizzo).
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
        // I messaggi si leggono sul thread che li manda e non restano sul bus (nessuno lo svuota).
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
        // In pausa fino al primo fotogramma: allora si conosce la misura del videoclip.
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
            // Come il client Iced: si aspetta un poco che il salto iniziale arrivi dove sta l'audio, così il
            // primo fotogramma mostrato è già allineato.
            let deadline = Instant::now() + SETTLE;
            while Instant::now() < deadline && opened.position().abs_diff(clock.at(Instant::now())) > DRIFT {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        if opened.shared.failed() { Err(()) } else { Ok(opened) }
    }

    /// Chiede a GStreamer la misura per un riquadro di `surface` pixel fisici, se cambia abbastanza
    /// (`size::worth_switching`): la pipeline la rinegozia senza fermarsi.
    pub fn fit_to(&mut self, surface: Size) {
        let Some(wanted) = super::size::fit(self.native, surface) else { return };
        if !super::size::worth_switching(self.target, wanted) {
            return;
        }
        self.target = wanted;
        self.size.set_property("caps", caps_for(Some(wanted)));
    }

    /// Dove si trova il video.
    pub fn position(&self) -> Duration {
        self.pipeline
            .query_position::<gst::ClockTime>()
            .map_or(Duration::ZERO, |position| Duration::from_nanos(position.nseconds()))
    }

    /// Ciò che il player misura di sé, per le regole di allineamento del core.
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

    /// Chiude la pipeline su un thread a parte: fermarla può aspettare la rete. Senza un thread (risorse
    /// esaurite) la chiusura avviene qui, quando `spawn` lascia la funzione che non ha eseguito.
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
        // Un salto che non riesce lascia il video dov'è: il prossimo controllo dello scarto riprova.
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
        // Le callback tengono il posto e la sveglia: si staccano con la pipeline.
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
        // Un solo buffer nell'uscita, scartato se la UI non l'ha preso: mai una coda di fotogrammi.
        assert!(description.contains("max-buffers=1 drop=true"));
        assert!(description.contains("format=BGRA"));
        assert!(!description.contains("uri="), "l'indirizzo è una proprietà, mai nel testo");
    }

    #[test]
    fn the_caps_ask_for_the_chosen_size() {
        // Senza GStreamer installato non c'è niente da provare.
        if gst::init().is_err() {
            return;
        }
        let caps = caps_for(Some(Size::new(288, 162)));
        let structure = caps.structure(0).map(ToOwned::to_owned);
        let size = structure.and_then(|s| Some((s.get::<i32>("width").ok()?, s.get::<i32>("height").ok()?)));
        assert_eq!(size, Some((288, 162)));
    }
}
