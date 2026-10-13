//! Live captions: cutting the audio into pieces a recogniser can keep up with, turning the
//! results into caption lines a person can read and the live stream that does both as audio
//! arrives.

use super::audio::{Audio, RECOGNITION_RATE, Resampler, dbfs, rms};
use super::vad::{self, FrameState, LiveVad, SpeechSpan, VadConfig};
use super::{Recognised, Segment, SpeechError, TRANSCRIPT_SCHEMA_VERSION, Transcriber, Transcript};
use crate::model::{Provenance, RanOn};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionConfig {
    /// The longest piece handed to the recogniser at once, in seconds.
    pub max_chunk_secs: f32,
    /// While someone keeps talking, an interim caption is refreshed this often, in seconds.
    /// `None` turns interim captions off, for an engine that reloads its model on every call.
    pub interim_every_secs: Option<f32>,
    /// Characters per caption line. 37 follows the broadcast subtitle guidelines.
    pub max_line_chars: usize,
    pub max_lines: usize,
    /// A caption stays at least this long, in seconds, when the next one is not already due.
    pub min_display_secs: f32,
    /// Reading speed the display is paced to, in words per minute.
    pub words_per_minute: f32,
    pub vad: VadConfig,
}

impl Default for CaptionConfig {
    fn default() -> Self {
        CaptionConfig {
            max_chunk_secs: 8.0,
            interim_every_secs: Some(1.5),
            max_line_chars: 37,
            max_lines: 2,
            min_display_secs: 1.5,
            words_per_minute: 180.0,
            vad: VadConfig::default(),
        }
    }
}

/// Cuts speech spans into chunks no longer than the maximum. Given the frame levels of the
/// recording, each cut goes at the quietest frame within a second of the even split, so it falls
/// between words rather than inside one.
pub fn chunk_spans(
    spans: &[SpeechSpan],
    max_chunk_secs: f32,
    levels: Option<(&[f32], f32)>,
) -> Vec<SpeechSpan> {
    let max = max_chunk_secs.max(0.5);
    let mut out = Vec::new();
    for span in spans {
        let pieces = (span.duration() / max).ceil().max(1.0) as usize;
        let each = span.duration() / pieces as f32;
        let mut previous = span.start;
        for i in 1..pieces {
            let target = span.start + each * i as f32;
            // the window keeps every piece within the maximum whichever frame is chosen
            let lo = (target - 1.0)
                .max(span.end - (pieces - i) as f32 * max)
                .max(previous);
            let hi = (target + 1.0).min(previous + max).min(span.end);
            let cut = match levels {
                Some((levels, frame_secs)) if hi > lo => {
                    quietest_between(levels, frame_secs, lo, hi)
                }
                _ => target,
            };
            out.push(SpeechSpan {
                start: previous,
                end: cut,
            });
            previous = cut;
        }
        out.push(SpeechSpan {
            start: previous,
            end: span.end,
        });
    }
    out
}

/// The time of the quietest frame between two times.
fn quietest_between(levels: &[f32], frame_secs: f32, lo: f32, hi: f32) -> f32 {
    let first = (lo / frame_secs).ceil() as usize;
    let last = ((hi / frame_secs).floor() as usize).min(levels.len());
    if first >= last {
        return (lo + hi) / 2.0;
    }
    let mut best = (first, f32::MAX);
    for (i, &level) in levels.iter().enumerate().take(last).skip(first) {
        if level < best.1 {
            best = (i, level);
        }
    }
    best.0 as f32 * frame_secs
}

/// A caption ready to show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Caption {
    pub start: f32,
    pub end: f32,
    pub text: String,
    pub lines: Vec<String>,
}

/// Breaks text into lines of at most `max_chars` at the most natural points: sentence ends first,
/// then clause punctuation, then before small joining words, then any space. A number is never
/// parted from the unit after it. Only a single word longer than the limit breaks it.
pub fn break_lines(text: &str, max_chars: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let max_chars = max_chars.max(8);
    let mut lines: Vec<String> = Vec::new();
    let mut remaining: &[&str] = &words;
    while !remaining.is_empty() {
        let fits = longest_fit(remaining, max_chars);
        if fits == remaining.len() {
            lines.push(remaining.join(" "));
            break;
        }
        let cut = best_cut(remaining, fits);
        lines.push(remaining[..cut].join(" "));
        remaining = &remaining[cut..];
    }
    lines
}

