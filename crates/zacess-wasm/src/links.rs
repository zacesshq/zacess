//! Links in the material people open or paste: link areas from a PDF and bare web addresses in
//! plain text become `Run::link` in the document. Nothing here ever fetches an address.

use serde::Deserialize;
use zacess_core::model::{BlockKind, Document, ListItem, Run};

use crate::TextItem;

/// A link area on a PDF page, in the page's units from its top-left corner.
#[derive(Debug, Clone, Deserialize)]
pub struct LinkInput {
    pub url: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// The address when it is one the app may show as a link: web and mail addresses only. Anything
/// else (a `javascript:` or `data:` address, an internal destination) stays plain text.
pub fn web_link(url: &str) -> Option<String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let scheme = ["https://", "http://", "mailto:"]
        .iter()
        .find(|s| lower.starts_with(**s))?;
    let usable =
        url.len() > scheme.len() && !url.chars().any(|c| c.is_whitespace() || c.is_control());
    usable.then(|| url.to_string())
}

/// Whether a text item sits on the lines a link area covers. The baseline is allowed a little below
/// the area, since descenders hang beneath it.
fn on_link_line(item: &TextItem, link: &LinkInput) -> bool {
    item.baseline >= link.y && item.baseline - 0.3 * item.size <= link.y + link.height
}

/// Splits a text item where a link area starts and ends, so the linked words become their own
/// fragment and their own run. Positions inside an item are estimated from its width, so a split
/// is widened to whole words rather than cutting one.
pub fn split_at_links(item: TextItem, links: &[LinkInput]) -> Vec<TextItem> {
    let chars: Vec<char> = item.text.chars().collect();
    let n = chars.len();
    if n == 0 || item.width.is_nan() || item.width <= 0.0 {
        return vec![item];
    }
    let per_char = item.width / n as f32;
    for (i, link) in links.iter().enumerate() {
        if !on_link_line(&item, link) {
            continue;
        }
        let x0 = item.x.max(link.x);
        let x1 = (item.x + item.width).min(link.x + link.width);
        if x1 <= x0 {
            continue;
        }
        let mut start = (((x0 - item.x) / per_char).round() as usize).min(n);
        let mut end = (((x1 - item.x) / per_char).round() as usize).min(n);
        if end <= start {
            continue;
        }
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while end < n && !chars[end].is_whitespace() {
            end += 1;
        }
        if start == 0 && end == n {
            return vec![item];
        }
        let piece = |from: usize, to: usize| TextItem {
            x: item.x + from as f32 * per_char,
            baseline: item.baseline,
            width: (to - from) as f32 * per_char,
            size: item.size,
            bold: item.bold,
            text: chars[from..to].iter().collect(),
        };
        let rest = &links[i + 1..];
        let mut out = Vec::new();
        if start > 0 {
            out.extend(split_at_links(piece(0, start), rest));
        }
        out.push(piece(start, end));
        if end < n {
            out.extend(split_at_links(piece(end, n), rest));
        }
        return out;
    }
    vec![item]
}

/// Gives every run whose middle lies inside a link area that area's address.
pub fn attach_links(document: &mut Document, links: &[(u32, LinkInput)]) {
    if links.is_empty() {
        return;
    }
    for block in &mut document.blocks {
        for run in block.runs_mut() {
            let Some(origin) = run.origin else { continue };
            let r = origin.region;
            let (cx, cy) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
            let found = links.iter().find(|(page, l)| {
                *page == origin.page
                    && cx >= l.x
                    && cx <= l.x + l.width
                    && cy >= l.y - r.height * 0.25
                    && cy <= l.y + l.height + r.height * 0.25
            });
            if let Some((_, link)) = found {
                run.link = Some(link.url.clone());
            }
        }
    }
}

/// Characters that end a sentence or a bracket rather than an address.
const TRAILING: &[char] = &['.', ',', ';', ':', '!', '?', '\'', '"', ']', '}', '>', ')'];

/// The byte ranges of bare web addresses in a piece of text.
pub fn find_addresses(text: &str) -> Vec<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let mut found = Vec::new();
    let mut from = 0;
    while from < text.len() {
        let next = ["http://", "https://"]
            .iter()
            .filter_map(|s| lower[from..].find(s).map(|i| (from + i, s.len())))
            .min();
        let Some((start, scheme)) = next else { break };
        let boundary = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || "([<\"'".contains(c));
        let mut end = text[start..]
            .find(char::is_whitespace)
            .map_or(text.len(), |i| start + i);
        // a closing bracket belongs to the address only when the address opened one
        while let Some(last) = text[start..end].chars().next_back() {
            let candidate = &text[start..end];
            let keep_bracket =
                last == ')' && candidate.matches('(').count() >= candidate.matches(')').count();
            if TRAILING.contains(&last) && !keep_bracket {
                end -= last.len_utf8();
            } else {
                break;
            }
        }
        if boundary && end > start + scheme {
            found.push((start, end));
        }
        from = end.max(start + scheme);
    }
    found
}

fn link_runs(runs: &mut Vec<Run>) {
    let mut out = Vec::with_capacity(runs.len());
    for run in runs.drain(..) {
        let spans = if run.link.is_none() {
            find_addresses(&run.text)
        } else {
            Vec::new()
        };
        if spans.is_empty() {
            out.push(run);
            continue;
        }
        let mut at = 0;
        for (start, end) in spans {
            if start > at {
                out.push(Run {
                    text: run.text[at..start].to_string(),
                    ..run.clone()
                });
            }
            out.push(Run {
                text: run.text[start..end].to_string(),
                link: Some(run.text[start..end].to_string()),
                ..run.clone()
            });
            at = end;
        }
        if at < run.text.len() {
            out.push(Run {
                text: run.text[at..].to_string(),
                ..run.clone()
            });
        }
    }
    *runs = out;
}

fn link_items(items: &mut [ListItem]) {
    for item in items {
        link_runs(&mut item.runs);
        link_items(&mut item.children);
    }
}

/// Turns every bare web address in the document's text into a link on exactly that address.
pub fn link_bare_addresses(document: &mut Document) {
    for block in &mut document.blocks {
        match &mut block.kind {
            BlockKind::Heading { runs, .. } | BlockKind::Paragraph { runs, .. } => link_runs(runs),
            BlockKind::List { items, .. } => link_items(items),
            BlockKind::Table { rows, caption } => {
                link_runs(caption);
                for cell in rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    link_runs(&mut cell.runs);
                }
            }
            BlockKind::Image { .. } | BlockKind::Equation { .. } | BlockKind::PageBreak => {}
        }
    }
}
