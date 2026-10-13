//! Layout analysis shared by every importer that starts from positioned text.
//!
//! A PDF text layer and an OCR result both arrive as [`Fragment`]s: pieces of text with a position,
//! a size and a confidence. This module turns them into lines, lines into blocks, puts the blocks
//! into reading order one column at a time and decides which blocks are headings and list items.
//! All of it is geometry and typography, so it needs no knowledge of where the text came from.

use crate::model::{
    BlockKind, Confidence, ListItem, Origin, Rect, Run, Script, TableCell, TableRow,
};

/// A piece of positioned text on a page, measured from the top-left corner in page units.
#[derive(Debug, Clone, PartialEq)]
pub struct Fragment {
    pub page: u32,
    /// Left edge.
    pub x: f32,
    /// Baseline.
    pub baseline: f32,
    pub width: f32,
    /// The font size, which stands in for the line height.
    pub size: f32,
    pub bold: bool,
    pub text: String,
    pub confidence: Confidence,
}

/// The size of a page, in the same units as its fragments.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub number: u32,
    pub width: f32,
    pub height: f32,
}

/// The blocks of one page in reading order.
#[derive(Debug, Clone, PartialEq)]
pub struct PageLayout {
    pub number: u32,
    pub blocks: Vec<(Origin, BlockKind)>,
    /// A page number or running header the page carried, dropped from the blocks.
    pub furniture: Vec<String>,
}

#[derive(Debug, Clone)]
struct Line {
    x0: f32,
    x1: f32,
    baseline: f32,
    size: f32,
    bold_chars: usize,
    chars: usize,
    runs: Vec<Run>,
}

impl Line {
    fn new(page: u32, f: &Fragment) -> Self {
        let mut line = Line {
            x0: f.x,
            x1: f.x,
            baseline: f.baseline,
            size: f.size,
            bold_chars: 0,
            chars: 0,
            runs: Vec::new(),
        };
        line.push(page, f, false, None);
        line
    }

    fn push(&mut self, page: u32, f: &Fragment, space_before: bool, script: Option<Script>) {
        if space_before && let Some(last) = self.runs.last_mut() {
            last.text.push(' ');
        }
        let count = f.text.chars().count();
        self.chars += count;
        if f.bold {
            self.bold_chars += count;
        }
        self.x1 = self.x1.max(f.x + f.width);
        self.size = self.size.max(f.size);
        let rect = Rect::new(f.x, f.baseline - 0.8 * f.size, f.width, f.size);
        let mut run = Run::recognised(f.text.clone(), f.confidence, Origin::new(page, rect));
        run.script = script;
        self.runs.push(run);
    }

    fn bold(&self) -> bool {
        self.bold_chars * 2 > self.chars
    }

    fn words(&self) -> usize {
        self.text().split_whitespace().count()
    }

    fn text(&self) -> String {
        crate::model::join_runs(&self.runs)
    }

    fn rect(&self) -> Rect {
        Rect::new(
            self.x0,
            self.baseline - 0.8 * self.size,
            self.x1 - self.x0,
            self.size,
        )
    }
}

#[derive(Debug, Clone)]
struct Block {
    lines: Vec<Line>,
    /// Set when the block is a table: its rows of cells, each cell a line.
    rows: Option<Vec<Vec<Line>>>,
}

