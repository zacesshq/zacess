//! Whiteboards and handwritten notes captured with the phone's camera.
//!
//! A capture is a photo of a board or a page. Each one is compared with what was captured before:
//! the same writing photographed again is dropped, writing added or rubbed out makes a new version
//! of the same board and a wiped or different board starts a new one. Every version that is kept
//! goes through OCR on the device and becomes a document, so what was on the board can be read,
//! searched and jumped to later while the earlier versions stay. The camera and the capture screen
//! come later; the tests use recorded sample images.
//!
//! Handwriting is read by the same OCR engine, which copes with neat print and little else. The
//! legibility score and the share of the ink the engine read say how far to trust the result, so a
//! page it could not read is marked rather than silently wrong.

pub mod change;
pub mod frame;
pub mod slides;

use crate::ask::{Index, SourceRef};
use crate::confidence::flags_for;
use crate::import::ImportError;
use crate::import::image::import_image;
use crate::import::ocr::OcrEngine;
use crate::model::{BlockKind, Document, Run};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub use change::{Change, ChangeConfig, Verdict};
pub use frame::{Frame, InkMask};
pub use slides::{SlideConfig, SlideEvent, SlideSession};

/// Bumped when a stored capture session's shape changes.
pub const CAPTURE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("reading the image: {0}")]
    Image(String),
    #[error(transparent)]
    Import(#[from] ImportError),
    #[error("no capture {0}")]
    NoCapture(u32),
    #[error("capture settings: {0}")]
    Config(String),
}

/// What was photographed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    Whiteboard,
    /// A page of handwritten notes.
    Handwritten,
    /// A slide photographed from a projector.
    Slide,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureConfig {
    /// The longer side of the shrunk frame used for change detection, in pixels, at most.
    pub max_side: usize,
    /// How far around a pixel the board's brightness is taken from, in shrunk pixels.
    pub radius: usize,
    /// How much darker than the board a pixel must be to count as ink, as a fraction.
    pub darker_by: f32,
    /// A photo with more than this share hidden behind something cannot be judged.
    pub max_hidden: f32,
    pub change: ChangeConfig,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        CaptureConfig {
            max_side: 320,
            radius: 8,
            darker_by: 0.2,
            max_hidden: 0.5,
            change: ChangeConfig::default(),
        }
    }
}

impl CaptureConfig {
    /// Refuses settings that would make every photo a repeat or every photo a change.
    pub fn validate(&self) -> Result<(), CaptureError> {
        let unit = |name: &str, v: f32| {
            if (0.0..=1.0).contains(&v) && !(name == "darker_by" && (v <= 0.0 || v >= 1.0)) {
                Ok(())
            } else {
                Err(CaptureError::Config(format!(
                    "{name} must be between 0 and 1, not {v}"
                )))
            }
        };
        if self.max_side < 32 {
            return Err(CaptureError::Config(format!(
                "max_side must be at least 32, not {}",
                self.max_side
            )));
        }
        if self.radius == 0 {
            return Err(CaptureError::Config("radius must be at least 1".into()));
        }
        unit("darker_by", self.darker_by)?;
        unit("max_hidden", self.max_hidden)?;
        unit("same_above", self.change.same_above)?;
        unit("repeat_below", self.change.repeat_below)?;
        unit("new_board_above", self.change.new_board_above)?;
        if self.change.max_shift < 0 {
            return Err(CaptureError::Config("max_shift cannot be negative".into()));
        }
        Ok(())
    }
}

/// One kept photo of a board, with what was read from it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capture {
    pub id: u32,
    /// Unix seconds.
    pub taken_at: i64,
    /// When the writing was last seen, in Unix seconds: the latest repeat, else the capture.
    pub last_seen_at: i64,
    pub file: PathBuf,
    /// A hash of the file's bytes, so a moved photo can be matched to its capture.
    pub fingerprint: u64,
    pub kind: CaptureKind,
    /// 1 for the first capture of a board, counting up.
    pub version: u32,
    /// How this version differs from the one before it; `None` for the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<Change>,
    /// How many later photos showed the same writing and were dropped.
    pub repeats: u32,
    pub document: Document,
    /// The mean confidence of the recognised words, 0 to 1; 0 when nothing was read.
    pub legibility: f32,
    /// The share of the ink that lies inside recognised words, 0 to 1, so a page where the engine
    /// picked out only a heading is not mistaken for one it read.
    pub read_share: f32,
    #[serde(skip)]
    mask: Option<InkMask>,
    #[serde(skip)]
    mask_failed: bool,
}

