//! Decodifica di tutti i formati del catalogo (FLAC, MP3, Ogg Vorbis, WAV, M4A) con Symphonia,
//! senza passare dal decoder di rodio.
//!
//! Rodio 0.20 apre ogni flusso come seekable ma senza lunghezza: Symphonia, senza lunghezza, non
//! sa cercare in un FLAC (il seek falliva sempre) e su un MP4 fallisce con `SeekError`, che rodio
//! tratta come impossibile (`unreachable!`: l'app si chiudeva appena partiva un m4a). Qui il
//! lettore del buffer dice la lunghezza annunciata dal server, quindi il seek funziona anche
//! mentre il brano si sta ancora scaricando.

use std::sync::Arc;
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, SampleRate, Source};
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;

use super::stream::StreamReader;

/// Byte che bastano a riconoscere un MP4: la parola `ftyp` dal quarto byte.
pub const SNIFF_BYTES: usize = 8;

pub fn is_mp4(head: &[u8]) -> bool {
    head.get(4..8) == Some(b"ftyp".as_slice())
}

impl MediaSource for StreamReader {
    fn is_seekable(&self) -> bool {
        self.total().is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.total().and_then(|total| u64::try_from(total).ok())
    }
}

/// Sorgente di campioni f32 per la coda del `Sink`.
pub struct SymphoniaSource {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    /// Canali e frequenza dello spec del primo pacchetto: i container non li danno sempre (MP4),
    /// quindi si leggono dai dati decodificati.
    channels: u16,
    sample_rate: u32,
    /// Numero di frame del brano, se il container lo dichiara.
    n_frames: Option<u64>,
    /// Campioni dell'ultimo pacchetto decodificato e posizione di lettura in essi.
    pending: Vec<f32>,
    cursor: usize,
    /// Frame da scartare dopo un seek: il container riparte dall'inizio del pacchetto che contiene
    /// il punto chiesto, che può cadere più avanti.
    skip_frames: u64,
}

impl SymphoniaSource {
    /// Apre un file da qualunque sorgente. Errore se non c'è una traccia audio che Symphonia sa
    /// decodificare.
    pub fn open(source: Box<dyn MediaSource>) -> Result<Self, Error> {
        let stream = MediaSourceStream::new(source, Default::default());
        let format = symphonia::default::get_probe().probe(
            &Hint::new(),
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )?;
        let track = format
            .first_track_known_codec(TrackType::Audio)
            .ok_or(Error::Unsupported("no audio track in the file"))?;
        let Some(CodecParameters::Audio(params)) = &track.codec_params else {
            return Err(Error::Unsupported("no audio track in the file"));
        };
        let track_id = track.id;
        let n_frames = track.num_frames;
        let decoder =
            symphonia::default::get_codecs().make_audio_decoder(params, &AudioDecoderOptions::default())?;
        let mut source = Self {
            format,
            decoder,
            track_id,
            channels: 0,
            sample_rate: 0,
            n_frames,
            pending: Vec::new(),
            cursor: 0,
            skip_frames: 0,
        };
        // Il primo pacchetto dice formato e frequenza; vale anche come prova che il file si decodifica.
        if !source.decode_next() || source.sample_rate == 0 {
            return Err(Error::Unsupported("no decodable audio in the file"));
        }
        Ok(source)
    }

    /// Decodifica il pacchetto successivo della traccia. `false` a fine file o su errore.
    fn decode_next(&mut self) -> bool {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) | Err(_) => return false,
            };
            if packet.track_id != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = decoded.spec();
                    self.channels = u16::try_from(spec.channels().count()).unwrap_or(2);
                    self.sample_rate = spec.rate();
                    decoded.copy_to_vec_interleaved(&mut self.pending);
                    let channels = usize::from(self.channels.max(1));
                    let decoded_frames = self.pending.len() / channels;
                    let skip = usize::try_from(self.skip_frames).unwrap_or(usize::MAX).min(decoded_frames);
                    self.skip_frames -= skip as u64;
                    self.pending.drain(..skip * channels);
                    self.cursor = 0;
                    return true;
                }
                // Un pacchetto danneggiato si salta, come fa rodio.
                Err(Error::DecodeError(_)) => continue,
                Err(_) => return false,
            }
        }
    }
}

