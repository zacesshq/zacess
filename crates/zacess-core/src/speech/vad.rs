//! Voice activity detection: where in a recording someone is speaking.
//!
//! Energy based and adaptive. The noise floor is estimated from the quietest frames of the
//! recording (or of what has been heard so far, live) and a frame counts as speech when it rises
//! clearly above that floor. Short gaps are bridged and short blips dropped, so a sentence comes
//! out as one span and a pause between sentences as a break.

use super::audio::{Audio, dbfs, rms};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VadConfig {
    /// Frame length in milliseconds.
    pub frame_ms: u32,
    /// How far above the noise floor a frame must be to count as speech, in decibels.
    pub margin_db: f32,
    /// Frames below this level never count as speech, whatever the floor, in dBFS.
    pub absolute_floor_db: f32,
    /// A gap shorter than this stays inside one span, in milliseconds.
    pub min_silence_ms: u32,
    /// A span shorter than this is dropped, in milliseconds.
    pub min_speech_ms: u32,
    /// Padding added to each end of a span, in milliseconds, so word edges are not clipped.
    pub padding_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        VadConfig {
            frame_ms: 20,
            margin_db: 12.0,
            absolute_floor_db: -55.0,
            min_silence_ms: 500,
            min_speech_ms: 150,
            padding_ms: 100,
        }
    }
}

impl VadConfig {
    /// Frames in a stretch of milliseconds, at least one.
    pub fn frames_in(&self, ms: u32) -> usize {
        (ms / self.frame_ms.max(1)).max(1) as usize
    }
}

/// Samples in one frame at a sample rate, at least one. Span times are derived from this rather
/// than from the nominal milliseconds, so the two never disagree.
pub fn frame_samples(sample_rate: u32, frame_ms: u32) -> usize {
    ((sample_rate as u64 * frame_ms.max(1) as u64) / 1000).max(1) as usize
}

pub fn frame_secs(sample_rate: u32, frame_ms: u32) -> f32 {
    frame_samples(sample_rate, frame_ms) as f32 / sample_rate.max(1) as f32
}

/// A stretch of speech, in seconds from the start of the recording.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpeechSpan {
    pub start: f32,
    pub end: f32,
}

impl SpeechSpan {
    pub fn duration(&self) -> f32 {
        self.end - self.start
    }
}

/// The level of each frame in dBFS.
pub fn frame_levels(audio: &Audio, frame_ms: u32) -> Vec<f32> {
    let frame = frame_samples(audio.sample_rate, frame_ms);
    audio.samples.chunks(frame).map(|c| dbfs(rms(c))).collect()
}

/// The noise floor: the level the quietest fifth of the frames sits at.
pub fn noise_floor(levels: &[f32]) -> f32 {
    if levels.is_empty() {
        return -100.0;
    }
    let mut sorted = levels.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted[sorted.len() / 5]
}

/// Finds the speech spans of a whole recording.
pub fn detect(audio: &Audio, config: &VadConfig) -> Vec<SpeechSpan> {
    let levels = frame_levels(audio, config.frame_ms);
    let floor = noise_floor(&levels);
    let threshold = (floor + config.margin_db).max(config.absolute_floor_db);
    let flags: Vec<bool> = levels.iter().map(|l| *l > threshold).collect();
    spans_from_flags(
        &flags,
        config,
        frame_secs(audio.sample_rate, config.frame_ms),
        audio.duration_secs(),
    )
}

