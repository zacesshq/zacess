//! Optical character recognition behind one trait, so the engine can differ by platform while
//! everything after it stays the same.
//!
//! The engine chosen for native builds is Tesseract, run as a command and read as TSV so every word
//! carries its confidence. The choice is justified in `docs/ocr.md`. Results become
//! [`Fragment`]s and go through the shared [layout](super::layout) analysis, which is how OCR
//! output feeds the same document model as a PDF text layer.

use super::ImportError;
use super::layout::Fragment;
use crate::model::Confidence;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One recognised word, in image pixels from the top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrWord {
    pub text: String,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub confidence: Confidence,
}

/// A line of words as the engine grouped them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OcrLine {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub words: Vec<OcrWord>,
}

/// A paragraph of lines as the engine grouped them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OcrParagraph {
    pub lines: Vec<OcrLine>,
}

/// Everything recognised on one image.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OcrPage {
    pub width: f32,
    pub height: f32,
    pub paragraphs: Vec<OcrParagraph>,
}

impl OcrPage {
    pub fn words(&self) -> impl Iterator<Item = &OcrWord> {
        self.paragraphs
            .iter()
            .flat_map(|p| p.lines.iter().flat_map(|l| l.words.iter()))
    }
}

/// An OCR engine that runs on the device.
pub trait OcrEngine {
    /// A short name for the processing dashboard and the import log.
    fn name(&self) -> String;
    fn recognise(&self, image: &Path) -> Result<OcrPage, ImportError>;
}

/// Tesseract 5, run as the `tesseract` command.
#[derive(Debug, Clone)]
pub struct Tesseract {
    command: PathBuf,
    language: String,
}

impl Tesseract {
    /// Finds `tesseract` on the path. Returns `None` when it is not installed.
    pub fn detect() -> Option<Self> {
        let candidate = Tesseract {
            command: PathBuf::from("tesseract"),
            language: "eng".into(),
        };
        let ok = Command::new(&candidate.command)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        ok.then_some(candidate)
    }

    pub fn new(command: impl Into<PathBuf>, language: impl Into<String>) -> Self {
        Tesseract {
            command: command.into(),
            language: language.into(),
        }
    }
}

impl OcrEngine for Tesseract {
    fn name(&self) -> String {
        format!("Tesseract ({})", self.language)
    }

    fn recognise(&self, image: &Path) -> Result<OcrPage, ImportError> {
        let output = Command::new(&self.command)
            // one thread per run: Tesseract's own threading scales badly and several pages are
            // recognised side by side anyway
            .env("OMP_THREAD_LIMIT", "1")
            .arg(image)
            .arg("stdout")
            .args(["-l", &self.language, "--psm", "3", "tsv"])
            .output()
            .map_err(|e| ImportError::Ocr(format!("running {}: {e}", self.command.display())))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ImportError::Ocr(format!(
                "tesseract failed: {}",
                stderr.trim()
            )));
        }
        parse_tsv(&String::from_utf8_lossy(&output.stdout))
    }
}

/// Parses Tesseract's TSV output: one row per page, block, paragraph, line and word.
pub fn parse_tsv(tsv: &str) -> Result<OcrPage, ImportError> {
    let mut page = OcrPage::default();
    let mut rows = tsv.lines();
    let header = rows
        .next()
        .ok_or_else(|| ImportError::Ocr("empty OCR output".into()))?;
    let columns: Vec<&str> = header.split('\t').collect();
    let column = |name: &str| columns.iter().position(|c| *c == name);
    let (Some(level), Some(left), Some(top), Some(width), Some(height), Some(conf), Some(text)) = (
        column("level"),
        column("left"),
        column("top"),
        column("width"),
        column("height"),
        column("conf"),
        column("text"),
    ) else {
        return Err(ImportError::Ocr("unexpected OCR output format".into()));
    };

    for row in rows {
        let fields: Vec<&str> = row.split('\t').collect();
        if fields.len() <= conf {
            continue;
        }
        let num = |i: usize| {
            fields
                .get(i)
                .and_then(|f| f.parse::<f32>().ok())
                .unwrap_or(0.0)
        };
        let (l, t, w, h) = (num(left), num(top), num(width), num(height));
        match fields[level] {
            "1" => {
                page.width = w;
                page.height = h;
            }
            "3" => page.paragraphs.push(OcrParagraph::default()),
            "4" => {
                if page.paragraphs.is_empty() {
                    page.paragraphs.push(OcrParagraph::default());
                }
                let paragraph = page.paragraphs.last_mut().expect("just ensured");
                paragraph.lines.push(OcrLine {
                    left: l,
                    top: t,
                    width: w,
                    height: h,
                    words: Vec::new(),
                });
            }
            "5" => {
                let word = fields.get(text).map(|s| s.trim()).unwrap_or("");
                if word.is_empty() {
                    continue;
                }
                let confidence = Confidence::new(num(conf) / 100.0);
                let Some(line) = page.paragraphs.last_mut().and_then(|p| p.lines.last_mut()) else {
                    continue;
                };
                line.words.push(OcrWord {
                    text: word.to_string(),
                    left: l,
                    top: t,
                    width: w,
                    height: h,
                    confidence,
                });
            }
            _ => {}
        }
    }
    page.paragraphs
        .retain(|p| p.lines.iter().any(|l| !l.words.is_empty()));
    Ok(page)
}