impl Block {
    fn x0(&self) -> f32 {
        self.lines.iter().map(|l| l.x0).fold(f32::MAX, f32::min)
    }
    fn x1(&self) -> f32 {
        self.lines.iter().map(|l| l.x1).fold(f32::MIN, f32::max)
    }
    fn top(&self) -> f32 {
        self.lines[0].baseline - 0.8 * self.lines[0].size
    }
    fn bottom(&self) -> f32 {
        let last = self.lines.last().expect("blocks hold at least one line");
        last.baseline + 0.2 * last.size
    }
    fn size(&self) -> f32 {
        // the size most of the block's characters use
        let mut sizes: Vec<(f32, usize)> = Vec::new();
        for l in &self.lines {
            match sizes.iter_mut().find(|(s, _)| (*s - l.size).abs() < 0.26) {
                Some(entry) => entry.1 += l.chars,
                None => sizes.push((l.size, l.chars)),
            }
        }
        sizes
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .map(|(s, _)| s)
            .unwrap_or(0.0)
    }
    fn bold(&self) -> bool {
        let bold: usize = self.lines.iter().map(|l| l.bold_chars).sum();
        let all: usize = self.lines.iter().map(|l| l.chars).sum();
        bold * 2 > all
    }
    fn chars(&self) -> usize {
        self.lines.iter().map(|l| l.chars).sum()
    }
    fn rect(&self) -> Rect {
        self.lines
            .iter()
            .skip(1)
            .fold(self.lines[0].rect(), |r, l| r.union(&l.rect()))
    }
    fn text(&self) -> String {
        let mut text = String::new();
        for line in &self.lines {
            let piece = line.text();
            if text.is_empty() {
                text = piece;
            } else if text.ends_with('-')
                && piece.starts_with(|c: char| c.is_lowercase())
                && !text.ends_with(" -")
            {
                // a word broken across lines
                text.pop();
                text.push_str(&piece);
            } else {
                text.push(' ');
                text.push_str(&piece);
            }
        }
        text
    }
    /// Every run in order, with a space carried between lines.
    fn runs(&self) -> Vec<Run> {
        let mut runs = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0
                && let Some(last) = runs.last_mut()
            {
                let last: &mut Run = last;
                if !last.text.ends_with(char::is_whitespace) {
                    last.text.push(' ');
                }
            }
            runs.extend(line.runs.iter().cloned());
        }
        runs
    }
}

/// Lays out every page. Fragments may arrive in any order.
pub fn analyse(pages: &[PageSize], fragments: &[Fragment]) -> Vec<PageLayout> {
    let mut per_page: Vec<Vec<&Fragment>> = vec![Vec::new(); pages.len()];
    for f in fragments {
        if f.text.trim().is_empty() {
            continue;
        }
        if let Some(i) = pages.iter().position(|p| p.number == f.page) {
            per_page[i].push(f);
        }
    }

    let blocks_per_page: Vec<Vec<Block>> = per_page
        .iter()
        .zip(pages)
        .map(|(frags, page)| {
            let (lines, mut blocks) = detect_tables(group_lines(page.number, frags));
            blocks.extend(group_blocks(lines));
            order_blocks(blocks, page)
        })
        .collect();

    let body = body_size(blocks_per_page.iter().flatten());
    let heading_sizes = heading_sizes(blocks_per_page.iter().flatten(), body);

    blocks_per_page
        .into_iter()
        .zip(pages)
        .map(|(blocks, page)| {
            let (blocks, furniture) = strip_furniture(blocks, page);
            PageLayout {
                number: page.number,
                blocks: classify(page.number, blocks, body, &heading_sizes),
                furniture,
            }
        })
        .collect()
}

fn group_lines(page: u32, fragments: &[&Fragment]) -> Vec<Line> {
    let mut sorted: Vec<&Fragment> = fragments.to_vec();
    sorted.sort_by(|a, b| a.baseline.total_cmp(&b.baseline).then(a.x.total_cmp(&b.x)));
    // a superscript sits above its line and would otherwise sort before it, so fragments are
    // grouped by baseline first and read left to right within each group
    let mut groups: Vec<(f32, f32, Vec<&Fragment>)> = Vec::new();
    for f in sorted {
        match groups.last_mut() {
            Some((baseline, size, members))
                if (f.baseline - *baseline).abs() <= 0.4 * size.max(f.size) =>
            {
                if f.size > *size {
                    *size = f.size;
                    *baseline = f.baseline;
                }
                members.push(f);
            }
            _ => groups.push((f.baseline, f.size, vec![f])),
        }
    }
    let mut sorted: Vec<&Fragment> = Vec::new();
    for (_, _, mut members) in groups {
        members.sort_by(|a, b| a.x.total_cmp(&b.x));
        sorted.extend(members);
    }

    let mut lines: Vec<Line> = Vec::new();
    for f in sorted {
        let joined = lines.iter_mut().rev().take(8).find_map(|line| {
            let size = line.size.max(f.size);
            let same_baseline = (line.baseline - f.baseline).abs() <= 0.4 * size;
            let gap = f.x - line.x1;
            // a large gap on the same baseline is another column, not the same line
            let adjacent = gap >= -0.3 * size && gap <= 2.5 * size;
            (same_baseline && adjacent).then_some((line, gap))
        });
        match joined {
            Some((line, gap)) => {
                let needs_space = gap > 0.12 * f.size
                    && !f.text.starts_with(char::is_whitespace)
                    && !line
                        .runs
                        .last()
                        .is_some_and(|r| r.text.ends_with(char::is_whitespace));
                // smaller text shifted off the baseline is a subscript or superscript
                let shift = f.baseline - line.baseline;
                let script = if f.size <= 0.8 * line.size && shift.abs() >= 0.15 * line.size {
                    Some(if shift < 0.0 {
                        Script::Superscript
                    } else {
                        Script::Subscript
                    })
                } else {
                    None
                };
                line.push(page, f, needs_space && script.is_none(), script);
            }
            None => lines.push(Line::new(page, f)),
        }
    }
    lines
}

