//! Chapters for a lecture, found from what happened in it.
//!
//! Two things in a real lecture mark a new chapter. The lecturer stops for a long time. Or a new
//! board, page or slide goes up. Each becomes a signal. Signals close together are one change of topic
//! so they merge. A chapter shorter than a minimum is not worth a heading so its signal is
//! dropped. The first chapter always starts at zero. A chapter a person added is never touched.

use super::{CaptureMark, Chapter, ChapterReason};
use crate::speech::Transcript;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChapterConfig {
    /// A silence in the speech at least this long, in seconds, is a signal.
    pub pause_secs: f32,
    /// Signals closer together than this, in seconds, are one change of topic.
    pub merge_within_secs: f32,
    /// A chapter shorter than this, in seconds, is not made.
    pub min_chapter_secs: f32,
}

impl Default for ChapterConfig {
    fn default() -> Self {
        ChapterConfig {
            pause_secs: 20.0,
            merge_within_secs: 45.0,
            min_chapter_secs: 120.0,
        }
    }
}

#[derive(Debug, Clone)]
struct Signal {
    at: f32,
    reason: ChapterReason,
    title: Option<String>,
}

/// The words of the first sentence at or after a time, cut to eight words.
fn title_from_speech(transcript: &Transcript, at: f32) -> Option<String> {
    let sentence = transcript.sentences().into_iter().find(|s| s.end > at)?;
    // a segment with no word times is one long sentence, so cut at the first full stop
    let first = sentence
        .text
        .split_inclusive(['.', '!', '?'])
        .next()
        .unwrap_or("");
    let words: Vec<&str> = first.split_whitespace().take(8).collect();
    let title = words.join(" ");
    let title = title
        .trim_end_matches(['.', ',', ';', ':', '!', '?'])
        .to_string();
    (!title.is_empty()).then_some(title)
}

/// A title worth showing: a board that was only numbered says nothing about the topic.
fn is_generic(title: &str) -> bool {
    let lower = title.to_lowercase();
    (lower.starts_with("board ") || lower.starts_with("page "))
        && lower.split_whitespace().count() <= 2
}

/// The signals in a lecture, in time order.
fn signals(
    transcript: Option<&Transcript>,
    captures: &[CaptureMark],
    config: &ChapterConfig,
) -> Vec<Signal> {
    let mut out = Vec::new();
    if let Some(transcript) = transcript {
        for pair in transcript.segments.windows(2) {
            if pair[1].start - pair[0].end >= config.pause_secs {
                out.push(Signal {
                    at: pair[1].start,
                    reason: ChapterReason::Pause,
                    title: title_from_speech(transcript, pair[1].start),
                });
            }
        }
    }
    for capture in captures.iter().filter(|c| c.version == 1) {
        out.push(Signal {
            at: capture.at,
            reason: ChapterReason::Board,
            title: Some(capture.title.clone()),
        });
    }
    out.sort_by(|a, b| a.at.total_cmp(&b.at));
    out
}

/// Merges signals within the window into one at the earliest time. A board outranks a pause, and
/// a title that names the topic outranks one that only numbers the board.
fn merge(signals: Vec<Signal>, within: f32) -> Vec<Signal> {
    let mut out: Vec<Signal> = Vec::new();
    for signal in signals {
        match out.last_mut() {
            Some(last) if signal.at - last.at <= within => {
                let better = match (&last.title, &signal.title) {
                    (None, Some(_)) => true,
                    (Some(old), Some(new)) if signal.reason == ChapterReason::Board => {
                        !is_generic(new) && (last.reason != ChapterReason::Board || is_generic(old))
                    }
                    (Some(old), Some(new)) => is_generic(old) && !is_generic(new),
                    _ => false,
                };
                if signal.reason == ChapterReason::Board {
                    last.reason = ChapterReason::Board;
                }
                if better {
                    last.title = signal.title;
                }
            }
            _ => out.push(signal),
        }
    }
    out
}

