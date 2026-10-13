//! On-device transcription and live captions.
//!
//! Audio never leaves the device: the recogniser sits behind the [`Transcriber`] trait and the
//! pipeline refuses one that would upload. Around the recogniser this module does the rest: it
//! decodes and resamples audio, finds where speech is, cuts it into pieces a recogniser can keep
//! up with, turns results into captions a person can read and keeps a transcript that can be
//! searched, corrected and jumped through. The synthetic sample in `tests/fixtures/audio`
//! exercises all of it; the recogniser itself runs in the tests only where whisper.cpp is
//! installed. Reading equations aloud is a different thing and lives in `maths::speech`.

pub mod audio;
pub mod captions;
pub mod vad;
pub mod whisper;

use crate::confidence::threshold_after;
use crate::model::{Confidence, Provenance, RanOn, RunStatus};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub use audio::{Audio, RECOGNITION_RATE, Resampler};
pub use captions::{
    Caption, CaptionConfig, CaptionEvent, LiveCaptions, break_lines, captions_for, schedule,
};
pub use vad::{SpeechSpan, VadConfig};
pub use whisper::WhisperCli;

/// Bumped when a stored transcript's shape changes.
pub const TRANSCRIPT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum SpeechError {
    #[error("reading audio: {0}")]
    Audio(String),
    #[error("the recogniser failed: {0}")]
    Engine(String),
    #[error("{0} would send audio off the device, which transcription never does")]
    WouldUpload(String),
    #[error("no recogniser is installed on this device")]
    NoEngine,
    #[error("no word {word} in segment {segment}")]
    NoWord { segment: u32, word: usize },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// One recognised word with its time and how sure the recogniser was. The confidence is `None`
/// when the recogniser gave none, which is never passed off as certainty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub start: f32,
    pub end: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    /// Whether a person has confirmed or corrected the word, as for a run of OCR text.
    #[serde(default, skip_serializing_if = "is_default")]
    pub status: RunStatus,
    /// The recognised text before a correction, so the original is always one step away.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_text: Option<String>,
}

impl Word {
    pub fn new(text: &str, start: f32, end: f32, confidence: Option<Confidence>) -> Self {
        Word {
            text: text.to_string(),
            start,
            end,
            confidence,
            status: RunStatus::Recognised,
            original_text: None,
        }
    }

    pub fn is_checked(&self) -> bool {
        self.status != RunStatus::Recognised
    }
}

/// A stretch of recognised speech, usually a sentence or two between pauses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub id: u32,
    pub start: f32,
    pub end: f32,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<Word>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
}

/// A sentence within a segment, with its own time, so one can be tapped to jump to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sentence {
    pub segment: u32,
    pub start: f32,
    pub end: f32,
    pub text: String,
}

impl Segment {
    pub fn new(id: u32, start: f32, end: f32, text: &str) -> Self {
        Segment {
            id,
            start,
            end,
            text: text.trim().to_string(),
            words: Vec::new(),
            speaker: None,
        }
    }

    /// Words spread evenly across the segment, for a recogniser that gives no word times. They
    /// carry no confidence, because none was measured.
    pub fn even_words(text: &str, start: f32, end: f32) -> Vec<Word> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let each = (end - start).max(0.0) / words.len().max(1) as f32;
        words
            .iter()
            .enumerate()
            .map(|(i, w)| {
                Word::new(
                    w,
                    start + each * i as f32,
                    start + each * (i + 1) as f32,
                    None,
                )
            })
            .collect()
    }

    /// Moves the segment later by `secs`, for results of a chunk cut from a recording.
    pub fn shift(&mut self, secs: f32) {
        self.start += secs;
        self.end += secs;
        for w in &mut self.words {
            w.start += secs;
            w.end += secs;
        }
    }

    /// Words the recogniser was unsure of, by the same thresholds as OCR: a number or a unit
    /// after one must be surer than an ordinary word. Checked words are never flagged.
    pub fn uncertain_words(&self) -> impl Iterator<Item = &Word> {
        self.words
            .iter()
            .enumerate()
            .filter(|(i, w)| {
                let previous = i.checked_sub(1).map(|j| self.words[j].text.as_str());
                !w.is_checked()
                    && w.confidence
                        .is_some_and(|c| c.value() < threshold_after(previous, &w.text))
            })
            .map(|(_, w)| w)
    }

    /// The sentences of the segment, split after sentence punctuation using the word times. A
    /// segment with no words is one sentence.
    pub fn sentences(&self) -> Vec<Sentence> {
        if self.words.is_empty() {
            return vec![Sentence {
                segment: self.id,
                start: self.start,
                end: self.end,
                text: self.text.clone(),
            }];
        }
        let mut out = Vec::new();
        let mut current: Vec<&Word> = Vec::new();
        for word in &self.words {
            current.push(word);
            if word.text.ends_with(['.', '!', '?']) {
                out.push(sentence_of(self.id, &current));
                current.clear();
            }
        }
        if !current.is_empty() {
            out.push(sentence_of(self.id, &current));
        }
        out
    }

    /// Puts the text back together from the words after one of them changed.
    fn rebuild_text(&mut self) {
        if !self.words.is_empty() {
            self.text = self
                .words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
        }
    }
}