impl Capture {
    pub fn text(&self) -> String {
        self.document.plain_text()
    }

    /// Runs the OCR engine was unsure of, by the same rules as a scanned page.
    pub fn needs_checking(&self) -> Vec<&Run> {
        let mut out = Vec::new();
        for block in &self.document.blocks {
            let runs = block.runs();
            for (run, flagged) in runs.iter().zip(flags_for(&runs)) {
                if flagged {
                    out.push(*run);
                }
            }
        }
        out
    }

    /// The lines on the board, for comparing versions: each line of each block, with its spacing
    /// tidied. A list or a table gives one line per item or row.
    pub fn lines(&self) -> Vec<String> {
        document_lines(&self.document)
    }

    /// Whether the engine read the capture well enough to trust: enough of the ink inside words
    /// and the words themselves confident.
    pub fn readable(&self, at_least: f32) -> bool {
        self.legibility >= at_least && self.read_share >= at_least
    }
}

/// A board (or a page) through the lecture: every version kept, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Board {
    pub id: u32,
    pub title: String,
    pub kind: CaptureKind,
    pub captures: Vec<Capture>,
}

/// The lines that differ between two versions, by their text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub kept: Vec<String>,
}

impl Board {
    pub fn latest(&self) -> Option<&Capture> {
        self.captures.last()
    }

    pub fn capture(&self, id: u32) -> Option<&Capture> {
        self.captures.iter().find(|c| c.id == id)
    }

    /// What was written and rubbed out between two versions, read from their text. Lines are
    /// matched with case and spacing tidied and one letter in five forgiven, so an OCR wobble
    /// between two photos of the same line does not read as a change; a changed digit, sign or
    /// symbol is never forgiven, because that is the change that matters.
    pub fn diff(&self, from: u32, to: u32) -> Result<TextDiff, CaptureError> {
        let a = self
            .capture(from)
            .ok_or(CaptureError::NoCapture(from))?
            .lines();
        let b = self.capture(to).ok_or(CaptureError::NoCapture(to))?.lines();
        Ok(TextDiff {
            added: b
                .iter()
                .filter(|l| !a.iter().any(|m| same_line(l, m)))
                .cloned()
                .collect(),
            removed: a
                .iter()
                .filter(|l| !b.iter().any(|m| same_line(l, m)))
                .cloned()
                .collect(),
            kept: b
                .iter()
                .filter(|l| a.iter().any(|m| same_line(l, m)))
                .cloned()
                .collect(),
        })
    }
}

/// What happened to a photo. Each outcome is a word the interface can say.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    /// No writing could be seen, so nothing was kept.
    Blank,
    /// Too much of the photo is hidden behind something to judge it.
    Obscured,
    /// The same writing as a kept capture, which was not kept again.
    Repeat {
        board: u32,
        capture: u32,
        change: Change,
    },
    /// Writing was added, rubbed out or rewritten on a board seen before.
    NewVersion {
        board: u32,
        capture: u32,
        previous: u32,
        change: Change,
        diff: TextDiff,
    },
    /// A board or page not seen before.
    NewBoard {
        board: u32,
        capture: u32,
        title: String,
    },
}

impl Outcome {
    /// The capture the photo belongs to, if it was kept or repeated one.
    pub fn capture(&self) -> Option<u32> {
        match self {
            Outcome::Repeat { capture, .. }
            | Outcome::NewVersion { capture, .. }
            | Outcome::NewBoard { capture, .. } => Some(*capture),
            Outcome::Blank | Outcome::Obscured => None,
        }
    }

    pub fn board(&self) -> Option<u32> {
        match self {
            Outcome::Repeat { board, .. }
            | Outcome::NewVersion { board, .. }
            | Outcome::NewBoard { board, .. } => Some(*board),
            Outcome::Blank | Outcome::Obscured => None,
        }
    }
}

