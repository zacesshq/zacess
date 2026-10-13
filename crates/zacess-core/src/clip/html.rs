//! Web pages as documents.
//!
//! The page is read with a small, forgiving HTML reader rather than a browser engine: a clip only
//! needs the structure that matters for reading (headings, paragraphs, lists, tables, links, image
//! descriptions, code and quotations). Page furniture such as menus, footers, cookie notices and
//! advertisements is left out and the main content is found when the whole page was sent. Nothing
//! the page refers to is ever fetched.

use super::text::{clip_run, numbered_blocks};
use super::{ClipError, collapse_spaces, shorten};
use crate::model::{
    BlockKind, Document, ListItem, Run, Script, Source, SourceFormat, TableCell, TableRow, TextRole,
};

/// How deep elements nest before deeper ones are read as part of their parent. Real pages stay
/// far below this. The limit keeps every walk over the tree within a small stack.
pub const MAX_DEPTH: usize = 128;
/// The most elements and text pieces read from one page.
pub const MAX_NODES: usize = 400_000;
/// How deep lists nest before deeper items join the deepest level. Each level takes three levels
/// of the document's JSON, which a reader stops at 128.
pub const MAX_LIST_LEVELS: usize = 16;
/// The most attributes read from one tag. Real tags carry a handful.
const MAX_ATTRIBUTES: usize = 64;
/// The longest title taken from a heading or the page.
const TITLE_CHARS: usize = 120;

/// What is known about the page besides its HTML.
#[derive(Debug, Clone, Default)]
pub struct ClipOptions<'a> {
    /// The page address, used to make relative links whole.
    pub url: Option<&'a str>,
    /// The title the sender gave, which wins over the page's own.
    pub title: Option<&'a str>,
    /// The HTML is a selection, kept whole rather than searched for the main content.
    pub selection_only: bool,
}

// --- reading the HTML ------------------------------------------------------------------------

#[derive(Debug)]
enum NodeKind {
    Element {
        name: String,
        attrs: Vec<(String, String)>,
    },
    Text(String),
}

#[derive(Debug)]
struct Node {
    kind: NodeKind,
    parent: usize,
    children: Vec<usize>,
}

/// A page as a tree. Node 0 is a root that holds everything. A child always has a higher
/// index than its parent, so a pass in index order sees every parent before its children.
#[derive(Debug)]
struct Tree {
    nodes: Vec<Node>,
    title: Option<String>,
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];
/// Elements whose content is read as plain text up to their end tag, never as markup.
const RAW_TEXT: &[&str] = &[
    "script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes", "noscript",
    "template",
];
/// Elements that a new block-level element closes an open paragraph through.
const INLINE: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "cite", "code", "data", "del", "dfn", "em", "font", "i", "ins",
    "kbd", "label", "mark", "q", "s", "samp", "small", "span", "strike", "strong", "sub", "sup",
    "time", "tt", "u", "var",
];
/// Elements that end an open paragraph when they start.
const CLOSES_P: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "dialog",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "ul",
];

impl Tree {
    fn name(&self, id: usize) -> &str {
        match &self.nodes[id].kind {
            NodeKind::Element { name, .. } => name,
            NodeKind::Text(_) => "",
        }
    }

    fn attr(&self, id: usize, key: &str) -> Option<&str> {
        match &self.nodes[id].kind {
            NodeKind::Element { attrs, .. } => attrs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str()),
            NodeKind::Text(_) => None,
        }
    }

    fn parse(html: &str) -> Result<Tree, ClipError> {
        let mut builder = Builder {
            tree: Tree {
                nodes: vec![Node {
                    kind: NodeKind::Element {
                        name: "#root".into(),
                        attrs: Vec::new(),
                    },
                    parent: 0,
                    children: Vec::new(),
                }],
                title: None,
            },
            stack: vec![0],
        };
        let bytes = html.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'<' {
                let end = memchr(b'<', bytes, i).unwrap_or(bytes.len());
                builder.text(decode_entities(&html[i..end]))?;
                i = end;
                continue;
            }
            let rest = &html[i..];
            if rest.starts_with("<!--") {
                i = find(bytes, b"-->", i + 4).map_or(bytes.len(), |e| e + 3);
            } else if rest.starts_with("<!") || rest.starts_with("<?") {
                i = memchr(b'>', bytes, i).map_or(bytes.len(), |e| e + 1);
            } else if rest.starts_with("</")
                && bytes.get(i + 2).is_some_and(u8::is_ascii_alphabetic)
            {
                let (name, after) = read_name(html, i + 2);
                i = memchr(b'>', bytes, after).map_or(bytes.len(), |e| e + 1);
                builder.end(&name);
            } else if bytes.get(i + 1).is_some_and(u8::is_ascii_alphabetic) {
                let (name, attrs, self_closing, after) = read_tag(html, i + 1);
                i = after;
                if RAW_TEXT.contains(&name.as_str()) {
                    let close = find_end_tag(bytes, &name, i);
                    let raw = &html[i..close.unwrap_or(bytes.len())];
                    if name == "title" && builder.tree.title.is_none() {
                        builder.tree.title = Some(collapse_spaces(&decode_entities(raw)));
                    }
                    // raw content is never shown, so only the element is kept, as a marker
                    builder.start(name.clone(), attrs, true)?;
                    i = close
                        .and_then(|c| memchr(b'>', bytes, c))
                        .map_or(bytes.len(), |e| e + 1);
                } else {
                    let void = self_closing || VOID.contains(&name.as_str());
                    builder.start(name, attrs, void)?;
                }
            } else {
                // a `<` that starts no tag, such as in `a < b`, is text
                let end = memchr(b'<', bytes, i + 1).unwrap_or(bytes.len());
                builder.text(decode_entities(&html[i..end]))?;
                i = end;
            }
        }
        Ok(builder.tree)
    }
}

