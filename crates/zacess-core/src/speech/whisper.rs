//! whisper.cpp as the on-device recogniser, run as the `whisper-cli` command.
//!
//! whisper.cpp runs the Whisper models on the processor of a laptop or a phone with nothing sent
//! anywhere, which is what the plan asks of transcription. Like the OCR and braille tools it is
//! detected at run time: the command and a model file on this device or nothing. The command
//! loads its model on every call, so it suits whole recordings and final captions; interim
//! captions every second or two want an engine that stays resident, which the trait allows.

use super::audio::{Audio, RECOGNITION_RATE};
use super::{Recognised, Segment, SpeechError, Transcriber, Word};
use crate::model::{Confidence, RanOn};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct WhisperCli {
    command: PathBuf,
    model: PathBuf,
}

impl WhisperCli {
    /// Finds `whisper-cli` on the path and a model at `ZACESS_WHISPER_MODEL`. Otherwise `None`.
    /// The command is resolved to the file that will actually receive the audio, so the
    /// provenance can name it.
    pub fn detect() -> Option<Self> {
        let model = PathBuf::from(std::env::var_os("ZACESS_WHISPER_MODEL")?);
        if !model.is_file() {
            return None;
        }
        let command = resolve_on_path("whisper-cli")?;
        let ok = Command::new(&command)
            .arg("--help")
            .output()
            .is_ok_and(|o| o.status.success());
        ok.then_some(WhisperCli { command, model })
    }

    pub fn new(command: impl Into<PathBuf>, model: impl Into<PathBuf>) -> Self {
        WhisperCli {
            command: command.into(),
            model: model.into(),
        }
    }

    pub fn model_name(&self) -> String {
        self.model
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("whisper")
            .to_string()
    }

    pub fn command(&self) -> &Path {
        &self.command
    }
}

/// The first file of that name in `PATH`.
fn resolve_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

impl Transcriber for WhisperCli {
    fn name(&self) -> String {
        format!("whisper.cpp ({})", self.command.display())
    }

    fn runs_on(&self) -> RanOn {
        RanOn::Device
    }

    fn model(&self) -> Option<String> {
        Some(self.model_name())
    }