/// Finds tables: two or more consecutive rows of short cells whose columns line up. The cells
/// become one block with rows; everything else goes back to be grouped into ordinary blocks.
fn detect_tables(mut lines: Vec<Line>) -> (Vec<Line>, Vec<Block>) {
    lines.sort_by(|a, b| {
        a.baseline
            .total_cmp(&b.baseline)
            .then(a.x0.total_cmp(&b.x0))
    });
    let mut rows: Vec<Vec<Line>> = Vec::new();
    for line in lines {
        match rows.last_mut() {
            Some(row)
                if (row[0].baseline - line.baseline).abs() <= 0.4 * row[0].size.max(line.size) =>
            {
                row.push(line)
            }
            _ => rows.push(vec![line]),
        }
    }
    // cells are short and never bulleted. A technical table has digits in most of its rows, which
    // keeps two columns of short prose lines from being read as one
    let is_cell = |l: &Line| l.chars <= 32 && l.words() <= 5 && list_marker(&l.text()).is_none();
    let candidate = |row: &[Line]| row.len() >= 2 && row.iter().all(is_cell);
    let has_digit = |row: &[Line]| {
        row.iter()
            .any(|l| l.text().chars().any(|c| c.is_ascii_digit()))
    };
    let aligned = |a: &[Line], b: &[Line]| {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(x, y)| (x.x0 - y.x0).abs() <= 1.5 * x.size.max(y.size))
            && (b[0].baseline - a[0].baseline) <= 2.5 * a[0].size.max(b[0].size)
    };

    let mut tables = Vec::new();
    let mut remaining = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let mut end = i;
        if candidate(&rows[i]) {
            while end + 1 < rows.len()
                && candidate(&rows[end + 1])
                && aligned(&rows[end], &rows[end + 1])
            {
                end += 1;
            }
        }
        let numeric_rows = rows[i + 1..=end].iter().filter(|r| has_digit(r)).count();
        if end > i && numeric_rows * 2 >= end - i {
            let table_rows: Vec<Vec<Line>> = rows.drain(i..=end).collect();
            // drain shifted everything down, so the next row is at i again
            let lines = table_rows.iter().flatten().cloned().collect();
            tables.push(Block {
                lines,
                rows: Some(table_rows),
            });
        } else {
            remaining.append(&mut rows[i]);
            i += 1;
        }
    }
    (remaining, tables)
}

/// How far the next line of a paragraph may sit below the last one, in line pitches of the page.
/// A paragraph break adds space on top of the pitch, so a break clears this with room to spare.
const PITCH_TOLERANCE: f32 = 1.2;

/// The typical distance between a line and the line printed under it on a page, found from the
/// nearest same-sized line below each one. `None` when there are too few pairs to call it typical.
fn line_pitch(lines: &[Line]) -> Option<f32> {
    let mut gaps: Vec<f32> = Vec::new();
    for a in lines {
        let nearest = lines
            .iter()
            .filter(|b| {
                let size = a.size.max(b.size);
                let overlap = a.x1.min(b.x1) - a.x0.max(b.x0);
                let gap = b.baseline - a.baseline;
                (a.size - b.size).abs() <= 0.15 * size
                    && overlap > 0.2 * size
                    && gap > 0.5 * size
                    && gap <= 3.5 * size
            })
            .map(|b| b.baseline - a.baseline)
            .min_by(f32::total_cmp);
        gaps.extend(nearest);
    }
    if gaps.len() < 3 {
        return None;
    }
    gaps.sort_by(f32::total_cmp);
    Some(gaps[gaps.len() / 2])
}