/// Characters an engine tends to read a bullet as.
const BULLET_LOOKALIKES: &[&str] = &[
    "e", "\u{a2}", "\u{a9}", "o", "*", "\u{b0}", "\u{b7}", "\u{2022}", "-", "\u{2013}", "\u{2014}",
    "\u{ab}", "\u{bb}", "\u{25cf}",
];

/// Letters that reach above the x-height and so set the top of a glyph box.
const ASCENDERS: &[char] = &[
    'b', 'd', 'f', 'h', 'k', 'l', 't', '(', ')', '[', ']', '{', '}', '|', '/', '!', '?', '\u{3a9}',
];
/// Characters that reach below the baseline.
const DESCENDERS: &[char] = &[
    'g', 'j', 'p', 'q', 'y', ',', ';', '(', ')', '[', ']', '{', '}', '|', '/',
];
/// Typical ascent above the baseline, cap height and descent, in em.
const ASCENT: f32 = 0.76;
const CAP_HEIGHT: f32 = 0.73;
const X_HEIGHT: f32 = 0.55;
const DESCENT: f32 = 0.24;

/// Estimates a word's font size in pixels from its box and which letters it holds, since a box of
/// short letters is far smaller than one with ascenders and descenders at the same size.
fn word_em(word: &OcrWord) -> Option<f32> {
    let text = &word.text;
    if !text.chars().any(|c| c.is_alphanumeric()) {
        return None;
    }
    let top = if text.chars().any(|c| c.is_uppercase() || c.is_ascii_digit()) {
        CAP_HEIGHT.max(if text.contains(ASCENDERS) {
            ASCENT
        } else {
            0.0
        })
    } else if text.contains(ASCENDERS) {
        ASCENT
    } else {
        X_HEIGHT
    };
    let bottom = if text.contains(DESCENDERS) {
        DESCENT
    } else {
        0.0
    };
    Some(word.height / (top + bottom))
}

/// The median word size estimate of a line.
fn line_em(line: &OcrLine) -> Option<f32> {
    let mut ems: Vec<f32> = line.words.iter().filter_map(word_em).collect();
    if ems.is_empty() {
        return None;
    }
    ems.sort_by(f32::total_cmp);
    Some(ems[ems.len() / 2])
}