    fn transcribe(
        &self,
        audio: &Audio,
        _offset_secs: f32,
        language: Option<&str>,
    ) -> Result<Recognised, SpeechError> {
        let dir = tempfile::Builder::new()
            .prefix("zacess-whisper-")
            .tempdir()?;
        let wav = dir.path().join("chunk.wav");
        std::fs::write(&wav, audio.resampled(RECOGNITION_RATE).to_wav_bytes()?)?;
        let prefix = dir.path().join("chunk");
        let mut command = Command::new(&self.command);
        command
            .arg("-m")
            .arg(&self.model)
            .arg("-f")
            .arg(&wav)
            .arg("-ojf")
            .arg("-of")
            .arg(&prefix)
            .arg("-np")
            // without a language the command assumes English rather than listening for one
            .args(["-l", language.unwrap_or("auto")]);
        let output = command
            .output()
            .map_err(|e| SpeechError::Engine(format!("running {}: {e}", self.command.display())))?;
        if !output.status.success() {
            return Err(SpeechError::Engine(format!(
                "whisper-cli failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        // older builds split a multibyte character across two tokens, so the file is read
        // leniently rather than losing the chunk
        let bytes = std::fs::read(prefix.with_extension("json"))?;
        parse_whisper_json(&String::from_utf8_lossy(&bytes))
    }
}

/// Reads whisper.cpp's JSON output into segments with words and confidences, with the language
/// it settled on.
pub fn parse_whisper_json(json: &str) -> Result<Recognised, SpeechError> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| SpeechError::Engine(format!("whisper output: {e}")))?;
    let Some(items) = value.get("transcription").and_then(|t| t.as_array()) else {
        return Err(SpeechError::Engine(
            "whisper output has no transcription".into(),
        ));
    };
    let language = value
        .get("result")
        .and_then(|r| r.get("language"))
        .and_then(|l| l.as_str())
        .filter(|l| !l.is_empty() && *l != "auto")
        .map(str::to_string);
    let ms = |v: &serde_json::Value, key: &str| {
        v.get("offsets")
            .and_then(|o| o.get(key))
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0) as f32
            / 1000.0
    };
    let mut segments = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let text = item
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        let (start, end) = (ms(item, "from"), ms(item, "to"));
        let mut words: Vec<Word> = Vec::new();
        if let Some(tokens) = item.get("tokens").and_then(|t| t.as_array()) {
            for token in tokens {
                let piece = token.get("text").and_then(|t| t.as_str()).unwrap_or("");
                let trimmed = piece.trim();
                // whisper.cpp's own control tokens look like [_BEG_] or [_TT_160] and carry no
                // speech; an annotation such as [Music] is kept as a word of its own
                if trimmed.is_empty() || trimmed.starts_with("[_") {
                    continue;
                }
                let p = token
                    .get("p")
                    .and_then(|p| p.as_f64())
                    .map(|p| Confidence::new(p as f32));
                let (t0, t1) = (ms(token, "from"), ms(token, "to"));
                let starts_word =
                    piece.starts_with(' ') || trimmed.starts_with(['[', '(']) || words.is_empty();
                match words.last_mut() {
                    Some(last) if !starts_word => {
                        last.text.push_str(trimmed);
                        last.end = t1.max(last.end);
                        last.confidence = match (last.confidence, p) {
                            (Some(a), Some(b)) => Some(Confidence::new(a.value().min(b.value()))),
                            _ => None,
                        };
                    }
                    _ => words.push(Word::new(trimmed, t0, t1, p)),
                }
            }
        }
        let mut segment = Segment::new(i as u32 + 1, start, end, &text);
        segment.words = if words.is_empty() {
            Segment::even_words(&text, start, end)
        } else {
            words
        };
        segments.push(segment);
    }
    Ok(Recognised { segments, language })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "systeminfo": "x",
      "model": {"type": "tiny"},
      "result": {"language": "en"},
      "transcription": [
        {"timestamps": {"from": "00:00:00,000", "to": "00:00:03,200"}, "offsets": {"from": 0, "to": 3200},
         "text": " Welcome to week three.",
         "tokens": [
           {"text": "[_BEG_]", "timestamps": {"from": "0", "to": "0"}, "offsets": {"from": 0, "to": 0}, "p": 0.99},
           {"text": " Welcome", "offsets": {"from": 0, "to": 600}, "p": 0.98},
           {"text": " to", "offsets": {"from": 600, "to": 800}, "p": 0.97},
           {"text": " week", "offsets": {"from": 800, "to": 1200}, "p": 0.9},
           {"text": " th", "offsets": {"from": 1200, "to": 1500}, "p": 0.6},
           {"text": "ree.", "offsets": {"from": 1500, "to": 1900}, "p": 0.8},
           {"text": " [", "offsets": {"from": 1900, "to": 1900}, "p": 0.5},
           {"text": "Music", "offsets": {"from": 1900, "to": 1900}, "p": 0.5},
           {"text": "]", "offsets": {"from": 1900, "to": 1900}, "p": 0.5},
           {"text": "[_TT_160]", "offsets": {"from": 3200, "to": 3200}, "p": 0.9}
         ]},
        {"offsets": {"from": 3400, "to": 6000}, "text": " Today we look at the Fourier series."},
        {"offsets": {"from": 6000, "to": 6100}, "text": "  "}
      ]
    }"#;

    #[test]
    fn whisper_json_becomes_segments_with_merged_words_and_confidence() {
        let recognised = parse_whisper_json(SAMPLE).unwrap();
        assert_eq!(recognised.language.as_deref(), Some("en"));
        let segments = recognised.segments;
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "Welcome to week three.");
        assert!((segments[0].start - 0.0).abs() < 1e-6 && (segments[0].end - 3.2).abs() < 1e-6);
        let words: Vec<&str> = segments[0].words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(words, ["Welcome", "to", "week", "three.", "[Music]"]);
        assert!(
            (segments[0].words[3].confidence.unwrap().value() - 0.6).abs() < 1e-6,
            "a word takes its least sure piece"
        );
        assert!(
            (segments[0].words[3].start - 1.2).abs() < 1e-6
                && (segments[0].words[3].end - 1.9).abs() < 1e-6
        );
        // without tokens the words are spread evenly across the segment with no confidence
        assert_eq!(segments[1].words.len(), 7);
        assert!((segments[1].words[0].start - 3.4).abs() < 1e-6);
        assert!(segments[1].words.iter().all(|w| w.confidence.is_none()));
        assert!(parse_whisper_json("{}").is_err());
        assert!(parse_whisper_json("nope").is_err());
    }

    #[test]
    fn a_command_is_looked_up_on_the_path() {
        assert!(resolve_on_path("definitely-not-a-command-zacess").is_none());
    }
}