/// Whether a line runs to the right edge its column keeps, which marks it as wrapped on to the
/// next line rather than the last line of a paragraph or a line of its own.
fn runs_full(line: &Line, lines: &[Line]) -> bool {
    let mut edges: Vec<f32> = lines
        .iter()
        .filter(|o| {
            let overlap = line.x1.min(o.x1) - line.x0.max(o.x0);
            overlap > 0.5 * line.size && (o.size - line.size).abs() <= 0.15 * line.size.max(o.size)
        })
        .map(|o| o.x1)
        .collect();
    edges.sort_by(f32::total_cmp);
    edges
        .get(edges.len() / 2)
        .is_some_and(|edge| line.x1 >= edge - 4.0 * line.size)
}

fn group_blocks(lines: Vec<Line>) -> Vec<Block> {
    let pitch = line_pitch(&lines);
    let marked: Vec<bool> = lines
        .iter()
        .map(|l| list_marker(&l.text()).is_some())
        .collect();
    let mut blocks: Vec<Block> = Vec::new();
    for (index, line) in lines.iter().cloned().enumerate() {
        let best = blocks
            .iter_mut()
            .filter_map(|block| {
                let last = block.lines.last().expect("blocks hold at least one line");
                let size = last.size.max(line.size);
                let gap = line.baseline - last.baseline;
                let overlap = line.x1.min(block.x1()) - line.x0.max(block.x0());
                let same_size = (last.size - line.size).abs() <= 0.15 * size;
                // loose leading (1.5 times or double spacing) is judged against the page's own
                // line pitch and only after a line that wrapped, so single-line paragraphs set
                // at a similar spacing stay apart
                let close = gap > 0.0
                    && (gap <= 1.5 * size
                        || pitch.is_some_and(|p| {
                            !marked[index]
                                && gap <= PITCH_TOLERANCE * p
                                && gap <= 3.0 * size
                                && gap >= 0.85 * p
                                && runs_full(last, &lines)
                        }));
                (same_size && close && overlap > 0.2 * size).then_some((block, overlap))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));
        match best {
            Some((block, _)) => block.lines.push(line),
            None => blocks.push(Block {
                lines: vec![line],
                rows: None,
            }),
        }
    }
    blocks
}

/// Puts blocks into reading order: wide blocks split the page into bands and within a band each
/// column is read top to bottom before the next column to its right.
fn order_blocks(blocks: Vec<Block>, page: &PageSize) -> Vec<Block> {
    if blocks.is_empty() {
        return blocks;
    }
    let text_x0 = blocks.iter().map(Block::x0).fold(f32::MAX, f32::min);
    let text_x1 = blocks.iter().map(Block::x1).fold(f32::MIN, f32::max);
    let text_width = (text_x1 - text_x0).max(1.0);
    let wide = |b: &Block| b.x1() - b.x0() > 0.55 * text_width;

    // columns come from blocks of a reasonable width, so a stray page number cannot start one
    let mut columns: Vec<(f32, f32)> = Vec::new();
    let mut narrow: Vec<(f32, f32)> = blocks
        .iter()
        .filter(|b| !wide(b) && b.x1() - b.x0() >= 0.2 * text_width)
        .map(|b| (b.x0(), b.x1()))
        .collect();
    narrow.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (x0, x1) in narrow {
        match columns.last_mut() {
            Some(last) if x0 < last.1 - 0.02 * page.width => last.1 = last.1.max(x1),
            _ => columns.push((x0, x1)),
        }
    }
    if columns.is_empty() {
        columns.push((text_x0, text_x1));
    }
    let column_of = |b: &Block| {
        let centre = (b.x0() + b.x1()) / 2.0;
        columns
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, c)| {
                let da = distance(centre, a);
                let dc = distance(centre, c);
                da.total_cmp(&dc)
            })
            .map(|(i, _)| i)
            .unwrap_or(0)
    };

    let mut wide_blocks: Vec<Block> = Vec::new();
    let mut column_blocks: Vec<Block> = Vec::new();
    for b in blocks {
        if wide(&b) {
            wide_blocks.push(b)
        } else {
            column_blocks.push(b)
        }
    }
    wide_blocks.sort_by(|a, b| a.top().total_cmp(&b.top()));

    let mut ordered = Vec::new();
    let mut band_top = f32::MIN;
    let mut separators = wide_blocks.into_iter().peekable();
    loop {
        let band_bottom = separators.peek().map_or(f32::MAX, |b| b.top());
        let mut band: Vec<(usize, Block)> = Vec::new();
        column_blocks.retain(|b| {
            let centre = (b.top() + b.bottom()) / 2.0;
            if centre >= band_top && centre < band_bottom {
                band.push((column_of(b), b.clone()));
                false
            } else {
                true
            }
        });
        band.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.top().total_cmp(&b.1.top())));
        ordered.extend(band.into_iter().map(|(_, b)| b));
        match separators.next() {
            Some(sep) => {
                band_top = sep.top();
                ordered.push(sep);
            }
            None => break,
        }
    }
    ordered
}