/// Turns a recognised page into fragments for layout analysis.
///
/// `scale` converts image pixels to the page unit, 1 for an image document. The font size comes
/// from each word's box and the letters in it, smoothed across the paragraph.
pub fn fragments(page: &OcrPage, page_number: u32, scale: f32) -> Vec<Fragment> {
    let mut out = Vec::new();
    for paragraph in &page.paragraphs {
        let mut sizes: Vec<f32> = paragraph.lines.iter().filter_map(line_em).collect();
        sizes.sort_by(f32::total_cmp);
        let paragraph_size = sizes.get(sizes.len() / 2).copied();
        for line in &paragraph.lines {
            if line.words.is_empty() {
                continue;
            }
            let own = line_em(line).unwrap_or(line.height);
            // lines near the paragraph's median size share it, so a line of short words is not
            // read as smaller text, while a line that differs clearly, such as a heading the
            // engine grouped with the paragraph below it, keeps its own size
            let size = match paragraph_size {
                Some(p) if sizes.len() >= 2 && (own - p).abs() <= 0.12 * p => p,
                _ => own,
            };
            let has_descender = line.words.iter().any(|w| w.text.contains(DESCENDERS));
            let baseline =
                line.top + line.height - if has_descender { DESCENT * size } else { 0.0 };
            for (i, word) in line.words.iter().enumerate() {
                let mut text = word.text.clone();
                // a lone low-confidence glyph at the start of an indented line is almost always a
                // bullet the engine could not name; it keeps its low confidence so it stays flagged
                if i == 0
                    && line.words.len() > 1
                    && text.chars().count() == 1
                    && word.confidence.value() < 0.6
                    && BULLET_LOOKALIKES.contains(&text.as_str())
                    && line.words[1].left - (word.left + word.width) > 0.4 * size
                {
                    text = "\u{2022}".to_string();
                }
                out.push(Fragment {
                    page: page_number,
                    x: word.left * scale,
                    baseline: baseline * scale,
                    width: word.width * scale,
                    size: size * scale,
                    bold: false,
                    text,
                    confidence: word.confidence,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TSV: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
        1\t1\t0\t0\t0\t0\t0\t0\t1000\t800\t-1\t\n\
        2\t1\t1\t0\t0\t0\t50\t40\t600\t200\t-1\t\n\
        3\t1\t1\t1\t0\t0\t50\t40\t600\t50\t-1\t\n\
        4\t1\t1\t1\t1\t0\t50\t40\t600\t50\t-1\t\n\
        5\t1\t1\t1\t1\t1\t50\t42\t200\t46\t96.5\tPotential\n\
        5\t1\t1\t1\t1\t2\t270\t40\t180\t50\t95.1\tdivider\n\
        3\t1\t1\t2\t0\t0\t60\t120\t500\t90\t-1\t\n\
        4\t1\t1\t2\t1\t0\t60\t120\t500\t30\t-1\t\n\
        5\t1\t1\t2\t1\t1\t60\t125\t12\t14\t22.8\t\u{a2}\n\
        5\t1\t1\t2\t1\t2\t95\t120\t60\t30\t64.4\tR1\n\
        5\t1\t1\t2\t1\t3\t170\t120\t20\t30\t0\t=\n\
        5\t1\t1\t2\t1\t4\t200\t120\t50\t30\t86.2\tkQ\n\
        4\t1\t1\t2\t2\t0\t60\t160\t500\t30\t-1\t\n\
        5\t1\t1\t2\t2\t1\t60\t160\t40\t30\t90\tnext\n";

    #[test]
    fn tsv_rows_become_paragraphs_lines_and_words_with_confidence() {
        let page = parse_tsv(TSV).unwrap();
        assert_eq!((page.width, page.height), (1000.0, 800.0));
        assert_eq!(page.paragraphs.len(), 2);
        assert_eq!(page.paragraphs[0].lines[0].words.len(), 2);
        let words: Vec<&str> = page.words().map(|w| w.text.as_str()).collect();
        assert_eq!(
            words,
            ["Potential", "divider", "\u{a2}", "R1", "=", "kQ", "next"]
        );
        assert!((page.words().nth(2).unwrap().confidence.value() - 0.228).abs() < 1e-5);
        assert_eq!(page.words().nth(4).unwrap().confidence.value(), 0.0);
    }

    #[test]
    fn fragments_keep_confidence_scale_positions_and_recover_bullets() {
        let page = parse_tsv(TSV).unwrap();
        let frags = fragments(&page, 3, 0.5);
        assert!(frags.iter().all(|f| f.page == 3));
        assert_eq!(frags[0].x, 25.0);
        assert_eq!(
            frags[2].text, "\u{2022}",
            "the lone low-confidence glyph became a bullet"
        );
        assert!(
            frags[2].confidence.value() < 0.3,
            "and kept its low confidence"
        );
        assert_eq!(frags[5].text, "kQ");
        // the two lines of the second paragraph share one size
        assert_eq!(frags[2].size, frags[6].size);
        assert!(frags[0].size > frags[2].size, "the heading line is larger");
    }

    #[test]
    fn rejects_output_without_the_expected_columns() {
        assert!(matches!(parse_tsv(""), Err(ImportError::Ocr(_))));
        assert!(matches!(
            parse_tsv("a\tb\n1\t2\n"),
            Err(ImportError::Ocr(_))
        ));
    }
}
