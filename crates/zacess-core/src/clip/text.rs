//! Plain text from the share sheet, the clipboard or a text file, as a document.
//!
//! A blank line ends a paragraph. Lines that start with a list marker (`-`, `*`, `+`, a bullet or a
//! number followed by `.` or `)`) become a list, nested by how far they are indented. A line
//! starting with one to six `#` and a space is a heading, as in Markdown.

use super::{collapse_spaces, shorten};
use crate::model::{
    Block, BlockId, BlockKind, Document, ListItem, Origin, Rect, Run, Source, SourceFormat,
};

/// The longest title taken from the first line of a text.
const TITLE_CHARS: usize = 80;

/// The address when the whole text is one web address and nothing else, so it can be kept as a
/// link note rather than fetched.
pub fn bare_url(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let trimmed = trimmed
        .strip_prefix('<')
        .and_then(|t| t.strip_suffix('>'))
        .unwrap_or(trimmed);
    if trimmed.is_empty()
        || trimmed.len() > super::MAX_URL_BYTES
        || trimmed.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    // there must be a host, not only a scheme
    if rest.is_empty() || rest.starts_with('/') {
        return None;
    }
    Some(trimmed.to_string())
}

/// A run of text that came from the capture itself.
pub(crate) fn clip_run(text: impl Into<String>) -> Run {
    Run::native(text, clip_origin())
}

/// Captured content has no page, so every block and run points at the start of page 1.
pub(crate) fn clip_origin() -> Origin {
    Origin::new(1, Rect::default())
}

/// Blocks numbered from 1 in order. Adding them one by one would look for the highest id each
/// time, which is slow for a page with many thousands of blocks.
pub(crate) fn numbered_blocks(kinds: Vec<BlockKind>) -> Vec<Block> {
    kinds
        .into_iter()
        .enumerate()
        .map(|(i, kind)| Block {
            id: BlockId(i as u64 + 1),
            kind,
            origin: clip_origin(),
        })
        .collect()
}

enum Line<'a> {
    Blank,
    Heading(u8, &'a str),
    Item {
        indent: usize,
        ordered: bool,
        text: &'a str,
    },
    Plain {
        indent: usize,
        text: &'a str,
    },
}

fn indent_of(line: &str) -> usize {
    let mut width = 0;
    for c in line.chars() {
        match c {
            ' ' => width += 1,
            '\t' => width += 4,
            _ => break,
        }
    }
    width
}

fn classify(line: &str) -> Line<'_> {
    let indent = indent_of(line);
    let body = line.trim();
    if body.is_empty() {
        return Line::Blank;
    }
    let hashes = body.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && body[hashes..].starts_with(' ') {
        return Line::Heading(hashes as u8, body[hashes..].trim());
    }
    for bullet in ['-', '*', '+', '\u{2022}', '\u{25e6}', '\u{2023}'] {
        if let Some(rest) = body.strip_prefix(bullet)
            && rest.starts_with([' ', '\t'])
        {
            return Line::Item {
                indent,
                ordered: false,
                text: rest.trim(),
            };
        }
    }
    let digits = body.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        let rest = &body[digits..];
        if let Some(rest) = rest.strip_prefix(['.', ')'])
            && rest.starts_with([' ', '\t'])
        {
            return Line::Item {
                indent,
                ordered: true,
                text: rest.trim(),
            };
        }
    }
    Line::Plain { indent, text: body }
}

/// A list being built, with the items still open at each depth.
struct OpenList {
    ordered: bool,
    /// The indent and the items of each open level, outermost first.
    levels: Vec<(usize, Vec<ListItem>)>,
}

impl OpenList {
    fn new(ordered: bool, indent: usize) -> Self {
        OpenList {
            ordered,
            levels: vec![(indent, Vec::new())],
        }
    }