fn distance(x: f32, (x0, x1): &(f32, f32)) -> f32 {
    if x < *x0 {
        x0 - x
    } else if x > *x1 {
        x - x1
    } else {
        0.0
    }
}

/// Removes page numbers and similar furniture from the top and bottom margins.
fn strip_furniture(blocks: Vec<Block>, page: &PageSize) -> (Vec<Block>, Vec<String>) {
    let mut kept = Vec::new();
    let mut furniture = Vec::new();
    for b in blocks {
        let in_margin = b.bottom() < 0.08 * page.height || b.top() > 0.92 * page.height;
        let text = b.text();
        let looks_like_page_number = {
            let t = text.trim().to_ascii_lowercase();
            let digits = t.trim_start_matches("page ").trim_start_matches("p. ");
            !digits.is_empty()
                && digits
                    .split(|c: char| c == '/' || c.is_whitespace())
                    .all(|part| part == "of" || part.chars().all(|c| c.is_ascii_digit()))
        };
        if in_margin && b.lines.len() == 1 && looks_like_page_number {
            furniture.push(text);
        } else {
            kept.push(b);
        }
    }
    (kept, furniture)
}

/// The font size most of the document's characters use.
fn body_size<'a>(blocks: impl Iterator<Item = &'a Block>) -> f32 {
    let mut sizes: Vec<(f32, usize)> = Vec::new();
    for b in blocks {
        for l in &b.lines {
            match sizes.iter_mut().find(|(s, _)| (*s - l.size).abs() < 0.26) {
                Some(entry) => entry.1 += l.chars,
                None => sizes.push((l.size, l.chars)),
            }
        }
    }
    sizes
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(s, _)| s)
        .unwrap_or(12.0)
}

fn is_heading(b: &Block, body: f32) -> bool {
    let size = b.size();
    let text = b.text();
    let short = b.lines.len() <= 3 && b.chars() <= 120;
    let larger = size >= 1.15 * body;
    let emphasised = b.bold() && size >= 0.95 * body && !text.ends_with(['.', ',', ';']);
    short && (larger || emphasised) && list_marker(&text).is_none()
}

/// Distinct heading sizes, largest first, so the largest becomes level 1.
fn heading_sizes<'a>(blocks: impl Iterator<Item = &'a Block>, body: f32) -> Vec<f32> {
    let mut sizes: Vec<f32> = Vec::new();
    for b in blocks.filter(|b| is_heading(b, body)) {
        let s = b.size();
        if !sizes.iter().any(|x| (*x - s).abs() < 0.26) {
            sizes.push(s);
        }
    }
    sizes.sort_by(|a, b| b.total_cmp(a));
    sizes
}

/// A list marker at the start of a line: the marker text and whether the list is ordered.
fn list_marker(text: &str) -> Option<(&str, bool)> {
    let trimmed = text.trim_start();
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    const BULLETS: &[char] = &[
        '\u{2022}', '\u{25e6}', '\u{25aa}', '\u{2023}', '\u{25cf}', '\u{25cb}', '\u{2013}', '-',
        '*', '\u{2219}',
    ];
    if BULLETS.contains(&first) && chars.next().is_some_and(char::is_whitespace) {
        return Some((&trimmed[..first.len_utf8()], false));
    }
    // "1." "12)" "a)" "iv."
    let label: String = trimmed
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect();
    let rest = &trimmed[label.len()..];
    let numbered = !label.is_empty()
        && label.chars().count() <= 3
        && (label.chars().all(|c| c.is_ascii_digit()) || label.chars().count() <= 2)
        && rest.starts_with(['.', ')'])
        && rest[1..].starts_with(char::is_whitespace);
    numbered.then(|| (&trimmed[..label.len() + 1], true))
}