fn sentence_of(segment: u32, words: &[&Word]) -> Sentence {
    Sentence {
        segment,
        start: words.first().map(|w| w.start).unwrap_or(0.0),
        end: words.last().map(|w| w.end).unwrap_or(0.0),
        text: words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// What a recogniser returns for one piece of audio.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Recognised {
    pub segments: Vec<Segment>,
    /// The language the recogniser settled on, when it listened for one.
    pub language: Option<String>,
}

/// A speech recogniser. Implementations run on the device; a cloud one is refused by the pipeline.
pub trait Transcriber {
    fn name(&self) -> String;
    fn runs_on(&self) -> RanOn;
    fn model(&self) -> Option<String> {
        None
    }
    /// Recognises one piece of audio at [`RECOGNITION_RATE`]. `offset_secs` is where the piece
    /// sits in the recording, for engines that want context; the returned times are relative to
    /// the piece and are shifted by the caller. With no language the engine listens for one.
    fn transcribe(
        &self,
        audio: &Audio,
        offset_secs: f32,
        language: Option<&str>,
    ) -> Result<Recognised, SpeechError>;
}

/// Where a search hit sits in the transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub segment: u32,
    /// The time to jump to: the first matching word when word times are known.
    pub start: f32,
    pub text: String,
}

/// The transcript of a recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    pub schema_version: u32,
    /// The file name of the recording it describes, when it came from a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording: Option<String>,
    pub language: Option<String>,
    pub duration_secs: f32,
    pub engine: Provenance,
    pub segments: Vec<Segment>,
}

impl Transcript {
    pub fn text(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The segment being spoken at a time, for following along in a recording.
    pub fn at(&self, secs: f32) -> Option<&Segment> {
        self.segments
            .iter()
            .find(|s| secs >= s.start && secs < s.end)
            .or_else(|| self.segments.iter().rev().find(|s| s.start <= secs))
    }

    /// Every sentence with its time, for a transcript view where any one can be tapped.
    pub fn sentences(&self) -> Vec<Sentence> {
        self.segments.iter().flat_map(Segment::sentences).collect()
    }

    /// Segments containing the words of a query, ignoring case, each with the time to jump to.
    pub fn search(&self, query: &str) -> Vec<Hit> {
        let terms: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();
        if terms.is_empty() {
            return Vec::new();
        }
        self.segments
            .iter()
            .filter(|s| {
                let lower = s.text.to_lowercase();
                terms.iter().all(|t| lower.contains(t))
            })
            .map(|s| {
                let first_word = s
                    .words
                    .iter()
                    .find(|w| terms.iter().any(|t| w.text.to_lowercase().contains(t)))
                    .map(|w| w.start);
                Hit {
                    segment: s.id,
                    start: first_word.unwrap_or(s.start),
                    text: s.text.clone(),
                }
            })
            .collect()
    }

    /// Every word below its confidence threshold, with its segment.
    pub fn uncertain_words(&self) -> Vec<(u32, &Word)> {
        self.segments
            .iter()
            .flat_map(|s| s.uncertain_words().map(move |w| (s.id, w)))
            .collect()
    }

    fn word_mut(
        &mut self,
        segment: u32,
        word: usize,
    ) -> Result<(&mut Segment, usize), SpeechError> {
        let segment_ref = self
            .segments
            .iter_mut()
            .find(|s| s.id == segment)
            .ok_or(SpeechError::NoWord { segment, word })?;
        if word >= segment_ref.words.len() {
            return Err(SpeechError::NoWord { segment, word });
        }
        Ok((segment_ref, word))
    }

    /// Marks a word as checked and right.
    pub fn confirm(&mut self, segment: u32, word: usize) -> Result<(), SpeechError> {
        let (s, i) = self.word_mut(segment, word)?;
        s.words[i].status = RunStatus::Confirmed;
        Ok(())
    }

    /// Replaces a word with what was really said, keeping the recognised text one step away.
    pub fn correct(&mut self, segment: u32, word: usize, text: &str) -> Result<(), SpeechError> {
        let (s, i) = self.word_mut(segment, word)?;
        let w = &mut s.words[i];
        if w.original_text.is_none() {
            w.original_text = Some(w.text.clone());
        }
        w.text = text.trim().to_string();
        w.status = RunStatus::Corrected;
        s.rebuild_text();
        Ok(())
    }

    /// WebVTT, the caption format browsers and players read.
    pub fn to_webvtt(&self, config: &CaptionConfig) -> String {
        let mut out = String::from("WEBVTT\n\n");
        for (i, caption) in schedule(&self.segments, config).iter().enumerate() {
            out.push_str(&format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                vtt_time(caption.start),
                vtt_time(caption.end),
                caption.lines.join("\n")
            ));
        }
        out
    }