fn longest_fit(words: &[&str], max_chars: usize) -> usize {
    let mut used = 0;
    for (i, w) in words.iter().enumerate() {
        let needed = if i == 0 {
            w.chars().count()
        } else {
            used + 1 + w.chars().count()
        };
        if needed > max_chars {
            return i.max(1);
        }
        used = needed;
    }
    words.len()
}

const JOINERS: &[&str] = &[
    "and", "or", "but", "so", "because", "which", "that", "with", "to", "of", "in", "on", "at",
    "for", "the", "a", "an", "is", "are",
];

fn is_unit(word: &str) -> bool {
    crate::confidence::is_numeric_or_symbol(word)
        && word.chars().next().is_some_and(char::is_alphabetic)
}

/// The best place to cut within the first `fits` words. A sentence end wins if it leaves at
/// least a third of the line, a clause break if it leaves half, a joining word if it leaves two
/// thirds; otherwise the line is filled. A number is never parted from its unit.
fn best_cut(words: &[&str], fits: usize) -> usize {
    let allowed = |cut: usize| {
        cut > 0
            && !(words[cut - 1].chars().any(|c| c.is_ascii_digit())
                && words.get(cut).is_some_and(|w| is_unit(w)))
    };
    let candidates = |least: usize, pred: &dyn Fn(usize) -> bool| {
        (1..=fits)
            .rev()
            .filter(|&c| c >= least)
            .find(|&c| allowed(c) && pred(c))
    };
    let joiner = |c: usize| {
        words.get(c).is_some_and(|w| {
            JOINERS.contains(
                &w.to_ascii_lowercase()
                    .trim_matches(|ch: char| !ch.is_alphanumeric()),
            )
        })
    };
    candidates(fits.div_ceil(3), &|c| {
        words[c - 1].ends_with(['.', '!', '?'])
    })
    .or_else(|| {
        candidates(fits.div_ceil(2), &|c| {
            words[c - 1].ends_with([',', ';', ':'])
        })
    })
    .or_else(|| candidates((fits * 2).div_ceil(3), &joiner))
    .or_else(|| (1..=fits).rev().find(|&c| allowed(c)))
    .unwrap_or(fits)
}

/// How long a caption needs on screen at the configured reading speed.
pub fn display_secs(text: &str, config: &CaptionConfig) -> f32 {
    let words = text.split_whitespace().count() as f32;
    (words / config.words_per_minute.max(60.0) * 60.0).max(config.min_display_secs)
}

/// Captions for one stretch of text between two times: lines grouped `max_lines` at a time and
/// the time shared out by characters, so a long sentence becomes several captions that each fit
/// the screen instead of one that overflows it.
pub fn captions_for(text: &str, start: f32, end: f32, config: &CaptionConfig) -> Vec<Caption> {
    let lines = break_lines(text, config.max_line_chars);
    if lines.is_empty() {
        return Vec::new();
    }
    let groups: Vec<&[String]> = lines.chunks(config.max_lines.max(1)).collect();
    let total_chars: usize = lines
        .iter()
        .map(|l| l.chars().count())
        .sum::<usize>()
        .max(1);
    let mut out = Vec::new();
    let mut at = start;
    for (i, group) in groups.iter().enumerate() {
        let chars: usize = group.iter().map(|l| l.chars().count()).sum();
        let group_end = if i + 1 == groups.len() {
            end.max(at)
        } else {
            (at + (end - start).max(0.0) * chars as f32 / total_chars as f32).min(end)
        };
        out.push(Caption {
            start: at,
            end: group_end,
            text: group.join(" "),
            lines: group.to_vec(),
        });
        at = group_end;
    }
    out
}

/// Turns recognised segments into timed captions: lines broken for reading, each shown long enough
/// to read and never overlapping the next.
pub fn schedule(segments: &[Segment], config: &CaptionConfig) -> Vec<Caption> {
    let mut out: Vec<Caption> = Vec::new();
    for (i, segment) in segments.iter().enumerate() {
        let text = segment.text.trim();
        if text.is_empty() {
            continue;
        }
        let wanted_end =
            segment.start + display_secs(text, config).max(segment.end - segment.start);
        let next_start = segments.get(i + 1).map(|s| s.start).unwrap_or(f32::MAX);
        let end = wanted_end.min(next_start).max(segment.end.min(next_start));
        out.extend(captions_for(text, segment.start, end, config));
    }
    out
}