fn strip_marker(runs: &mut [Run], marker_len: usize) {
    let mut remaining = marker_len;
    for run in runs.iter_mut() {
        if remaining == 0 {
            break;
        }
        let take = remaining.min(run.text.len());
        run.text.drain(..take);
        remaining -= take;
        if remaining == 0 {
            let trimmed = run.text.trim_start().to_string();
            run.text = trimmed;
        }
    }
}

/// Rows of short cells become a table. The first row is the header when it is bold or when it
/// holds no digits while the rows under it do.
fn table_block(rows: &[Vec<Line>]) -> BlockKind {
    let first = &rows[0];
    let has_digit = |l: &Line| l.text().chars().any(|c| c.is_ascii_digit());
    let header_bold =
        first.iter().all(Line::bold) && !rows[1..].iter().all(|r| r.iter().all(Line::bold));
    let header_text =
        !first.iter().any(has_digit) && rows[1..].iter().any(|r| r.iter().any(has_digit));
    let header = header_bold || header_text;
    let table_rows = rows
        .iter()
        .enumerate()
        .map(|(i, row)| TableRow {
            cells: row
                .iter()
                .map(|cell| {
                    let runs = cell.runs.clone();
                    if header && i == 0 {
                        TableCell::header(runs)
                    } else {
                        TableCell::new(runs)
                    }
                })
                .collect(),
        })
        .collect();
    BlockKind::Table {
        rows: table_rows,
        caption: Vec::new(),
    }
}

const RELATIONS: &[char] = &[
    '=', '\u{2248}', '\u{2260}', '\u{2264}', '\u{2265}', '\u{221d}', '<', '>',
];