/// Where a search found something.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Found {
    pub board: u32,
    pub capture: u32,
    /// Unix seconds.
    pub taken_at: i64,
    pub line: String,
}

/// The boards and pages captured in one sitting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureSession {
    pub schema_version: u32,
    pub boards: Vec<Board>,
    next_id: u32,
}

impl Default for CaptureSession {
    fn default() -> Self {
        CaptureSession {
            schema_version: CAPTURE_SCHEMA_VERSION,
            boards: Vec::new(),
            next_id: 0,
        }
    }
}

impl CaptureSession {
    pub fn new() -> Self {
        CaptureSession::default()
    }

    /// Takes in a photo. It is compared with the latest version of every board of its kind, so
    /// turning back to an earlier board is recognised; the closest match decides whether it is a
    /// repeat, a new version or a new board. OCR runs only for a capture that is kept. A photo
    /// whose OCR fails is left unrecorded so it can be offered again.
    pub fn capture(
        &mut self,
        path: &Path,
        kind: CaptureKind,
        taken_at: i64,
        ocr: &dyn OcrEngine,
        config: &CaptureConfig,
    ) -> Result<Outcome, CaptureError> {
        config.validate()?;
        let (image, turned) = Frame::load_turned(path)?;
        let frame = Frame::from_image(&image).shrunk_to(config.max_side);
        let mask = InkMask::from_frame(&frame, config.radius, config.darker_by);
        if mask.hidden_share() > config.max_hidden {
            return Ok(Outcome::Obscured);
        }
        if mask.count() == 0 {
            return Ok(Outcome::Blank);
        }

        // a session read back from JSON has no masks; they are rebuilt from the files once
        for board in &mut self.boards {
            if let Some(latest) = board.captures.last_mut()
                && latest.mask.is_none()
                && !latest.mask_failed
            {
                match Frame::load(&latest.file) {
                    Ok(frame) => {
                        let frame = frame.shrunk_to(config.max_side);
                        latest.mask =
                            Some(InkMask::from_frame(&frame, config.radius, config.darker_by));
                    }
                    Err(_) => latest.mask_failed = true,
                }
            }
        }

        let mut closest: Option<(usize, Change)> = None;
        for (i, board) in self.boards.iter().enumerate() {
            if board.kind != kind {
                continue;
            }
            let Some(previous) = board.latest().and_then(|c| c.mask.as_ref()) else {
                continue;
            };
            if previous.width != mask.width || previous.height != mask.height {
                continue;
            }
            let change = change::compare(previous, &mask, &config.change);
            if closest.is_none_or(|(_, c)| change.fraction() < c.fraction()) {
                closest = Some((i, change));
            }
        }

        let verdict = closest
            .map(|(_, c)| change::classify(&c, &config.change))
            .unwrap_or(Verdict::NewBoard);
        if let (Verdict::Repeat, Some((i, change))) = (verdict, closest) {
            let board = &mut self.boards[i];
            let latest = board
                .captures
                .last_mut()
                .expect("a board with a mask has a capture");
            latest.repeats += 1;
            latest.last_seen_at = latest.last_seen_at.max(taken_at);
            return Ok(Outcome::Repeat {
                board: board.id,
                capture: latest.id,
                change,
            });
        }

        // OCR reads the file itself and ignores the orientation note, so a turned photo is
        // written out the right way up for it
        let imported = if turned {
            let dir = tempfile::tempdir().map_err(|e| CaptureError::Image(e.to_string()))?;
            let upright = dir.path().join("upright.png");
            image
                .save(&upright)
                .map_err(|e| CaptureError::Image(e.to_string()))?;
            import_image(&upright, ocr)?
        } else {
            import_image(path, ocr)?
        };
        let mut document = imported.document;
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            document.source.file_name = name.to_string();
        }
        let legibility = mean_confidence(&document);
        let read_share = read_share(&mask, &document);
        let fingerprint = fingerprint(path)?;
        self.next_id += 1;
        let id = self.next_id;
        let kept = |document: Document, version: u32, change: Option<Change>| Capture {
            id,
            taken_at,
            last_seen_at: taken_at,
            file: path.to_path_buf(),
            fingerprint,
            kind,
            version,
            change,
            repeats: 0,
            document,
            legibility,
            read_share,
            mask: Some(mask.clone()),
            mask_failed: false,
        };

