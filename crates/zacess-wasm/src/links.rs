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
    // the words of the item as char ranges
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < n {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let from = i;
        while i < n && !chars[i].is_whitespace() {
            i += 1;
        }
        words.push((from, i));
    }
    for (i, link) in links.iter().enumerate() {
        if !on_link_line(&item, link) {
            continue;
        }
        // a word belongs to the link only when most of its width lies inside the box, so a box
        // that merely touches a neighbouring word leaves that word out
        let inside: Vec<(usize, usize)> = words
            .iter()
            .copied()
            .filter(|&(from, to)| {
                let w0 = item.x + from as f32 * per_char;
                let w1 = item.x + to as f32 * per_char;
                let overlap = w1.min(link.x + link.width) - w0.max(link.x);
                overlap * 2.0 > w1 - w0
            })
            .collect();
        let (Some(first), Some(last)) = (inside.first(), inside.last()) else {
            continue;
        };
        let (start, end) = (first.0, last.1);
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

/// Gives every run most of whose width lies inside a link area that area's address. The run's
/// middle must also sit on the area's lines.
pub fn attach_links(document: &mut Document, links: &[(u32, LinkInput)]) {
    if links.is_empty() {
        return;
    }
    for block in &mut document.blocks {
        for run in block.runs_mut() {
            let Some(origin) = run.origin else { continue };
            let r = origin.region;
            let cy = r.y + r.height / 2.0;
            let found = links.iter().find(|(page, l)| {
                let overlap = (r.x + r.width).min(l.x + l.width) - r.x.max(l.x);
                *page == origin.page
                    && overlap * 2.0 > r.width
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

/// Characters that may open an address without belonging to it.
const LEADING: &[char] = &['(', '[', '<', '"', '\'', '{'];

fn is_email(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c));
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        && labels
            .last()
            .is_some_and(|t| t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphabetic()));
    local_ok && domain_ok
}

/// The address a bare piece of text links to: web addresses and `mailto:` addresses as written,
/// a bare email address as a `mailto:` link.
pub fn address_link(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        text.to_string()
    } else {
        format!("mailto:{text}")
    }
}

/// The byte ranges of bare web, `mailto:` and email addresses in a piece of text.
pub fn find_addresses(text: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut at = 0;
    for token in text.split_inclusive(char::is_whitespace) {
        let base = at;
        at += token.len();
        let word = token.trim_end_matches(char::is_whitespace);
        let lead = word.len() - word.trim_start_matches(LEADING).len();
        let start = base + lead;
        let mut end = base + word.len();
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
        let candidate = &text[start..end];
        let lower = candidate.to_ascii_lowercase();
        let ok = if lower.starts_with("https://") {
            candidate.len() > "https://".len()
        } else if lower.starts_with("http://") {
            candidate.len() > "http://".len()
        } else if lower.starts_with("mailto:") {
            is_email(candidate["mailto:".len()..].split('?').next().unwrap_or(""))
        } else {
            is_email(candidate)
        };
        if ok {
            found.push((start, end));
        }
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
                link: Some(address_link(&run.text[start..end])),
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
