//! 10-band graphic equalizer: a cascade of "peaking" filters (RBJ Audio EQ Cookbook) on the
//! sample stream, before the audio output.
//!
//! The settings sit in an [`EqControl`] shared with the UI: changing them while it plays is
//! immediate, because the filter checks the version on every sample and recomputes the coefficients
//! only when it changes. With the equalizer off the signal passes through untouched.

use std::f32::consts::PI;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, SampleRate, Source};
use serde::{Deserialize, Serialize};

/// Centre frequencies of the bands, in Hz (octaves from 31 Hz to 16 kHz).
pub const BAND_FREQUENCIES: [f32; BANDS] =
    [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
pub const BANDS: usize = 10;
/// Maximum gain of a band, in dB, in both directions.
pub const MAX_GAIN_DB: f32 = 12.0;
/// Bandwidth: about one and a half octaves, like a studio graphic equalizer.
const Q: f32 = 1.41;

/// The equalizer settings. They are saved in the configuration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    /// Gain of each band in dB, from `-MAX_GAIN_DB` to `MAX_GAIN_DB`.
    pub gains: [f32; BANDS],
}

impl Default for Settings {
    fn default() -> Self {
        Self { enabled: false, gains: [0.0; BANDS] }
    }
}

impl Settings {
    /// Gains brought into the allowed range: values read from a file are not trusted.
    pub fn clamped(mut self) -> Self {
        for gain in &mut self.gains {
            *gain = if gain.is_finite() { gain.clamp(-MAX_GAIN_DB, MAX_GAIN_DB) } else { 0.0 };
        }
        self
    }
}