        match (verdict, closest) {
            (Verdict::NewVersion, Some((i, change))) => {
                let board = &mut self.boards[i];
                let previous = board.captures.last().map(|c| c.id).unwrap_or(0);
                let version = board.captures.len() as u32 + 1;
                document.title = format!("{}, version {version}", board.title);
                board.captures.push(kept(document, version, Some(change)));
                let diff = board.diff(previous, id)?;
                Ok(Outcome::NewVersion {
                    board: board.id,
                    capture: id,
                    previous,
                    change,
                    diff,
                })
            }
            _ => {
                let board_id = self.boards.len() as u32 + 1;
                let title = title_for(&document, kind, board_id);
                document.title = title.clone();
                self.boards.push(Board {
                    id: board_id,
                    title: title.clone(),
                    kind,
                    captures: vec![kept(document, 1, None)],
                });
                Ok(Outcome::NewBoard {
                    board: board_id,
                    capture: id,
                    title,
                })
            }
        }
    }

    pub fn board(&self, id: u32) -> Option<&Board> {
        self.boards.iter().find(|b| b.id == id)
    }

    /// Every kept capture in the order it was taken.
    pub fn captures(&self) -> Vec<&Capture> {
        let mut all: Vec<&Capture> = self.boards.iter().flat_map(|b| b.captures.iter()).collect();
        all.sort_by_key(|c| c.id);
        all
    }

    /// An index over every kept capture, for asking questions of the day's boards alongside the
    /// rest of the library.
    pub fn index(&self) -> Index {
        let mut index = Index::new();
        for board in &self.boards {
            for capture in &board.captures {
                index.add(
                    SourceRef {
                        id: format!("capture-{}", capture.id),
                        title: capture.document.title.clone(),
                    },
                    &capture.document,
                );
            }
        }
        index
    }

    /// Lines holding every word of a query, as a whole word or the start of one, ignoring case,
    /// with the capture to open.
    pub fn search(&self, query: &str) -> Vec<Found> {
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for board in &self.boards {
            for capture in &board.captures {
                for line in capture.lines() {
                    let words: Vec<String> = line
                        .split(|c: char| !c.is_alphanumeric())
                        .filter(|w| !w.is_empty())
                        .map(str::to_lowercase)
                        .collect();
                    if terms
                        .iter()
                        .all(|t| words.iter().any(|w| w.starts_with(t.as_str())))
                    {
                        out.push(Found {
                            board: board.id,
                            capture: capture.id,
                            taken_at: capture.taken_at,
                            line,
                        });
                    }
                }
            }
        }
        out
    }

    /// Captures the engine could not read well: below the given legibility or with less than
    /// that share of their ink inside recognised words.
    pub fn hard_to_read(&self, below: f32) -> Vec<&Capture> {
        self.captures()
            .into_iter()
            .filter(|c| !c.readable(below))
            .collect()
    }
}

/// Whether two lines read as the same writing: equal once case and spaces are dropped and the
/// letters OCR confuses with digits (o, l, i, s, z) are read as those digits. Failing that, they
/// match within one edit in five with every digit and symbol the same.
fn same_line(a: &str, b: &str) -> bool {
    let tidy = |s: &str| {
        s.chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .map(|c| match c {
                'o' => '0',
                'l' | 'i' => '1',
                's' => '5',
                'z' => '2',
                other => other,
            })
            .collect::<Vec<char>>()
    };
    let (a, b) = (tidy(a), tidy(b));
    if a == b {
        return true;
    }
    let skeleton = |s: &[char]| {
        s.iter()
            .copied()
            .filter(|c| !c.is_alphabetic())
            .collect::<Vec<char>>()
    };
    if skeleton(&a) != skeleton(&b) {
        return false;
    }
    let longest = a.len().max(b.len());
    longest > 0 && edit_distance(&a, &b) * 5 <= longest
}

fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let substitute = previous[j] + usize::from(ca != cb);
            current.push(substitute.min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous[b.len()]
}

/// The mean confidence of the runs that hold a word or a number; a bullet or a lone symbol the
/// engine could not place is not a word and does not count.
fn mean_confidence(document: &Document) -> f32 {
    let runs: Vec<&Run> = document
        .runs()
        .filter(|r| r.text.chars().any(char::is_alphanumeric))
        .collect();
    if runs.is_empty() {
        return 0.0;
    }
    runs.iter().map(|r| r.confidence.value()).sum::<f32>() / runs.len() as f32
}

/// The share of the ink that lies inside the boxes of recognised runs.
fn read_share(mask: &InkMask, document: &Document) -> f32 {
    let total = mask.count();
    if total == 0 {
        return 0.0;
    }
    let scale = mask.scale.max(1) as f32;
    let mut covered = vec![false; mask.bits.len()];
    for run in document.runs() {
        let Some(origin) = &run.origin else { continue };
        let r = &origin.region;
        let x0 = ((r.x / scale).floor().max(0.0) as usize).min(mask.width);
        let x1 = (((r.x + r.width) / scale).ceil().max(0.0) as usize).min(mask.width);
        let y0 = ((r.y / scale).floor().max(0.0) as usize).min(mask.height);
        let y1 = (((r.y + r.height) / scale).ceil().max(0.0) as usize).min(mask.height);
        for y in y0..y1 {
            for x in x0..x1 {
                covered[y * mask.width + x] = true;
            }
        }
    }
    let inside = mask
        .bits
        .iter()
        .zip(&covered)
        .filter(|(b, c)| **b && **c)
        .count();
    inside as f32 / total as f32
}

/// A 64-bit FNV-1a hash of the file's bytes.
fn fingerprint(path: &Path) -> Result<u64, CaptureError> {
    let bytes =
        std::fs::read(path).map_err(|e| CaptureError::Image(format!("{}: {e}", path.display())))?;
    Ok(bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    }))
}

