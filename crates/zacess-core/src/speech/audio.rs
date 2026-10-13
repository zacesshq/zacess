//! Audio as the rest of the speech code wants it: mono samples from -1 to 1 at a known rate.

use super::SpeechError;
use std::path::Path;

/// The rate recognisers expect. Everything is brought to it before recognition.
pub const RECOGNITION_RATE: u32 = 16_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    pub sample_rate: u32,
    /// Mono samples from -1 to 1.
    pub samples: Vec<f32>,
}

impl Audio {
    pub fn new(sample_rate: u32, samples: Vec<f32>) -> Self {
        Audio {
            sample_rate,
            samples,
        }
    }

    /// Reads a WAV file: 8, 16, 24 or 32-bit integer or 32-bit float, any channel count.
    /// Channels are averaged to mono.
    pub fn from_wav(path: &Path) -> Result<Self, SpeechError> {
        let reader = hound::WavReader::open(path)
            .map_err(|e| SpeechError::Audio(format!("{}: {e}", path.display())))?;
        Self::from_reader(reader)
    }

    pub fn from_wav_bytes(bytes: &[u8]) -> Result<Self, SpeechError> {
        let reader = hound::WavReader::new(std::io::Cursor::new(bytes))
            .map_err(|e| SpeechError::Audio(e.to_string()))?;
        Self::from_reader(reader)
    }

    fn from_reader<R: std::io::Read>(mut reader: hound::WavReader<R>) -> Result<Self, SpeechError> {
        let spec = reader.spec();
        if spec.channels == 0 || spec.sample_rate == 0 {
            return Err(SpeechError::Audio(
                "the WAV header has no channels or no sample rate".into(),
            ));
        }
        // the header is checked before it is used as a shift count: hound only rejects a width
        // wider than its byte count, so a corrupt file could claim 72 bits
        if !(8..=32).contains(&spec.bits_per_sample) {
            return Err(SpeechError::Audio(format!(
                "{}-bit samples are not supported",
                spec.bits_per_sample
            )));
        }
        let interleaved: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| SpeechError::Audio(e.to_string()))?,
            hound::SampleFormat::Int => {
                let scale = (1u64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| v as f32 / scale))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| SpeechError::Audio(e.to_string()))?
            }
        };
        let channels = spec.channels as usize;
        let samples = if channels == 1 {
            interleaved
        } else {
            interleaved
                .chunks(channels)
                .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
                .collect()
        };
        Ok(Audio {
            sample_rate: spec.sample_rate,
            samples,
        })
    }

    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.samples.len() as f32 / self.sample_rate as f32
    }

    /// Changes the sample rate by linear interpolation, which is enough for speech recognition
    /// input and keeps the code dependency free. A rate of zero on either side gives silence.
    pub fn resampled(&self, sample_rate: u32) -> Audio {
        if sample_rate == self.sample_rate || self.samples.is_empty() {
            return Audio {
                sample_rate,
                samples: self.samples.clone(),
            };
        }
        if sample_rate == 0 || self.sample_rate == 0 {
            return Audio {
                sample_rate,
                samples: Vec::new(),
            };
        }
        let mut resampler = Resampler::new(self.sample_rate, sample_rate);
        let mut samples = resampler.push(&self.samples);
        samples.extend(resampler.finish());
        Audio {
            sample_rate,
            samples,
        }
    }

    /// The part of the audio between two times, clamped to the recording.
    pub fn slice(&self, start_secs: f32, end_secs: f32) -> Audio {
        let to_index = |t: f32| {
            ((t.max(0.0) * self.sample_rate as f32).round() as usize).min(self.samples.len())
        };
        let (a, b) = (to_index(start_secs), to_index(end_secs));
        Audio {
            sample_rate: self.sample_rate,
            samples: self.samples[a..b.max(a)].to_vec(),
        }
    }

    /// Root mean square level from 0 to 1.
    pub fn rms(&self) -> f32 {
        rms(&self.samples)
    }

    /// Writes 16-bit mono WAV, for handing a chunk to a command-line recogniser.
    pub fn to_wav_bytes(&self) -> Result<Vec<u8>, SpeechError> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: self.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec)
                .map_err(|e| SpeechError::Audio(e.to_string()))?;
            for s in &self.samples {
                writer
                    .write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                    .map_err(|e| SpeechError::Audio(e.to_string()))?;
            }
            writer
                .finalize()
                .map_err(|e| SpeechError::Audio(e.to_string()))?;
        }
        Ok(cursor.into_inner())
    }
}

/// A resampler that is fed in pieces, as a microphone delivers them. It keeps its place between
/// pieces. Resampling each piece on its own would round its length and the rounding would add up
/// to seconds over a lecture.
#[derive(Debug, Clone)]
pub struct Resampler {
    from: u32,
    ratio: f64,
    /// Where the next output sample falls in `buffer`, in input samples.
    position: f64,
    buffer: Vec<f32>,
    /// Input samples taken in so far.
    pub input_samples: u64,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Resampler {
            from,
            ratio: from as f64 / to.max(1) as f64,
            position: 0.0,
            buffer: Vec::new(),
            input_samples: 0,
        }
    }

    pub fn input_rate(&self) -> u32 {
        self.from
    }

    /// Takes in more input and returns the output it makes possible.
    pub fn push(&mut self, samples: &[f32]) -> Vec<f32> {
        self.input_samples += samples.len() as u64;
        if (self.ratio - 1.0).abs() < 1e-12 {
            return samples.to_vec();
        }
        self.buffer.extend_from_slice(samples);
        let mut out = Vec::new();
        loop {
            let index = self.position.floor() as usize;
            if index + 1 >= self.buffer.len() {
                break;
            }
            let t = (self.position - index as f64) as f32;
            out.push(self.buffer[index] * (1.0 - t) + self.buffer[index + 1] * t);
            self.position += self.ratio;
        }
        let consumed = (self.position.floor() as usize).min(self.buffer.len());
        self.buffer.drain(..consumed);
        self.position -= consumed as f64;
        out
    }

    /// The last output sample, which needs an input sample after it that never comes.
    pub fn finish(&mut self) -> Vec<f32> {
        let mut out = Vec::new();
        if let Some(&last) = self.buffer.last()
            && (self.position.floor() as usize) < self.buffer.len()
        {
            out.push(last);
        }
        self.buffer.clear();
        self.position = 0.0;
        out
    }
}