/// What the live stream tells the interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptionEvent {
    /// Someone is still talking. The first `stable_lines` lines of the caption are the same text
    /// on the same rows as in the previous interim caption; only the rest needs redrawing.
    Interim {
        caption: Caption,
        stable_lines: usize,
    },
    /// The utterance has ended; this text is settled and has gone into the transcript.
    Final {
        captions: Vec<Caption>,
        segments: Vec<Segment>,
    },
}

/// Captions from audio as it arrives. Feed samples in any size; events come back as utterances
/// finish and, while they are still going, as interim refreshes. Recognition runs inside `push`,
/// so a caller feeds this from a thread of its own, never from the audio callback itself.
pub struct LiveCaptions<'a> {
    engine: &'a dyn Transcriber,
    config: CaptionConfig,
    language: Option<String>,
    detected_language: Option<String>,
    vad: LiveVad,
    frame_len: usize,
    resampler: Option<Resampler>,
    /// Samples at the recognition rate not yet gathered into a frame.
    pending: Vec<f32>,
    /// The last frames of silence, with their levels, prepended to the next utterance so the
    /// soft start of its first word is heard.
    preroll: VecDeque<(Vec<f32>, f32)>,
    preroll_frames: usize,
    /// The utterance being spoken, at the recognition rate, with the level of each frame.
    utterance: Vec<f32>,
    utterance_levels: Vec<f32>,
    utterance_start_frame: u64,
    /// Frames taken in so far. Time is derived from this count, never accumulated, so it does
    /// not drift over a long lecture.
    frames_heard: u64,
    since_interim: usize,
    /// Interim lines already shown and kept in place until the final.
    frozen: Vec<String>,
    frozen_words: usize,
    /// The lines the last interim caption showed, to tell the next one how many are unchanged.
    last_shown: Vec<String>,
    pub transcript: Vec<Segment>,
}

impl<'a> LiveCaptions<'a> {
    /// Refuses an engine that would send audio off the device.
    pub fn new(
        engine: &'a dyn Transcriber,
        config: CaptionConfig,
        language: Option<&str>,
    ) -> Result<Self, SpeechError> {
        Self::resume(engine, config, language, 0.0, Vec::new())
    }

    /// Continues a recording from a known time with the transcript so far, after a pause or a
    /// restart, so the two halves share one timeline and one run of ids.
    pub fn resume(
        engine: &'a dyn Transcriber,
        config: CaptionConfig,
        language: Option<&str>,
        at_secs: f32,
        transcript: Vec<Segment>,
    ) -> Result<Self, SpeechError> {
        if engine.runs_on() != RanOn::Device {
            return Err(SpeechError::WouldUpload(engine.name()));
        }
        let frame_len = vad::frame_samples(RECOGNITION_RATE, config.vad.frame_ms);
        let frames_heard =
            (at_secs.max(0.0) as f64 * RECOGNITION_RATE as f64 / frame_len as f64).round() as u64;
        Ok(LiveCaptions {
            engine,
            vad: LiveVad::new(config.vad.clone()),
            preroll_frames: config.vad.frames_in(config.vad.padding_ms),
            config,
            language: language.map(str::to_string),
            detected_language: None,
            frame_len,
            resampler: None,
            pending: Vec::new(),
            preroll: VecDeque::new(),
            utterance: Vec::new(),
            utterance_levels: Vec::new(),
            utterance_start_frame: 0,
            frames_heard,
            since_interim: 0,
            frozen: Vec::new(),
            frozen_words: 0,
            last_shown: Vec::new(),
            transcript,
        })
    }

    fn secs(&self, frames: u64) -> f32 {
        (frames as f64 * self.frame_len as f64 / RECOGNITION_RATE as f64) as f32
    }

    /// How much audio has been taken in, in seconds.
    pub fn position_secs(&self) -> f32 {
        self.secs(self.frames_heard)
    }

    /// The language the recogniser settled on, when none was given.
    pub fn detected_language(&self) -> Option<&str> {
        self.detected_language.as_deref()
    }

    fn frames_in_secs(&self, secs: f32) -> usize {
        ((secs * RECOGNITION_RATE as f32) / self.frame_len as f32)
            .round()
            .max(1.0) as usize
    }