struct Builder {
    tree: Tree,
    stack: Vec<usize>,
}

impl Builder {
    fn top(&self) -> usize {
        self.stack.last().copied().unwrap_or(0)
    }

    fn add(&mut self, kind: NodeKind) -> Result<usize, ClipError> {
        if self.tree.nodes.len() >= MAX_NODES {
            return Err(ClipError::TooLarge {
                what: "page's element count",
                size: self.tree.nodes.len() as u64,
                limit: MAX_NODES as u64,
            });
        }
        let parent = self.top();
        let id = self.tree.nodes.len();
        self.tree.nodes.push(Node {
            kind,
            parent,
            children: Vec::new(),
        });
        self.tree.nodes[parent].children.push(id);
        Ok(id)
    }

    fn text(&mut self, text: String) -> Result<(), ClipError> {
        if text.is_empty() {
            return Ok(());
        }
        let parent = self.top();
        if let Some(&last) = self.tree.nodes[parent].children.last()
            && let NodeKind::Text(existing) = &mut self.tree.nodes[last].kind
        {
            existing.push_str(&text);
            return Ok(());
        }
        self.add(NodeKind::Text(text))?;
        Ok(())
    }

    /// Pops up to and including the nearest open element named in `names`, unless an element
    /// named in `boundary` is met first.
    fn close_nearest(&mut self, names: &[&str], boundary: &[&str]) {
        for depth in (1..self.stack.len()).rev() {
            let name = self.tree.name(self.stack[depth]);
            if names.contains(&name) {
                self.stack.truncate(depth);
                return;
            }
            if boundary.contains(&name) {
                return;
            }
        }
    }

    fn start(
        &mut self,
        name: String,
        attrs: Vec<(String, String)>,
        void: bool,
    ) -> Result<(), ClipError> {
        // the end tags HTML lets authors leave out
        match name.as_str() {
            "li" => self.close_nearest(&["li"], &["ul", "ol", "menu", "table"]),
            "dt" | "dd" => self.close_nearest(&["dt", "dd"], &["dl", "table"]),
            "tr" => self.close_nearest(&["tr"], &["table"]),
            "td" | "th" => self.close_nearest(&["td", "th"], &["tr", "table"]),
            "thead" | "tbody" | "tfoot" => {
                self.close_nearest(&["thead", "tbody", "tfoot"], &["table"])
            }
            "option" => self.close_nearest(&["option"], &["select"]),
            _ => {}
        }
        if CLOSES_P.contains(&name.as_str()) {
            // an open paragraph ends here, as long as only inline elements sit inside it
            for depth in (1..self.stack.len()).rev() {
                let open = self.tree.name(self.stack[depth]);
                if open == "p" {
                    self.stack.truncate(depth);
                    break;
                }
                if !INLINE.contains(&open) {
                    break;
                }
            }
        }
        let id = self.add(NodeKind::Element { name, attrs })?;
        // past the depth limit, deeper content joins the deepest element kept
        if !void && self.stack.len() <= MAX_DEPTH {
            self.stack.push(id);
        }
        Ok(())
    }

    fn end(&mut self, name: &str) {
        for depth in (1..self.stack.len()).rev() {
            if self.tree.name(self.stack[depth]) == name {
                self.stack.truncate(depth);
                return;
            }
        }
        // an end tag with nothing open to close is ignored, as a browser does
    }
}

fn memchr(needle: u8, bytes: &[u8], from: usize) -> Option<usize> {
    bytes
        .get(from..)?
        .iter()
        .position(|&b| b == needle)
        .map(|p| p + from)
}