/// Chapters for a lecture of the given length. `duration` of zero means it is not known.
pub fn detect(
    transcript: Option<&Transcript>,
    captures: &[CaptureMark],
    duration: f32,
    config: &ChapterConfig,
) -> Vec<Chapter> {
    let merged = merge(
        signals(transcript, captures, config),
        config.merge_within_secs,
    );
    let mut chapters: Vec<Chapter> = Vec::new();
    let mut opening_title: Option<String> = None;
    let mut last_start = 0.0f32;
    for signal in merged {
        let tail_too_short = duration > 0.0 && duration - signal.at < config.min_chapter_secs / 2.0;
        let too_close = signal.at <= last_start || signal.at - last_start < config.min_chapter_secs;
        if too_close || tail_too_short {
            // too close to the last chapter to deserve its own, but it can still name the first.
            // A board in the closing minute has nothing to do with how the lecture opened
            if chapters.is_empty()
                && !tail_too_short
                && opening_title.is_none()
                && signal.reason == ChapterReason::Board
            {
                opening_title = signal.title.filter(|t| !is_generic(t));
            }
            continue;
        }
        let number = chapters.len() + 2;
        chapters.push(Chapter {
            start: signal.at,
            title: signal.title.unwrap_or_else(|| format!("Chapter {number}")),
            reason: signal.reason,
        });
        last_start = signal.at;
    }
    let first_title = opening_title
        .or_else(|| transcript.and_then(|t| title_from_speech(t, 0.0)))
        .unwrap_or_else(|| "Start".to_string());
    chapters.insert(
        0,
        Chapter {
            start: 0.0,
            title: first_title,
            reason: ChapterReason::Start,
        },
    );
    chapters
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::capture::CaptureKind;
    use crate::model::{Confidence, Provenance, RanOn};
    use crate::speech::{Segment, TRANSCRIPT_SCHEMA_VERSION};

    pub(crate) fn transcript(segments: Vec<Segment>) -> Transcript {
        let duration = segments.last().map(|s| s.end).unwrap_or(0.0);
        let _ = Confidence::CERTAIN;
        Transcript {
            schema_version: TRANSCRIPT_SCHEMA_VERSION,
            recording: None,
            language: Some("en".into()),
            duration_secs: duration,
            engine: Provenance {
                helper: "test".into(),
                model: None,
                ran_on: RanOn::Device,
                at: 0,
            },
            segments,
        }
    }

    fn mark(at: f32, version: u32, title: &str) -> CaptureMark {
        CaptureMark {
            capture: 1,
            board: 1,
            version,
            kind: CaptureKind::Whiteboard,
            at,
            last_seen_at: at,
            title: title.into(),
            lines: Vec::new(),
            repeats: 0,
            clamped: false,
        }
    }

    fn lecture() -> Transcript {
        // speech in blocks with a 40 s silence at 300 s and a 25 s silence at 600 s
        let mut segments = Vec::new();
        let mut id = 0;
        for (from, to, text) in [
            (
                5.0,
                295.0,
                "Welcome to the lecture. Today is the Fourier series.",
            ),
            (
                340.0,
                595.0,
                "Now the Laplace transform. It generalises the Fourier transform.",
            ),
            (625.0, 900.0, "Finally some worked examples."),
        ] {
            id += 1;
            segments.push(Segment::new(id, from, to, text));
        }
        transcript(segments)
    }

    #[test]
    fn long_pauses_start_chapters_named_from_what_follows() {
        let chapters = detect(Some(&lecture()), &[], 900.0, &ChapterConfig::default());
        let starts: Vec<f32> = chapters.iter().map(|c| c.start).collect();
        assert_eq!(starts, [0.0, 340.0, 625.0]);
        assert_eq!(chapters[0].reason, ChapterReason::Start);
        assert_eq!(chapters[0].title, "Welcome to the lecture");
        assert_eq!(chapters[1].reason, ChapterReason::Pause);
        assert_eq!(chapters[1].title, "Now the Laplace transform");
        assert_eq!(chapters[2].title, "Finally some worked examples");
    }

    #[test]
    fn a_new_board_starts_a_chapter_and_outranks_a_nearby_pause() {
        // the board goes up 12 s after the long silence ends at 340 s
        let captures = [
            mark(60.0, 1, "Board 1"),
            mark(352.0, 1, "Week 4 Laplace transform"),
            mark(700.0, 2, "Week 4 Laplace transform, version 2"),
        ];
        let chapters = detect(
            Some(&lecture()),
            &captures,
            900.0,
            &ChapterConfig::default(),
        );
        let starts: Vec<f32> = chapters.iter().map(|c| c.start).collect();
        assert_eq!(starts, [0.0, 340.0, 625.0], "{chapters:?}");
        assert_eq!(chapters[1].reason, ChapterReason::Board);
        assert_eq!(
            chapters[1].title, "Week 4 Laplace transform",
            "the board names the topic"
        );
        // a new version of a board is a change on the same board, not a new chapter
        assert!(chapters.iter().all(|c| c.start != 700.0));
    }

    #[test]
    fn short_chapters_are_not_made_and_the_first_can_take_a_boards_name() {
        let config = ChapterConfig::default();
        // a board 30 s in names the opening chapter instead of starting one
        let chapters = detect(
            None,
            &[mark(30.0, 1, "Week 3 Fourier series")],
            900.0,
            &config,
        );
        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].title, "Week 3 Fourier series");
        // two boards 60 s apart: the second would make a chapter under two minutes
        let chapters = detect(
            None,
            &[mark(200.0, 1, "A"), mark(260.0, 1, "B")],
            900.0,
            &config,
        );
        assert_eq!(chapters.len(), 2);
        // a board in the last minute of a long lecture is not a chapter
        let chapters = detect(None, &[mark(880.0, 1, "Last")], 900.0, &config);
        assert_eq!(chapters.len(), 1);
        let late = detect(None, &[mark(880.0, 1, "Last")], 900.0, &config);
        assert_eq!(
            late[0].title, "Start",
            "a closing board does not name the opening"
        );
        // equal times and a zero setting cannot make two chapters at the same moment
        let zero = ChapterConfig {
            merge_within_secs: 0.0,
            min_chapter_secs: 0.0,
            ..config.clone()
        };
        let same = detect(
            None,
            &[mark(0.0, 1, "A"), mark(300.0, 1, "B"), mark(300.0, 1, "C")],
            900.0,
            &zero,
        );
        let starts: Vec<f32> = same.iter().map(|c| c.start).collect();
        assert_eq!(starts, [0.0, 300.0], "{same:?}");
        // nothing at all still gives a first chapter
        let only = detect(None, &[], 0.0, &config);
        assert_eq!(
            (only.len(), only[0].start, only[0].title.as_str()),
            (1, 0.0, "Start")
        );
    }

    #[test]
    fn numbered_boards_without_text_get_numbered_chapters() {
        let chapters = detect(
            None,
            &[mark(300.0, 1, "Board 2")],
            900.0,
            &ChapterConfig::default(),
        );
        assert_eq!(chapters[1].title, "Board 2");
        let pause_only = transcript(vec![
            Segment::new(1, 0.0, 100.0, "..."),
            Segment::new(2, 400.0, 500.0, ""),
        ]);
        let chapters = detect(Some(&pause_only), &[], 600.0, &ChapterConfig::default());
        assert_eq!(chapters[1].title, "Chapter 2");
    }
}