    /// Feeds audio at any rate, which must stay the same for the life of the stream. Returns the
    /// events this batch produced.
    pub fn push(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<Vec<CaptionEvent>, SpeechError> {
        if sample_rate == 0 {
            return Err(SpeechError::Audio("a sample rate of zero".into()));
        }
        let resampler = self
            .resampler
            .get_or_insert_with(|| Resampler::new(sample_rate, RECOGNITION_RATE));
        if resampler.input_rate() != sample_rate {
            return Err(SpeechError::Audio(format!(
                "the sample rate changed from {} to {sample_rate} mid-stream",
                resampler.input_rate()
            )));
        }
        let resampled = resampler.push(samples);
        self.pending.extend(resampled);
        let mut events = Vec::new();
        while self.pending.len() >= self.frame_len {
            let frame: Vec<f32> = self.pending.drain(..self.frame_len).collect();
            let frame_index = self.frames_heard;
            // the clock moves before anything can fail, so an engine error never stalls it
            self.frames_heard += 1;
            let level = dbfs(rms(&frame));
            match self.vad.push_level(level) {
                FrameState::Speech => {
                    if self.utterance.is_empty() {
                        self.utterance_start_frame = frame_index - self.preroll.len() as u64;
                        for (samples, level) in self.preroll.drain(..) {
                            self.utterance.extend(samples);
                            self.utterance_levels.push(level);
                        }
                        self.since_interim = 0;
                        self.frozen.clear();
                        self.frozen_words = 0;
                        self.last_shown.clear();
                    }
                    self.utterance.extend_from_slice(&frame);
                    self.utterance_levels.push(level);
                    self.since_interim += 1;
                    let max_frames = self.frames_in_secs(self.config.max_chunk_secs);
                    if self.utterance_levels.len() >= max_frames {
                        events.extend(self.split_utterance()?);
                    } else if let Some(every) = self.config.interim_every_secs
                        && self.since_interim >= self.frames_in_secs(every)
                    {
                        self.since_interim = 0;
                        events.extend(self.interim()?);
                    }
                }
                FrameState::SpeechEnded { speech_frames } => {
                    self.utterance.extend_from_slice(&frame);
                    self.utterance_levels.push(level);
                    if self.vad.enough_speech(speech_frames) {
                        events.extend(self.finish_utterance()?);
                    } else {
                        // a door or a cough: not worth a recogniser call, which could invent words
                        self.discard_utterance();
                    }
                }
                FrameState::Silence => {
                    self.preroll.push_back((frame, level));
                    while self.preroll.len() > self.preroll_frames {
                        self.preroll.pop_front();
                    }
                }
            }
        }
        Ok(events)
    }

    /// Flushes whatever is still being spoken, at the end of a recording.
    pub fn finish(&mut self) -> Result<Vec<CaptionEvent>, SpeechError> {
        if self.utterance.is_empty() {
            return Ok(Vec::new());
        }
        if !self.vad.enough_speech(self.vad.speech_frames()) {
            self.discard_utterance();
            return Ok(Vec::new());
        }
        self.finish_utterance()
    }

    /// The transcript so far, as a whole recording's would be.
    pub fn into_transcript(self, now: i64) -> Transcript {
        Transcript {
            schema_version: TRANSCRIPT_SCHEMA_VERSION,
            recording: None,
            language: self.language.clone().or(self.detected_language.clone()),
            duration_secs: self.position_secs(),
            engine: Provenance {
                helper: self.engine.name(),
                model: self.engine.model(),
                ran_on: RanOn::Device,
                at: now,
            },
            segments: self.transcript,
        }
    }

    fn discard_utterance(&mut self) {
        self.utterance.clear();
        self.utterance_levels.clear();
        self.frozen.clear();
        self.frozen_words = 0;
        self.last_shown.clear();
    }

    fn recognise(&mut self, samples: &[f32], start_secs: f32) -> Result<Vec<Segment>, SpeechError> {
        let audio = Audio::new(RECOGNITION_RATE, samples.to_vec());
        let Recognised {
            mut segments,
            language,
        } = self
            .engine
            .transcribe(&audio, start_secs, self.language.as_deref())?;
        if self.detected_language.is_none() {
            self.detected_language = language;
        }
        for s in &mut segments {
            s.shift(start_secs);
        }
        Ok(segments)
    }

    fn interim(&mut self) -> Result<Vec<CaptionEvent>, SpeechError> {
        let samples = self.utterance.clone();
        let start = self.secs(self.utterance_start_frame);
        let segments = self.recognise(&samples, start)?;
        let text = joined(&segments);
        if text.is_empty() {
            return Ok(Vec::new());
        }
        // lines already shown stay where they are; only the tail is broken again
        let words: Vec<&str> = text.split_whitespace().collect();
        let rest = words[self.frozen_words.min(words.len())..].join(" ");
        let mut lines = break_lines(&rest, self.config.max_line_chars);
        if lines.len() > 1 {
            let complete = lines.len() - 1;
            for line in &lines[..complete] {
                self.frozen_words += line.split_whitespace().count();
            }
            self.frozen.extend(lines.drain(..complete));
        }
        let mut shown = self.frozen.clone();
        shown.extend(lines);
        let keep_from = shown.len().saturating_sub(self.config.max_lines.max(1));
        let shown = shown[keep_from..].to_vec();
        // a line counts as stable only when it is the same text on the same row as last time
        let stable_lines = shown
            .iter()
            .zip(&self.last_shown)
            .take_while(|(now, before)| now == before)
            .count();
        self.last_shown = shown.clone();
        let end = start + samples.len() as f32 / RECOGNITION_RATE as f32;
        Ok(vec![CaptionEvent::Interim {
            caption: Caption {
                start,
                end,
                text,
                lines: shown,
            },
            stable_lines,
        }])
    }

    /// Ends the utterance at the quietest moment of its last second, so a cut forced by length
    /// falls between words. The rest carries over as the start of the next one.
    fn split_utterance(&mut self) -> Result<Vec<CaptionEvent>, SpeechError> {
        let total = self.utterance_levels.len();
        let lookback = self.frames_in_secs(1.0).min(total / 2).max(1);
        let from = total - lookback;
        let mut quietest = (from, f32::MAX);
        for (i, &level) in self.utterance_levels.iter().enumerate().skip(from) {
            if level < quietest.1 {
                quietest = (i, level);
            }
        }
        let cut = quietest.0.max(1);
        let tail = self.utterance.split_off(cut * self.frame_len);
        let tail_levels = self.utterance_levels.split_off(cut);
        let head = std::mem::replace(&mut self.utterance, tail);
        self.utterance_levels = tail_levels;
        let start_frame = self.utterance_start_frame;
        self.utterance_start_frame += cut as u64;
        self.since_interim = 0;
        self.frozen.clear();
        self.frozen_words = 0;
        self.last_shown.clear();
        self.emit_final(head, start_frame)
    }

    fn finish_utterance(&mut self) -> Result<Vec<CaptionEvent>, SpeechError> {
        let samples = std::mem::take(&mut self.utterance);
        self.utterance_levels.clear();
        self.frozen.clear();
        self.frozen_words = 0;
        self.last_shown.clear();
        let start_frame = self.utterance_start_frame;
        self.emit_final(samples, start_frame)
    }

    fn emit_final(
        &mut self,
        samples: Vec<f32>,
        start_frame: u64,
    ) -> Result<Vec<CaptionEvent>, SpeechError> {
        let start = self.secs(start_frame);
        let audio_end = start + samples.len() as f32 / RECOGNITION_RATE as f32;
        let mut segments = self.recognise(&samples, start)?;
        let text = joined(&segments);
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let next_id = self.transcript.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        for (i, s) in segments.iter_mut().enumerate() {
            s.id = next_id + i as u32;
        }
        self.transcript.extend(segments.iter().cloned());
        let first_start = segments.first().map(|s| s.start).unwrap_or(start);
        let end = audio_end.max(first_start + display_secs(&text, &self.config));
        let captions = captions_for(&text, first_start, end, &self.config);
        Ok(vec![CaptionEvent::Final { captions, segments }])
    }
}

fn joined(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|s| s.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_spans_are_split_evenly_without_levels() {
        let spans = [
            SpeechSpan {
                start: 0.0,
                end: 20.0,
            },
            SpeechSpan {
                start: 25.0,
                end: 27.0,
            },
        ];
        let chunks = chunk_spans(&spans, 8.0, None);
        assert_eq!(chunks.len(), 4);
        assert!((chunks[0].duration() - 20.0 / 3.0).abs() < 1e-4);
        assert!((chunks[2].end - 20.0).abs() < 1e-4);
        assert_eq!(chunks[3], spans[1]);
    }

    #[test]
    fn long_spans_are_cut_at_the_quietest_moment() {
        // 12 s of sound with one dip at 6.4 s: the cut goes there, not at the 6.0 s midpoint
        let frame_secs = 0.02;
        let levels: Vec<f32> = (0..600)
            .map(|i| {
                if i == 320 {
                    -60.0
                } else {
                    -20.0 + (i % 3) as f32
                }
            })
            .collect();
        let spans = [SpeechSpan {
            start: 0.0,
            end: 12.0,
        }];
        let chunks = chunk_spans(&spans, 8.0, Some((&levels, frame_secs)));
        assert_eq!(chunks.len(), 2);
        assert!((chunks[0].end - 6.4).abs() < 1e-4, "{chunks:?}");
        assert!(chunks.iter().all(|c| c.duration() <= 8.0 + 1e-4));
        // the window never lets a piece grow past the maximum
        let long = [SpeechSpan {
            start: 0.0,
            end: 15.9,
        }];
        let quiet_late: Vec<f32> = (0..800)
            .map(|i| if i == 440 { -60.0 } else { -20.0 })
            .collect();
        let chunks = chunk_spans(&long, 8.0, Some((&quiet_late, frame_secs)));
        assert!(
            chunks.iter().all(|c| c.duration() <= 8.0 + 1e-4),
            "{chunks:?}"
        );
    }

    #[test]
    fn lines_break_at_natural_points_and_keep_numbers_with_units() {
        let lines = break_lines(
            "A square wave at 50 Hz has only odd harmonics, so the third is at 150 Hz.",
            37,
        );
        assert_eq!(
            lines,
            [
                "A square wave at 50 Hz has only odd",
                "harmonics, so the third is at 150 Hz."
            ]
        );
        assert!(lines.iter().all(|l| l.chars().count() <= 37));
        let tight = break_lines("The resistor is 4.7 kΩ at most", 20);
        assert!(tight.iter().all(|l| !l.ends_with("4.7")), "{tight:?}");
        assert_eq!(
            break_lines(
                "Welcome to week three. Today we look at the Fourier series.",
                37
            ),
            [
                "Welcome to week three.",
                "Today we look at the Fourier series."
            ]
        );
        assert!(break_lines("   ", 37).is_empty());
        let long = break_lines(
            "one two three four five six seven eight nine ten eleven twelve",
            12,
        );
        assert!(
            long.len() >= 5 && long.iter().all(|l| l.chars().count() <= 12),
            "{long:?}"
        );
    }

    #[test]
    fn long_text_becomes_several_captions_that_fit_the_screen() {
        let config = CaptionConfig::default();
        let text = "A lecturer who talks for eight seconds without a pause says about twenty words, which is more than two lines of thirty seven characters can hold.";
        let captions = captions_for(text, 10.0, 18.0, &config);
        assert!(captions.len() >= 2, "{captions:?}");
        assert!(captions.iter().all(|c| c.lines.len() <= 2));
        assert!(
            captions
                .iter()
                .all(|c| c.lines.iter().all(|l| l.chars().count() <= 37)),
            "{captions:?}"
        );
        assert_eq!(captions[0].start, 10.0);
        assert_eq!(captions.last().unwrap().end, 18.0);
        assert!(
            captions
                .windows(2)
                .all(|w| (w[0].end - w[1].start).abs() < 1e-6)
        );
        assert_eq!(
            captions
                .iter()
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            text
        );
    }

    #[test]
    fn captions_stay_long_enough_to_read_and_never_overlap() {
        let config = CaptionConfig::default();
        let segments = vec![
            Segment::new(1, 0.0, 0.5, "Yes."),
            Segment::new(
                2,
                1.0,
                4.0,
                "A square wave at fifty hertz has only odd harmonics.",
            ),
            Segment::new(3, 4.0, 4.4, "Right."),
        ];
        let captions = schedule(&segments, &config);
        assert_eq!(captions.len(), 3);
        assert!(
            (captions[0].end - 1.0).abs() < 1e-6,
            "held until the next caption starts"
        );
        assert!(
            (captions[1].end - 4.0).abs() < 1e-6,
            "a long sentence keeps its own timing"
        );
        assert!(
            (captions[2].end - 5.5).abs() < 1e-6,
            "the last caption gets the minimum display time"
        );
        assert_eq!(display_secs("one two three four five six", &config), 2.0);
    }
}