fn find(bytes: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Where `</name` starts, in any letter case, from `from`.
fn find_end_tag(bytes: &[u8], name: &str, from: usize) -> Option<usize> {
    let name = name.as_bytes();
    let mut at = from;
    while let Some(lt) = find(bytes, b"</", at) {
        let candidate = bytes.get(lt + 2..lt + 2 + name.len());
        let after = bytes.get(lt + 2 + name.len()).copied();
        if candidate.is_some_and(|c| c.eq_ignore_ascii_case(name))
            && after.is_none_or(|b| b == b'>' || b == b'/' || b.is_ascii_whitespace())
        {
            return Some(lt);
        }
        at = lt + 2;
    }
    None
}

/// Reads a tag or attribute name from `from`, lowercased.
fn read_name(html: &str, from: usize) -> (String, usize) {
    let bytes = html.as_bytes();
    let mut end = from;
    while end < bytes.len()
        && !bytes[end].is_ascii_whitespace()
        && !matches!(bytes[end], b'>' | b'/' | b'=')
    {
        end += 1;
    }
    (html[from..end].to_ascii_lowercase(), end)
}

type Tag = (String, Vec<(String, String)>, bool, usize);

/// Reads a start tag from just after its `<`: the name, the attributes, whether it closes itself
/// and where the tag ends.
fn read_tag(html: &str, from: usize) -> Tag {
    let bytes = html.as_bytes();
    let (name, mut i) = read_name(html, from);
    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i) {
            None => return (name, attrs, self_closing, i),
            Some(b'>') => return (name, attrs, self_closing, i + 1),
            Some(b'/') => {
                self_closing = bytes.get(i + 1) == Some(&b'>');
                i += 1;
                continue;
            }
            Some(b'=') => {
                // a stray `=` with no name before it
                i += 1;
                continue;
            }
            _ => {}
        }
        self_closing = false;
        let (key, after) = read_name(html, i);
        i = after;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            match bytes.get(i) {
                Some(&quote @ (b'"' | b'\'')) => {
                    let end = memchr(quote, bytes, i + 1).unwrap_or(bytes.len());
                    value = decode_entities(&html[i + 1..end]);
                    i = (end + 1).min(bytes.len());
                }
                _ => {
                    let start = i;
                    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                        i += 1;
                    }
                    value = decode_entities(&html[start..i]);
                }
            }
        }
        // a tag with thousands of attributes is junk. Checking each for a repeat would be slow
        if !key.is_empty() && attrs.len() < MAX_ATTRIBUTES && !attrs.iter().any(|(k, _)| *k == key)
        {
            attrs.push((key, value));
        }
    }
}

