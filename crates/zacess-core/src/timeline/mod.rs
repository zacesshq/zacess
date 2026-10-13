//! One lecture on one clock.
//!
//! A lecture leaves several records: the recording, the transcript made from it, the boards and
//! pages photographed during it, the places the student marked and the chapters it falls into.
//! The timeline joins them all on the recording's clock, seconds from its start, so any one can
//! be found from any other. It can search across everything, jump to a moment and show what was
//! being said and what was on the board. It keeps the whole thing beside the recording file. The
//! screens that show it are the app's. This module is the data and the rules.

pub mod chapters;

use crate::capture::slides::{SLIDE_ID_BASE, SlideSession};
use crate::capture::{CaptureKind, CaptureSession};
use crate::speech::{Segment, Sentence, Transcript};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub use chapters::ChapterConfig;

/// Bumped when a stored timeline's shape changes.
pub const TIMELINE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum TimelineError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("the timeline file is not valid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no bookmark {0}")]
    NoBookmark(u32),
    #[error("{0} seconds is outside the lecture")]
    OutsideLecture(f32),
    #[error("the timeline was saved by a newer version (format {0})")]
    Newer(u32),
    #[error("no chapter starts at {0} seconds")]
    NoChapter(f32),
    #[error("the first chapter cannot be removed")]
    FirstChapter,
}

/// The recording the timeline describes, enough to tell if a file is that recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub file_name: String,
    /// When the recording started, in Unix seconds. The captures' own times are measured from here.
    pub started_at: i64,
    /// Length in seconds; 0 when not known yet.
    pub duration_secs: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// A hash of the first and last megabyte, quick to take on a long recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<u64>,
    /// Stretches where the recording was paused, in the order they happened. Audio is not kept
    /// while paused, so a Unix time after a pause sits earlier in the audio than the wall clock
    /// says.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pauses: Vec<Pause>,
}

/// A pause in a recording: where in the audio it fell and how long it lasted.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pause {
    /// Seconds into the audio at which the recording was paused.
    pub at_secs: f32,
    /// Seconds the pause lasted.
    pub length_secs: f32,
}

impl Recording {
    pub fn new(file_name: &str, started_at: i64, duration_secs: f32) -> Self {
        let duration_secs = if duration_secs.is_finite() {
            duration_secs.max(0.0)
        } else {
            0.0
        };
        Recording {
            file_name: file_name.to_string(),
            started_at,
            duration_secs,
            size_bytes: None,
            fingerprint: None,
            pauses: Vec::new(),
        }
    }

    /// Describes a recording file, including its size and fingerprint.
    pub fn from_file(
        path: &Path,
        started_at: i64,
        duration_secs: f32,
    ) -> Result<Self, TimelineError> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("recording")
            .to_string();
        let (size, fingerprint) = fingerprint(path)?;
        Ok(Recording {
            file_name: name,
            started_at,
            duration_secs,
            size_bytes: Some(size),
            fingerprint: Some(fingerprint),
            pauses: Vec::new(),
        })
    }
}

impl Recording {
    /// Seconds since the recording started, with pauses counted, as seconds into the audio. A
    /// moment inside a pause is the start of that pause, which is also said.
    fn audio_secs(&self, wall: f32) -> (f32, bool) {
        let mut paused = 0.0f32;
        for pause in &self.pauses {
            let length = pause.length_secs.max(0.0);
            let begins = pause.at_secs + paused;
            if wall < begins {
                break;
            }
            if wall < begins + length {
                return (pause.at_secs, true);
            }
            paused += length;
        }
        (wall - paused, false)
    }
}

/// FNV-1a over the first and last megabyte, with the length mixed in.
pub(crate) fn fingerprint(path: &Path) -> std::io::Result<(u64, u64)> {
    const EDGE: u64 = 1 << 20;
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    let mut hash = 0xcbf2_9ce4_8422_2325u64 ^ size;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            hash = (hash ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    let mut buffer = vec![0u8; EDGE.min(size) as usize];
    file.read_exact(&mut buffer)?;
    feed(&buffer);
    if size > EDGE {
        let tail = EDGE.min(size - EDGE);
        file.seek(SeekFrom::Start(size - tail))?;
        let mut buffer = vec![0u8; tail as usize];
        file.read_exact(&mut buffer)?;
        feed(&buffer);
    }
    Ok((size, hash))
}

/// A board or page photographed during the lecture, placed on the clock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureMark {
    pub capture: u32,
    pub board: u32,
    /// 1 for the first photo of a board, counting up for each change.
    pub version: u32,
    pub kind: CaptureKind,
    /// Seconds from the start of the recording.
    pub at: f32,
    /// When the same writing was last seen again, on the same clock.
    pub last_seen_at: f32,
    /// The board's title, which is also what its chapter is called.
    pub title: String,
    /// What was written, a line at a time, so the board can be searched without its photo.
    pub lines: Vec<String>,
    pub repeats: u32,
    /// The photo was taken outside the recording, so its time was moved to the nearest end.
    pub clamped: bool,
}

/// A place the student marked, with an optional note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: u32,
    pub at: f32,
    pub note: String,
    /// Unix seconds.
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChapterReason {
    /// The opening chapter, which always starts at zero.
    Start,
    /// The lecturer stopped speaking for a long time.
    Pause,
    /// A new board or page went up.
    Board,
    /// A person added it. Detection never touches it.
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub start: f32,
    pub title: String,
    pub reason: ChapterReason,
}