impl Iterator for SymphoniaSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        while self.cursor >= self.pending.len() {
            if !self.decode_next() {
                return None;
            }
        }
        let sample = self.pending[self.cursor];
        self.cursor += 1;
        Some(sample)
    }
}

impl Source for SymphoniaSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.channels).unwrap_or(ChannelCount::MIN)
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.sample_rate).unwrap_or(SampleRate::MIN)
    }

    fn total_duration(&self) -> Option<Duration> {
        let frames = self.n_frames?;
        Some(Duration::from_secs_f64(frames as f64 / f64::from(self.sample_rate)))
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let time = Time::try_from_secs_f64(pos.as_secs_f64()).unwrap_or(Time::ZERO);
        let to = SeekTo::Time { time, track_id: Some(self.track_id) };
        let seeked =
            self.format.seek(SeekMode::Accurate, to).map_err(|err| SeekError::Other(Arc::new(err)))?;
        self.decoder.reset();
        self.pending.clear();
        self.cursor = 0;
        self.skip_frames =
            u64::try_from(seeked.required_ts.saturating_delta(seeked.actual_ts).get()).unwrap_or(0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::player::stream::StreamBuffer;

    /// Un secondo di AAC con il box `moov` in fondo: il caso che fa fallire il decoder di rodio.
    const TONE_MOOV_END: &[u8] = include_bytes!("../../tests/fixtures/tone-moov-end.m4a");
    /// Lo stesso segnale con l'indice in testa.
    const TONE_FASTSTART: &[u8] = include_bytes!("../../tests/fixtures/tone-faststart.m4a");
    /// Quattro secondi di FLAC mono a 22 050 Hz, senza seektable.
    const TONE_FLAC: &[u8] = include_bytes!("../../tests/fixtures/tone-4s.flac");

    fn open(bytes: &[u8]) -> SymphoniaSource {
        SymphoniaSource::open(Box::new(Cursor::new(bytes.to_vec()))).expect("a valid file")
    }

    /// Un WAV PCM in memoria, `frames` frame a dente di sega: nessun file di prova nel repository e
    /// ogni frame ha un valore diverso, così un seek sbagliato si vede dal conteggio.
    fn wav(channels: u16, rate: u32, bits: u16, frames: u32) -> Vec<u8> {
        let block = channels * bits / 8;
        let data_len = frames * u32::from(block);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
        bytes.extend_from_slice(&block.to_le_bytes());
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for frame in 0..frames {
            // Da -0,5 a +0,5 della scala intera, in `bits` bit little-endian con segno.
            let level = (i64::from(frame % 1000) - 500) * (1i64 << (bits - 1)) / 1000;
            for _ in 0..channels {
                bytes.extend_from_slice(&level.to_le_bytes()[..usize::from(bits / 8)]);
            }
        }
        bytes
    }

    #[test]
    fn recognises_mp4_by_its_ftyp_box() {
        assert!(is_mp4(&TONE_MOOV_END[..SNIFF_BYTES]));
        assert!(!is_mp4(b"fLaC\0\0\0\0"));
        assert!(!is_mp4(b"ftyp"));
    }

    #[test]
    fn decodes_a_whole_m4a_with_the_index_at_the_end() {
        let source = open(TONE_MOOV_END);
        assert_eq!((source.channels().get(), source.sample_rate().get()), (2, 44_100));
        let seconds = source.total_duration().expect("the length is known").as_secs_f64();
        assert!((seconds - 1.0).abs() < 0.1, "about one second, got {seconds}");
        // Con il priming dell'AAC il conteggio non è esatto: basta che sia intorno a un secondo.
        let samples = source.count();
        assert!((80_000..100_000).contains(&samples), "got {samples} samples");
    }

    #[test]
    fn decodes_a_faststart_m4a_too() {
        assert!(open(TONE_FASTSTART).count() > 40_000);
    }

    #[test]
    fn seeking_moves_back_into_an_m4a() {
        let all = open(TONE_MOOV_END).count();
        let mut source = open(TONE_MOOV_END);
        // Si ascolta circa tre quarti del brano, poi si torna a un quarto di secondo. Symphonia 0.6
        // non sa più cercare in un MP4 letto fino in fondo, ma nell'app non serve: a fine brano la
        // sorgente esce dalla coda di rodio.
        source.by_ref().take(all * 3 / 4).for_each(drop);
        source.try_seek(Duration::from_millis(250)).expect("the file is seekable");
        let rest = source.count();
        assert!(rest > all / 4 && rest < all, "after seeking to 0.25 s: {rest} of {all} samples");
    }

    #[test]
    fn decodes_a_whole_flac() {
        let source = open(TONE_FLAC);
        assert_eq!((source.channels().get(), source.sample_rate().get()), (1, 22_050));
        assert_eq!(source.count(), 4 * 22_050);
    }

    #[test]
    fn seeking_a_flac_lands_on_the_requested_sample() {
        let mut source = open(TONE_FLAC);
        source.try_seek(Duration::from_secs(3)).expect("the flac is seekable");
        assert_eq!(source.count(), 22_050);
    }

    #[test]
    fn seeking_a_flac_works_while_it_is_still_downloading() {
        // Il server ha annunciato la lunghezza, ma sono arrivati solo i primi byte: il lettore
        // aspetta il resto, che qui arriva da un altro thread.
        let buffer = StreamBuffer::new();
        buffer.set_total(TONE_FLAC.len(), true);
        buffer.write_at(0, &TONE_FLAC[..4096]);
        let writer = buffer.clone();
        let feeder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            writer.write_at(4096, &TONE_FLAC[4096..]);
        });
        let mut source = SymphoniaSource::open(Box::new(buffer.reader())).expect("the header arrived");
        source.try_seek(Duration::from_secs(2)).expect("the length is known");
        feeder.join().unwrap();
        assert_eq!(source.count(), 2 * 22_050);
    }

    #[test]
    fn decodes_a_cd_quality_wav() {
        let source = open(&wav(2, 44_100, 16, 3 * 44_100));
        assert_eq!((source.channels().get(), source.sample_rate().get()), (2, 44_100));
        let seconds = source.total_duration().expect("a wav declares its length").as_secs_f64();
        assert!((seconds - 3.0).abs() < 0.001, "three seconds, got {seconds}");
        assert_eq!(source.count(), 2 * 3 * 44_100);
    }

    #[test]
    fn decodes_a_24_bit_wav_without_clipping_it() {
        let samples: Vec<f32> = open(&wav(2, 48_000, 24, 48_000)).collect();
        assert_eq!(samples.len(), 2 * 48_000);
        // Il picco del dente di sega è metà scala: se il bit depth fosse sbagliato sarebbe 0 o 1.
        let peak = samples.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!((peak - 0.5).abs() < 0.01, "peak {peak}");
    }

    #[test]
    fn seeking_a_wav_lands_on_the_requested_frame() {
        let mut source = open(&wav(2, 44_100, 16, 4 * 44_100));
        source.try_seek(Duration::from_secs(3)).expect("a wav is seekable");
        assert_eq!(source.count(), 2 * 44_100);
    }

    #[test]
    fn seeking_a_wav_works_while_it_is_still_downloading() {
        // Un WAV pesa dieci volte un FLAC: è il caso in cui si cerca con il file a metà.
        let bytes = wav(2, 44_100, 16, 4 * 44_100);
        let buffer = StreamBuffer::new();
        buffer.set_total(bytes.len(), true);
        buffer.write_at(0, &bytes[..4096]);
        let writer = buffer.clone();
        let rest = bytes.clone();
        let feeder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            writer.write_at(4096, &rest[4096..]);
        });
        let mut source = SymphoniaSource::open(Box::new(buffer.reader())).expect("the header arrived");
        source.try_seek(Duration::from_secs(2)).expect("the length is known");
        feeder.join().unwrap();
        assert_eq!(source.count(), 2 * 2 * 44_100);
    }

    #[test]
    fn a_truncated_file_is_an_error_not_a_panic() {
        assert!(SymphoniaSource::open(Box::new(Cursor::new(TONE_MOOV_END[..64].to_vec()))).is_err());
    }
}