    /// SubRip, for players that do not read WebVTT.
    pub fn to_srt(&self, config: &CaptionConfig) -> String {
        let mut out = String::new();
        for (i, caption) in schedule(&self.segments, config).iter().enumerate() {
            out.push_str(&format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                vtt_time(caption.start).replace('.', ","),
                vtt_time(caption.end).replace('.', ","),
                caption.lines.join("\n")
            ));
        }
        out
    }
}

fn vtt_time(secs: f32) -> String {
    let total_ms = (secs.max(0.0) * 1000.0).round() as u64;
    let (h, m, s, ms) = (
        total_ms / 3_600_000,
        total_ms / 60_000 % 60,
        total_ms / 1000 % 60,
        total_ms % 1000,
    );
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

/// Transcribes a whole recording on the device: speech is found, cut into chunks at quiet moments
/// and recognised chunk by chunk, so even a long lecture needs only a little memory at a time.
pub fn transcribe(
    audio: &Audio,
    engine: &dyn Transcriber,
    config: &CaptionConfig,
    language: Option<&str>,
    now: i64,
) -> Result<Transcript, SpeechError> {
    if engine.runs_on() != RanOn::Device {
        return Err(SpeechError::WouldUpload(engine.name()));
    }
    let audio = audio.resampled(RECOGNITION_RATE);
    let levels = vad::frame_levels(&audio, config.vad.frame_ms);
    let frame_secs = vad::frame_secs(RECOGNITION_RATE, config.vad.frame_ms);
    let spans = vad::detect(&audio, &config.vad);
    let chunks = captions::chunk_spans(&spans, config.max_chunk_secs, Some((&levels, frame_secs)));
    let mut segments = Vec::new();
    let mut detected: Option<String> = None;
    for chunk in chunks {
        let piece = audio.slice(chunk.start, chunk.end);
        let recognised = engine.transcribe(&piece, chunk.start, language)?;
        if detected.is_none() {
            detected = recognised.language;
        }
        for mut segment in recognised.segments {
            segment.shift(chunk.start);
            if segment.text.trim().is_empty() {
                continue;
            }
            segment.id = segments.len() as u32 + 1;
            segments.push(segment);
        }
    }
    Ok(Transcript {
        schema_version: TRANSCRIPT_SCHEMA_VERSION,
        recording: None,
        language: language.map(str::to_string).or(detected),
        duration_secs: audio.duration_secs(),
        engine: Provenance {
            helper: engine.name(),
            model: engine.model(),
            ran_on: RanOn::Device,
            at: now,
        },
        segments,
    })
}

/// Transcribes a WAV file and records which file it was.
pub fn transcribe_file(
    path: &Path,
    engine: &dyn Transcriber,
    config: &CaptionConfig,
    language: Option<&str>,
    now: i64,
) -> Result<Transcript, SpeechError> {
    let mut transcript = transcribe(&Audio::from_wav(path)?, engine, config, language, now)?;
    transcript.recording = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string);
    Ok(transcript)
}