/// Character references such as `&amp;` and `&#8217;` as the characters they stand for.
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match decode_one(rest) {
            Some((c, used)) => {
                out.push(c);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// One reference at the start of `text`, with how many bytes it used.
fn decode_one(text: &str) -> Option<(char, usize)> {
    let body = &text[1..];
    if let Some(number) = body.strip_prefix('#') {
        let (digits, radix, skip) = match number.strip_prefix(['x', 'X']) {
            Some(hex) => (hex, 16, 3),
            None => (number, 10, 2),
        };
        let len = digits
            .bytes()
            .take_while(|b| b.is_ascii_digit() || (radix == 16 && b.is_ascii_hexdigit()))
            .count();
        if len == 0 || len > 8 {
            return None;
        }
        let value = u32::from_str_radix(&digits[..len], radix).ok()?;
        let c = match value {
            0 => '\u{fffd}',
            v => char::from_u32(v).unwrap_or('\u{fffd}'),
        };
        let semicolon = usize::from(digits[len..].starts_with(';'));
        return Some((c, skip + len + semicolon));
    }
    let end = body.find(';').filter(|&e| e <= 10)?;
    let c = match &body[..end] {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "copy" => '\u{a9}',
        "reg" => '\u{ae}',
        "trade" => '\u{2122}',
        "hellip" => '\u{2026}',
        "mdash" => '\u{2014}',
        "ndash" => '\u{2013}',
        "lsquo" => '\u{2018}',
        "rsquo" => '\u{2019}',
        "ldquo" => '\u{201c}',
        "rdquo" => '\u{201d}',
        "laquo" => '\u{ab}',
        "raquo" => '\u{bb}',
        "bull" => '\u{2022}',
        "middot" => '\u{b7}',
        "deg" => '\u{b0}',
        "times" => '\u{d7}',
        "divide" => '\u{f7}',
        "plusmn" => '\u{b1}',
        "minus" => '\u{2212}',
        "pound" => '\u{a3}',
        "euro" => '\u{20ac}',
        "cent" => '\u{a2}',
        "sect" => '\u{a7}',
        "para" => '\u{b6}',
        "micro" => '\u{b5}',
        "frac12" => '\u{bd}',
        "frac14" => '\u{bc}',
        "frac34" => '\u{be}',
        "sup2" => '\u{b2}',
        "sup3" => '\u{b3}',
        "larr" => '\u{2190}',
        "rarr" => '\u{2192}',
        "le" => '\u{2264}',
        "ge" => '\u{2265}',
        "ne" => '\u{2260}',
        "asymp" => '\u{2248}',
        "infin" => '\u{221e}',
        "pi" => '\u{3c0}',
        "Omega" => '\u{3a9}',
        "omega" => '\u{3c9}',
        "alpha" => '\u{3b1}',
        "beta" => '\u{3b2}',
        "mu" => '\u{3bc}',
        "shy" | "zwj" | "zwnj" => '\u{200b}',
        _ => return None,
    };
    Some((c, end + 2))
}

// --- leaving out page furniture --------------------------------------------------------------

/// Elements that are never content.
const NEVER_CONTENT: &[&str] = &[
    "script", "style", "noscript", "template", "head", "meta", "link", "base", "title", "iframe",
    "frame", "frameset", "object", "embed", "canvas", "input", "select", "textarea", "button",
    "option", "datalist", "audio", "video", "source", "track", "map", "dialog",
];
/// Elements that hold page furniture when the whole page was sent.
const FURNITURE: &[&str] = &["nav", "aside", "footer", "form", "menu", "svg"];
/// ARIA roles of page furniture.
const FURNITURE_ROLES: &[&str] = &[
    "navigation",
    "contentinfo",
    "complementary",
    "search",
    "dialog",
    "alertdialog",
    "menu",
    "menubar",
    "toolbar",
    "banner",
];
/// Words in a class, id or label that mark page furniture.
const FURNITURE_HINTS: &[&str] = &[
    "ad",
    "ads",
    "adsbygoogle",
    "advert",
    "adverts",
    "advertisement",
    "advertising",
    "sponsor",
    "sponsored",
    "promo",
    "promoted",
    "cookie",
    "cookies",
    "consent",
    "gdpr",
    "newsletter",
    "subscribe",
    "share",
    "sharing",
    "social",
    "related",
    "recommended",
    "sidebar",
    "breadcrumb",
    "breadcrumbs",
    "popup",
    "modal",
    "overlay",
    "nav",
    "navbar",
    "navigation",
    "menu",
    "footer",
    "masthead",
    "comments",
    "outbrain",
    "taboola",
    "skip",
    "paywall",
];
/// Words in a class or id that mark content, which outweigh a furniture word on a large block.
const CONTENT_HINTS: &[&str] = &[
    "article", "body", "content", "main", "post", "entry", "story", "text", "prose",
];
/// A block with content words keeps its place when it holds at least this much text.
const LARGE_BLOCK_CHARS: usize = 400;

fn hint_words(tree: &Tree, id: usize) -> Vec<String> {
    let mut words = Vec::new();
    for key in ["class", "id", "aria-label"] {
        if let Some(value) = tree.attr(id, key) {
            words.extend(
                value
                    .split(|c: char| !c.is_ascii_alphanumeric())
                    .filter(|w| !w.is_empty())
                    .map(str::to_ascii_lowercase),
            );
        }
    }
    words
}

fn is_hidden(tree: &Tree, id: usize) -> bool {
    if tree.attr(id, "hidden").is_some()
        || tree
            .attr(id, "aria-hidden")
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
        || tree
            .attr(id, "type")
            .is_some_and(|v| v.eq_ignore_ascii_case("hidden"))
    {
        return true;
    }
    tree.attr(id, "style").is_some_and(|style| {
        let style: String = style
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        style.contains("display:none") || style.contains("visibility:hidden")
    })
}

/// Marks every node that is left out. Text that is visually hidden but there for screen readers,
/// such as an `sr-only` label, is kept: it was written for exactly the readers a clip is for.
fn left_out(tree: &Tree, selection_only: bool) -> Vec<bool> {
    let count = tree.nodes.len();
    let mut dropped = vec![false; count];
    // whether an article or main element encloses the node, where a header is the article's own
    let mut in_content = vec![false; count];
    let mut text_len = vec![0usize; count];
    for id in (1..count).rev() {
        let own = match &tree.nodes[id].kind {
            NodeKind::Text(t) => t.trim().len(),
            NodeKind::Element { .. } => 0,
        };
        text_len[id] += own;
        let parent = tree.nodes[id].parent;
        text_len[parent] += text_len[id];
    }
    for id in 1..count {
        let parent = tree.nodes[id].parent;
        if dropped[parent] {
            dropped[id] = true;
            continue;
        }
        let name = tree.name(id);
        in_content[id] = in_content[parent] || matches!(name, "article" | "main");
        if name.is_empty() || matches!(name, "html" | "body" | "main" | "article") {
            continue;
        }
        if NEVER_CONTENT.contains(&name) || is_hidden(tree, id) {
            dropped[id] = true;
            continue;
        }
        if selection_only {
            continue;
        }
        let role = tree.attr(id, "role").map(str::to_ascii_lowercase);
        let header_like = name == "header" || role.as_deref() == Some("banner");
        if header_like {
            dropped[id] = !in_content[parent];
            continue;
        }
        if FURNITURE.contains(&name) || role.is_some_and(|r| FURNITURE_ROLES.contains(&r.as_str()))
        {
            dropped[id] = true;
            continue;
        }
        if tree.nodes[id].has_attr_prefix("data-ad") {
            dropped[id] = true;
            continue;
        }
        let words = hint_words(tree, id);
        let furniture = words.iter().any(|w| FURNITURE_HINTS.contains(&w.as_str()));
        if furniture {
            let content = words.iter().any(|w| CONTENT_HINTS.contains(&w.as_str()));
            dropped[id] = !(content && text_len[id] >= LARGE_BLOCK_CHARS);
        }
    }
    dropped
}

impl Node {
    fn has_attr_prefix(&self, prefix: &str) -> bool {
        match &self.kind {
            NodeKind::Element { attrs, .. } => attrs.iter().any(|(k, _)| k.starts_with(prefix)),
            NodeKind::Text(_) => false,
        }
    }
}

// --- finding the main content ----------------------------------------------------------------

/// The element holding the main content: a `main` element, then an element with the main role,
/// then the largest `article` when it holds a fair share of the page, then the block with the
/// most paragraph text.
fn main_content(tree: &Tree, dropped: &[bool]) -> usize {
    let count = tree.nodes.len();
    let mut text_len = vec![0usize; count];
    for id in (1..count).rev() {
        if dropped[id] {
            continue;
        }
        if let NodeKind::Text(t) = &tree.nodes[id].kind {
            text_len[id] = t.split_whitespace().map(str::len).sum();
        }
        let parent = tree.nodes[id].parent;
        text_len[parent] += text_len[id];
    }
    let total = text_len[0].max(1);
    let largest = |pick: &dyn Fn(usize) -> bool| -> Option<usize> {
        (1..count)
            .filter(|&id| !dropped[id] && pick(id) && text_len[id] > 0)
            .max_by(|&a, &b| text_len[a].cmp(&text_len[b]).then(b.cmp(&a)))
    };
    if let Some(main) = largest(&|id| tree.name(id) == "main") {
        return main;
    }
    if let Some(main) = largest(&|id| tree.attr(id, "role") == Some("main")) {
        return main;
    }
    if let Some(article) = largest(&|id| tree.name(id) == "article")
        && text_len[article] * 4 >= total
    {
        return article;
    }

    // each paragraph scores its parent fully and its grandparent by half, so the block that holds
    // most of the running text wins over a wrapper around everything
    let mut score = vec![0usize; count];
    for id in 1..count {
        if dropped[id] || !matches!(tree.name(id), "p" | "pre" | "blockquote") {
            continue;
        }
        let len = text_len[id];
        if len < 25 {
            continue;
        }
        let parent = tree.nodes[id].parent;
        score[parent] += len;
        let grandparent = tree.nodes[parent].parent;
        if grandparent != parent {
            score[grandparent] += len / 2;
        }
    }
    let best = (1..count)
        .filter(|&id| !dropped[id] && score[id] > 0)
        .max_by(|&a, &b| score[a].cmp(&score[b]).then(b.cmp(&a)));
    match best {
        Some(id) => id,
        None => (1..count)
            .find(|&id| tree.name(id) == "body" && !dropped[id])
            .unwrap_or(0),
    }
}

// --- building the document -------------------------------------------------------------------

/// Makes a link whole against the page address. Script, data and other non-web addresses are not
/// kept as links, so they can never be followed from a clip.
pub(crate) fn resolve_link(base: Option<&str>, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.chars().any(char::is_control) {
        return None;
    }
    let lower = href.to_ascii_lowercase();
    if ["http://", "https://", "mailto:", "tel:"]
        .iter()
        .any(|s| lower.starts_with(s))
    {
        return Some(href.to_string());
    }
    let scheme_end = href.find(':');
    let has_scheme = scheme_end.is_some_and(|end| {
        end > 0
            && href[..end].starts_with(|c: char| c.is_ascii_alphabetic())
            && href[..end]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    if has_scheme {
        return None;
    }
    let base = base?;
    let base_lower = base.to_ascii_lowercase();
    if !(base_lower.starts_with("http://") || base_lower.starts_with("https://")) {
        return None;
    }
    let scheme = &base[..base.find(':')?];
    let after_scheme = base.find("://")? + 3;
    let authority_end = base[after_scheme..]
        .find(['/', '?', '#'])
        .map_or(base.len(), |e| e + after_scheme);
    let origin = &base[..authority_end];
    let without_fragment = &base[..base.find('#').unwrap_or(base.len())];
    let without_query =
        &without_fragment[..without_fragment.find('?').unwrap_or(without_fragment.len())];
    if let Some(rest) = href.strip_prefix("//") {
        return Some(format!("{scheme}://{rest}"));
    }
    if href.starts_with('#') {
        return Some(format!("{without_fragment}{href}"));
    }
    if href.starts_with('?') {
        return Some(format!("{without_query}{href}"));
    }
    let path = if href.starts_with('/') {
        href.to_string()
    } else {
        let base_path = &without_query[authority_end.min(without_query.len())..];
        let dir = &base_path[..base_path.rfind('/').map_or(0, |e| e + 1)];
        let dir = if dir.is_empty() { "/" } else { dir };
        format!("{dir}{href}")
    };
    Some(format!("{origin}{}", remove_dot_segments(&path)))
}

/// Resolves `.` and `..` in a path, keeping any query or fragment as written.
fn remove_dot_segments(path: &str) -> String {
    let split = path.find(['?', '#']).unwrap_or(path.len());
    let (path_part, tail) = path.split_at(split);
    let mut out: Vec<&str> = Vec::new();
    let segments: Vec<&str> = path_part.split('/').collect();
    for (i, segment) in segments.iter().enumerate() {
        let last = i == segments.len() - 1;
        match *segment {
            "." => {
                if last {
                    out.push("");
                }
            }
            ".." => {
                if out.len() > 1 {
                    out.pop();
                }
                if last {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    let mut joined = out.join("/");
    if !joined.starts_with('/') {
        joined.insert(0, '/');
    }
    format!("{joined}{tail}")
}

#[derive(Clone, Default, PartialEq)]
struct Style {
    link: Option<String>,
    script: Option<Script>,
}

struct Converter<'a> {
    tree: &'a Tree,
    dropped: &'a [bool],
    base: Option<&'a str>,
    blocks: Vec<BlockKind>,
    /// Inline content waiting to become a paragraph.
    runs: Vec<Run>,
    /// How many quotations enclose the content being read.
    quotes: u32,
    /// A line break was the last thing read, so a second one ends the paragraph.
    after_break: bool,
}

/// Adds text to a list of runs, joining it to the last run when it has the same link and script,
/// and keeping only one space where two pieces meet.
fn push_text(runs: &mut Vec<Run>, text: &str, style: &Style) {
    let mut text = collapse_whitespace(text);
    if text.is_empty() {
        return;
    }
    let ends_in_space = runs
        .last()
        .is_none_or(|r| r.text.ends_with(' ') || r.text.is_empty());
    if ends_in_space && text.starts_with(' ') {
        text.remove(0);
        if text.is_empty() {
            return;
        }
    }
    if let Some(last) = runs.last_mut()
        && (text.trim().is_empty() || (last.link == style.link && last.script == style.script))
    {
        last.text.push_str(&text);
        return;
    }
    let mut run = clip_run(text);
    run.link = style.link.clone();
    run.script = style.script;
    runs.push(run);
}

/// Every stretch of whitespace as one space, keeping a space at either end when there was one.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c == '\u{200b}' {
            // a soft hyphen or joiner marks where a word may break, so the word stays whole
            continue;
        }
        if c.is_whitespace() {
            space = true;
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

/// Trims the runs' outer spaces and drops empty runs. Returns nothing when no text is left.
fn finish_runs(mut runs: Vec<Run>) -> Option<Vec<Run>> {
    runs.retain(|r| !r.text.trim().is_empty());
    if let Some(first) = runs.first_mut() {
        first.text = first.text.trim_start().to_string();
    }
    if let Some(last) = runs.last_mut() {
        last.text = last.text.trim_end().to_string();
    }
    (!runs.is_empty()).then_some(runs)
}

fn is_heading(name: &str) -> Option<u8> {
    match name {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

impl Converter<'_> {
    fn flush(&mut self) {
        let runs = std::mem::take(&mut self.runs);
        self.after_break = false;
        if let Some(runs) = finish_runs(runs) {
            let role = if self.quotes > 0 {
                TextRole::Quote
            } else {
                TextRole::Body
            };
            self.blocks.push(BlockKind::Paragraph { runs, role });
        }
    }

    fn style_for(&self, id: usize, style: &Style) -> Style {
        let mut style = style.clone();
        match self.tree.name(id) {
            "a" => {
                if let Some(href) = self.tree.attr(id, "href") {
                    style.link = resolve_link(self.base, href);
                }
            }
            "sub" => style.script = Some(Script::Subscript),
            "sup" => style.script = Some(Script::Superscript),
            _ => {}
        }
        style
    }

    /// The image block for an `img`. A decorative image or a tracking pixel gives nothing.
    fn image(&self, id: usize) -> Option<BlockKind> {
        let tiny = |key| {
            self.tree
                .attr(id, key)
                .is_some_and(|v| matches!(v.trim(), "0" | "1" | "0px" | "1px"))
        };
        if tiny("width") || tiny("height") {
            return None;
        }
        if self.tree.attr(id, "role") == Some("presentation") {
            return None;
        }
        let alt_text = match self.tree.attr(id, "alt") {
            // an empty description marks a decorative image, which a reader skips
            Some(alt) if alt.trim().is_empty() => return None,
            Some(alt) => Some(collapse_spaces(alt)),
            None => self
                .tree
                .attr(id, "aria-label")
                .or_else(|| self.tree.attr(id, "title"))
                .map(collapse_spaces)
                .filter(|t| !t.is_empty()),
        };
        // the picture is never fetched, so the block holds only its description
        Some(BlockKind::Image {
            alt_text,
            asset: None,
        })
    }

    /// Reads a node as block content.
    fn block(&mut self, id: usize, style: &Style, depth: usize) {
        if self.dropped[id] || depth > MAX_DEPTH + 2 {
            return;
        }
        let name = match &self.tree.nodes[id].kind {
            NodeKind::Text(text) => {
                self.after_break = self.after_break && text.trim().is_empty();
                push_text(&mut self.runs, text, style);
                return;
            }
            NodeKind::Element { name, .. } => name.as_str(),
        };
        if let Some(level) = is_heading(name) {
            self.flush();
            let mut runs = Vec::new();
            self.inline(id, style, &mut runs, depth);
            if let Some(runs) = finish_runs(runs)
                && let Ok(heading) = BlockKind::heading(level, runs)
            {
                self.blocks.push(heading);
            }
            return;
        }
        match name {
            "br" => {
                if self.after_break {
                    self.flush();
                } else {
                    push_text(&mut self.runs, " ", style);
                    self.after_break = true;
                }
            }
            "hr" => self.flush(),
            "img" => {
                if let Some(image) = self.image(id) {
                    self.flush();
                    self.blocks.push(image);
                }
            }
            "ul" | "ol" | "menu" => {
                self.flush();
                let items = self.list_items(id, style, depth, 0);
                if !items.is_empty() {
                    self.blocks.push(BlockKind::List {
                        ordered: name == "ol",
                        items,
                    });
                }
            }
            "table" if self.is_layout_table(id) => self.children_as_blocks(id, style, depth),
            "table" => {
                self.flush();
                if let Some(table) = self.table(id, style, depth) {
                    self.blocks.push(table);
                }
            }
            "pre" => {
                self.flush();
                let mut text = String::new();
                self.raw_text(id, &mut text, depth);
                let text = text.strip_prefix('\n').unwrap_or(&text).trim_end();
                if !text.trim().is_empty() {
                    self.blocks.push(BlockKind::Paragraph {
                        runs: vec![clip_run(text)],
                        role: TextRole::Code,
                    });
                }
            }
            "blockquote" => {
                self.flush();
                self.quotes += 1;
                self.children_as_blocks(id, style, depth);
                self.quotes -= 1;
            }
            n if INLINE.contains(&n) => {
                let style = self.style_for(id, style);
                for &child in &self.tree.nodes[id].children {
                    self.block(child, &style, depth + 1);
                }
            }
            _ => self.children_as_blocks(id, style, depth),
        }
    }

    fn children_as_blocks(&mut self, id: usize, style: &Style, depth: usize) {
        self.flush();
        for &child in &self.tree.nodes[id].children {
            self.block(child, style, depth + 1);
        }
        self.flush();
    }

    /// Reads a node as inline text only, as in a heading, a list item or a table cell. Blocks
    /// inside become spaces and images their descriptions.
    fn inline(&self, id: usize, style: &Style, runs: &mut Vec<Run>, depth: usize) {
        if self.dropped[id] || depth > MAX_DEPTH + 2 {
            return;
        }
        match &self.tree.nodes[id].kind {
            NodeKind::Text(text) => push_text(runs, text, style),
            NodeKind::Element { name, .. } => {
                if name == "img" {
                    if let Some(BlockKind::Image {
                        alt_text: Some(alt),
                        ..
                    }) = self.image(id)
                    {
                        push_text(runs, &format!(" {alt} "), style);
                    }
                    return;
                }
                let inline = INLINE.contains(&name.as_str());
                if !inline {
                    push_text(runs, " ", style);
                }
                let style = self.style_for(id, style);
                for &child in &self.tree.nodes[id].children {
                    self.inline(child, &style, runs, depth + 1);
                }
                if !inline {
                    push_text(runs, " ", &style);
                }
            }
        }
    }

    /// The text of a `pre` element exactly as written, with line breaks for `br`.
    fn raw_text(&self, id: usize, out: &mut String, depth: usize) {
        if self.dropped[id] || depth > MAX_DEPTH + 2 {
            return;
        }
        match &self.tree.nodes[id].kind {
            NodeKind::Text(text) => out.push_str(text),
            NodeKind::Element { name, .. } => {
                if name == "br" {
                    out.push('\n');
                }
                for &child in &self.tree.nodes[id].children {
                    self.raw_text(child, out, depth + 1);
                }
            }
        }
    }

    /// The items of a list at nesting `level`. Lists nested deeper than [`MAX_LIST_LEVELS`] join
    /// the deepest level kept, so the document stays within what its JSON form can be read back at.
    fn list_items(&self, id: usize, style: &Style, depth: usize, level: usize) -> Vec<ListItem> {
        let mut items = Vec::new();
        let nest = level + 1 < MAX_LIST_LEVELS;
        for &child in &self.tree.nodes[id].children {
            if self.dropped[child] {
                continue;
            }
            match self.tree.name(child) {
                "li" => {
                    let mut runs = Vec::new();
                    let mut children = Vec::new();
                    for &part in &self.tree.nodes[child].children {
                        match self.tree.name(part) {
                            "ul" | "ol" | "menu" if !self.dropped[part] => {
                                children.extend(self.list_items(
                                    part,
                                    style,
                                    depth + 2,
                                    level + usize::from(nest),
                                ));
                            }
                            _ => self.inline(part, style, &mut runs, depth + 2),
                        }
                    }
                    if !nest {
                        let flat = std::mem::take(&mut children);
                        if let Some(runs) = finish_runs(runs) {
                            items.push(ListItem::new(runs));
                        }
                        items.extend(flat);
                        continue;
                    }
                    match finish_runs(runs) {
                        Some(runs) => items.push(ListItem { runs, children }),
                        // an item holding only a nested list still keeps that list
                        None if !children.is_empty() => items.push(ListItem {
                            runs: vec![clip_run("")],
                            children,
                        }),
                        None => {}
                    }
                }
                // a list nested straight inside a list belongs to the item before it
                "ul" | "ol" | "menu" => {
                    let nested =
                        self.list_items(child, style, depth + 1, level + usize::from(nest));
                    match items.last_mut() {
                        Some(last) if nest => last.children.extend(nested),
                        _ => items.extend(nested),
                    }
                }
                _ => {
                    // loose text or anything else inside a list is read as one more item
                    let mut runs = Vec::new();
                    self.inline(child, style, &mut runs, depth + 1);
                    if let Some(runs) = finish_runs(runs) {
                        items.push(ListItem::new(runs));
                    }
                }
            }
        }
        items
    }

    /// A table used only to place things on the page rather than to hold data.
    fn is_layout_table(&self, id: usize) -> bool {
        if matches!(
            self.tree
                .attr(id, "role")
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("presentation" | "none")
        ) {
            return true;
        }
        let mut cells = 0;
        self.count_cells(id, &mut cells, 0);
        cells <= 1
    }

    fn count_cells(&self, id: usize, cells: &mut usize, depth: usize) {
        if depth > 3 || *cells > 1 {
            return;
        }
        for &child in &self.tree.nodes[id].children {
            match self.tree.name(child) {
                "td" | "th" => *cells += 1,
                "tr" | "thead" | "tbody" | "tfoot" => self.count_cells(child, cells, depth + 1),
                _ => {}
            }
        }
    }

    fn table(&self, id: usize, style: &Style, depth: usize) -> Option<BlockKind> {
        let mut rows = Vec::new();
        let mut caption = Vec::new();
        let mut row_nodes: Vec<(usize, bool)> = Vec::new();
        for &child in &self.tree.nodes[id].children {
            if self.dropped[child] {
                continue;
            }
            match self.tree.name(child) {
                "caption" => self.inline(child, style, &mut caption, depth + 1),
                "tr" => row_nodes.push((child, false)),
                section @ ("thead" | "tbody" | "tfoot") => {
                    for &row in &self.tree.nodes[child].children {
                        if self.tree.name(row) == "tr" && !self.dropped[row] {
                            row_nodes.push((row, section == "thead"));
                        }
                    }
                }
                _ => {}
            }
        }
        for (row, in_head) in row_nodes {
            let mut cells = Vec::new();
            for &cell in &self.tree.nodes[row].children {
                let name = self.tree.name(cell);
                if !matches!(name, "td" | "th") || self.dropped[cell] {
                    continue;
                }
                let mut runs = Vec::new();
                self.inline(cell, style, &mut runs, depth + 3);
                let span = |key| {
                    self.tree
                        .attr(cell, key)
                        .and_then(|v| v.trim().parse::<u32>().ok())
                        .map_or(1, |v| v.clamp(1, 1000))
                };
                let mut table_cell = TableCell::new(finish_runs(runs).unwrap_or_default());
                table_cell.header = name == "th" || in_head;
                table_cell.col_span = span("colspan");
                table_cell.row_span = span("rowspan");
                cells.push(table_cell);
            }
            if !cells.is_empty() {
                rows.push(TableRow { cells });
            }
        }
        if rows.is_empty() {
            return None;
        }
        Some(BlockKind::Table {
            rows,
            caption: finish_runs(caption).unwrap_or_default(),
        })
    }
}

/// Turns a page or a selection into a document. Returns `None` when nothing readable is left once
/// the page furniture is gone.
pub fn html_to_document(
    html: &str,
    options: &ClipOptions<'_>,
) -> Result<Option<Document>, ClipError> {
    if html.len() > super::MAX_HTML_BYTES {
        return Err(ClipError::TooLarge {
            what: "page",
            size: html.len() as u64,
            limit: super::MAX_HTML_BYTES as u64,
        });
    }
    let tree = Tree::parse(html)?;
    let dropped = left_out(&tree, options.selection_only);
    let root = if options.selection_only {
        0
    } else {
        main_content(&tree, &dropped)
    };
    let mut converter = Converter {
        tree: &tree,
        dropped: &dropped,
        base: options.url,
        blocks: Vec::new(),
        runs: Vec::new(),
        quotes: 0,
        after_break: false,
    };
    // the root is read as a block too, since the content found may itself be a list or a table
    converter.block(root, &Style::default(), 0);
    converter.flush();
    let blocks = converter.blocks;
    let readable = blocks.iter().any(|b| match b {
        BlockKind::Image { alt_text, .. } => alt_text.is_some(),
        _ => true,
    });
    if !readable {
        return Ok(None);
    }

    let first_heading = blocks.iter().find_map(|b| match b {
        BlockKind::Heading { runs, .. } => Some(collapse_spaces(&crate::model::join_runs(runs))),
        _ => None,
    });
    let title = options
        .title
        .map(collapse_spaces)
        .filter(|t| !t.is_empty())
        .or_else(|| tree.title.clone().filter(|t| !t.is_empty()))
        .or(first_heading)
        .or_else(|| options.url.map(str::to_string))
        .unwrap_or_default();
    let mut document = Document::new(
        shorten(&title, TITLE_CHARS),
        Source {
            file_name: options.url.unwrap_or("page.html").to_string(),
            format: SourceFormat::Html,
        },
    );
    document.blocks = numbered_blocks(blocks);
    Ok(Some(document))
}