/// A short block that reads like an equation: a relation such as `=` and mostly symbols, numbers
/// and single letters rather than words.
fn is_equation(b: &Block, text: &str) -> bool {
    if b.lines.len() > 2 || text.chars().count() > 80 || !text.contains(RELATIONS) {
        return false;
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() < 3 {
        return false;
    }
    let wordy = |t: &str| {
        let letters: String = t.chars().filter(|c| c.is_alphabetic()).collect();
        letters.chars().count() > 3 && t.chars().all(|c| c.is_alphabetic() || c == ',' || c == '.')
    };
    let words = tokens.iter().filter(|t| wordy(t)).count();
    words == 0 || (words <= 1 && b.runs().iter().any(|r| r.script.is_some()))
}

/// Rebuilds an equation's source from its runs, with subscripts and superscripts marked.
fn equation_source(runs: &[Run]) -> String {
    let mut out = String::new();
    for run in runs {
        let text = run.text.trim();
        if text.is_empty() {
            continue;
        }
        match run.script {
            Some(Script::Subscript) => out.push_str(&format!("_{{{text}}}")),
            Some(Script::Superscript) => out.push_str(&format!("^{{{text}}}")),
            None => {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(text);
            }
        }
    }
    out
}

fn classify(
    page: u32,
    blocks: Vec<Block>,
    body: f32,
    heading_sizes: &[f32],
) -> Vec<(Origin, BlockKind)> {
    let mut out: Vec<(Origin, BlockKind)> = Vec::new();
    let mut list_indent: Option<f32> = None;
    for b in blocks {
        let origin = Origin::new(page, b.rect());
        let text = b.text();
        if let Some(rows) = &b.rows {
            out.push((origin, table_block(rows)));
            list_indent = None;
            continue;
        }
        if is_heading(&b, body) {
            let level = heading_sizes
                .iter()
                .position(|s| (*s - b.size()).abs() < 0.26)
                .unwrap_or(0);
            let kind = BlockKind::Heading {
                level: (level as u8 + 1).min(6),
                runs: b.runs(),
            };
            out.push((origin, kind));
            list_indent = None;
        } else if let Some((marker, ordered)) = list_marker(&text) {
            let mut runs = b.runs();
            let leading = text.len() - text.trim_start().len();
            strip_marker(&mut runs, leading + marker.len());
            runs.retain(|r| !r.text.is_empty());
            let item = ListItem::new(runs);
            let nested = list_indent.is_some_and(|x| b.x0() > x + 0.8 * b.size());
            match out.last_mut() {
                Some((last_origin, BlockKind::List { ordered: o, items }))
                    if *o == ordered || nested =>
                {
                    *last_origin = Origin::new(page, last_origin.region.union(&origin.region));
                    if nested && let Some(parent) = items.last_mut() {
                        parent.children.push(item);
                    } else {
                        items.push(item);
                    }
                }
                _ => {
                    out.push((
                        origin,
                        BlockKind::List {
                            ordered,
                            items: vec![item],
                        },
                    ));
                    list_indent = Some(b.x0());
                }
            }
        } else if is_equation(&b, &text) {
            let runs = b.runs();
            let source = equation_source(&runs);
            out.push((
                origin,
                BlockKind::Equation {
                    runs,
                    latex: Some(source),
                    mathml: None,
                    display: true,
                },
            ));
            list_indent = None;
        } else {
            out.push((origin, BlockKind::paragraph(b.runs())));
            list_indent = None;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frag(page: u32, x: f32, baseline: f32, size: f32, bold: bool, text: &str) -> Fragment {
        Fragment {
            page,
            x,
            baseline,
            width: text.chars().count() as f32 * size * 0.5,
            size,
            bold,
            text: text.to_string(),
            confidence: Confidence::CERTAIN,
        }
    }

    fn texts(layout: &PageLayout) -> Vec<String> {
        layout
            .blocks
            .iter()
            .map(|(_, k)| match k {
                BlockKind::Heading { level, runs } => {
                    format!("h{level}:{}", crate::model::join_runs(runs))
                }
                BlockKind::Paragraph { runs, .. } => format!("p:{}", crate::model::join_runs(runs)),
                BlockKind::List { ordered, items } => format!(
                    "{}:{}",
                    if *ordered { "ol" } else { "ul" },
                    items
                        .iter()
                        .map(|i| crate::model::join_runs(&i.runs))
                        .collect::<Vec<_>>()
                        .join("|")
                ),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn lines_join_fragments_and_add_missing_spaces() {
        let page = PageSize {
            number: 1,
            width: 600.0,
            height: 800.0,
        };
        let frags = [
            frag(1, 72.0, 100.0, 12.0, false, "The period is"),
            frag(1, 160.0, 100.0, 12.0, false, "2.5"),
            frag(1, 178.1, 100.0, 12.0, false, "ms."),
        ];
        let layout = analyse(&[page], &frags);
        assert_eq!(texts(&layout[0]), ["p:The period is 2.5 ms."]);
    }

    #[test]
    fn two_columns_read_left_then_right_under_a_spanning_title() {
        let page = PageSize {
            number: 1,
            width: 600.0,
            height: 800.0,
        };
        let mut frags = vec![frag(
            1,
            50.0,
            80.0,
            20.0,
            true,
            "Chapter 4: Operational Amplifiers",
        )];
        // the right column arrives first, as it was drawn first
        for (i, t) in ["right one", "right two", "right three"].iter().enumerate() {
            frags.push(frag(1, 320.0, 130.0 + 14.0 * i as f32, 10.0, false, t));
        }
        frags.push(frag(1, 320.0, 200.0, 13.0, true, "4.3 Non-inverting"));
        for (i, t) in ["left one", "left two"].iter().enumerate() {
            frags.push(frag(1, 50.0, 130.0 + 14.0 * i as f32, 10.0, false, t));
        }
        frags.push(frag(1, 50.0, 170.0, 13.0, true, "4.2 Inverting"));
        frags.push(frag(1, 50.0, 190.0, 10.0, false, "left three"));
        frags.push(frag(1, 295.0, 780.0, 9.0, false, "47"));
        let layout = analyse(&[page], &frags);
        assert_eq!(
            texts(&layout[0]),
            [
                "h1:Chapter 4: Operational Amplifiers",
                "p:left one left two",
                "h2:4.2 Inverting",
                "p:left three",
                "p:right one right two right three",
                "h2:4.3 Non-inverting",
            ]
        );
        assert_eq!(layout[0].furniture, ["47"]);
    }

    #[test]
    fn bullets_become_a_list_and_numbers_an_ordered_list() {
        let page = PageSize {
            number: 1,
            width: 600.0,
            height: 800.0,
        };
        let frags = [
            frag(1, 60.0, 60.0, 20.0, true, "Why Fourier series?"),
            frag(1, 70.0, 100.0, 12.0, false, "\u{2022}"),
            frag(
                1,
                88.0,
                100.0,
                12.0,
                false,
                "Any periodic signal is a sum of",
            ),
            frag(1, 88.0, 117.0, 12.0, false, "sinusoids"),
            frag(1, 70.0, 139.0, 12.0, false, "\u{2022}"),
            frag(
                1,
                88.0,
                139.0,
                12.0,
                false,
                "Each harmonic has its own phase",
            ),
            frag(1, 60.0, 180.0, 12.0, false, "Steps:"),
            frag(1, 60.0, 200.0, 12.0, false, "1. Sample"),
            frag(1, 60.0, 222.0, 12.0, false, "2. Transform"),
        ];
        let layout = analyse(&[page], &frags);
        assert_eq!(
            texts(&layout[0]),
            [
                "h1:Why Fourier series?",
                "ul:Any periodic signal is a sum of sinusoids|Each harmonic has its own phase",
                "p:Steps:",
                "ol:Sample|Transform",
            ]
        );
    }

    #[test]
    fn hyphenated_line_breaks_are_joined() {
        let page = PageSize {
            number: 1,
            width: 600.0,
            height: 800.0,
        };
        let frags = [
            frag(1, 60.0, 100.0, 12.0, false, "a very long com-"),
            frag(1, 60.0, 117.0, 12.0, false, "pound word"),
        ];
        let layout = analyse(&[page], &frags);
        let text = match &layout[0].blocks[0].1 {
            BlockKind::Paragraph { runs, .. } => crate::model::join_runs(runs),
            _ => unreachable!(),
        };
        // runs keep the original pieces, the block text joins the word
        assert_eq!(text, "a very long com- pound word");
    }

    #[test]
    fn a_page_without_fragments_has_no_blocks() {
        let pages = [
            PageSize {
                number: 1,
                width: 600.0,
                height: 800.0,
            },
            PageSize {
                number: 2,
                width: 600.0,
                height: 800.0,
            },
        ];
        let layout = analyse(
            &pages,
            &[frag(2, 60.0, 100.0, 12.0, false, "only page two")],
        );
        assert!(layout[0].blocks.is_empty());
        assert_eq!(layout[1].blocks.len(), 1);
    }

    /// A page of two paragraphs set at the given leading: the second after `extra` more space.
    fn spaced_page(leading: f32, extra: f32) -> Vec<Fragment> {
        let paragraphs = [
            [
                "Reading aloud works best when a paragraph arrives",
                "as one piece of text rather than a pile of lines",
                "so every sentence is spoken from its first word",
                "to its last without stopping part way through.",
            ],
            [
                "A second paragraph begins after a clear break and",
                "carries on for several lines at the same size so",
                "the importer can tell it from the one above it",
                "and keep the two apart on the page.",
            ],
        ];
        let mut out = Vec::new();
        let mut y = 100.0;
        for (p, lines) in paragraphs.iter().enumerate() {
            if p > 0 {
                y += extra;
            }
            for (i, text) in lines.iter().enumerate() {
                if i > 0 {
                    y += leading;
                }
                // every line but the last is the same width so it runs to the right edge
                let mut f = frag(1, 72.0, y, 12.5, false, text);
                if i + 1 < lines.len() {
                    f.width = 400.0;
                } else {
                    f.width = 250.0;
                }
                out.push(f);
            }
            y += leading;
        }
        out
    }

    fn paragraph_count(frags: &[Fragment]) -> Vec<usize> {
        let page = PageSize {
            number: 1,
            width: 612.0,
            height: 792.0,
        };
        let layout = analyse(&[page], frags);
        layout[0]
            .blocks
            .iter()
            .map(|(_, k)| match k {
                BlockKind::Paragraph { runs, .. } => runs.len(),
                _ => 0,
            })
            .collect()
    }

    #[test]
    fn the_same_paragraphs_come_out_at_single_one_and_a_half_and_double_spacing() {
        for leading in [14.4, 19.0, 25.0] {
            // a clear extra gap of about half a line separates the two paragraphs
            let frags = spaced_page(leading, leading * 0.6);
            assert_eq!(
                paragraph_count(&frags),
                [4, 4],
                "leading {leading} must give two four-line paragraphs"
            );
        }
    }

    #[test]
    fn wide_spacing_without_a_break_is_one_paragraph() {
        let mut frags = spaced_page(19.0, 0.0);
        for f in &mut frags {
            f.width = 400.0;
        }
        // with no extra gap and every line running full, the text is one run at 1.5 times spacing
        let counts = paragraph_count(&frags);
        assert_eq!(counts, [8]);
        // a heading-sized change still splits
        frags[4].size = 18.0;
        assert!(paragraph_count(&frags).len() > 1);
    }
}