/// Turns per-frame speech flags into padded spans with short gaps bridged and blips dropped.
/// Spans never overlap, whatever the padding.
pub fn spans_from_flags(
    flags: &[bool],
    config: &VadConfig,
    frame_secs: f32,
    total_secs: f32,
) -> Vec<SpeechSpan> {
    let mut raw: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for (i, &speech) in flags.iter().enumerate() {
        match (speech, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                raw.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        raw.push((s, flags.len()));
    }

    // bridge gaps shorter than the minimum silence
    let min_gap = config.frames_in(config.min_silence_ms);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in raw {
        match merged.last_mut() {
            Some(last) if s.saturating_sub(last.1) < min_gap => last.1 = e,
            _ => merged.push((s, e)),
        }
    }

    let min_frames = config.frames_in(config.min_speech_ms);
    let pad = config.padding_ms as f32 / 1000.0;
    let mut out: Vec<SpeechSpan> = Vec::new();
    for (s, e) in merged.into_iter().filter(|(s, e)| e - s >= min_frames) {
        let mut start = (s as f32 * frame_secs - pad).max(0.0);
        if let Some(previous) = out.last() {
            start = start.max(previous.end);
        }
        let end = (e as f32 * frame_secs + pad).min(total_secs).max(start);
        out.push(SpeechSpan { start, end });
    }
    out
}

/// A noise floor that follows the room while someone is talking. It drops quickly to meet a
/// quieter frame and rises only slowly, so a long unbroken stretch of speech is never taken for
/// the new background. It costs the same however long the session has run.
#[derive(Debug, Clone)]
struct FloorTracker {
    floor: Option<f32>,
}

impl FloorTracker {
    /// How far the floor falls towards a quieter frame, as a share of the gap.
    const FALL: f32 = 0.05;
    /// How far the floor may rise per frame, in decibels: 6 dB a minute at 50 frames a second.
    const RISE_DB: f32 = 0.002;

    fn new() -> Self {
        FloorTracker { floor: None }
    }

    fn push(&mut self, level_db: f32) -> f32 {
        let floor = match self.floor {
            None => level_db,
            Some(f) if level_db < f => f + (level_db - f) * Self::FALL,
            Some(f) => (f + Self::RISE_DB).min(level_db),
        };
        self.floor = Some(floor);
        floor
    }
}

/// Frame-by-frame detection for live audio, with the noise floor learnt as it goes.
#[derive(Debug, Clone)]
pub struct LiveVad {
    config: VadConfig,
    floor: FloorTracker,
    frames: u64,
    in_speech: bool,
    silent_frames: usize,
    speech_frames: usize,
}

/// What the live detector concluded about the latest frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameState {
    Silence,
    Speech,
    /// Speech just ended: a pause long enough to split on has passed. Carries how many frames of
    /// the stretch were speech, so a blip can be dropped as it is in a whole recording.
    SpeechEnded {
        speech_frames: usize,
    },
}

impl LiveVad {
    pub fn new(config: VadConfig) -> Self {
        LiveVad {
            config,
            floor: FloorTracker::new(),
            frames: 0,
            in_speech: false,
            silent_frames: 0,
            speech_frames: 0,
        }
    }

    pub fn config(&self) -> &VadConfig {
        &self.config
    }

    /// Classifies one frame of samples. The floor is learnt from everything heard so far, with
    /// the first second treated as silence until there is enough to judge by.
    pub fn push(&mut self, frame: &[f32]) -> FrameState {
        self.push_level(dbfs(rms(frame)))
    }

    /// [`LiveVad::push`] for a level already measured.
    pub fn push_level(&mut self, level: f32) -> FrameState {
        let floor = self.floor.push(level);
        self.frames += 1;
        let warm = self.frames >= self.config.frames_in(1000) as u64;
        let threshold = (floor + self.config.margin_db).max(self.config.absolute_floor_db);
        let speech = if warm {
            level > threshold
        } else {
            level > self.config.absolute_floor_db + 20.0
        };
        let min_gap = self.config.frames_in(self.config.min_silence_ms);
        if speech {
            self.silent_frames = 0;
            self.speech_frames += 1;
            self.in_speech = true;
            FrameState::Speech
        } else if self.in_speech {
            self.silent_frames += 1;
            if self.silent_frames >= min_gap {
                self.in_speech = false;
                let speech_frames = std::mem::take(&mut self.speech_frames);
                FrameState::SpeechEnded { speech_frames }
            } else {
                FrameState::Speech
            }
        } else {
            FrameState::Silence
        }
    }

    pub fn in_speech(&self) -> bool {
        self.in_speech
    }

    /// Frames of the current stretch that were speech rather than the pause after it.
    pub fn speech_frames(&self) -> usize {
        self.speech_frames
    }

