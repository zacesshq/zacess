//! Braille-ready output through liblouis.
//!
//! A document becomes a BRF file: ASCII braille in lines of 40 cells and pages of 25 lines, which
//! embossers and braille displays read directly. The same text is also available as Unicode
//! braille for software that shows dot patterns. Translation itself is liblouis's job, run as the
//! `lou_translate` command behind a trait, so another translator can take its place.

use super::ExportError;
use crate::confidence::flags_for;
use crate::model::{Block, BlockKind, Document, ListItem, Run, join_runs};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

/// Which braille code to produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrailleCode {
    /// Unified English Braille, uncontracted: every letter spelt out, easiest for new readers.
    UebGrade1,
    /// Unified English Braille, contracted: the everyday code for fluent readers.
    UebGrade2,
}

impl BrailleCode {
    /// The liblouis tables for ordinary text.
    pub fn text_tables(self) -> &'static str {
        match self {
            BrailleCode::UebGrade1 => "en-ueb-g1.ctb",
            BrailleCode::UebGrade2 => "en-ueb-g2.ctb",
        }
    }

    /// The liblouis tables for equations: uncontracted with the UEB technical material rules.
    pub fn maths_tables(self) -> &'static str {
        "en-ueb-g1.ctb,en-ueb-math.ctb"
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrailleOptions {
    pub code: BrailleCode,
    /// Cells per line. Embossers and most displays take 40.
    pub cells_per_line: usize,
    /// Lines per page. 25 is the embosser standard; 0 means no page breaks.
    pub lines_per_page: usize,
    /// Braille page numbers at the top right of every page after the first, which take one of
    /// the page's lines. Off by default so a page holds the full line count.
    pub page_numbers: bool,
}

impl BrailleOptions {
    /// Refuses sizes no braille device uses, instead of quietly changing them.
    pub fn validate(&self) -> Result<(), ExportError> {
        if !(10..=255).contains(&self.cells_per_line) {
            return Err(ExportError::Braille(format!(
                "{} cells per line is outside the 10 to 255 a display or embosser takes",
                self.cells_per_line
            )));
        }
        let least = if self.page_numbers { 4 } else { 3 };
        if self.lines_per_page != 0 && self.lines_per_page < least {
            return Err(ExportError::Braille(format!(
                "{} lines per page is too few; use 0 for no page breaks or at least {least}",
                self.lines_per_page
            )));
        }
        Ok(())
    }
}

impl Default for BrailleOptions {
    fn default() -> Self {
        BrailleOptions {
            code: BrailleCode::UebGrade2,
            cells_per_line: 40,
            lines_per_page: 25,
            page_numbers: false,
        }
    }
}

/// Turns text into ASCII braille (North American Braille Computer Code, as BRF files use).
pub trait BrailleTranslator {
    fn name(&self) -> String;
    /// Translates one piece of text with the given comma-separated liblouis table list.
    fn translate(&self, text: &str, tables: &str) -> Result<String, ExportError>;
}

/// Wraps a translator so that nothing it is given or returns can corrupt a line. Whitespace in
/// the text, such as a newline inside a run, becomes one space before translation. The cells
/// that come back keep only printable ASCII, because a BRF file is ASCII and a tab or a stray
/// character would shift every cell after it.
struct Tidy<'a> {
    inner: &'a dyn BrailleTranslator,
    /// Characters that could not be shown and were replaced by a question mark.
    replaced: std::cell::Cell<usize>,
}

impl BrailleTranslator for Tidy<'_> {
    fn name(&self) -> String {
        self.inner.name()
    }

    fn translate(&self, text: &str, tables: &str) -> Result<String, ExportError> {
        let single_spaced = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if single_spaced.is_empty() {
            return Ok(String::new());
        }
        let cells = self.inner.translate(&single_spaced, tables)?;
        Ok(cells
            .split_whitespace()
            .map(|word| word.chars().map(|c| self.ascii_cell(c)).collect::<String>())
            .collect::<Vec<_>>()
            .join(" "))
    }
}

impl Tidy<'_> {
    /// One cell as printable ASCII. A translator that returns dot patterns is read back to
    /// ASCII braille; anything else that cannot be shown is counted and marked with a question
    /// mark, so the loss is reported rather than silent.
    fn ascii_cell(&self, c: char) -> char {
        if c.is_ascii_graphic() {
            return c;
        }
        if ('\u{2800}'..='\u{283f}').contains(&c) {
            return ASCII_BRAILLE[(c as u32 - 0x2800) as usize] as char;
        }
        self.replaced.set(self.replaced.get() + 1);
        '?'
    }
}

/// liblouis, run as the `lou_translate` command.
#[derive(Debug, Clone)]
pub struct LibLouis {
    command: std::path::PathBuf,
}

impl LibLouis {
    /// Finds `lou_translate` on the path. Returns `None` when liblouis is not installed.
    pub fn detect() -> Option<Self> {
        let candidate = LibLouis {
            command: "lou_translate".into(),
        };
        let ok = Command::new(&candidate.command)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        ok.then_some(candidate)
    }
}

/// The display table that makes liblouis print North American Braille Computer Code, the
/// characters a BRF file holds. Without it dot 4 comes out as a backtick, which no embosser
/// reads as a braille cell.
const DISPLAY_TABLE: &str = "en-us-brf.dis";

fn with_display_table(tables: &str) -> String {
    if tables.contains(".dis") || tables.contains(".utb") {
        tables.to_string()
    } else {
        format!("{DISPLAY_TABLE},{tables}")
    }
}

impl LibLouis {
    /// Turns cells back into print with the same tables, to check a translation reads back as
    /// the text it came from.
    pub fn back_translate(&self, cells: &str, tables: &str) -> Result<String, ExportError> {
        use std::io::Write;
        let mut child = Command::new(&self.command)
            .args(["--backward", &with_display_table(tables)])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                ExportError::Braille(format!("running {}: {e}", self.command.display()))
            })?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(cells.as_bytes())?;
            stdin.write_all(b"\n")?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(ExportError::Braille(format!(
                "lou_translate failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end_matches(['\n', '\r'])
            .to_string())
    }
}

