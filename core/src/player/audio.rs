//! Wrapper for rodio: one audio output open for the whole life of the app and a `Player` per track.

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

/// Format of a detached output: stereo at 44.1 kHz, like a common sound card.
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
    // The output must be kept alive: if it is destroyed the audio stops. `None` for a detached output
    // (`detached`), with no sound card.
    _device: Option<MixerDeviceSink>,
    /// The mixer each track goes into: the sound card's, or that of a detached output.
    mixer: Mixer,
    sink: Option<Arc<Player>>,
    /// Seek in progress, off the interface's thread: `Player::try_seek` waits for the audio thread to run it,
    /// and in a FLAC still arriving that means waiting for the network.
    seeking: Arc<Seeking>,
    /// Equalizer shared with every track: the changes apply even while it plays.
    eq: Arc<EqControl>,
}

impl AudioOutput {
    pub fn open() -> Result<Self, AppError> {
        let device = DeviceSinkBuilder::open_default_sink().map_err(|_| AppError::AudioDevice)?;
        let mixer = device.mixer().clone();
        Ok(Self::with_mixer(Some(device), mixer))
    }

    /// An output without a sound card, for the tests: the samples do not go to a device but come out of the
    /// [`MixerSource`] returned, to whoever reads them. Reading them advances the position and, once
    /// the whole track is read, [`Self::finished`] becomes true.
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

    /// Starts a track as it arrives, stopping the previous one. The decoder reads from the buffer and
    /// waits for the bytes not yet downloaded.
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
        // A seek still in progress concerns the stopped track: the position goes back to the new one's.
        self.seeking.reset();
    }

    /// Position of the track; during a seek, the requested point.
    pub fn position(&self) -> Duration {
        if let Some(target) = self.seeking.target() {
            return target;
        }
        self.sink.as_deref().map_or(Duration::ZERO, Player::get_pos)
    }

    /// Starts the seek and returns at once. A failure is read afterwards with [`Self::take_seek_failure`].
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

    /// `true` only once after a failed seek.
    pub fn take_seek_failure(&self) -> bool {
        self.seeking.failed.swap(false, Ordering::AcqRel)
    }

    pub fn set_volume(&self, volume: f32) {
        if let Some(sink) = &self.sink {
            sink.set_volume(amplitude(volume));
        }
    }

    /// The loaded track has ended.
    pub fn finished(&self) -> bool {
        self.sink.as_deref().is_some_and(Player::empty)
    }
}

/// State of the most recent seek: the previous ones, still in progress, no longer change it.
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

/// The slider goes from 0 to 1; the ear perceives the volume roughly quadratically.
fn amplitude(volume: f32) -> f32 {
    let v = volume.clamp(0.0, 1.0);
    v * v
}

#[cfg(test)]
mod tests {
    use rodio::Source;

    use super::*;

    /// Four seconds of mono FLAC at 22,050 Hz.
    const TONE_FLAC: &[u8] = include_bytes!("../../tests/fixtures/tone-4s.flac");

    /// Reads from the detached mixer the samples of `duration`, as the sound card would.
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