/// The recogniser installed on this device, if any.
pub fn detect_engine() -> Option<Box<dyn Transcriber>> {
    WhisperCli::detect().map(|w| Box::new(w) as Box<dyn Transcriber>)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;

    pub(crate) fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/audio")
            .join(name)
    }

    /// The sample's sidecar: what was said and when.
    #[derive(Debug, serde::Deserialize)]
    pub(crate) struct Sidecar {
        pub sample_rate: u32,
        pub duration: f32,
        pub utterances: Vec<Utterance>,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    pub(crate) struct Utterance {
        pub text: String,
        pub start: f32,
        pub end: f32,
    }

    pub(crate) fn sidecar() -> Sidecar {
        serde_json::from_str(&std::fs::read_to_string(fixture("lecture-sample.json")).unwrap())
            .unwrap()
    }

    /// Stands in for a recogniser: it knows what was said when, so a chunk's text is whatever the
    /// sidecar places inside it. It exercises everything but the recognition itself.
    pub(crate) struct Scripted {
        pub utterances: Vec<Utterance>,
        pub runs_on: RanOn,
    }

    impl Scripted {
        pub(crate) fn from_sidecar() -> Self {
            Scripted {
                utterances: sidecar().utterances,
                runs_on: RanOn::Device,
            }
        }
    }

    impl Transcriber for Scripted {
        fn name(&self) -> String {
            "scripted".into()
        }
        fn runs_on(&self) -> RanOn {
            self.runs_on
        }
        fn transcribe(
            &self,
            audio: &Audio,
            offset_secs: f32,
            language: Option<&str>,
        ) -> Result<Recognised, SpeechError> {
            let end = offset_secs + audio.duration_secs();
            let segments = self
                .utterances
                .iter()
                .filter(|u| u.end > offset_secs + 0.05 && u.start < end - 0.05)
                .enumerate()
                .map(|(i, u)| {
                    let (s, e) = (
                        u.start.max(offset_secs) - offset_secs,
                        u.end.min(end) - offset_secs,
                    );
                    let mut seg = Segment::new(i as u32 + 1, s, e, &u.text);
                    seg.words = Segment::even_words(&u.text, s, e);
                    seg
                })
                .collect();
            Ok(Recognised {
                segments,
                language: language.is_none().then(|| "en".to_string()),
            })
        }
    }

    #[test]
    fn the_sample_decodes_as_its_sidecar_says() {
        let meta = sidecar();
        let audio = Audio::from_wav(&fixture("lecture-sample.wav")).unwrap();
        assert_eq!(audio.sample_rate, meta.sample_rate);
        assert!((audio.duration_secs() - meta.duration).abs() < 0.01);
        let at_16k = audio.resampled(RECOGNITION_RATE);
        assert!((at_16k.duration_secs() - meta.duration).abs() < 0.01);
    }

    #[test]
    fn speech_is_found_where_the_sample_has_utterances() {
        let meta = sidecar();
        let audio = Audio::from_wav(&fixture("lecture-sample.wav"))
            .unwrap()
            .resampled(RECOGNITION_RATE);
        let spans = vad::detect(&audio, &VadConfig::default());
        assert_eq!(spans.len(), meta.utterances.len(), "{spans:?}");
        for (span, u) in spans.iter().zip(&meta.utterances) {
            assert!(
                (span.start - u.start).abs() <= 0.25,
                "start {span:?} vs {u:?}"
            );
            assert!((span.end - u.end).abs() <= 0.35, "end {span:?} vs {u:?}");
        }
    }

    #[test]
    fn a_recording_becomes_a_searchable_transcript_with_captions() {
        let engine = Scripted::from_sidecar();
        let config = CaptionConfig::default();
        let transcript =
            transcribe_file(&fixture("lecture-sample.wav"), &engine, &config, None, 7).unwrap();
        assert_eq!(transcript.segments.len(), 3);
        assert_eq!(transcript.engine.ran_on, RanOn::Device);
        assert_eq!(transcript.recording.as_deref(), Some("lecture-sample.wav"));
        assert_eq!(
            transcript.language.as_deref(),
            Some("en"),
            "the engine listened for the language"
        );
        assert_eq!(
            transcript.segments[1].text,
            "A square wave at fifty hertz has only odd harmonics."
        );
        assert!(
            (transcript.segments[1].start - 6.275).abs() < 0.3,
            "{}",
            transcript.segments[1].start
        );
        assert!(
            transcript
                .segments
                .windows(2)
                .all(|w| w[0].end <= w[1].start + 0.01)
        );

        let hits = transcript.search("odd harmonics");
        assert_eq!(hits.len(), 1);
        assert_eq!(transcript.search("harmonic")[0].segment, 2);
        assert!(transcript.search("capital of france").is_empty());
        let fourier = &transcript.search("fourier")[0];
        assert!(
            fourier.start > transcript.segments[0].start + 1.0,
            "a hit jumps to the word, not the segment: {fourier:?}"
        );
        assert_eq!(transcript.at(8.0).map(|s| s.id), Some(2));
        assert_eq!(
            transcript.at(5.5).map(|s| s.id),
            Some(1),
            "a pause belongs to the sentence before it"
        );

        let sentences = transcript.sentences();
        assert_eq!(sentences.len(), 4, "{sentences:?}");
        assert_eq!(sentences[1].text, "Today we look at the Fourier series.");
        assert!(sentences[1].start > sentences[0].start && sentences[1].segment == 1);

        let vtt = transcript.to_webvtt(&config);
        assert!(vtt.starts_with("WEBVTT\n\n1\n00:00:00."), "{vtt}");
        assert!(
            vtt.contains("A square wave at fifty hertz has only\nodd harmonics."),
            "{vtt}"
        );
        assert!(transcript.to_srt(&config).contains(" --> 00:00:0"));
        let json = serde_json::to_string(&transcript).unwrap();
        assert_eq!(
            serde_json::from_str::<Transcript>(&json).unwrap(),
            transcript
        );
    }

    #[test]
    fn words_can_be_confirmed_and_corrected_and_numbers_need_more_certainty() {
        let engine = Scripted::from_sidecar();
        let mut transcript = transcribe_file(
            &fixture("lecture-sample.wav"),
            &engine,
            &CaptionConfig::default(),
            Some("en"),
            0,
        )
        .unwrap();
        assert!(
            transcript.uncertain_words().is_empty(),
            "no confidence was measured, so nothing is claimed"
        );
        transcript.correct(2, 4, "50").unwrap();
        let word = &transcript.segments[1].words[4];
        assert_eq!(
            (
                word.text.as_str(),
                word.original_text.as_deref(),
                word.status
            ),
            ("50", Some("fifty"), RunStatus::Corrected)
        );
        assert!(
            transcript.segments[1].text.contains("at 50 hertz"),
            "{}",
            transcript.segments[1].text
        );
        transcript.confirm(2, 5).unwrap();
        assert!(transcript.segments[1].words[5].is_checked());
        assert!(matches!(
            transcript.correct(9, 0, "x"),
            Err(SpeechError::NoWord {
                segment: 9,
                word: 0
            })
        ));
        assert!(transcript.correct(2, 99, "x").is_err());

        let mut segment = Segment::new(1, 0.0, 2.0, "about 150 Hz today");
        segment.words = vec![
            Word::new("about", 0.0, 0.5, Some(Confidence::new(0.9))),
            Word::new("150", 0.5, 1.0, Some(Confidence::new(0.9))),
            Word::new("Hz", 1.0, 1.5, Some(Confidence::new(0.9))),
            Word::new("today", 1.5, 2.0, Some(Confidence::new(0.7))),
        ];
        let flagged: Vec<&str> = segment.uncertain_words().map(|w| w.text.as_str()).collect();
        assert_eq!(
            flagged,
            ["150", "Hz", "today"],
            "a number and its unit need 0.95, a word 0.8"
        );
        segment.words[3].status = RunStatus::Confirmed;
        assert_eq!(segment.uncertain_words().count(), 2);
    }

    #[test]
    fn a_recogniser_that_would_upload_is_refused() {
        let cloud = Scripted {
            utterances: Vec::new(),
            runs_on: RanOn::Cloud,
        };
        let audio = Audio::new(16_000, vec![0.0; 16_000]);
        assert!(matches!(
            transcribe(&audio, &cloud, &CaptionConfig::default(), None, 0),
            Err(SpeechError::WouldUpload(_))
        ));
        assert!(matches!(
            LiveCaptions::new(&cloud, CaptionConfig::default(), None),
            Err(SpeechError::WouldUpload(_))
        ));
    }

    fn finals(events: &[CaptionEvent]) -> Vec<(&Vec<Caption>, &Vec<Segment>)> {
        events
            .iter()
            .filter_map(|e| match e {
                CaptionEvent::Final { captions, segments } => Some((captions, segments)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn live_captions_arrive_utterance_by_utterance_with_interim_updates() {
        let engine = Scripted::from_sidecar();
        let audio = Audio::from_wav(&fixture("lecture-sample.wav")).unwrap();
        let mut live = LiveCaptions::new(&engine, CaptionConfig::default(), None).unwrap();
        let mut events = Vec::new();
        // fed in 100 ms batches at the file's own rate, as a microphone callback would
        for batch in audio.samples.chunks(audio.sample_rate as usize / 10) {
            events.extend(live.push(batch, audio.sample_rate).unwrap());
        }
        events.extend(live.finish().unwrap());
        let finals = finals(&events);
        let interims: Vec<(&Caption, usize)> = events
            .iter()
            .filter_map(|e| match e {
                CaptionEvent::Interim {
                    caption,
                    stable_lines,
                } => Some((caption, *stable_lines)),
                _ => None,
            })
            .collect();
        assert_eq!(finals.len(), 3, "{events:#?}");
        assert!(
            interims.len() >= 3,
            "interim refreshes while each utterance is spoken: {}",
            interims.len()
        );
        assert!(
            interims
                .iter()
                .all(|(c, stable)| c.lines.len() <= 2 && *stable <= c.lines.len())
        );
        assert_eq!(finals[0].0.len(), 1);
        assert_eq!(
            finals[0].0[0].text,
            "Welcome to week three. Today we look at the Fourier series."
        );
        assert_eq!(
            finals[0].0[0].lines,
            [
                "Welcome to week three.",
                "Today we look at the Fourier series."
            ]
        );
        assert!(
            (finals[1].1[0].start - 6.275).abs() < 0.3,
            "{}",
            finals[1].1[0].start
        );
        assert!(
            (finals[0].1[0].start - 0.5).abs() < 0.3,
            "the utterance opens with its padding: {}",
            finals[0].1[0].start
        );
        assert_eq!(live.transcript.len(), 3);
        assert_eq!(
            live.transcript.iter().map(|s| s.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(live.detected_language(), Some("en"));
        assert!((live.position_secs() - audio.duration_secs()).abs() < 0.05);
        let transcript = live.into_transcript(3);
        assert_eq!(transcript.language.as_deref(), Some("en"));
        assert_eq!(transcript.segments.len(), 3);
    }

    #[test]
    fn a_blip_is_not_recognised_and_a_long_utterance_is_cut_at_a_quiet_moment() {
        // a tone that runs for 12 s with one short dip at 7.3 s, after a lone 20 ms click
        let rate = RECOGNITION_RATE;
        let mut samples = vec![0.0f32; rate as usize];
        samples.extend((0..320).map(|i| if i % 2 == 0 { 0.3 } else { -0.3 }));
        samples.extend(vec![0.0f32; rate as usize]);
        let tone_start = samples.len();
        samples.extend((0..rate as usize * 12).map(|i| {
            let t = i as f32 / rate as f32;
            let quiet = (7.25..7.35).contains(&t);
            (t * 440.0 * std::f32::consts::TAU).sin() * if quiet { 0.002 } else { 0.3 }
        }));
        samples.extend(vec![0.0f32; rate as usize]);
        let tone_start_secs = tone_start as f32 / rate as f32;
        let engine = Scripted {
            utterances: vec![Utterance {
                text: "one long sentence".into(),
                start: tone_start_secs,
                end: tone_start_secs + 12.0,
            }],
            runs_on: RanOn::Device,
        };
        let config = CaptionConfig {
            interim_every_secs: None,
            ..CaptionConfig::default()
        };
        let mut live = LiveCaptions::new(&engine, config, Some("en")).unwrap();
        let mut events = Vec::new();
        for batch in samples.chunks(1000) {
            events.extend(live.push(batch, rate).unwrap());
        }
        events.extend(live.finish().unwrap());
        let finals = finals(&events);
        assert_eq!(
            finals.len(),
            2,
            "the click alone is never recognised: {events:#?}"
        );
        let first_end = finals[0].1[0].end;
        assert!(
            (first_end - (tone_start_secs + 7.3)).abs() < 0.15,
            "cut at the dip, not at 8 s: {first_end}"
        );
        assert!(matches!(
            live.push(&[0.0; 10], 0),
            Err(SpeechError::Audio(_))
        ));
        assert!(
            matches!(live.push(&[0.0; 10], 8_000), Err(SpeechError::Audio(_))),
            "the rate may not change"
        );
        assert!(
            live.detected_language().is_none(),
            "a given language is not detected"
        );

        let resumed = LiveCaptions::resume(
            &engine,
            CaptionConfig::default(),
            None,
            1500.0,
            live.transcript.clone(),
        )
        .unwrap();
        assert!((resumed.position_secs() - 1500.0).abs() < 0.03);
    }

    /// Says one more word for every 0.4 s of audio it is given, as a recogniser does when it is
    /// handed a growing utterance again and again.
    struct Counting;

    impl Transcriber for Counting {
        fn name(&self) -> String {
            "counting".into()
        }
        fn runs_on(&self) -> RanOn {
            RanOn::Device
        }
        fn transcribe(
            &self,
            audio: &Audio,
            _offset: f32,
            _language: Option<&str>,
        ) -> Result<Recognised, SpeechError> {
            let words = (audio.duration_secs() / 0.4) as usize;
            let text = (1..=words)
                .map(|i| format!("word{i:02}"))
                .collect::<Vec<_>>()
                .join(" ");
            let segments = if text.is_empty() {
                Vec::new()
            } else {
                vec![Segment::new(1, 0.0, audio.duration_secs(), &text)]
            };
            Ok(Recognised {
                segments,
                language: None,
            })
        }
    }

    #[test]
    fn interim_captions_say_truthfully_how_many_lines_are_unchanged() {
        let rate = RECOGNITION_RATE;
        let mut samples = vec![0.0f32; rate as usize];
        samples.extend(
            (0..rate as usize * 7)
                .map(|i| (i as f32 / rate as f32 * 440.0 * std::f32::consts::TAU).sin() * 0.3),
        );
        samples.extend(vec![0.0f32; rate as usize]);
        let engine = Counting;
        let mut live = LiveCaptions::new(&engine, CaptionConfig::default(), Some("en")).unwrap();
        let mut events = Vec::new();
        for batch in samples.chunks(800) {
            events.extend(live.push(batch, rate).unwrap());
        }
        events.extend(live.finish().unwrap());
        let interims: Vec<(&Caption, usize)> = events
            .iter()
            .filter_map(|e| match e {
                CaptionEvent::Interim {
                    caption,
                    stable_lines,
                } => Some((caption, *stable_lines)),
                _ => None,
            })
            .collect();
        assert!(interims.len() >= 4, "{}", interims.len());
        assert!(interims.iter().all(|(c, _)| c.lines.len() <= 2 && c.lines.iter().all(|l| l.chars().count() <= 37)));
        let mut froze_a_line = false;
        for pair in interims.windows(2) {
            let ((before, _), (now, stable)) = (pair[0], pair[1]);
            assert!(stable <= now.lines.len() && stable <= before.lines.len());
            assert_eq!(
                now.lines[..stable],
                before.lines[..stable],
                "stable lines are the same text on the same rows"
            );
            if stable < now.lines.len().min(before.lines.len()) {
                assert_ne!(
                    now.lines[stable], before.lines[stable],
                    "the first line after the stable ones did change"
                );
            }
            froze_a_line |= stable >= 1;
        }
        assert!(
            froze_a_line,
            "once a line is complete it stays where it is: {interims:#?}"
        );
        assert_eq!(
            interims[0].1, 0,
            "nothing was shown before the first interim caption"
        );
    }

    #[test]
    fn whisper_runs_the_sample_when_it_is_installed() {
        let Some(engine) = detect_engine() else {
            eprintln!("skipped: whisper-cli with ZACESS_WHISPER_MODEL is not installed");
            return;
        };
        let meta = sidecar();
        let transcript = transcribe_file(
            &fixture("lecture-sample.wav"),
            engine.as_ref(),
            &CaptionConfig::default(),
            Some("en"),
            0,
        )
        .unwrap();
        let text = transcript.text().to_lowercase();
        assert!(text.contains("harmonic"), "{text}");
        // the utterances come back in order, each starting near where the sidecar says
        let mut from = 0;
        for u in &meta.utterances {
            let key = u
                .text
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            let position = text[from..].find(&key).map(|p| p + from);
            assert!(position.is_some(), "{key} missing after {from} in {text}");
            from = position.unwrap();
            let segment = transcript
                .segments
                .iter()
                .find(|s| s.text.to_lowercase().contains(&key))
                .unwrap();
            assert!(
                (segment.start - u.start).abs() < 0.5,
                "{segment:?} vs {u:?}"
            );
        }
    }
}