/// What kind of record a hit or an event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Chapter,
    Sentence,
    Capture,
    Bookmark,
}

/// Something found by a search. It can also be the next thing on the clock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub at: f32,
    pub kind: EventKind,
    pub text: String,
    /// The transcript segment, for a sentence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bookmark: Option<u32>,
}

/// Where the lecture stands at a moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub at: f32,
    /// The sentence being spoken. In a pause it is the last one before it.
    pub sentence: Option<Sentence>,
    /// What is on the board: the latest photo taken at or before the moment.
    pub capture: Option<CaptureMark>,
    /// The chapter the moment is in, with its position in the list.
    pub chapter: Option<(usize, Chapter)>,
    pub previous_bookmark: Option<Bookmark>,
    pub next_bookmark: Option<Bookmark>,
}

/// One lecture on one clock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub schema_version: u32,
    pub title: String,
    pub recording: Recording,
    pub transcript: Option<Transcript>,
    pub captures: Vec<CaptureMark>,
    pub bookmarks: Vec<Bookmark>,
    pub chapters: Vec<Chapter>,
    #[serde(default)]
    next_bookmark: u32,
}

impl Timeline {
    pub fn new(title: &str, recording: Recording) -> Self {
        Timeline {
            schema_version: TIMELINE_SCHEMA_VERSION,
            title: title.to_string(),
            recording,
            transcript: None,
            captures: Vec::new(),
            bookmarks: Vec::new(),
            chapters: vec![Chapter {
                start: 0.0,
                title: "Start".into(),
                reason: ChapterReason::Start,
            }],
            next_bookmark: 0,
        }
    }

    /// Puts right what a damaged or hand-edited file can get wrong: an opening chapter at zero,
    /// everything in time order and a bookmark counter that never reuses an id.
    fn repair(&mut self) {
        self.chapters.sort_by(|a, b| a.start.total_cmp(&b.start));
        if self.chapters.first().is_none_or(|c| c.start != 0.0) {
            self.chapters.insert(
                0,
                Chapter {
                    start: 0.0,
                    title: "Start".into(),
                    reason: ChapterReason::Start,
                },
            );
        }
        self.captures
            .sort_by(|a, b| a.at.total_cmp(&b.at).then(a.capture.cmp(&b.capture)));
        self.bookmarks
            .sort_by(|a, b| a.at.total_cmp(&b.at).then(a.id.cmp(&b.id)));
        let highest = self.bookmarks.iter().map(|b| b.id).max().unwrap_or(0);
        self.next_bookmark = self.next_bookmark.max(highest);
    }

    /// The length of the lecture: the recording's. Failing that it is the longest thing on the clock.
    pub fn duration(&self) -> f32 {
        let latest_capture = self
            .captures
            .iter()
            .map(|c| c.last_seen_at.max(c.at))
            .fold(0.0f32, f32::max);
        let transcript = self
            .transcript
            .as_ref()
            .map(|t| t.duration_secs)
            .unwrap_or(0.0);
        self.recording
            .duration_secs
            .max(transcript)
            .max(latest_capture)
    }

    pub fn set_transcript(&mut self, transcript: Transcript) {
        self.transcript = Some(transcript);
    }

    /// A Unix time on the recording's clock. Also says whether it had to be moved to fit.
    fn place(&self, unix: i64) -> (f32, bool) {
        let wall = unix.saturating_sub(self.recording.started_at) as f32;
        let (raw, in_pause) = self.recording.audio_secs(wall);
        let top = if self.recording.duration_secs > 0.0 {
            self.recording.duration_secs
        } else {
            f32::MAX
        };
        let clamped = raw.clamp(0.0, top);
        (clamped, clamped != raw || in_pause)
    }

