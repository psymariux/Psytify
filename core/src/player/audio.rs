//! Wrapper di rodio: un'uscita audio aperta per tutta la vita dell'app e un `Player` per brano.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::mixer::Mixer;
#[cfg(any(test, feature = "test-support"))]
use rodio::mixer::MixerSource;
#[cfg(any(test, feature = "test-support"))]
use rodio::{ChannelCount, SampleRate};
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};

use super::decoder::SymphoniaSource;
use super::eq::{EqControl, Equalized, Settings};
use super::stream::StreamBuffer;
use crate::error::AppError;

/// Formato di un'uscita staccata: stereo a 44,1 kHz, come una scheda audio comune.
#[cfg(any(test, feature = "test-support"))]
const DETACHED_CHANNELS: ChannelCount = match ChannelCount::new(2) {
    Some(channels) => channels,
    None => ChannelCount::MIN,
};
#[cfg(any(test, feature = "test-support"))]
const DETACHED_RATE: SampleRate = match SampleRate::new(44_100) {
    Some(rate) => rate,
    None => SampleRate::MIN,
};

pub struct AudioOutput {
    // L'uscita va tenuta in vita: se viene distrutta l'audio si ferma. `None` per un'uscita
    // staccata (`detached`), senza scheda audio.
    _device: Option<MixerDeviceSink>,
    /// Il mixer in cui entra ogni brano: quello della scheda audio, o quello di un'uscita staccata.
    mixer: Mixer,
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
        let mixer = device.mixer().clone();
        Ok(Self::with_mixer(Some(device), mixer))
    }

    /// Un'uscita senza scheda audio, per i test: i campioni non vanno a un dispositivo ma escono
    /// dal [`MixerSource`] restituito, a chi li legge. Leggerli fa avanzare la posizione e, letto
    /// tutto il brano, [`Self::finished`] diventa vero.
    #[cfg(any(test, feature = "test-support"))]
    pub fn detached() -> (Self, MixerSource) {
        let (mixer, source) = rodio::mixer::mixer(DETACHED_CHANNELS, DETACHED_RATE);
        (Self::with_mixer(None, mixer), source)
    }

    fn with_mixer(device: Option<MixerDeviceSink>, mixer: Mixer) -> Self {
        Self {
            _device: device,
            mixer,
            sink: None,
            seeking: Arc::default(),
            eq: EqControl::new(Settings::default()),
        }
    }

    pub fn set_equalizer(&self, settings: Settings) {
        self.eq.set(settings);
    }

    /// Avvia un brano mentre arriva, fermando quello precedente. Il decoder legge dal buffer e
    /// aspetta i byte non ancora scaricati.
    pub fn play(&mut self, stream: &StreamBuffer, volume: f32) -> Result<(), AppError> {
        self.stop();
        let sink = Player::connect_new(&self.mixer);
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

#[cfg(test)]
mod tests {
    use rodio::Source;

    use super::*;

    /// Quattro secondi di FLAC mono a 22 050 Hz.
    const TONE_FLAC: &[u8] = include_bytes!("../../tests/fixtures/tone-4s.flac");

    /// Legge dal mixer staccato i campioni di `duration`, come farebbe la scheda audio.
    fn pull(source: &mut MixerSource, duration: Duration) {
        let per_second = f64::from(source.sample_rate().get()) * f64::from(source.channels().get());
        let samples = (duration.as_secs_f64() * per_second) as usize;
        source.by_ref().take(samples).for_each(drop);
    }

    fn near(actual: Duration, expected: Duration) -> bool {
        actual.abs_diff(expected) <= Duration::from_millis(50)
    }

    #[test]
    fn a_detached_output_moves_only_while_its_samples_are_read() {
        let (mut output, mut mixer) = AudioOutput::detached();
        output.play(&StreamBuffer::from_bytes(TONE_FLAC.to_vec()), 1.0).unwrap();
        assert_eq!(output.position(), Duration::ZERO, "nessuno ha ancora letto un campione");

        pull(&mut mixer, Duration::from_secs(1));
        let heard = output.position();
        assert!(near(heard, Duration::from_secs(1)), "{heard:?}");
        assert!(!output.finished());

        output.pause();
        pull(&mut mixer, Duration::from_secs(1));
        assert!(near(output.position(), heard), "in pausa la posizione resta ferma");

        output.resume();
        pull(&mut mixer, Duration::from_secs(1));
        assert!(near(output.position(), heard + Duration::from_secs(1)));
    }

    #[test]
    fn a_detached_output_finishes_once_the_whole_track_is_read() {
        let (mut output, mut mixer) = AudioOutput::detached();
        output.play(&StreamBuffer::from_bytes(TONE_FLAC.to_vec()), 1.0).unwrap();
        pull(&mut mixer, Duration::from_millis(3_500));
        assert!(!output.finished());
        pull(&mut mixer, Duration::from_secs(1));
        assert!(output.finished(), "letti tutti i campioni, il brano è finito");
    }

    #[test]
    fn stopping_a_detached_output_resets_the_position() {
        let (mut output, mut mixer) = AudioOutput::detached();
        output.play(&StreamBuffer::from_bytes(TONE_FLAC.to_vec()), 1.0).unwrap();
        pull(&mut mixer, Duration::from_secs(1));
        output.stop();
        assert_eq!(output.position(), Duration::ZERO);
        assert!(!output.finished(), "senza brano non c'è niente di finito");
    }
}