    fn add(&mut self, indent: usize, text: &str) {
        // close levels indented deeper than this item
        while self.levels.len() > 1 && indent < self.levels.last().map_or(0, |l| l.0) {
            self.close_level();
        }
        let deepest = self.levels.last().map_or(0, |l| l.0);
        let has_parent = self.levels.last().is_some_and(|l| !l.1.is_empty());
        if indent > deepest && has_parent && self.levels.len() < 16 {
            self.levels.push((indent, Vec::new()));
        }
        if let Some(level) = self.levels.last_mut() {
            level
                .1
                .push(ListItem::new(vec![clip_run(collapse_spaces(text))]));
        }
    }

    /// Adds a wrapped line to the item before it.
    fn continue_item(&mut self, text: &str) {
        if let Some(item) = self.levels.last_mut().and_then(|l| l.1.last_mut())
            && let Some(run) = item.runs.last_mut()
        {
            run.text.push(' ');
            run.text.push_str(&collapse_spaces(text));
        }
    }

    fn close_level(&mut self) {
        if let Some((_, items)) = self.levels.pop()
            && let Some(parent) = self.levels.last_mut().and_then(|l| l.1.last_mut())
        {
            parent.children.extend(items);
        }
    }

    fn finish(mut self) -> BlockKind {
        while self.levels.len() > 1 {
            self.close_level();
        }
        BlockKind::List {
            ordered: self.ordered,
            items: self.levels.pop().map(|l| l.1).unwrap_or_default(),
        }
    }
}

/// Turns plain text into a document. Returns `None` when it holds nothing but whitespace.
pub fn text_to_document(text: &str) -> Option<Document> {
    let text = text.trim_start_matches('\u{feff}');
    let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut blocks: Vec<BlockKind> = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut list: Option<OpenList> = None;
    let mut blank_before = false;

    let flush_paragraph = |paragraph: &mut Vec<&str>, blocks: &mut Vec<BlockKind>| {
        if !paragraph.is_empty() {
            let joined = collapse_spaces(&paragraph.join(" "));
            blocks.push(BlockKind::paragraph(vec![clip_run(joined)]));
            paragraph.clear();
        }
    };

    for line in normalised.lines() {
        match classify(line) {
            Line::Blank => {
                flush_paragraph(&mut paragraph, &mut blocks);
                blank_before = true;
                continue;
            }
            Line::Heading(level, body) => {
                flush_paragraph(&mut paragraph, &mut blocks);
                if let Some(open) = list.take() {
                    blocks.push(open.finish());
                }
                if let Ok(heading) = BlockKind::heading(level, vec![clip_run(body)]) {
                    blocks.push(heading);
                }
            }
            Line::Item {
                indent,
                ordered,
                text,
            } => {
                flush_paragraph(&mut paragraph, &mut blocks);
                match &mut list {
                    Some(open) if open.ordered == ordered || indent > open.levels[0].0 => {
                        open.add(indent, text);
                    }
                    _ => {
                        if let Some(open) = list.take() {
                            blocks.push(open.finish());
                        }
                        let mut open = OpenList::new(ordered, indent);
                        open.add(indent, text);
                        list = Some(open);
                    }
                }
            }
            Line::Plain { indent, text } => match &mut list {
                // an indented line straight after an item is that item wrapping
                Some(open) if !blank_before && indent > open.levels[0].0 => {
                    open.continue_item(text);
                }
                _ => {
                    if let Some(open) = list.take() {
                        blocks.push(open.finish());
                    }
                    paragraph.push(text);
                }
            },
        }
        blank_before = false;
    }
    flush_paragraph(&mut paragraph, &mut blocks);
    if let Some(open) = list.take() {
        blocks.push(open.finish());
    }
    if blocks.is_empty() {
        return None;
    }

    let first_line = normalised
        .lines()
        .map(|l| {
            let body = l.trim().trim_start_matches('#').trim();
            match classify(l) {
                Line::Item { text, .. } => collapse_spaces(text),
                _ => collapse_spaces(body),
            }
        })
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let mut document = Document::new(
        shorten(&first_line, TITLE_CHARS),
        Source {
            file_name: "text".into(),
            format: SourceFormat::Other,
        },
    );
    document.blocks = numbered_blocks(blocks);
    Some(document)
}