pub(crate) fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// A level from 0 to 1 in decibels relative to full scale, floored at -100 dB.
pub(crate) fn dbfs(level: f32) -> f32 {
    if level <= 1e-5 {
        -100.0
    } else {
        20.0 * level.log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, secs: f32, hz: f32) -> Audio {
        let n = (rate as f32 * secs) as usize;
        Audio::new(
            rate,
            (0..n)
                .map(|i| (i as f32 / rate as f32 * hz * std::f32::consts::TAU).sin() * 0.5)
                .collect(),
        )
    }

    #[test]
    fn wav_round_trips_through_bytes_and_averages_channels() {
        let audio = sine(16_000, 0.5, 440.0);
        let bytes = audio.to_wav_bytes().unwrap();
        let back = Audio::from_wav_bytes(&bytes).unwrap();
        assert_eq!(back.sample_rate, 16_000);
        assert_eq!(back.samples.len(), audio.samples.len());
        assert!((back.rms() - audio.rms()).abs() < 1e-3);

        // a stereo file with one silent channel comes out at half the level
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut w = hound::WavWriter::new(&mut cursor, spec).unwrap();
            for _ in 0..800 {
                w.write_sample(16_000i16).unwrap();
                w.write_sample(0i16).unwrap();
            }
            w.finalize().unwrap();
        }
        let stereo = Audio::from_wav_bytes(&cursor.into_inner()).unwrap();
        assert_eq!(stereo.samples.len(), 800);
        assert!((stereo.samples[0] - 16_000.0 / 32_768.0 / 2.0).abs() < 1e-6);
        assert!(Audio::from_wav_bytes(b"not a wav").is_err());
    }

    #[test]
    fn a_header_claiming_an_absurd_width_is_refused() {
        // hound accepts the header because 72 bits fit in the 9-byte block; the scale must not
        // be computed from it
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&36u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8_000u32.to_le_bytes());
        bytes.extend_from_slice(&72_000u32.to_le_bytes());
        bytes.extend_from_slice(&9u16.to_le_bytes());
        bytes.extend_from_slice(&72u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        assert!(Audio::from_wav_bytes(&bytes).is_err());
    }

    #[test]
    fn resampling_keeps_duration_and_level() {
        let audio = sine(22_050, 1.0, 300.0);
        let down = audio.resampled(16_000);
        assert_eq!(down.sample_rate, 16_000);
        assert!((down.duration_secs() - 1.0).abs() < 0.001);
        assert!((down.rms() - audio.rms()).abs() < 0.01);
        let same = audio.resampled(22_050);
        assert_eq!(same.samples, audio.samples);
        assert!(
            Audio::new(8_000, vec![])
                .resampled(16_000)
                .samples
                .is_empty()
        );
        assert!(
            Audio::new(0, vec![0.0; 100])
                .resampled(16_000)
                .samples
                .is_empty()
        );
        let up = sine(8_000, 0.5, 100.0).resampled(16_000);
        assert!((up.duration_secs() - 0.5).abs() < 0.001);
    }

    #[test]
    fn a_resampler_fed_in_odd_pieces_keeps_time() {
        // 1024-sample callbacks at 44.1 kHz do not divide into 16 kHz frames; the output must
        // still come out at the right rate over a long run
        let audio = sine(44_100, 30.0, 440.0);
        let mut resampler = Resampler::new(44_100, 16_000);
        let mut out = Vec::new();
        for piece in audio.samples.chunks(1024) {
            out.extend(resampler.push(piece));
        }
        out.extend(resampler.finish());
        let expected = (audio.samples.len() as f64 * 16_000.0 / 44_100.0).round() as i64;
        assert!(
            (out.len() as i64 - expected).abs() <= 2,
            "{} vs {expected}",
            out.len()
        );
        assert!((rms(&out) - audio.rms()).abs() < 0.01);
        assert_eq!(resampler.input_samples, audio.samples.len() as u64);
    }

    #[test]
    fn slices_are_clamped_to_the_recording() {
        let audio = sine(1_000, 2.0, 50.0);
        assert_eq!(audio.slice(0.5, 1.0).samples.len(), 500);
        assert_eq!(audio.slice(1.5, 9.0).samples.len(), 500);
        assert!(audio.slice(3.0, 4.0).samples.is_empty());
        assert!(audio.slice(1.0, 0.5).samples.is_empty());
    }

    #[test]
    fn levels_in_decibels() {
        assert_eq!(dbfs(0.0), -100.0);
        assert!((dbfs(1.0)).abs() < 1e-6);
        assert!((dbfs(0.1) + 20.0).abs() < 1e-4);
    }
}