    /// Whether a stretch had enough speech to be worth recognising.
    pub fn enough_speech(&self, speech_frames: usize) -> bool {
        speech_frames >= self.config.frames_in(self.config.min_speech_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone_with_gaps(rate: u32) -> Audio {
        // 0.5 s silence, 1 s tone, 0.6 s silence, 1 s tone, 0.1 s silence, 0.5 s tone, 0.5 s silence
        let mut samples = Vec::new();
        let mut push = |secs: f32, on: bool| {
            let n = (rate as f32 * secs) as usize;
            for i in 0..n {
                let t = i as f32 / rate as f32;
                samples.push(if on {
                    (t * 440.0 * std::f32::consts::TAU).sin() * 0.3
                } else {
                    0.0005 * ((i % 7) as f32 - 3.0)
                });
            }
        };
        push(0.5, false);
        push(1.0, true);
        push(0.6, false);
        push(1.0, true);
        push(0.1, false);
        push(0.5, true);
        push(0.5, false);
        Audio::new(rate, samples)
    }

    #[test]
    fn spans_follow_the_sound_and_short_gaps_are_bridged() {
        let audio = tone_with_gaps(16_000);
        let spans = detect(&audio, &VadConfig::default());
        assert_eq!(spans.len(), 2, "{spans:?}");
        assert!(
            (spans[0].start - 0.5).abs() <= 0.12 && (spans[0].end - 1.5).abs() <= 0.12,
            "{spans:?}"
        );
        // the 0.1 s gap is bridged, so the second span runs to the end of the third tone
        assert!(
            (spans[1].start - 2.1).abs() <= 0.12 && (spans[1].end - 3.7).abs() <= 0.12,
            "{spans:?}"
        );
        // an odd rate still gives spans at the right times
        let spans = detect(&tone_with_gaps(11_025), &VadConfig::default());
        assert_eq!(spans.len(), 2, "{spans:?}");
        assert!((spans[1].start - 2.1).abs() <= 0.12, "{spans:?}");
    }

    #[test]
    fn silence_alone_gives_no_spans_and_blips_are_dropped() {
        let quiet = Audio::new(16_000, vec![0.0001; 16_000]);
        assert!(detect(&quiet, &VadConfig::default()).is_empty());
        let config = VadConfig {
            frame_ms: 20,
            min_speech_ms: 200,
            min_silence_ms: 100,
            padding_ms: 0,
            ..VadConfig::default()
        };
        let flags: Vec<bool> = (0..100)
            .map(|i| (40..45).contains(&i) || (60..90).contains(&i))
            .collect();
        let spans = spans_from_flags(&flags, &config, 0.02, 2.0);
        assert_eq!(spans.len(), 1);
        assert!((spans[0].start - 1.2).abs() < 1e-6 && (spans[0].end - 1.8).abs() < 1e-6);
    }

    #[test]
    fn padded_spans_never_overlap() {
        // a 160 ms gap survives bridging at 150 ms but is shorter than twice the padding
        let config = VadConfig {
            min_silence_ms: 150,
            padding_ms: 100,
            ..VadConfig::default()
        };
        let flags: Vec<bool> = (0..100)
            .map(|i| (10..40).contains(&i) || (48..80).contains(&i))
            .collect();
        let spans = spans_from_flags(&flags, &config, 0.02, 2.0);
        assert_eq!(spans.len(), 2);
        assert!(spans[1].start >= spans[0].end, "{spans:?}");
        assert!((spans[0].end - 0.9).abs() < 1e-6 && (spans[1].start - 0.9).abs() < 1e-6);
    }

    #[test]
    fn the_live_detector_reports_when_speech_ends() {
        let audio = tone_with_gaps(16_000);
        let mut vad = LiveVad::new(VadConfig::default());
        let frame = 320;
        let mut ended_at = Vec::new();
        for (i, chunk) in audio.samples.chunks(frame).enumerate() {
            if let FrameState::SpeechEnded { speech_frames } = vad.push(chunk) {
                ended_at.push((i as f32 * 0.02, speech_frames));
            }
        }
        assert_eq!(ended_at.len(), 2, "{ended_at:?}");
        assert!((ended_at[0].0 - 2.0).abs() <= 0.1, "{ended_at:?}");
        assert!((ended_at[1].0 - 4.2).abs() <= 0.1, "{ended_at:?}");
        assert!(
            (45..=55).contains(&ended_at[0].1),
            "a second of tone: {ended_at:?}"
        );
        assert!(vad.enough_speech(ended_at[0].1) && !vad.enough_speech(2));
        assert!(!vad.in_speech());
    }

    #[test]
    fn the_floor_follows_the_room_and_is_not_lifted_by_long_speech() {
        let mut tracker = FloorTracker::new();
        // a quiet room, then a minute and a half of unbroken speech at 40 dB above it
        for _ in 0..100 {
            tracker.push(-60.0);
        }
        let quiet = tracker.floor.unwrap();
        assert!((quiet + 60.0).abs() < 0.5, "{quiet}");
        for _ in 0..4500 {
            tracker.push(-20.0);
        }
        let during = tracker.floor.unwrap();
        assert!(
            during < -45.0,
            "unbroken speech lifts the floor by under 10 dB: {during}"
        );
        // one pause brings it straight back
        for _ in 0..100 {
            tracker.push(-60.0);
        }
        assert!(tracker.floor.unwrap() < -55.0);
    }

    #[test]
    fn a_long_unbroken_stretch_is_one_utterance() {
        // a second of near silence, then 20 s of steady sound, then silence
        let rate = 16_000;
        let mut samples = vec![0.0001f32; rate];
        samples.extend(
            (0..rate * 20)
                .map(|i| (i as f32 / rate as f32 * 440.0 * std::f32::consts::TAU).sin() * 0.3),
        );
        samples.extend(vec![0.0001f32; rate]);
        let mut vad = LiveVad::new(VadConfig::default());
        let mut ended = 0;
        let mut early = 0;
        for (i, chunk) in samples.chunks(320).enumerate() {
            if let FrameState::SpeechEnded { .. } = vad.push(chunk) {
                ended += 1;
                if (i as f32 * 0.02) < 21.0 {
                    early += 1;
                }
            }
        }
        assert_eq!(
            (ended, early),
            (1, 0),
            "speech is not mistaken for background after a few seconds"
        );
    }
}