impl BrailleTranslator for LibLouis {
    fn name(&self) -> String {
        "liblouis".into()
    }

    fn translate(&self, text: &str, tables: &str) -> Result<String, ExportError> {
        use std::io::Write;
        let mut child = Command::new(&self.command)
            .arg(with_display_table(tables))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                ExportError::Braille(format!("running {}: {e}", self.command.display()))
            })?;
        // a translator that rejects its tables exits before reading the text, so the write can fail
        // with a broken pipe. That is not the error to report: its exit status and stderr are
        let mut closed_early = false;
        if let Some(mut stdin) = child.stdin.take() {
            let written = stdin
                .write_all(text.as_bytes())
                .and_then(|()| stdin.write_all(b"\n"));
            match written {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => closed_early = true,
                Err(e) => return Err(e.into()),
            }
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ExportError::Braille(format!(
                "lou_translate failed: {}",
                stderr.trim()
            )));
        }
        if closed_early {
            return Err(ExportError::Braille(
                "lou_translate stopped reading before the text was sent".into(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end_matches(['\n', '\r'])
            .to_string())
    }
}

/// The 64 ASCII braille characters in dot order, so index `n` holds the cell whose dots are the
/// set bits of `n` (dot 1 is bit 0 and dot 6 is bit 5).
const ASCII_BRAILLE: &[u8; 64] =
    b" A1B'K2L@CIF/MSP\"E3H9O6R^DJG>NTQ,*5<-U8V.%[$+X!&;:4\\0Z7(_?W]#Y)=";

/// Converts ASCII braille to Unicode braille patterns. Other characters pass through unchanged.
pub fn ascii_to_unicode(ascii: &str) -> String {
    ascii
        .chars()
        .map(|c| {
            let upper = c.to_ascii_uppercase();
            match ASCII_BRAILLE.iter().position(|&b| b as char == upper) {
                Some(dots) if c.is_ascii() => char::from_u32(0x2800 + dots as u32).unwrap_or(c),
                _ => c,
            }
        })
        .collect()
}

/// UEB transcriber's note indicators in ASCII braille, which frame text the transcriber added.
const NOTE_OPEN: &str = "@.<";
const NOTE_CLOSE: &str = "@.>";

/// A document as a BRF file.
pub fn to_brf(
    document: &Document,
    translator: &dyn BrailleTranslator,
    options: &BrailleOptions,
) -> Result<String, ExportError> {
    Ok(to_brf_with_report(document, translator, options)?.text)
}

/// A BRF file and what was lost making it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brf {
    pub text: String,
    /// Characters the translator returned that braille cannot show, each marked with a question
    /// mark. Zero for ordinary text. A caller should tell the person when it is not.
    pub replaced_characters: usize,
}

/// [`to_brf`] with the count of characters that could not be shown.
pub fn to_brf_with_report(
    document: &Document,
    translator: &dyn BrailleTranslator,
    options: &BrailleOptions,
) -> Result<Brf, ExportError> {
    let (lines, keep, replaced_characters) = layout(document, translator, options)?;
    Ok(Brf {
        text: paginate(&lines, &keep, options),
        replaced_characters,
    })
}

pub fn write_brf(
    document: &Document,
    translator: &dyn BrailleTranslator,
    options: &BrailleOptions,
    path: &Path,
) -> Result<(), ExportError> {
    Ok(std::fs::write(
        path,
        to_brf(document, translator, options)?,
    )?)
}