/// Factory presets, with the name shown in the UI.
pub const PRESETS: [(&str, [f32; BANDS]); 5] = [
    ("Piatto", [0.0; BANDS]),
    ("Bassi", [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Voce", [-2.0, -2.0, -1.0, 0.0, 2.0, 3.0, 3.0, 2.0, 1.0, 0.0]),
    ("Rock", [4.0, 3.0, 2.0, 0.0, -1.0, -1.0, 0.0, 2.0, 3.0, 3.0]),
    ("Acustico", [2.0, 2.0, 1.0, 0.0, 1.0, 1.0, 2.0, 3.0, 3.0, 2.0]),
];

/// Settings shared between the UI (which changes them) and the audio stream (which reads them).
#[derive(Debug, Default)]
pub struct EqControl {
    settings: Mutex<Settings>,
    /// Grows at each change: the filter compares this number before recomputing.
    version: AtomicU64,
}

impl EqControl {
    pub fn new(settings: Settings) -> Arc<Self> {
        Arc::new(Self { settings: Mutex::new(settings.clamped()), version: AtomicU64::new(0) })
    }

    pub fn get(&self) -> Settings {
        *self.lock()
    }

    pub fn set(&self, settings: Settings) {
        *self.lock() = settings.clamped();
        self.version.fetch_add(1, Ordering::Release);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Settings> {
        self.settings.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Coefficients of a biquad filter, already normalised (a0 = 1).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coefficients {
    /// A filter that does not change the signal.
    const IDENTITY: Self = Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    /// Gain bell `gain_db` around `frequency`. Above the Nyquist frequency, or at
    /// zero gain, the filter is neutral.
    fn peaking(frequency: f32, gain_db: f32, sample_rate: f32) -> Self {
        if frequency >= sample_rate / 2.0 || gain_db.abs() < 1e-3 {
            return Self::IDENTITY;
        }
        let amplitude = 10f32.powf(gain_db / 40.0);
        let omega = 2.0 * PI * frequency / sample_rate;
        let (sin, cos) = omega.sin_cos();
        let alpha = sin / (2.0 * Q);
        let a0 = 1.0 + alpha / amplitude;
        Self {
            b0: (1.0 + alpha * amplitude) / a0,
            b1: -2.0 * cos / a0,
            b2: (1.0 - alpha * amplitude) / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha / amplitude) / a0,
        }
    }
}

/// Memory of a filter: the last two input and output samples of a channel.
#[derive(Debug, Clone, Copy, Default)]
struct State {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl State {
    fn apply(&mut self, c: &Coefficients, x: f32) -> f32 {
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Source with the equalizer applied. The sample type is `f32`: whoever reads from rodio
/// converts first with `convert_samples`.
pub struct Equalized<S> {
    input: S,
    control: Arc<EqControl>,
    /// Version of the settings used for the current coefficients.
    version: u64,
    enabled: bool,
    sample_rate: f32,
    coefficients: [Coefficients; BANDS],
    /// Memory per channel and per band.
    states: Vec<[State; BANDS]>,
    /// Channel of the next sample (the samples are interleaved).
    channel: usize,
}

impl<S: Source<Item = f32>> Equalized<S> {
    pub fn new(input: S, control: Arc<EqControl>) -> Self {
        let channels = usize::from(input.channels().get());
        let sample_rate = input.sample_rate().get() as f32;
        let mut equalized = Self {
            input,
            control,
            // An impossible value: at the first read the coefficients are recomputed.
            version: u64::MAX,
            enabled: false,
            sample_rate,
            coefficients: [Coefficients::IDENTITY; BANDS],
            states: vec![[State::default(); BANDS]; channels],
            channel: 0,
        };
        equalized.refresh();
        equalized
    }

    /// Reads the settings only if they have changed since the last time.
    fn refresh(&mut self) {
        let version = self.control.version.load(Ordering::Acquire);
        if version == self.version {
            return;
        }
        let settings = self.control.get();
        self.version = version;
        self.enabled = settings.enabled;
        for (band, coefficients) in self.coefficients.iter_mut().enumerate() {
            *coefficients =
                Coefficients::peaking(BAND_FREQUENCIES[band], settings.gains[band], self.sample_rate);
        }
    }
}

impl<S: Source<Item = f32>> Iterator for Equalized<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.input.next()?;
        self.refresh();
        let channel = self.channel;
        self.channel = (self.channel + 1) % self.states.len().max(1);
        if !self.enabled {
            return Some(sample);
        }
        let states = &mut self.states[channel];
        let mut out = sample;
        for (state, coefficients) in states.iter_mut().zip(&self.coefficients) {
            out = state.apply(coefficients, out);
        }
        Some(out)
    }
}

impl<S: Source<Item = f32>> Source for Equalized<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.input.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.input.try_seek(pos)?;
        // The filters' memory belongs to the previous point: it is zeroed.
        for states in &mut self.states {
            *states = [State::default(); BANDS];
        }
        self.channel = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constant signal, in a single output, for the gain tests.
    struct Dc {
        left: usize,
    }

    impl Iterator for Dc {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            (self.left > 0).then(|| {
                self.left -= 1;
                0.5
            })
        }
    }

    impl Source for Dc {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> ChannelCount {
            ChannelCount::MIN
        }
        fn sample_rate(&self) -> SampleRate {
            SampleRate::new(44_100).expect("a non-zero rate")
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    fn rms_of(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn flat_settings_pass_the_signal_unchanged() {
        let control = EqControl::new(Settings { enabled: true, gains: [0.0; BANDS] });
        let out: Vec<f32> = Equalized::new(Dc { left: 2000 }, control).collect();
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn disabled_equalizer_is_a_bypass() {
        let control = EqControl::new(Settings { enabled: false, gains: [12.0; BANDS] });
        let out: Vec<f32> = Equalized::new(Dc { left: 500 }, control).collect();
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn a_boost_at_the_signal_frequency_raises_its_level() {
        // A 1 kHz tone passes through the 1 kHz band: with +12 dB the level rises.
        let tone = |n: usize| (2.0 * PI * 1000.0 * n as f32 / 44_100.0).sin() * 0.1;
        let samples: Vec<f32> = (0..8192).map(tone).collect();
        let flat = EqControl::new(Settings { enabled: true, gains: [0.0; BANDS] });
        let mut gains = [0.0; BANDS];
        gains[5] = 12.0;
        let boosted = EqControl::new(Settings { enabled: true, gains });
        struct Samples(std::vec::IntoIter<f32>);
        impl Iterator for Samples {
            type Item = f32;
            fn next(&mut self) -> Option<f32> {
                self.0.next()
            }
        }
        impl Source for Samples {
            fn current_span_len(&self) -> Option<usize> {
                None
            }
            fn channels(&self) -> ChannelCount {
                ChannelCount::MIN
            }
            fn sample_rate(&self) -> SampleRate {
                SampleRate::new(44_100).expect("a non-zero rate")
            }
            fn total_duration(&self) -> Option<Duration> {
                None
            }
        }
        let flat_out: Vec<f32> = Equalized::new(Samples(samples.clone().into_iter()), flat).collect();
        let boosted_out: Vec<f32> = Equalized::new(Samples(samples.into_iter()), boosted).collect();
        // The initial transient of the filter is skipped.
        let (a, b) = (rms_of(&flat_out[4096..]), rms_of(&boosted_out[4096..]));
        assert!(b > a * 3.0, "boosted {b} should be about 4x the flat {a}");
    }

    #[test]
    fn changes_apply_while_playing() {
        let control = EqControl::new(Settings { enabled: true, gains: [0.0; BANDS] });
        let mut eq = Equalized::new(Dc { left: 10_000 }, Arc::clone(&control));
        let before: Vec<f32> = eq.by_ref().take(100).collect();
        assert!(before.iter().all(|s| (s - 0.5).abs() < 1e-6));
        control.set(Settings { enabled: false, gains: [0.0; BANDS] });
        // The change applies from the next sample, without recreating the source.
        assert!(eq.next().is_some());
        assert!(!eq.enabled);
    }

    #[test]
    fn gains_outside_the_range_are_clamped() {
        let s = Settings { enabled: true, gains: [40.0, -40.0, f32::NAN, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] }
            .clamped();
        assert_eq!(s.gains[0], MAX_GAIN_DB);
        assert_eq!(s.gains[1], -MAX_GAIN_DB);
        assert_eq!(s.gains[2], 0.0);
        assert_eq!(s.gains[3], 3.0);
    }

    #[test]
    fn bands_above_nyquist_are_neutral() {
        assert_eq!(Coefficients::peaking(16_000.0, 6.0, 22_050.0), Coefficients::IDENTITY);
    }
}
