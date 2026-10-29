//! Wrapper di rodio: un'uscita audio aperta per tutta la vita dell'app e un `Player` per brano.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};

use super::decoder::SymphoniaSource;
use super::eq::{EqControl, Equalized, Settings};
use super::stream::StreamBuffer;
use crate::error::AppError;

pub struct AudioOutput {
    // L'uscita va tenuta in vita: se viene distrutta l'audio si ferma.
    device: MixerDeviceSink,
    sink: Option<Arc<Player>>,
    /// Seek in corso, fuori dal thread dell'interfaccia: `Player::try_seek` aspetta che il thread
    /// audio lo esegua, e in un FLAC ancora in arrivo vuol dire aspettare la rete.
    seeking: Arc<Seeking>,
    /// Equalizzatore condiviso con ogni brano: le modifiche valgono anche mentre suona.
    eq: Arc<EqControl>,
}

impl AudioOutput {
    pub fn open() -> Result<Self, AppError> {
        let device = DeviceSinkBuilder::open_default_sink().map_err(|_| AppError::AudioDevice)?;
        Ok(Self { device, sink: None, seeking: Arc::default(), eq: EqControl::new(Settings::default()) })
    }

    pub fn set_equalizer(&self, settings: Settings) {
        self.eq.set(settings);
    }

    /// Avvia un brano mentre arriva, fermando quello precedente. Il decoder legge dal buffer e
    /// aspetta i byte non ancora scaricati.
    pub fn play(&mut self, stream: &StreamBuffer, volume: f32) -> Result<(), AppError> {
        self.stop();
        let sink = Player::connect_new(self.device.mixer());
        sink.set_volume(amplitude(volume));
        let eq = Arc::clone(&self.eq);
        let source = SymphoniaSource::open(Box::new(stream.reader())).map_err(|_| AppError::AudioDecode)?;
        sink.append(Equalized::new(source, eq));
        self.sink = Some(Arc::new(sink));
        Ok(())
    }

    pub fn pause(&self) {
        if let Some(sink) = &self.sink {
            sink.pause();
        }
    }

    pub fn resume(&self) {
        if let Some(sink) = &self.sink {
            sink.play();
        }
    }

    pub fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        // Un seek ancora in corso riguarda il brano fermato: la posizione torna quella del nuovo.
        self.seeking.reset();
    }

    /// Posizione del brano; durante un seek, il punto chiesto.
    pub fn position(&self) -> Duration {
        if let Some(target) = self.seeking.target() {
            return target;
        }
        self.sink.as_deref().map_or(Duration::ZERO, Player::get_pos)
    }

    /// Avvia il seek e torna subito. Un fallimento si legge dopo con [`Self::take_seek_failure`].
    pub fn seek(&self, to: Duration) {
        let Some(sink) = &self.sink else { return };
        let sink = Arc::clone(sink);
        let seeking = Arc::clone(&self.seeking);
        let generation = seeking.start(to);
        std::thread::spawn(move || {
            let ok = sink.try_seek(to).is_ok();
            seeking.finish(generation, ok);
        });
    }

    /// `true` una sola volta dopo un seek fallito.
    pub fn take_seek_failure(&self) -> bool {
        self.seeking.failed.swap(false, Ordering::AcqRel)
    }

    pub fn set_volume(&self, volume: f32) {
        if let Some(sink) = &self.sink {
            sink.set_volume(amplitude(volume));
        }
    }

    /// Il brano caricato è finito.
    pub fn finished(&self) -> bool {
        self.sink.as_deref().is_some_and(Player::empty)
    }
}

/// Stato del seek più recente: i precedenti, ancora in corso, non lo cambiano più.
#[derive(Default)]
struct Seeking {
    generation: AtomicU64,
    target: Mutex<Option<(u64, Duration)>>,
    failed: AtomicBool,
}

impl Seeking {
    fn start(&self, to: Duration) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(mut target) = self.target.lock() {
            *target = Some((generation, to));
        }
        generation
    }

    fn finish(&self, generation: u64, ok: bool) {
        let Ok(mut target) = self.target.lock() else { return };
        if target.is_some_and(|(current, _)| current == generation) {
            *target = None;
            if !ok {
                self.failed.store(true, Ordering::Release);
            }
        }
    }

    fn reset(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut target) = self.target.lock() {
            *target = None;
        }
        self.failed.store(false, Ordering::Release);
    }

    fn target(&self) -> Option<Duration> {
        self.target.lock().ok().and_then(|target| target.map(|(_, at)| at))
    }
}

/// Lo slider va da 0 a 1; l'orecchio percepisce il volume in modo circa quadratico.
fn amplitude(volume: f32) -> f32 {
    let v = volume.clamp(0.0, 1.0);
    v * v
}