    /// Places a session's boards on the clock, replacing the boards and pages held before.
    /// Slides placed by [`Timeline::set_slides`] are kept. A photo's time is its Unix time less
    /// the recording's start; one taken outside the recording is moved to the nearest end and
    /// marked.
    pub fn set_captures(&mut self, session: &CaptureSession) {
        let mut marks: Vec<CaptureMark> = self
            .captures
            .iter()
            .filter(|c| c.kind == CaptureKind::Slide)
            .cloned()
            .collect();
        for board in &session.boards {
            for capture in &board.captures {
                let (at, moved) = self.place(capture.taken_at);
                let (seen, moved_seen) = self.place(capture.last_seen_at);
                marks.push(CaptureMark {
                    capture: capture.id,
                    board: board.id,
                    version: capture.version,
                    kind: capture.kind,
                    at,
                    last_seen_at: seen.max(at),
                    title: board.title.clone(),
                    lines: capture.lines(),
                    repeats: capture.repeats,
                    clamped: moved || moved_seen,
                });
            }
        }
        marks.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.capture.cmp(&b.capture)));
        self.captures = marks;
    }

    /// Places a session's slides on the clock, replacing the slides held before and keeping the
    /// boards. Each state of a slide is a mark: the first is version 1 and each build adds one.
    /// Slide and frame ids are offset by [`SLIDE_ID_BASE`] so they never collide with a board's.
    /// A slide counts as last seen when the lecturer last came back to it.
    pub fn set_slides(&mut self, session: &SlideSession) {
        let mut marks: Vec<CaptureMark> = self
            .captures
            .iter()
            .filter(|c| c.kind != CaptureKind::Slide)
            .cloned()
            .collect();
        for slide in &session.slides {
            let last_step = slide.steps.len().saturating_sub(1);
            for (index, step) in slide.steps.iter().enumerate() {
                let (at, moved) = self.place(step.first_seen_at);
                let seen_unix = if index == last_step {
                    slide.last_seen_at()
                } else {
                    step.last_seen_at
                };
                let (seen, moved_seen) = self.place(seen_unix);
                marks.push(CaptureMark {
                    capture: SLIDE_ID_BASE + step.id,
                    board: SLIDE_ID_BASE + slide.id,
                    version: step.number,
                    kind: CaptureKind::Slide,
                    at,
                    last_seen_at: seen.max(at),
                    title: slide.title.clone(),
                    lines: step.lines.clone(),
                    repeats: step.frames.saturating_sub(1),
                    clamped: moved || moved_seen,
                });
            }
        }
        marks.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.capture.cmp(&b.capture)));
        self.captures = marks;
    }

    fn check_time(&self, at: f32) -> Result<(), TimelineError> {
        let duration = self.duration();
        if !at.is_finite() || at < 0.0 || (duration > 0.0 && at > duration) {
            return Err(TimelineError::OutsideLecture(at));
        }
        Ok(())
    }

    /// Marks a moment with an optional note. Returns the bookmark's id.
    pub fn add_bookmark(&mut self, at: f32, note: &str, now: i64) -> Result<u32, TimelineError> {
        self.check_time(at)?;
        self.next_bookmark += 1;
        let id = self.next_bookmark;
        self.bookmarks.push(Bookmark {
            id,
            at,
            note: note.trim().to_string(),
            created_at: now,
        });
        self.bookmarks
            .sort_by(|a, b| a.at.total_cmp(&b.at).then(a.id.cmp(&b.id)));
        Ok(id)
    }

    pub fn edit_bookmark(&mut self, id: u32, note: &str) -> Result<(), TimelineError> {
        let bookmark = self
            .bookmarks
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or(TimelineError::NoBookmark(id))?;
        bookmark.note = note.trim().to_string();
        Ok(())
    }

    pub fn remove_bookmark(&mut self, id: u32) -> Result<(), TimelineError> {
        let before = self.bookmarks.len();
        self.bookmarks.retain(|b| b.id != id);
        if self.bookmarks.len() == before {
            return Err(TimelineError::NoBookmark(id));
        }
        Ok(())
    }

    /// Adds a chapter a person chose. Detection never moves or removes it.
    pub fn add_chapter(&mut self, at: f32, title: &str) -> Result<(), TimelineError> {
        self.check_time(at)?;
        if at == 0.0 {
            // the opening chapter is renamed rather than doubled
            self.repair();
            self.chapters[0].title = title.trim().to_string();
            self.chapters[0].reason = ChapterReason::Manual;
            return Ok(());
        }
        self.chapters.retain(|c| c.start != at);
        self.chapters.push(Chapter {
            start: at,
            title: title.trim().to_string(),
            reason: ChapterReason::Manual,
        });
        self.chapters.sort_by(|a, b| a.start.total_cmp(&b.start));
        Ok(())
    }

    pub fn remove_chapter(&mut self, start: f32) -> Result<(), TimelineError> {
        if start == 0.0 {
            return Err(TimelineError::FirstChapter);
        }
        let before = self.chapters.len();
        self.chapters.retain(|c| c.start != start);
        if self.chapters.len() == before {
            return Err(TimelineError::NoChapter(start));
        }
        Ok(())
    }

    /// Finds chapters from long pauses and new boards, replacing the ones found earlier and
    /// keeping any a person added. A manual chapter takes the place of a detected one near it.
    pub fn detect_chapters(&mut self, config: &ChapterConfig) {
        let manual: Vec<Chapter> = self
            .chapters
            .iter()
            .filter(|c| c.reason == ChapterReason::Manual)
            .cloned()
            .collect();
        let mut found = chapters::detect(
            self.transcript.as_ref(),
            &self.captures,
            self.duration(),
            config,
        );
        if manual.iter().any(|c| c.start == 0.0) {
            found.remove(0);
        }
        found.retain(|c| {
            c.start == 0.0
                || !manual
                    .iter()
                    .any(|m| (m.start - c.start).abs() < config.merge_within_secs)
        });
        found.extend(manual);
        found.sort_by(|a, b| a.start.total_cmp(&b.start));
        self.chapters = found;
    }

    /// The chapters with the time each one ends.
    pub fn chapter_spans(&self) -> Vec<(Chapter, f32)> {
        let duration = self.duration();
        self.chapters
            .iter()
            .enumerate()
            .map(|(i, c)| {
                (
                    c.clone(),
                    self.chapters
                        .get(i + 1)
                        .map(|n| n.start)
                        .unwrap_or(duration),
                )
            })
            .collect()
    }

    /// The sentence being spoken at a time, else the last one before it.
    fn sentence_at(&self, at: f32) -> Option<Sentence> {
        let sentences = self.transcript.as_ref()?.sentences();
        sentences
            .iter()
            .find(|s| at >= s.start && at < s.end)
            .or_else(|| sentences.iter().rev().find(|s| s.start <= at))
            .cloned()
    }

    /// Where the lecture stands at a time: what is being said, what is on the board, which chapter
    /// it is and the bookmarks either side. A time outside the lecture is moved to the nearest end.
    pub fn position(&self, at: f32) -> Position {
        let duration = self.duration();
        let at = if at.is_finite() {
            at.clamp(0.0, if duration > 0.0 { duration } else { f32::MAX })
        } else {
            0.0
        };
        let chapter = self
            .chapters
            .iter()
            .rposition(|c| c.start <= at)
            .map(|i| (i, self.chapters[i].clone()));
        Position {
            at,
            sentence: self.sentence_at(at),
            capture: self.captures.iter().rev().find(|c| c.at <= at).cloned(),
            chapter,
            previous_bookmark: self.bookmarks.iter().rev().find(|b| b.at <= at).cloned(),
            next_bookmark: self.bookmarks.iter().find(|b| b.at > at).cloned(),
        }
    }

    /// Everything on the clock, in time order.
    pub fn events(&self) -> Vec<Event> {
        let mut out = Vec::new();
        for chapter in &self.chapters {
            out.push(Event {
                at: chapter.start,
                kind: EventKind::Chapter,
                text: chapter.title.clone(),
                segment: None,
                capture: None,
                bookmark: None,
            });
        }
        if let Some(transcript) = &self.transcript {
            for s in transcript.sentences() {
                out.push(Event {
                    at: s.start,
                    kind: EventKind::Sentence,
                    text: s.text,
                    segment: Some(s.segment),
                    capture: None,
                    bookmark: None,
                });
            }
        }
        for c in &self.captures {
            let text = if c.lines.is_empty() {
                c.title.clone()
            } else {
                format!("{}: {}", c.title, c.lines.join(" / "))
            };
            out.push(Event {
                at: c.at,
                kind: EventKind::Capture,
                text,
                segment: None,
                capture: Some(c.capture),
                bookmark: None,
            });
        }
        for b in &self.bookmarks {
            out.push(Event {
                at: b.at,
                kind: EventKind::Bookmark,
                text: b.note.clone(),
                segment: None,
                capture: None,
                bookmark: Some(b.id),
            });
        }
        out.sort_by(|a, b| a.at.total_cmp(&b.at));
        out
    }

    /// The first event strictly after a time, optionally of one kind, for stepping forward.
    pub fn next_event(&self, at: f32, kind: Option<EventKind>) -> Option<Event> {
        self.events()
            .into_iter()
            .find(|e| e.at > at && kind.is_none_or(|k| k == e.kind))
    }

    /// The last event strictly before a time, optionally of one kind, for stepping back.
    pub fn previous_event(&self, at: f32, kind: Option<EventKind>) -> Option<Event> {
        self.events()
            .into_iter()
            .rev()
            .find(|e| e.at < at && kind.is_none_or(|k| k == e.kind))
    }

    /// Everything holding every word of a query as a whole word or the start of one, ignoring
    /// case: spoken sentences, board and page lines, bookmark notes and chapter titles. Hits come
    /// in time order, each with the moment to jump to. A sentence jumps to its first matching word
    /// when word times are known.
    pub fn search(&self, query: &str) -> Vec<Event> {
        let terms = words_of(query);
        if terms.is_empty() {
            return Vec::new();
        }
        let matches = |text: &str| {
            let words = words_of(text);
            terms
                .iter()
                .all(|t| words.iter().any(|w| w.starts_with(t.as_str())))
        };
        let mut out = Vec::new();
        for chapter in self.chapters.iter().filter(|c| matches(&c.title)) {
            out.push(Event {
                at: chapter.start,
                kind: EventKind::Chapter,
                text: chapter.title.clone(),
                segment: None,
                capture: None,
                bookmark: None,
            });
        }
        if let Some(transcript) = &self.transcript {
            for segment in &transcript.segments {
                for sentence in segment_sentences(segment) {
                    if matches(&sentence.text) {
                        let at =
                            first_word_time(segment, &sentence, &terms).unwrap_or(sentence.start);
                        out.push(Event {
                            at,
                            kind: EventKind::Sentence,
                            text: sentence.text,
                            segment: Some(segment.id),
                            capture: None,
                            bookmark: None,
                        });
                    }
                }
            }
        }
        for c in &self.captures {
            for line in c.lines.iter().filter(|l| matches(l)) {
                out.push(Event {
                    at: c.at,
                    kind: EventKind::Capture,
                    text: line.clone(),
                    segment: None,
                    capture: Some(c.capture),
                    bookmark: None,
                });
            }
        }
        for b in self.bookmarks.iter().filter(|b| matches(&b.note)) {
            out.push(Event {
                at: b.at,
                kind: EventKind::Bookmark,
                text: b.note.clone(),
                segment: None,
                capture: None,
                bookmark: Some(b.id),
            });
        }
        out.sort_by(|a, b| a.at.total_cmp(&b.at));
        out
    }

    /// The file the timeline is kept in, beside its recording.
    pub fn sidecar_path(recording: &Path) -> PathBuf {
        // built from the raw file name, so two recordings whose names are not valid UTF-8 never
        // share a sidecar
        let mut name = recording
            .file_name()
            .unwrap_or(std::ffi::OsStr::new("recording"))
            .to_os_string();
        name.push(".zacess-timeline.json");
        recording.with_file_name(name)
    }

    /// Saves beside the recording. The file is written whole and moved into place, so a crash
    /// cannot leave half a timeline.
    pub fn save_beside(&self, recording: &Path) -> Result<PathBuf, TimelineError> {
        let path = Timeline::sidecar_path(recording);
        let dir = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        file.write_all(b"\n")?;
        // on disk before the rename, so a power cut cannot leave the new name on an empty file
        file.as_file().sync_all()?;
        file.persist(&path).map_err(|e| e.error)?;
        Ok(path)
    }

    /// Loads the timeline kept beside a recording. Also says whether the file there is still the
    /// recording it was made for.
    pub fn load_beside(recording: &Path) -> Result<Loaded, TimelineError> {
        let text = std::fs::read_to_string(Timeline::sidecar_path(recording))?;
        // the version is read first, so a newer file whose shape changed is reported as newer
        // rather than as damaged
        let value: serde_json::Value = serde_json::from_str(&text)?;
        let version = value
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if version > TIMELINE_SCHEMA_VERSION {
            return Err(TimelineError::Newer(version));
        }
        let mut timeline: Timeline = serde_json::from_value(value)?;
        timeline.repair();
        let recording_matches = match (
            timeline.recording.size_bytes,
            timeline.recording.fingerprint,
        ) {
            (Some(size), Some(print)) => {
                fingerprint(recording).is_ok_and(|(s, p)| s == size && p == print)
            }
            _ => recording.is_file(),
        };
        Ok(Loaded {
            timeline,
            recording_matches,
        })
    }
}