/// The lines of a document with their spacing tidied: each line of each block, so a list or a
/// table gives one line per item or row.
pub(crate) fn document_lines(document: &Document) -> Vec<String> {
    document
        .blocks
        .iter()
        .filter(|b| !matches!(b.kind, BlockKind::PageBreak))
        .flat_map(|b| {
            b.text()
                .lines()
                .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

/// A board is named after its first heading, else its first line, cut to eight words, else its
/// number.
pub(crate) fn title_for(document: &Document, kind: CaptureKind, number: u32) -> String {
    let heading = document
        .blocks
        .iter()
        .find(|b| matches!(b.kind, BlockKind::Heading { .. }))
        .map(|b| b.text());
    let first = heading.or_else(|| {
        document
            .blocks
            .iter()
            .map(|b| b.text())
            .find(|t| !t.trim().is_empty())
    });
    match first {
        Some(text) if !text.trim().is_empty() => text
            .split_whitespace()
            .take(8)
            .collect::<Vec<_>>()
            .join(" "),
        _ => match kind {
            CaptureKind::Whiteboard => format!("Board {number}"),
            CaptureKind::Handwritten => format!("Page {number}"),
            CaptureKind::Slide => format!("Slide {number}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::ocr::{OcrPage, Tesseract};

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/whiteboard")
            .join(name)
    }

    #[derive(Debug, serde::Deserialize)]
    struct Sidecar {
        captures: Vec<Expected>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct Expected {
        file: String,
        board: String,
        expect: String,
        text: Vec<String>,
    }

    fn sidecar() -> Sidecar {
        serde_json::from_str(&std::fs::read_to_string(fixture("captures.json")).unwrap()).unwrap()
    }

    fn kind_of(file: &str) -> CaptureKind {
        if file.starts_with("notes") {
            CaptureKind::Handwritten
        } else {
            CaptureKind::Whiteboard
        }
    }

    /// Reads nothing, so change detection can be tested without an OCR engine installed.
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

    fn tesseract() -> Option<Tesseract> {
        let found = Tesseract::detect();
        if found.is_none() {
            eprintln!("skipped: tesseract is not installed");
        }
        found
    }

    #[test]
    fn repeats_are_dropped_and_changes_kept_as_the_sidecar_says() {
        let meta = sidecar();
        let mut session = CaptureSession::new();
        let config = CaptureConfig::default();
        let mut boards: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for (i, expected) in meta.captures.iter().enumerate() {
            let outcome = session
                .capture(
                    &fixture(&expected.file),
                    kind_of(&expected.file),
                    i as i64 * 60,
                    &NoOcr,
                    &config,
                )
                .unwrap();
            match expected.expect.as_str() {
                "new" => assert!(
                    matches!(outcome, Outcome::NewBoard { .. }),
                    "{}: {outcome:?}",
                    expected.file
                ),
                "repeat" => assert!(
                    matches!(outcome, Outcome::Repeat { .. }),
                    "{}: {outcome:?}",
                    expected.file
                ),
                "changed" => {
                    if boards.contains_key(&expected.board) {
                        assert!(
                            matches!(outcome, Outcome::NewVersion { .. }),
                            "{}: {outcome:?}",
                            expected.file
                        );
                    } else {
                        assert!(
                            matches!(outcome, Outcome::NewBoard { .. }),
                            "{}: {outcome:?}",
                            expected.file
                        );
                    }
                }
                other => panic!("unknown expectation {other}"),
            }
            if let Some(&board) = boards.get(&expected.board) {
                assert_eq!(
                    outcome.board(),
                    Some(board),
                    "{} belongs to board {board}",
                    expected.file
                );
            } else {
                boards.insert(expected.board.clone(), outcome.board().unwrap());
            }
        }
        assert_eq!(
            session.boards.len(),
            3,
            "the Fourier board, the Laplace board and the notes page"
        );
        let fourier = &session.boards[0];
        assert_eq!(fourier.captures.len(), 3);
        assert_eq!(fourier.captures[0].repeats, 1);
        assert_eq!(
            (
                fourier.captures[0].taken_at,
                fourier.captures[0].last_seen_at
            ),
            (0, 60),
            "a repeat updates when the writing was last seen"
        );
        assert_eq!(
            fourier
                .captures
                .iter()
                .map(|c| c.version)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        let change = fourier.captures[1].change.unwrap();
        assert!(
            change.added > 0.1 && change.removed < 0.03,
            "a line was added: {change:?}"
        );
        let change = fourier.captures[2].change.unwrap();
        assert!(
            change.added > 0.05 && change.removed > 0.05,
            "a line was rubbed out and another written: {change:?}"
        );
        assert_eq!(
            session.boards[1].title, "Board 2",
            "nothing was read, so the board is numbered"
        );
        assert_eq!(session.boards[2].kind, CaptureKind::Handwritten);
        assert_eq!(session.captures().len(), 5);
        assert!(session.captures().iter().all(|c| c.fingerprint != 0));
        assert_eq!(session.schema_version, CAPTURE_SCHEMA_VERSION);

        let json = serde_json::to_string(&session).unwrap();
        let mut back: CaptureSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.boards.len(), 3);
        assert!(
            back.boards[0].captures[0].mask.is_none(),
            "masks are not stored"
        );
        // a reloaded session rebuilds masks from the files, so a repeat is still a repeat
        let again = back
            .capture(
                &fixture("board-5-new-topic.jpg"),
                CaptureKind::Whiteboard,
                999,
                &NoOcr,
                &config,
            )
            .unwrap();
        assert!(
            matches!(again, Outcome::Repeat { board: 2, .. }),
            "{again:?}"
        );
        assert_eq!(back.boards[1].captures[0].last_seen_at, 999);
        // a photo of the other kind never joins a board of this kind
        let page = back
            .capture(
                &fixture("board-5-new-topic.jpg"),
                CaptureKind::Handwritten,
                1000,
                &NoOcr,
                &config,
            )
            .unwrap();
        assert!(
            matches!(page, Outcome::NewBoard { board: 4, .. }),
            "{page:?}"
        );
    }

    fn capture_with(blocks: Vec<crate::model::Block>, legibility: f32, read_share: f32) -> Capture {
        let mut document = Document::new(
            "test",
            crate::model::Source {
                file_name: "x.png".into(),
                format: crate::model::SourceFormat::Image,
            },
        );
        document.blocks = blocks;
        Capture {
            id: 1,
            taken_at: 0,
            last_seen_at: 0,
            file: PathBuf::from("x.png"),
            fingerprint: 1,
            kind: CaptureKind::Whiteboard,
            version: 1,
            change: None,
            repeats: 0,
            document,
            legibility,
            read_share,
            mask: None,
            mask_failed: false,
        }
    }

    #[test]
    fn a_list_gives_one_line_per_item_and_readability_needs_both_scores() {
        use crate::model::{Block, BlockId, ListItem, Origin, Rect};
        let origin = Origin::new(1, Rect::new(0.0, 0.0, 1.0, 1.0));
        let list = Block {
            id: BlockId(1),
            kind: BlockKind::List {
                ordered: false,
                items: [
                    "periodic signal",
                    "fundamental f0 = 1/T",
                    "harmonics at n f0",
                ]
                .iter()
                .map(|t| ListItem::new(vec![Run::typed(*t)]))
                .collect(),
            },
            origin,
        };
        let capture = capture_with(vec![list], 0.9, 0.9);
        assert_eq!(
            capture.lines(),
            [
                "periodic signal",
                "fundamental f0 = 1/T",
                "harmonics at n f0"
            ]
        );
        assert!(capture.readable(0.6));
        assert!(
            !capture_with(Vec::new(), 0.95, 0.1).readable(0.6),
            "a clear heading on an unread page"
        );
        assert!(!capture_with(Vec::new(), 0.3, 0.9).readable(0.6));
    }

    #[test]
    fn blank_photos_and_bad_settings_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let blank = dir.path().join("blank.png");
        image::RgbImage::from_pixel(400, 300, image::Rgb([240, 238, 230]))
            .save(&blank)
            .unwrap();
        let mut session = CaptureSession::new();
        let config = CaptureConfig::default();
        assert_eq!(
            session
                .capture(&blank, CaptureKind::Whiteboard, 0, &NoOcr, &config)
                .unwrap(),
            Outcome::Blank
        );
        assert!(session.boards.is_empty());

        // a dark figure over most of the frame
        let mut covered = image::RgbImage::from_pixel(400, 300, image::Rgb([240, 238, 230]));
        for y in 0..300 {
            for x in 50..350 {
                covered.put_pixel(x, y, image::Rgb([30, 30, 30]));
            }
        }
        let obscured = dir.path().join("obscured.png");
        covered.save(&obscured).unwrap();
        assert_eq!(
            session
                .capture(&obscured, CaptureKind::Whiteboard, 0, &NoOcr, &config)
                .unwrap(),
            Outcome::Obscured
        );

        let bad = CaptureConfig {
            darker_by: 0.0,
            ..CaptureConfig::default()
        };
        assert!(matches!(
            session.capture(
                &fixture("board-1-first.jpg"),
                CaptureKind::Whiteboard,
                0,
                &NoOcr,
                &bad
            ),
            Err(CaptureError::Config(_))
        ));
        assert!(
            CaptureConfig {
                max_side: 8,
                ..CaptureConfig::default()
            }
            .validate()
            .is_err()
        );
        assert!(CaptureConfig::default().validate().is_ok());
    }

    #[test]
    fn lines_match_through_ocr_wobble_but_not_through_a_changed_number() {
        assert!(
            same_line("T = 1/f0 and w0 = 2 pi f0", "T=1/f0 and w0 =2 pi fo"),
            "o for 0 is an OCR confusion"
        );
        assert!(same_line(
            "f(t) = a0/2 + sum an cos(n w0 t)",
            "f(t) = a0/2 + sum an cos(n wO t)"
        ));
        assert!(same_line("harmonics at n f0", "harmonlcs at n f0"));
        assert!(same_line("Week 3 Fourier series", "week 3 fourier serles"));
        assert!(!same_line("V = 4.7 V", "V = 4.1 V"), "a digit");
        assert!(!same_line("x > 0", "x < 0"), "a sign");
        assert!(
            !same_line("T", "I"),
            "nothing to forgive in a one letter line"
        );
        assert_eq!(edit_distance(&['a', 'b', 'c'], &['a', 'x', 'c', 'd']), 2);
    }

    #[test]
    fn kept_captures_become_searchable_notes_with_versions() {
        let Some(ocr) = tesseract() else { return };
        let meta = sidecar();
        let mut session = CaptureSession::new();
        let config = CaptureConfig::default();
        let mut outcomes = Vec::new();
        for (i, expected) in meta.captures.iter().enumerate() {
            outcomes.push(
                session
                    .capture(
                        &fixture(&expected.file),
                        kind_of(&expected.file),
                        i as i64 * 60,
                        &ocr,
                        &config,
                    )
                    .unwrap(),
            );
        }
        let fourier = session.board(1).unwrap();
        assert!(fourier.title.contains("Fourier"), "{}", fourier.title);
        assert_eq!(fourier.captures.len(), 3);
        for (capture, expected) in
            fourier
                .captures
                .iter()
                .zip([&meta.captures[0], &meta.captures[2], &meta.captures[3]])
        {
            let text = capture.text().to_lowercase();
            for line in &expected.text {
                let key = line.split_whitespace().next().unwrap().to_lowercase();
                assert!(
                    text.contains(&key),
                    "{} should contain {key}: {text}",
                    expected.file
                );
            }
            assert!(
                capture.readable(0.6),
                "{} legibility {} read share {}",
                expected.file,
                capture.legibility,
                capture.read_share
            );
        }
        // the outcome of a new version already carries what changed, in words
        match &outcomes[2] {
            Outcome::NewVersion { previous, diff, .. } => {
                assert_eq!(*previous, fourier.captures[0].id);
                assert_eq!(diff.added.len(), 1, "{diff:?}");
                assert!(
                    diff.added[0].to_lowercase().contains("integral"),
                    "{diff:?}"
                );
                assert!(diff.removed.is_empty(), "{diff:?}");
            }
            other => panic!("{other:?}"),
        }
        let diff = fourier
            .diff(fourier.captures[1].id, fourier.captures[2].id)
            .unwrap();
        // the rewritten line is reported; a long formula the engine read differently in the two
        // photos may be reported too, because a changed digit is never forgiven
        assert!(
            diff.added
                .iter()
                .any(|l| l.to_lowercase().contains("square wave")),
            "{diff:?}"
        );
        assert!(diff.removed.iter().any(|l| l.contains("1/f0")), "{diff:?}");
        assert!(
            diff.kept.iter().any(|l| l.contains("integral")),
            "the line that did not change is kept: {diff:?}"
        );
        assert!(
            matches!(&outcomes[4], Outcome::NewBoard { title, .. } if title.contains("Laplace")),
            "{:?}",
            outcomes[4]
        );

        let laplace = session.board(2).unwrap();
        let found = session.search("laplace transform");
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].board, found[0].capture),
            (2, laplace.captures[0].id)
        );
        let fourier_hits = session.search("Fourier");
        assert!(
            fourier_hits.iter().any(|f| f.board == 1) && fourier_hits.iter().all(|f| f.board != 2),
            "{fourier_hits:?}"
        );
        assert!(session.search("at").iter().all(|f| {
            f.line
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w.starts_with("at"))
        }));
        assert!(
            session.search("ransform").is_empty(),
            "a term matches the start of a word, not its middle"
        );

        let notes = session.board(3).unwrap();
        assert_eq!(notes.kind, CaptureKind::Handwritten);
        let page = &notes.captures[0];
        assert!(
            page.text().to_lowercase().contains("harmonics"),
            "{}",
            page.text()
        );
        assert!(
            page.readable(0.6),
            "legibility {} read share {}",
            page.legibility,
            page.read_share
        );
        assert!(session.hard_to_read(0.5).is_empty());

        let answer = session
            .index()
            .ask("what are the harmonics of a square wave", 3);
        assert!(
            answer
                .passages
                .iter()
                .any(|p| p.source.id.starts_with("capture-")),
            "{answer:?}"
        );
        assert!(fourier.diff(1, 99).is_err());
    }
}