/// A document as Unicode braille, one line per braille line, for displays and screen readers.
pub fn to_unicode_braille(
    document: &Document,
    translator: &dyn BrailleTranslator,
    options: &BrailleOptions,
) -> Result<String, ExportError> {
    let lines = braille_lines(document, translator, options)?;
    Ok(lines
        .iter()
        .map(|l| ascii_to_unicode(l))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n")
}

/// The formatted braille lines of a document, before pagination.
///
/// Formatting follows the common conventions: a title centred, headings at cell 5 and cell 7,
/// paragraphs starting at cell 3 with runovers at cell 1, list items at cell 1 with runovers at
/// cell 3 and tables listed row by row. Blank lines separate blocks only where braille readers
/// expect them.
pub fn braille_lines(
    document: &Document,
    translator: &dyn BrailleTranslator,
    options: &BrailleOptions,
) -> Result<Vec<String>, ExportError> {
    Ok(layout(document, translator, options)?.0)
}

/// The lines, plus the indexes of those that must stay on a page with the line after them, plus
/// how many characters could not be shown. The indexes are the title, every line of every heading
/// and the blank after one, so none is left alone at the foot of a page.
fn layout(
    document: &Document,
    translator: &dyn BrailleTranslator,
    options: &BrailleOptions,
) -> Result<(Vec<String>, BTreeSet<usize>, usize), ExportError> {
    options.validate()?;
    let tidy = Tidy {
        inner: translator,
        replaced: std::cell::Cell::new(0),
    };
    let translator: &dyn BrailleTranslator = &tidy;
    let width = options.cells_per_line;
    let mut keep: BTreeSet<usize> = BTreeSet::new();
    let mut out: Vec<String> = Vec::new();
    let mut unclear = None;
    let mut unclear_word = |t: &dyn BrailleTranslator| -> Result<String, ExportError> {
        if unclear.is_none() {
            unclear = Some(t.translate("unclear", options.code.text_tables())?);
        }
        Ok(unclear.clone().unwrap_or_default())
    };

    let title = translator.translate(&document.title, options.code.text_tables())?;
    out.extend(centre(&title, width));
    // an untitled document starts straight in on its first block
    if !out.is_empty() {
        out.push(String::new());
    }
    keep.extend(0..out.len());

    for block in &document.blocks {
        match &block.kind {
            BlockKind::Heading { level, runs } => {
                let text = run_text(runs, translator, options, &mut unclear_word)?;
                if !out.last().is_some_and(String::is_empty) {
                    out.push(String::new());
                }
                // cell 5 for a major heading, cell 7 for a minor one
                let indent = if *level <= 1 { 4 } else { 6 };
                let first = out.len();
                out.extend(wrap(&text, width, indent, indent));
                keep.extend(first..out.len());
            }
            BlockKind::Paragraph { runs, .. } => {
                let text = run_text(runs, translator, options, &mut unclear_word)?;
                out.extend(wrap(&text, width, 2, 0));
            }
            BlockKind::List { ordered, items } => {
                list_lines(
                    &mut out,
                    items,
                    *ordered,
                    0,
                    width,
                    translator,
                    options,
                    &mut unclear_word,
                )?;
            }
            BlockKind::Table { rows, caption } => {
                if !caption.is_empty() {
                    let text = run_text(caption, translator, options, &mut unclear_word)?;
                    out.extend(wrap(&text, width, 2, 0));
                }
                table_lines(
                    &mut out,
                    block,
                    rows,
                    width,
                    translator,
                    options,
                    &mut unclear_word,
                )?;
            }
            BlockKind::Image { alt_text, .. } => {
                let described = alt_text.clone().unwrap_or_else(|| {
                    format!(
                        "image on page {} without a description yet",
                        block.origin.page
                    )
                });
                let text = translator.translate(&described, options.code.text_tables())?;
                out.extend(wrap(&format!("{NOTE_OPEN}{text}{NOTE_CLOSE}"), width, 2, 0));
            }
            BlockKind::Equation { runs, latex, .. } => {
                let source = latex.clone().unwrap_or_else(|| join_runs(runs));
                let readable = crate::maths::speech::speak(&source);
                // technical material in uncontracted braille, then the spoken form as a note for
                // readers who have not met the technical code
                let maths =
                    translator.translate(&plain_equation(&source), options.code.maths_tables())?;
                out.extend(wrap(&maths, width, 2, 4));
                let spoken = translator.translate(&readable, options.code.text_tables())?;
                out.extend(wrap(
                    &format!("{NOTE_OPEN}{spoken}{NOTE_CLOSE}"),
                    width,
                    4,
                    4,
                ));
            }
            BlockKind::PageBreak => {
                if !out.last().is_some_and(String::is_empty) {
                    out.push(String::new());
                    // a blank after a heading goes with it. Otherwise the heading would be left alone
                    if keep.contains(&(out.len() - 2)) {
                        keep.insert(out.len() - 1);
                    }
                }
            }
        }
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    keep.retain(|&i| i < out.len());
    Ok((out, keep, tidy.replaced.get()))
}

/// An equation's source with LaTeX braces removed, as a braille reader expects to see it.
fn plain_equation(source: &str) -> String {
    let flat = flatten_fractions(source);
    flat.replace("_{", "_")
        .replace("^{", "^")
        .replace(['{', '}'], "")
        .replace("\\times", "\u{d7}")
}

/// Turns each `\frac{a}{b}` into `(a)/(b)`. Parts of a single character each give `a/b`, so
/// the structure of a fraction survives the loss of its braces. Anything else in the source is
/// left as it is; the spoken form beside the equation carries its meaning.
fn flatten_fractions(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    while let Some(at) = rest.find("\\frac") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 5..];
        match (
            braced(after),
            braced(after).and_then(|(_, tail)| braced(tail)),
        ) {
            (Some((top, _)), Some((bottom, tail))) => {
                let (top, bottom) = (flatten_fractions(top), flatten_fractions(bottom));
                if top.chars().count() == 1 && bottom.chars().count() == 1 {
                    out.push_str(&format!("{top}/{bottom}"));
                } else {
                    out.push_str(&format!("({top})/({bottom})"));
                }
                rest = tail;
            }
            _ => {
                out.push_str("\\frac");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The text inside the braces that open the string, with what follows the closing brace.
fn braced(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    let inner = text.strip_prefix('{')?;
    let mut depth = 1usize;
    for (i, c) in inner.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&inner[..i], &inner[i + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

/// Translates runs, framing any run that needs checking in a transcriber's note.
fn run_text(
    runs: &[Run],
    translator: &dyn BrailleTranslator,
    options: &BrailleOptions,
    unclear_word: &mut dyn FnMut(&dyn BrailleTranslator) -> Result<String, ExportError>,
) -> Result<String, ExportError> {
    let refs: Vec<&Run> = runs.iter().collect();
    let flags = flags_for(&refs);
    if !flags.iter().any(|f| *f) {
        return translator.translate(&join_runs(runs), options.code.text_tables());
    }
    let mut out = String::new();
    let mut plain = String::new();
    let flush = |plain: &mut String, out: &mut String| -> Result<(), ExportError> {
        if !plain.trim().is_empty() {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            out.push_str(&translator.translate(plain.trim(), options.code.text_tables())?);
        }
        plain.clear();
        Ok(())
    };
    for (run, flagged) in runs.iter().zip(flags) {
        if flagged {
            flush(&mut plain, &mut out)?;
            let word = run.text.trim();
            let cells = translator.translate(word, options.code.text_tables())?;
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&format!(
                "{NOTE_OPEN}{}{NOTE_CLOSE} {cells}",
                unclear_word(translator)?
            ));
        } else {
            if !plain.is_empty() && !plain.ends_with(' ') && !run.text.starts_with(' ') {
                plain.push(' ');
            }
            plain.push_str(&run.text);
        }
    }
    flush(&mut plain, &mut out)?;
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn list_lines(
    out: &mut Vec<String>,
    items: &[ListItem],
    ordered: bool,
    depth: usize,
    width: usize,
    translator: &dyn BrailleTranslator,
    options: &BrailleOptions,
    unclear_word: &mut dyn FnMut(&dyn BrailleTranslator) -> Result<String, ExportError>,
) -> Result<(), ExportError> {
    for (i, item) in items.iter().enumerate() {
        let text = run_text(&item.runs, translator, options, unclear_word)?;
        let marker = if ordered {
            translator.translate(&format!("{}.", i + 1), options.code.text_tables())? + " "
        } else {
            String::new()
        };
        // 1-3 format: the item starts at cell 1 and runs over at cell 3, deeper levels step in by 2
        let first = depth * 2;
        out.extend(wrap(&format!("{marker}{text}"), width, first, first + 2));
        if !item.children.is_empty() {
            list_lines(
                out,
                &item.children,
                ordered,
                depth + 1,
                width,
                translator,
                options,
                unclear_word,
            )?;
        }
    }
    Ok(())
}

/// A table listed row by row: each row on its own with its cells joined by a semicolon. The header
/// names repeated before each value so a row makes sense on its own.
#[allow(clippy::too_many_arguments)]
fn table_lines(
    out: &mut Vec<String>,
    block: &Block,
    rows: &[crate::model::TableRow],
    width: usize,
    translator: &dyn BrailleTranslator,
    options: &BrailleOptions,
    unclear_word: &mut dyn FnMut(&dyn BrailleTranslator) -> Result<String, ExportError>,
) -> Result<(), ExportError> {
    let header: Option<Vec<String>> = rows
        .first()
        .filter(|r| r.cells.iter().all(|c| c.header))
        .map(|r| r.cells.iter().map(|c| join_runs(&c.runs)).collect());
    let body = if header.is_some() { &rows[1..] } else { rows };
    let note = translator.translate(
        &format!("table with {} rows, listed", body.len()),
        options.code.text_tables(),
    )?;
    out.extend(wrap(&format!("{NOTE_OPEN}{note}{NOTE_CLOSE}"), width, 2, 0));
    // the separator is translated like any other punctuation, not inserted as print
    let separator = format!(
        "{} ",
        translator.translate(";", options.code.text_tables())?
    );
    for row in body {
        let mut parts = Vec::new();
        for (i, cell) in row.cells.iter().enumerate() {
            let value = run_text(&cell.runs, translator, options, unclear_word)?;
            match header.as_ref().and_then(|h| h.get(i)) {
                Some(name) => {
                    let name =
                        translator.translate(&format!("{name}:"), options.code.text_tables())?;
                    parts.push(format!("{name} {value}"));
                }
                None => parts.push(value),
            }
        }
        out.extend(wrap(&parts.join(&separator), width, 0, 2));
    }
    let _ = block;
    Ok(())
}

/// Wraps braille cells at spaces, with the first line and runover lines indented as given.
fn wrap(cells: &str, width: usize, first_indent: usize, runover_indent: usize) -> Vec<String> {
    // indents never take more than half a line, so a deep list on a narrow display still fits
    let (first_indent, runover_indent) =
        (first_indent.min(width / 2), runover_indent.min(width / 2));
    let mut lines = Vec::new();
    let mut line = " ".repeat(first_indent);
    let mut line_has_text = false;
    for word in cells.split(' ').filter(|w| !w.is_empty()) {
        let indent = if lines.is_empty() {
            first_indent
        } else {
            runover_indent
        };
        let needed = if line_has_text {
            line.chars().count() + 1 + word.chars().count()
        } else {
            indent + word.chars().count()
        };
        if line_has_text && needed > width {
            lines.push(line);
            line = " ".repeat(runover_indent);
            line_has_text = false;
        }
        if line_has_text {
            line.push(' ');
        }
        // a single word longer than a line is broken at the margin
        let mut word = word;
        while indent_len(&line) + word.chars().count() > width {
            let room = width.saturating_sub(indent_len(&line)).max(1);
            let split = word
                .char_indices()
                .nth(room)
                .map(|(i, _)| i)
                .unwrap_or(word.len());
            line.push_str(&word[..split]);
            lines.push(line);
            line = " ".repeat(runover_indent);
            word = &word[split..];
        }
        line.push_str(word);
        line_has_text = true;
    }
    if line_has_text {
        lines.push(line);
    }
    lines
}

fn indent_len(line: &str) -> usize {
    line.chars().count()
}

fn centre(cells: &str, width: usize) -> Vec<String> {
    wrap(cells, width, 0, 0)
        .into_iter()
        .map(|l| {
            let text = l.trim();
            let pad = width.saturating_sub(text.chars().count()) / 2;
            format!("{}{}", " ".repeat(pad), text)
        })
        .collect()
}

/// The braille page number: the numeric indicator, then the digits as the letters a to j.
fn page_number(number: usize) -> String {
    let digits: String = number
        .to_string()
        .chars()
        .map(|d| match d {
            '0' => 'J',
            other => (b'A' + (other as u8 - b'1')) as char,
        })
        .collect();
    format!("#{digits}")
}

/// Joins lines into pages with CRLF line endings and a form feed between pages, as BRF requires.
/// A page never opens with a blank line. A line marked to stay with the next one is carried to
/// the next page rather than left at the foot of this one. With page numbers on, every page
/// after the first starts with its number at the right margin.
fn paginate(lines: &[String], keep: &BTreeSet<usize>, options: &BrailleOptions) -> String {
    let per_page = options.lines_per_page;
    if per_page == 0 {
        return lines
            .iter()
            .skip_while(|l| l.trim().is_empty())
            .map(|l| format!("{}\r\n", l.trim_end()))
            .collect();
    }
    let header = usize::from(options.page_numbers);
    let mut pages: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let capacity = per_page - if pages.is_empty() { 0 } else { header };
        if current.len() >= capacity {
            let mut carried: Vec<usize> = Vec::new();
            while current.len() > 1
                && carried.len() + 1 < capacity
                && current.last().is_some_and(|last| keep.contains(last))
            {
                carried.insert(0, current.pop().unwrap_or_default());
            }
            pages.push(std::mem::replace(&mut current, carried));
        }
        if current.is_empty() && line.trim().is_empty() {
            continue;
        }
        current.push(i);
    }
    if !current.is_empty() {
        pages.push(current);
    }
    let mut out = String::new();
    for (number, page) in pages.iter().enumerate() {
        if number > 0 {
            out.push('\u{c}');
            if options.page_numbers {
                let label = page_number(number + 1);
                let pad = options.cells_per_line.saturating_sub(label.len());
                out.push_str(&format!("{}{label}\r\n", " ".repeat(pad)));
            }
        }
        for &i in page {
            out.push_str(lines[i].trim_end());
            out.push_str("\r\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlockKind, Confidence, Origin, Rect, Source, SourceFormat};

    /// Stands in for liblouis: upper-cases text so the output is recognisable in assertions.
    struct Fake;
    impl BrailleTranslator for Fake {
        fn name(&self) -> String {
            "fake".into()
        }
        fn translate(&self, text: &str, _tables: &str) -> Result<String, ExportError> {
            Ok(text.to_ascii_uppercase())
        }
    }

    fn origin() -> Origin {
        Origin::new(1, Rect::new(0.0, 0.0, 10.0, 10.0))
    }

    fn doc() -> Document {
        Document::from_json(include_str!("../../../../docs/examples/document.json")).unwrap()
    }

    #[test]
    fn ascii_braille_maps_to_unicode_patterns() {
        assert_eq!(ascii_to_unicode("A"), "\u{2801}");
        assert_eq!(ascii_to_unicode("B"), "\u{2803}");
        assert_eq!(ascii_to_unicode(" "), "\u{2800}");
        assert_eq!(ascii_to_unicode("="), "\u{283f}");
        assert_eq!(
            ascii_to_unicode("!"),
            "\u{282e}",
            "the UEB contraction for the"
        );
        assert_eq!(
            ascii_to_unicode("a"),
            "\u{2801}",
            "lower case reads as the same cell"
        );
    }

    #[test]
    fn lines_wrap_at_the_cell_width_with_the_right_indents() {
        let text = "ONE TWO THREE FOUR FIVE SIX SEVEN EIGHT NINE TEN";
        let lines = wrap(text, 20, 2, 0);
        assert_eq!(
            lines,
            ["  ONE TWO THREE FOUR", "FIVE SIX SEVEN EIGHT", "NINE TEN"]
        );
        assert!(lines.iter().all(|l| l.chars().count() <= 20));
        assert_eq!(
            wrap("ABCDEFGHIJKLMNOPQRSTUVWXYZ", 10, 0, 2),
            ["ABCDEFGHIJ", "  KLMNOPQR", "  STUVWXYZ"]
        );
        assert_eq!(centre("TITLE", 11), ["   TITLE"]);
    }

    #[test]
    fn the_example_document_becomes_formatted_brf() {
        let brf = to_brf(&doc(), &Fake, &BrailleOptions::default()).unwrap();
        let lines: Vec<&str> = brf.split("\r\n").collect();
        assert_eq!(lines[0].trim(), "SIGNALS WEEK 3");
        assert!(
            lines[0].starts_with("             "),
            "the title is centred"
        );
        assert!(
            lines.contains(&"    FOURIER SERIES"),
            "a major heading sits at cell 5: {lines:?}"
        );
        assert!(
            lines.contains(&"  THE PERIOD IS 2.6 MS."),
            "a paragraph starts at cell 3"
        );
        assert!(lines.contains(&"1. SAMPLE"));
        assert!(
            lines.contains(&"  1. WINDOW FIRST"),
            "a nested item steps in"
        );
        assert!(
            lines.iter().any(|l| l.starts_with("F (HZ): 50; GAIN: 0.7")),
            "a table row lists its headers: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("@.<IMAGE ON PAGE 2")),
            "an undescribed image says so in a note"
        );
        assert!(lines.contains(&"  V = IR"));
        assert!(
            lines.iter().any(|l| l.contains("@.<V EQUALS I R@.>")),
            "the spoken form follows the equation"
        );
        assert!(lines.iter().all(|l| l.chars().count() <= 40));
        assert!(!brf.contains('\u{c}'), "a short document fits one page");
    }

    #[test]
    fn unclear_text_is_framed_in_a_transcribers_note() {
        let mut d = Document::new(
            "t",
            Source {
                file_name: "t.pdf".into(),
                format: SourceFormat::Pdf,
            },
        );
        d.push(
            BlockKind::paragraph(vec![
                Run::native("Supply ", origin()),
                Run::recognised("12", Confidence::new(0.5), origin()),
                Run::native(" volts", origin()),
            ]),
            origin(),
        );
        let lines = braille_lines(&d, &Fake, &BrailleOptions::default()).unwrap();
        assert!(
            lines.contains(&"  SUPPLY @.<UNCLEAR@.> 12 VOLTS".to_string()),
            "{lines:?}"
        );
    }

    fn opts(lines_per_page: usize, page_numbers: bool) -> BrailleOptions {
        BrailleOptions {
            lines_per_page,
            page_numbers,
            ..BrailleOptions::default()
        }
    }

    #[test]
    fn pages_break_every_twenty_five_lines() {
        let lines: Vec<String> = (0..60).map(|i| format!("L{i}")).collect();
        let none = BTreeSet::new();
        let brf = paginate(&lines, &none, &opts(25, false));
        assert_eq!(brf.matches('\u{c}').count(), 2);
        assert!(brf.starts_with("L0\r\nL1\r\n"));
        assert!(brf.contains("L24\r\n\u{c}L25\r\n"));
        assert_eq!(
            paginate(&lines, &none, &opts(0, false))
                .matches('\u{c}')
                .count(),
            0
        );
    }

    #[test]
    fn a_page_never_opens_with_a_blank_and_a_heading_is_not_left_alone_at_the_foot() {
        // lines 0..=22 are text, 23 is blank, 24 is a heading that must stay with line 25
        let mut lines: Vec<String> = (0..23).map(|i| format!("L{i}")).collect();
        lines.push(String::new());
        lines.push("HEADING".into());
        lines.extend((25..40).map(|i| format!("L{i}")));
        let keep: BTreeSet<usize> = [24].into();
        let brf = paginate(&lines, &keep, &opts(25, false));
        let pages: Vec<&str> = brf.split('\u{c}').collect();
        assert_eq!(pages.len(), 2);
        assert!(!pages[0].contains("HEADING"), "the heading moved on");
        assert!(pages[1].starts_with("HEADING\r\nL25"), "{:?}", pages[1]);

        // a blank that would fall at the top of a page is dropped
        let mut lines: Vec<String> = (0..25).map(|i| format!("L{i}")).collect();
        lines.push(String::new());
        lines.push("NEXT".into());
        let brf = paginate(&lines, &BTreeSet::new(), &opts(25, false));
        assert!(brf.contains("\u{c}NEXT\r\n"), "{brf:?}");
    }

    #[test]
    fn page_numbers_sit_at_the_top_right_of_every_page_after_the_first() {
        let lines: Vec<String> = (0..70).map(|i| format!("L{i}")).collect();
        let options = BrailleOptions {
            cells_per_line: 20,
            ..opts(25, true)
        };
        let brf = paginate(&lines, &BTreeSet::new(), &options);
        let pages: Vec<&str> = brf.split('\u{c}').collect();
        assert_eq!(pages.len(), 3);
        assert!(pages[0].starts_with("L0\r\n") && pages[0].lines().count() == 25);
        assert!(
            pages[1].starts_with(&format!("{}#B\r\n", " ".repeat(18))),
            "{:?}",
            &pages[1][..30]
        );
        assert_eq!(
            pages[1].lines().count(),
            25,
            "the number takes one of the page's lines"
        );
        assert!(pages[2].contains("#C\r\n"));
        assert_eq!(page_number(10), "#AJ");
        assert_eq!(page_number(7), "#G");
    }

    #[test]
    fn settings_no_device_uses_are_refused() {
        assert!(BrailleOptions::default().validate().is_ok());
        assert!(
            BrailleOptions {
                cells_per_line: 5,
                ..BrailleOptions::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            BrailleOptions {
                cells_per_line: 300,
                ..BrailleOptions::default()
            }
            .validate()
            .is_err()
        );
        assert!(opts(2, false).validate().is_err());
        assert!(opts(3, true).validate().is_err());
        assert!(opts(0, true).validate().is_ok());
        assert!(
            to_brf(
                &doc(),
                &Fake,
                &BrailleOptions {
                    cells_per_line: 4,
                    ..BrailleOptions::default()
                }
            )
            .is_err()
        );
    }

    /// Returns what a real translator can: tabs, newlines and characters outside ASCII.
    struct Messy;
    impl BrailleTranslator for Messy {
        fn name(&self) -> String {
            "messy".into()
        }
        fn translate(&self, text: &str, _tables: &str) -> Result<String, ExportError> {
            Ok(format!("{}\t\u{2603}\nEND", text.to_ascii_uppercase()))
        }
    }

    #[test]
    fn text_with_newlines_and_a_translator_with_stray_characters_cannot_corrupt_the_file() {
        let mut d = Document::new(
            "Two\nlines",
            Source {
                file_name: "t.pdf".into(),
                format: SourceFormat::Pdf,
            },
        );
        d.push(
            BlockKind::paragraph(vec![Run::native("first line\nsecond\tline", origin())]),
            origin(),
        );
        for translator in [&Fake as &dyn BrailleTranslator, &Messy] {
            let brf = to_brf(&d, translator, &BrailleOptions::default()).unwrap();
            for ch in brf.chars() {
                assert!(
                    ch == '\r' || ch == '\n' || ch == '\u{c}' || (' '..='~').contains(&ch),
                    "{ch:?} in {brf:?}"
                );
            }
            assert!(brf.lines().all(|l| l.chars().count() <= 40));
            assert!(!brf.contains('\t'));
        }
        let lines = braille_lines(&d, &Fake, &BrailleOptions::default()).unwrap();
        assert!(
            lines.contains(&"  FIRST LINE SECOND LINE".to_string()),
            "{lines:?}"
        );
    }

    // a translator that rejects its tables exits without reading any text. The text is large so the
    // write meets the closed pipe every time, which is the case that used to come back as an I/O error
    #[cfg(unix)]
    #[test]
    fn a_translator_that_exits_without_reading_reports_its_own_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("zacess-louis-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("lou_translate");
        std::fs::write(&script, "#!/bin/sh\necho 'no such table' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let louis = LibLouis {
            command: script.clone(),
        };
        let text = "word ".repeat(1_000_000);
        let result = louis.translate(&text, "no-such-table.ctb");
        std::fs::remove_dir_all(&dir).ok();
        match result {
            Err(ExportError::Braille(message)) => assert!(message.contains("no such table")),
            other => panic!("expected the translator's own error, got {other:?}"),
        }
    }

    #[test]
    fn liblouis_contracts_english_when_it_is_installed() {
        let Some(louis) = LibLouis::detect() else {
            eprintln!("skipped: liblouis (lou_translate) is not installed");
            return;
        };
        let cells = louis
            .translate("the period", BrailleCode::UebGrade2.text_tables())
            .unwrap();
        assert!(
            cells.starts_with("! "),
            "the is one cell in grade 2: {cells}"
        );
        let uncontracted = louis
            .translate("the period", BrailleCode::UebGrade1.text_tables())
            .unwrap();
        assert!(
            uncontracted.to_ascii_lowercase().starts_with("the"),
            "{uncontracted}"
        );
        let maths = louis
            .translate("x^2 = 4", BrailleCode::UebGrade2.maths_tables())
            .unwrap();
        assert!(!maths.is_empty());
        assert!(matches!(
            louis.translate("x", "no-such-table.ctb"),
            Err(ExportError::Braille(_))
        ));

        let brf = to_brf(&doc(), &louis, &BrailleOptions::default()).unwrap();
        assert!(
            brf.lines()
                .all(|l| l.trim_end_matches('\u{c}').chars().count() <= 40)
        );
        let unicode = to_unicode_braille(&doc(), &louis, &BrailleOptions::default()).unwrap();
        assert!(
            unicode
                .chars()
                .filter(|c| ('\u{2800}'..='\u{28ff}').contains(c))
                .count()
                > 50
        );
    }

    /// Translations that follow from the UEB rules, checked against the liblouis tables. The
    /// expected cells are written out here so that a change in a table is noticed. So is a change
    /// in how the module calls it.
    const GRADE_2: &[(&str, &str)] = &[
        // the strong contractions that are one cell each
        ("the", "!"),
        ("and", "&"),
        ("for", "="),
        ("with", ")"),
        ("of", "("),
        // alphabetic wordsigns
        ("knowledge", "K"),
        ("children", "*N"),
        ("braille", "BRL"),
        // lower groupsigns and two-cell contractions
        ("ing", "9G"),
        ("mother", "\"M"),
        ("through", "\"?"),
        // capitals, numbers and a sentence
        ("ABC", ",,ABC"),
        ("12", "#AB"),
        ("Hello World", ",HELLO ,_W"),
    ];

    const GRADE_1: &[(&str, &str)] = &[
        ("the", "THE"),
        ("braille", "BRAILLE"),
        ("Hello World", ",HELLO ,WORLD"),
        ("ABC", ",,ABC"),
        ("12", "#AB"),
        ("2.6 ms", "#B4F MS"),
    ];

    #[test]
    fn known_ueb_translations_come_out_as_the_tables_say() {
        let Some(louis) = LibLouis::detect() else {
            eprintln!("skipped: liblouis (lou_translate) is not installed");
            return;
        };
        for (print, cells) in GRADE_2 {
            let got = louis
                .translate(print, BrailleCode::UebGrade2.text_tables())
                .unwrap();
            assert_eq!(&got, cells, "grade 2 for {print:?}");
        }
        for (print, cells) in GRADE_1 {
            let got = louis
                .translate(print, BrailleCode::UebGrade1.text_tables())
                .unwrap();
            assert_eq!(&got, cells, "grade 1 for {print:?}");
        }
    }

    #[test]
    fn translations_read_back_as_the_text_they_came_from() {
        let Some(louis) = LibLouis::detect() else {
            eprintln!("skipped: liblouis (lou_translate) is not installed");
            return;
        };
        let sentences = [
            "The quick brown fox jumps over the lazy dog.",
            "A square wave has only odd harmonics.",
            "Supply 12 volts to the board.",
        ];
        for code in [BrailleCode::UebGrade1, BrailleCode::UebGrade2] {
            for sentence in sentences {
                let cells = louis.translate(sentence, code.text_tables()).unwrap();
                let back = louis.back_translate(&cells, code.text_tables()).unwrap();
                assert_eq!(back, sentence, "{code:?} through {cells:?}");
            }
        }
        // grade 2 is shorter than grade 1 for ordinary prose, which is why readers want it
        let text = sentences[0];
        let g1 = louis
            .translate(text, BrailleCode::UebGrade1.text_tables())
            .unwrap();
        let g2 = louis
            .translate(text, BrailleCode::UebGrade2.text_tables())
            .unwrap();
        assert!(g2.len() < g1.len(), "{g2} vs {g1}");
    }

    #[test]
    fn a_real_translation_fits_every_width_a_device_uses() {
        let Some(louis) = LibLouis::detect() else {
            eprintln!("skipped: liblouis (lou_translate) is not installed");
            return;
        };
        for cells in [20usize, 32, 40, 80] {
            for code in [BrailleCode::UebGrade1, BrailleCode::UebGrade2] {
                for pages in [0usize, 10, 25] {
                    let options = BrailleOptions {
                        code,
                        cells_per_line: cells,
                        lines_per_page: pages,
                        page_numbers: pages > 0,
                    };
                    let brf = to_brf(&doc(), &louis, &options).unwrap();
                    for page in brf.split('\u{c}') {
                        let lines: Vec<&str> =
                            page.split("\r\n").filter(|l| !l.is_empty()).collect();
                        assert!(
                            pages == 0 || lines.len() <= pages,
                            "{cells}/{pages}: {} lines",
                            lines.len()
                        );
                        assert!(
                            lines.iter().all(|l| l.chars().count() <= cells),
                            "{cells}/{pages}"
                        );
                        assert!(
                            page.chars()
                                .all(|c| c == '\r' || c == '\n' || (' '..='~').contains(&c))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn headings_stay_whole_and_with_what_follows_even_across_a_page_edge() {
        // a heading that wraps to two lines, then a page break block, then another heading
        let mut lines: Vec<String> = (0..23).map(|i| format!("L{i}")).collect();
        lines.extend(["", "ALPHA BETA", "GAMMA", "", "SECOND", "TEXT"].map(String::from));
        let keep: BTreeSet<usize> = [24, 25, 26, 27].into();
        let brf = paginate(&lines, &keep, &opts(25, false));
        let pages: Vec<&str> = brf.split('\u{c}').collect();
        assert!(
            pages[0].trim_end().ends_with("L22"),
            "the whole heading moved on: {:?}",
            pages[0]
        );
        assert!(
            pages[1].starts_with("ALPHA BETA\r\nGAMMA\r\n"),
            "{:?}",
            pages[1]
        );
        assert!(pages[1].contains("SECOND"));
    }

    #[test]
    fn an_untitled_document_and_an_empty_heading_leave_no_stray_blank_or_keep() {
        let mut d = Document::new(
            "",
            Source {
                file_name: "t.pdf".into(),
                format: SourceFormat::Pdf,
            },
        );
        d.push(
            BlockKind::paragraph(vec![Run::native("first", origin())]),
            origin(),
        );
        d.push(
            BlockKind::Heading {
                level: 1,
                runs: vec![Run::native("  ", origin())],
            },
            origin(),
        );
        d.push(
            BlockKind::paragraph(vec![Run::native("second", origin())]),
            origin(),
        );
        for per_page in [0, 25] {
            let brf = to_brf(&d, &Fake, &opts(per_page, false)).unwrap();
            assert!(brf.starts_with("  FIRST\r\n"), "{brf:?}");
        }
        let (_, keep, _) = layout(&d, &Fake, &BrailleOptions::default()).unwrap();
        assert!(keep.is_empty(), "an empty heading keeps nothing: {keep:?}");
    }

    #[test]
    fn deep_lists_and_long_words_fit_the_narrowest_display() {
        use crate::model::ListItem;
        let mut item = ListItem::new(vec![Run::native(
            "deepest item with a supercalifragilistic word",
            origin(),
        )]);
        for _ in 0..6 {
            item = ListItem {
                runs: vec![Run::native("level", origin())],
                children: vec![item],
            };
        }
        let mut d = Document::new(
            "Deep",
            Source {
                file_name: "t.pdf".into(),
                format: SourceFormat::Pdf,
            },
        );
        d.push(
            BlockKind::List {
                ordered: true,
                items: vec![item],
            },
            origin(),
        );
        for cells in [10, 12, 20, 40] {
            let options = BrailleOptions {
                cells_per_line: cells,
                ..BrailleOptions::default()
            };
            let lines = braille_lines(&d, &Fake, &options).unwrap();
            assert!(
                lines.iter().all(|l| l.chars().count() <= cells),
                "{cells}: {lines:?}"
            );
        }
    }

    /// Returns dot patterns, as a translator for a braille display might.
    struct DotPatterns;
    impl BrailleTranslator for DotPatterns {
        fn name(&self) -> String {
            "dots".into()
        }
        fn translate(&self, text: &str, _tables: &str) -> Result<String, ExportError> {
            let cells = if text.contains('\u{2603}') {
                "\u{2801}\u{2803}\u{28ff}"
            } else {
                "\u{2801}\u{2803}"
            };
            Ok(cells.to_string())
        }
    }

    #[test]
    fn dot_patterns_are_read_back_to_ascii_and_losses_are_counted() {
        let mut d = Document::new(
            "t",
            Source {
                file_name: "t.pdf".into(),
                format: SourceFormat::Pdf,
            },
        );
        d.push(
            BlockKind::paragraph(vec![Run::native("plain", origin())]),
            origin(),
        );
        let brf = to_brf_with_report(&d, &DotPatterns, &BrailleOptions::default()).unwrap();
        assert_eq!(brf.replaced_characters, 0);
        assert!(brf.text.contains("  AB"), "{:?}", brf.text);

        d.push(
            BlockKind::paragraph(vec![Run::native("snow \u{2603}", origin())]),
            origin(),
        );
        let brf = to_brf_with_report(&d, &DotPatterns, &BrailleOptions::default()).unwrap();
        assert_eq!(
            brf.replaced_characters, 1,
            "the eight-dot cell cannot be shown"
        );
        assert!(brf.text.contains("AB?"));
        // a translator that passes the snowman through unchanged loses it too
        assert_eq!(
            to_brf_with_report(&d, &Fake, &BrailleOptions::default())
                .unwrap()
                .replaced_characters,
            1
        );
    }

    #[test]
    fn tables_join_cells_with_a_translated_separator_and_count_only_data_rows() {
        use crate::model::{TableCell, TableRow};
        let cell = |text: &str, header: bool| TableCell {
            runs: vec![Run::native(text, origin())],
            header,
            col_span: 1,
            row_span: 1,
        };
        let mut d = Document::new(
            "t",
            Source {
                file_name: "t.pdf".into(),
                format: SourceFormat::Pdf,
            },
        );
        d.push(
            BlockKind::Table {
                rows: vec![
                    TableRow {
                        cells: vec![cell("name", true), cell("value", true)],
                    },
                    TableRow {
                        cells: vec![cell("a", false), cell("1", false)],
                    },
                    TableRow {
                        cells: vec![cell("b", false), cell("2", false)],
                    },
                ],
                caption: Vec::new(),
            },
            origin(),
        );
        let lines = braille_lines(&d, &Fake, &BrailleOptions::default()).unwrap();
        assert!(
            lines.iter().any(|l| l.contains("TABLE WITH 2 ROWS")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l == "NAME: A; VALUE: 1"), "{lines:?}");
    }

    #[test]
    fn fractions_keep_their_structure_when_the_braces_go() {
        assert_eq!(plain_equation("\\frac{1}{2}"), "1/2");
        assert_eq!(plain_equation("x = \\frac{a+b}{c}"), "x = (a+b)/(c)");
        assert_eq!(plain_equation("\\frac{V_{out}}{V_{in}}"), "(V_out)/(V_in)");
        assert_eq!(plain_equation("\\frac{\\frac{1}{2}}{3}"), "(1/2)/(3)");
        assert_eq!(plain_equation("a \\times b^{2}"), "a \u{d7} b^2");
        assert_eq!(
            plain_equation("\\frac{1"),
            "\\frac1",
            "an unfinished fraction is left alone"
        );
    }

    #[test]
    fn liblouis_prints_braille_computer_code_not_lowercase_look_alikes() {
        let Some(louis) = LibLouis::detect() else {
            eprintln!("skipped: liblouis (lou_translate) is not installed");
            return;
        };
        let cells = louis
            .translate("the @home", BrailleCode::UebGrade2.text_tables())
            .unwrap();
        assert!(
            !cells.contains('`'),
            "dot 4 must be @, not a backtick: {cells}"
        );
        assert!(cells.chars().all(|c| !c.is_ascii_lowercase()), "{cells}");
        let brf = to_brf(&doc(), &louis, &BrailleOptions::default()).unwrap();
        assert!(
            brf.chars().all(|c| !c.is_ascii_lowercase() && c != '`'),
            "{brf:?}"
        );
    }
}