/// A timeline read from disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub timeline: Timeline,
    /// False when the recording is missing or has changed since the timeline was made, so its
    /// times may no longer line up.
    pub recording_matches: bool,
}

fn words_of(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// A segment's sentences, with word times when it has them.
fn segment_sentences(segment: &Segment) -> Vec<Sentence> {
    segment.sentences()
}

/// The time of the first word in a sentence that starts with any search term.
fn first_word_time(segment: &Segment, sentence: &Sentence, terms: &[String]) -> Option<f32> {
    segment
        .words
        .iter()
        .filter(|w| w.start >= sentence.start - 1e-4 && w.end <= sentence.end + 1e-4)
        .find(|w| {
            words_of(&w.text)
                .iter()
                .any(|word| terms.iter().any(|t| word.starts_with(t.as_str())))
        })
        .map(|w| w.start)
}

#[cfg(test)]
mod tests {
    use super::chapters::tests::transcript;
    use super::*;
    use crate::capture::{CaptureConfig, CaptureSession};
    use crate::import::ImportError;
    use crate::import::ocr::{OcrEngine, OcrPage};
    use crate::speech::tests::{Scripted, fixture};
    use crate::speech::{CaptionConfig, transcribe_file};

    struct NoOcr;
    impl OcrEngine for NoOcr {
        fn name(&self) -> String {
            "none".into()
        }
        fn recognise(&self, _image: &Path) -> Result<OcrPage, ImportError> {
            Ok(OcrPage {
                width: 1.0,
                height: 1.0,
                paragraphs: Vec::new(),
            })
        }
    }

    const STARTED: i64 = 1_000_000;

    fn board(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/whiteboard")
            .join(name)
    }

    /// The speech sample's transcript with four photos taken during it.
    fn lecture() -> Timeline {
        let engine = Scripted::from_sidecar();
        let transcript = transcribe_file(
            &fixture("lecture-sample.wav"),
            &engine,
            &CaptionConfig::default(),
            Some("en"),
            0,
        )
        .unwrap();
        let mut session = CaptureSession::new();
        let config = CaptureConfig::default();
        for (file, offset) in [
            ("board-1-first.jpg", 2),
            ("board-2-relit.jpg", 4),
            ("board-3-added.jpg", 9),
            ("board-5-new-topic.jpg", 12),
        ] {
            session
                .capture(
                    &board(file),
                    CaptureKind::Whiteboard,
                    STARTED + offset,
                    &NoOcr,
                    &config,
                )
                .unwrap();
        }
        let mut timeline = Timeline::new(
            "Signals week 3",
            Recording::new("lecture-sample.wav", STARTED, 15.0),
        );
        timeline.set_transcript(transcript);
        timeline.set_captures(&session);
        timeline
    }

    #[test]
    fn the_transcript_and_the_boards_share_one_clock() {
        let t = lecture();
        let marks: Vec<(u32, u32, f32)> = t
            .captures
            .iter()
            .map(|c| (c.board, c.version, c.at))
            .collect();
        assert_eq!(
            marks,
            [(1, 1, 2.0), (1, 2, 9.0), (2, 1, 12.0)],
            "the relit photo was a repeat"
        );
        assert_eq!(t.captures[0].repeats, 1);
        assert_eq!(
            t.captures[0].last_seen_at, 4.0,
            "the repeat moved when the writing was last seen"
        );

        let early = t.position(3.0);
        assert_eq!(early.capture.as_ref().map(|c| c.version), Some(1));
        assert!(
            early
                .sentence
                .as_ref()
                .is_some_and(|s| s.text.contains("Fourier series")),
            "{early:?}"
        );
        let opening = t.position(1.0);
        assert!(
            opening
                .sentence
                .as_ref()
                .is_some_and(|s| s.text.starts_with("Welcome")),
            "{opening:?}"
        );
        let middle = t.position(8.0);
        assert!(
            middle
                .sentence
                .as_ref()
                .is_some_and(|s| s.text.contains("square wave")),
            "{middle:?}"
        );
        assert_eq!(
            middle.capture.as_ref().map(|c| c.version),
            Some(1),
            "version 2 goes up at 9 s"
        );
        let later = t.position(10.5);
        assert_eq!(
            later.capture.as_ref().map(|c| (c.board, c.version)),
            Some((1, 2))
        );
        assert!(
            t.position(0.5).capture.is_none(),
            "nothing on the board yet"
        );
        // a time outside the lecture is moved to the nearest end
        assert_eq!(t.position(-4.0).at, 0.0);
        assert_eq!(t.position(900.0).at, 15.0);
        assert_eq!(t.position(f32::NAN).at, 0.0);
    }

    #[test]
    fn a_photo_taken_outside_the_recording_is_moved_to_the_nearest_end_and_marked() {
        let mut session = CaptureSession::new();
        session
            .capture(
                &board("board-1-first.jpg"),
                CaptureKind::Whiteboard,
                STARTED - 30,
                &NoOcr,
                &CaptureConfig::default(),
            )
            .unwrap();
        session
            .capture(
                &board("board-5-new-topic.jpg"),
                CaptureKind::Whiteboard,
                STARTED + 500,
                &NoOcr,
                &CaptureConfig::default(),
            )
            .unwrap();
        let mut t = Timeline::new("t", Recording::new("r.wav", STARTED, 60.0));
        t.set_captures(&session);
        assert_eq!((t.captures[0].at, t.captures[0].clamped), (0.0, true));
        assert_eq!((t.captures[1].at, t.captures[1].clamped), (60.0, true));
    }

    #[test]
    fn a_search_finds_speech_boards_bookmarks_and_chapters_in_time_order() {
        let mut t = lecture();
        t.captures[0].lines = vec![
            "Week 3 Fourier series".into(),
            "square wave: odd harmonics only".into(),
        ];
        t.captures[1].lines = vec!["an = (2/T) integral f(t) cos(n w0 t) dt".into()];
        let id = t
            .add_bookmark(7.0, "revise the square wave harmonics", 5)
            .unwrap();
        t.add_chapter(5.0, "Square waves").unwrap();

        let hits = t.search("square wave");
        let kinds: Vec<EventKind> = hits.iter().map(|h| h.kind).collect();
        assert_eq!(
            kinds,
            [
                EventKind::Capture,
                EventKind::Chapter,
                EventKind::Sentence,
                EventKind::Bookmark
            ],
            "{hits:#?}"
        );
        assert!(hits.windows(2).all(|w| w[0].at <= w[1].at));
        let sentence = hits.iter().find(|h| h.kind == EventKind::Sentence).unwrap();
        assert!(
            sentence.at > 6.2 && sentence.at < 7.5,
            "it lands on the word: {}",
            sentence.at
        );
        assert_eq!(sentence.segment, Some(2));
        assert_eq!(hits.last().unwrap().bookmark, Some(id));
        assert_eq!(hits[0].capture, Some(1));

        assert_eq!(
            t.search("fourier")
                .iter()
                .filter(|h| h.kind == EventKind::Capture)
                .count(),
            1
        );
        assert!(
            t.search("fourier")
                .iter()
                .any(|h| h.kind == EventKind::Sentence),
            "the lecturer says it too"
        );
        assert!(
            t.search("integra")
                .iter()
                .all(|h| h.kind == EventKind::Capture),
            "the start of a word matches"
        );
        assert!(
            t.search("tegral").is_empty(),
            "the middle of a word does not"
        );
        assert!(
            t.search("square wave laplace").is_empty(),
            "every word must match"
        );
        assert!(t.search("  ").is_empty());
    }

    #[test]
    fn bookmarks_can_be_added_edited_and_removed_within_the_lecture() {
        let mut t = lecture();
        let a = t.add_bookmark(10.0, "  later  ", 1).unwrap();
        let b = t.add_bookmark(3.0, "earlier", 2).unwrap();
        assert_eq!(
            t.bookmarks.iter().map(|x| x.id).collect::<Vec<_>>(),
            [b, a],
            "kept in time order"
        );
        assert_eq!(t.bookmarks[1].note, "later");
        assert!(matches!(
            t.add_bookmark(-1.0, "x", 0),
            Err(TimelineError::OutsideLecture(_))
        ));
        assert!(matches!(
            t.add_bookmark(15.5, "x", 0),
            Err(TimelineError::OutsideLecture(_))
        ));
        assert!(t.add_bookmark(f32::NAN, "x", 0).is_err());
        t.edit_bookmark(a, "changed").unwrap();
        assert_eq!(t.bookmarks[1].note, "changed");
        let around = t.position(5.0);
        assert_eq!(
            (
                around.previous_bookmark.map(|x| x.id),
                around.next_bookmark.map(|x| x.id)
            ),
            (Some(b), Some(a))
        );
        t.remove_bookmark(b).unwrap();
        assert!(matches!(
            t.remove_bookmark(b),
            Err(TimelineError::NoBookmark(_))
        ));
        assert!(matches!(
            t.edit_bookmark(99, "x"),
            Err(TimelineError::NoBookmark(99))
        ));
        let c = t.add_bookmark(1.0, "new", 3).unwrap();
        assert!(c > a, "ids are never reused");
    }

    #[test]
    fn events_step_forward_and_back_by_kind() {
        let mut t = lecture();
        t.add_bookmark(7.0, "mark", 0).unwrap();
        let events = t.events();
        assert!(events.windows(2).all(|w| w[0].at <= w[1].at));
        assert_eq!(events[0].kind, EventKind::Chapter);
        let next = t.next_event(0.0, Some(EventKind::Capture)).unwrap();
        assert_eq!((next.at, next.capture), (2.0, Some(1)));
        assert_eq!(
            t.next_event(2.0, Some(EventKind::Capture)).map(|e| e.at),
            Some(9.0)
        );
        assert_eq!(
            t.previous_event(9.0, Some(EventKind::Capture))
                .map(|e| e.at),
            Some(2.0)
        );
        assert_eq!(t.next_event(7.0, Some(EventKind::Bookmark)), None);
        assert_eq!(
            t.previous_event(7.5, Some(EventKind::Bookmark))
                .map(|e| e.at),
            Some(7.0)
        );
        assert!(t.previous_event(0.0, None).is_none());
    }

    #[test]
    fn detected_chapters_replace_earlier_ones_and_leave_manual_ones_alone() {
        let mut t = Timeline::new("Long lecture", Recording::new("l.wav", STARTED, 900.0));
        t.set_transcript(transcript(vec![
            Segment::new(1, 5.0, 295.0, "Welcome to the lecture."),
            Segment::new(2, 340.0, 595.0, "Now the Laplace transform."),
            Segment::new(3, 625.0, 900.0, "Finally some worked examples."),
        ]));
        t.add_chapter(480.0, "My own heading").unwrap();
        t.detect_chapters(&ChapterConfig::default());
        let starts: Vec<f32> = t.chapters.iter().map(|c| c.start).collect();
        assert_eq!(starts, [0.0, 340.0, 480.0, 625.0], "{:?}", t.chapters);
        assert_eq!(t.chapters[2].reason, ChapterReason::Manual);
        // running it again changes nothing
        let before = t.chapters.clone();
        t.detect_chapters(&ChapterConfig::default());
        assert_eq!(t.chapters, before);
        // a manual chapter near a detected one wins
        t.add_chapter(350.0, "Laplace, my words").unwrap();
        t.detect_chapters(&ChapterConfig::default());
        assert!(t.chapters.iter().any(|c| c.title == "Laplace, my words"));
        assert!(
            !t.chapters.iter().any(|c| c.start == 340.0),
            "{:?}",
            t.chapters
        );
        // the spans end where the next begins, the last at the end of the lecture
        let spans = t.chapter_spans();
        assert_eq!(spans.last().map(|(_, end)| *end), Some(900.0));
        assert_eq!(spans[0].1, spans[1].0.start);
        assert_eq!(
            t.position(400.0).chapter.map(|(i, c)| (i, c.title)),
            Some((1, "Laplace, my words".to_string()))
        );
        // chapters can be removed except the first
        assert!(matches!(
            t.remove_chapter(0.0),
            Err(TimelineError::FirstChapter)
        ));
        assert!(matches!(
            t.remove_chapter(1.5),
            Err(TimelineError::NoChapter(_))
        ));
        t.remove_chapter(350.0).unwrap();
        t.add_chapter(0.0, "Intro").unwrap();
        assert_eq!(
            (t.chapters[0].title.as_str(), t.chapters[0].reason),
            ("Intro", ChapterReason::Manual)
        );
        t.detect_chapters(&ChapterConfig::default());
        assert_eq!(
            t.chapters[0].title, "Intro",
            "a renamed opening chapter stays renamed"
        );
        assert!(t.add_chapter(5000.0, "x").is_err());
    }

    #[test]
    fn boards_make_chapters_in_a_joined_lecture() {
        let mut t = Timeline::new("Signals", Recording::new("l.wav", STARTED, 1200.0));
        let mut session = CaptureSession::new();
        let config = CaptureConfig::default();
        for (file, offset) in [
            ("board-1-first.jpg", 20),
            ("board-3-added.jpg", 300),
            ("board-5-new-topic.jpg", 600),
        ] {
            session
                .capture(
                    &board(file),
                    CaptureKind::Whiteboard,
                    STARTED + offset,
                    &NoOcr,
                    &config,
                )
                .unwrap();
        }
        t.set_captures(&session);
        t.detect_chapters(&ChapterConfig::default());
        let starts: Vec<(f32, ChapterReason)> =
            t.chapters.iter().map(|c| (c.start, c.reason)).collect();
        assert_eq!(
            starts,
            [(0.0, ChapterReason::Start), (600.0, ChapterReason::Board)],
            "{:?}",
            t.chapters
        );
    }

    /// A stand-in recording of a few megabytes.
    fn recording_file(dir: &Path) -> PathBuf {
        let path = dir.join("lecture.wav");
        let bytes: Vec<u8> = (0..3_000_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_timeline_is_saved_and_loaded_beside_its_recording() {
        let dir = tempfile::tempdir().unwrap();
        let recording = recording_file(dir.path());
        let mut t = lecture();
        t.recording = Recording::from_file(&recording, STARTED, 15.0).unwrap();
        t.add_bookmark(4.0, "keep this", 9).unwrap();
        t.detect_chapters(&ChapterConfig::default());

        let saved = t.save_beside(&recording).unwrap();
        assert_eq!(saved, dir.path().join("lecture.wav.zacess-timeline.json"));
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            2,
            "no temporary file is left behind"
        );
        let loaded = Timeline::load_beside(&recording).unwrap();
        assert_eq!(loaded.timeline, t);
        assert!(loaded.recording_matches);
        assert_eq!(loaded.timeline.search("keep").len(), 1);

        // saving again replaces the file whole
        t.add_bookmark(5.0, "second", 10).unwrap();
        t.save_beside(&recording).unwrap();
        assert_eq!(
            Timeline::load_beside(&recording)
                .unwrap()
                .timeline
                .bookmarks
                .len(),
            2
        );

        // the recording changed at its end: the times may no longer line up
        let mut bytes = std::fs::read(&recording).unwrap();
        let last = bytes.len() - 10;
        bytes[last] ^= 0xff;
        std::fs::write(&recording, &bytes).unwrap();
        assert!(!Timeline::load_beside(&recording).unwrap().recording_matches);
        std::fs::remove_file(&recording).unwrap();
        assert!(
            !Timeline::load_beside(&recording).unwrap().recording_matches,
            "the recording is gone"
        );
    }

    #[test]
    fn a_missing_a_damaged_or_a_newer_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let recording = recording_file(dir.path());
        assert!(matches!(
            Timeline::load_beside(&recording),
            Err(TimelineError::Io(_))
        ));
        std::fs::write(Timeline::sidecar_path(&recording), "{ not json").unwrap();
        assert!(matches!(
            Timeline::load_beside(&recording),
            Err(TimelineError::Json(_))
        ));
        let mut t = Timeline::new("t", Recording::new("lecture.wav", 0, 10.0));
        t.schema_version = TIMELINE_SCHEMA_VERSION + 1;
        std::fs::write(
            Timeline::sidecar_path(&recording),
            serde_json::to_string(&t).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            Timeline::load_beside(&recording),
            Err(TimelineError::Newer(_))
        ));
        // a recording with no stored fingerprint only has to exist
        t.schema_version = TIMELINE_SCHEMA_VERSION;
        std::fs::write(
            Timeline::sidecar_path(&recording),
            serde_json::to_string(&t).unwrap(),
        )
        .unwrap();
        assert!(Timeline::load_beside(&recording).unwrap().recording_matches);
    }

    #[test]
    fn a_damaged_file_is_repaired_and_a_newer_one_is_reported_as_newer() {
        let dir = tempfile::tempdir().unwrap();
        let recording = recording_file(dir.path());
        let mut t = lecture();
        t.add_bookmark(5.0, "five", 0).unwrap();
        let mut value = serde_json::to_value(&t).unwrap();
        value["chapters"] = serde_json::json!([]);
        value["next_bookmark"] = serde_json::json!(0);
        std::fs::write(Timeline::sidecar_path(&recording), value.to_string()).unwrap();
        let mut loaded = Timeline::load_beside(&recording).unwrap().timeline;
        assert_eq!(loaded.chapters.len(), 1, "an opening chapter was put back");
        loaded.add_chapter(0.0, "Intro").unwrap();
        let next = loaded.add_bookmark(6.0, "six", 0).unwrap();
        assert!(
            next > 1,
            "a counter that was lost never reuses an id: {next}"
        );

        std::fs::write(
            Timeline::sidecar_path(&recording),
            r#"{"schema_version": 9, "something_new": true}"#,
        )
        .unwrap();
        assert!(matches!(
            Timeline::load_beside(&recording),
            Err(TimelineError::Newer(9))
        ));
        let odd = Recording::new("x.wav", 0, f32::NAN);
        assert_eq!(odd.duration_secs, 0.0);
    }

    #[test]
    fn the_duration_is_the_longest_thing_on_the_clock() {
        let mut t = Timeline::new("t", Recording::new("r.wav", 0, 0.0));
        assert_eq!(t.duration(), 0.0);
        assert!(
            t.add_bookmark(500.0, "no length yet, so any time is allowed", 0)
                .is_ok()
        );
        t.set_transcript(transcript(vec![Segment::new(1, 0.0, 42.0, "x")]));
        assert_eq!(t.duration(), 42.0);
        t.recording.duration_secs = 60.0;
        assert_eq!(t.duration(), 60.0);
    }

    #[test]
    fn photos_after_a_pause_are_placed_by_the_audio_and_not_the_wall_clock() {
        let mut recording = Recording::new("r.wav", STARTED, 120.0);
        recording.pauses = vec![
            Pause {
                at_secs: 10.0,
                length_secs: 600.0,
            },
            Pause {
                at_secs: 50.0,
                length_secs: 30.0,
            },
        ];
        let t = Timeline::new("t", recording);
        assert_eq!(t.place(STARTED + 4), (4.0, false));
        assert_eq!(t.place(STARTED + 300), (10.0, true), "in the first pause");
        assert_eq!(t.place(STARTED + 610), (10.0, false));
        assert_eq!(t.place(STARTED + 660), (50.0, true), "in the second pause");
        assert_eq!(t.place(STARTED + 680), (50.0, false));
        assert_eq!(
            t.place(STARTED + 700),
            (70.0, false),
            "both pauses are behind it"
        );
        assert_eq!(t.place(STARTED + 9_000), (120.0, true), "after the end");
        // a timeline made before pauses were kept still loads
        let json = serde_json::to_string(&t.recording).unwrap();
        let old = json.replace(r#","pauses":[{"at_secs":10.0,"length_secs":600.0},{"at_secs":50.0,"length_secs":30.0}]"#, "");
        assert!(!old.contains("pauses"));
        let back: Recording = serde_json::from_str(&old).unwrap();
        assert!(back.pauses.is_empty());
    }
}
