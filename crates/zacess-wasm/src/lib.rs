//! The parts of the Zacess engine the browser version uses, compiled to WebAssembly.
//!
//! Everything here takes and returns plain strings and bytes, with documents and profiles in
//! their documented JSON forms, so the same functions are tested natively and called from the
//! page through the thin bindings in `bindings.rs`. The browser does the work only a browser can
//! do (pdf.js reads the text layer, tesseract.js recognises images, mammoth turns a Word file into
//! HTML) and hands the result to the core, which builds the document, applies the profile and
//! writes the exports exactly as the native builds do.

#[cfg(target_arch = "wasm32")]
mod bindings;
pub mod links;

use serde::{Deserialize, Serialize};
use zacess_core::clip::html::{ClipOptions, html_to_document};
use zacess_core::clip::text::text_to_document;
use zacess_core::confidence::uncertain_runs;
use zacess_core::export::html::{render_body, stylesheet};
use zacess_core::export::{ExportOptions, to_epub, to_html};
use zacess_core::import::layout::Fragment;
use zacess_core::import::ocr::{self, OcrLine, OcrPage, OcrParagraph, OcrWord};
use zacess_core::import::pdf::PdfText;
use zacess_core::model::{BlockKind, Confidence, Document, Page, SourceFormat, Unit};
use zacess_core::profile::{Preset, ReadingProfile};

/// Every error crosses into the page as a sentence it can show.
pub type Result<T> = std::result::Result<T, String>;

fn text_error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn read_document(json: &str) -> Result<Document> {
    Document::from_json(json).map_err(|e| format!("The document could not be read: {e}"))
}

fn read_profile(json: &str) -> Result<ReadingProfile> {
    let profile = ReadingProfile::from_json(json).map_err(text_error)?;
    profile.validate().map_err(text_error)?;
    Ok(profile)
}

fn write_document(document: &Document) -> Result<String> {
    document.to_json().map_err(text_error)
}

/// The file name without its extension, for a title when the content gives none.
fn stem(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => file_name,
    }
}

// --- reading profiles --------------------------------------------------------------------------

#[derive(Serialize)]
struct PresetEntry {
    id: Preset,
    name: &'static str,
    profile: ReadingProfile,
}

/// The standard profile in its JSON form.
pub fn default_profile() -> String {
    serde_json::to_string(&ReadingProfile::default()).expect("a profile always serialises")
}

/// Every preset as `{ id, name, profile }`, in the order the core lists them.
pub fn presets() -> String {
    let entries: Vec<PresetEntry> = Preset::ALL
        .iter()
        .map(|&p| PresetEntry {
            id: p,
            name: p.name(),
            profile: ReadingProfile::preset(p),
        })
        .collect();
    serde_json::to_string(&entries).expect("presets always serialise")
}

/// A profile read and checked by the core, written back in its current form. A profile saved by
/// an older build is upgraded and one with impossible settings is refused with the reasons.
pub fn check_profile(json: &str) -> Result<String> {
    let profile = read_profile(json)?;
    profile.to_json().map_err(text_error)
}

/// The profile's style sheet for the reading view, with every rule kept inside `scope` so it
/// styles the document and never the rest of the app.
pub fn reading_css(profile_json: &str, scope: &str) -> Result<String> {
    let profile = read_profile(profile_json)?;
    Ok(scope_css(&stylesheet(&profile), scope))
}

/// Prefixes every selector with `scope`. The core writes one rule per line, which is what makes
/// a line-based rewrite safe here; a line with no rule on it is kept as it is.
fn scope_css(css: &str, scope: &str) -> String {
    let mut out = String::with_capacity(css.len() + 256);
    for line in css.lines() {
        let Some((selectors, rest)) = line.split_once(" {") else {
            out.push_str(line);
            out.push('\n');
            continue;
        };
        let scoped: Vec<String> = selectors
            .split(',')
            .map(str::trim)
            .map(|sel| match sel {
                "body" | ":root" | "html" => scope.to_string(),
                "*" => format!("{scope}, {scope} *"),
                other => format!("{scope} {other}"),
            })
            .collect();
        out.push_str(&scoped.join(", "));
        out.push_str(" {");
        out.push_str(rest);
        out.push('\n');
    }
    out
}

// --- building documents ------------------------------------------------------------------------

/// Plain text or Markdown as a document.
pub fn document_from_text(text: &str, file_name: &str) -> Result<String> {
    let mut document = text_to_document(text).ok_or("The text is empty.")?;
    links::link_bare_addresses(&mut document);
    document.source.file_name = file_name.to_string();
    document.source.format = SourceFormat::Other;
    if document.title.trim().is_empty() {
        document.title = stem(file_name).to_string();
    }
    write_document(&document)
}

/// HTML as a document, read by the same reader universal capture uses. `format` is `html` for a
/// web page or pasted HTML and `docx` for a Word file already turned into HTML. A whole page is
/// searched for its main content; a fragment (pasted HTML or a Word file) is kept whole.
pub fn document_from_html(
    html: &str,
    file_name: &str,
    format: &str,
    fragment: bool,
) -> Result<String> {
    let format = match format {
        "docx" => SourceFormat::Docx,
        "html" => SourceFormat::Html,
        other => return Err(format!("{other} is not a format read as HTML.")),
    };
    let options = ClipOptions {
        url: None,
        title: None,
        selection_only: fragment,
    };
    let mut document = html_to_document(html, &options)
        .map_err(text_error)?
        .ok_or("Nothing readable was found in the file.")?;
    document.source.file_name = file_name.to_string();
    document.source.format = format;
    if document.title.trim().is_empty() {
        document.title = stem(file_name).to_string();
    }
    write_document(&document)
}

/// A piece of positioned text from a PDF text layer, measured from the top-left corner of the
/// page in PDF points.
#[derive(Debug, Deserialize)]
pub struct TextItem {
    pub x: f32,
    pub baseline: f32,
    pub width: f32,
    pub size: f32,
    #[serde(default)]
    pub bold: bool,
    pub text: String,
}

/// One recognised word, in image pixels, with a confidence from 0 to 100 as OCR engines report it.
#[derive(Debug, Deserialize)]
pub struct WordInput {
    pub text: String,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub confidence: f32,
}

#[derive(Debug, Deserialize)]
pub struct LineInput {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub words: Vec<WordInput>,
}

#[derive(Debug, Deserialize)]
pub struct ParagraphInput {
    pub lines: Vec<LineInput>,
}

/// What OCR found on a page image.
#[derive(Debug, Deserialize)]
pub struct OcrInput {
    /// The size of the image that was recognised, in pixels.
    pub width: f32,
    pub height: f32,
    pub paragraphs: Vec<ParagraphInput>,
}

/// One page as the browser read it: its text layer or what OCR found when it had none.
#[derive(Debug, Deserialize)]
pub struct PageInput {
    pub number: u32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub items: Vec<TextItem>,
    #[serde(default)]
    pub ocr: Option<OcrInput>,
    /// Link areas on the page, from its link annotations.
    #[serde(default)]
    pub links: Vec<links::LinkInput>,
}

/// A PDF or an image as pages of positioned text.
#[derive(Debug, Deserialize)]
pub struct PagesInput {
    pub file_name: String,
    #[serde(default)]
    pub title: String,
    /// `pdf` or `image`.
    pub format: String,
    pub pages: Vec<PageInput>,
}

#[derive(Debug, Serialize)]
struct PagesOutput {
    document: Document,
    pages_needing_ocr: Vec<u32>,
    pages_recognised: Vec<u32>,
}

impl OcrInput {
    fn into_page(self) -> OcrPage {
        OcrPage {
            width: self.width,
            height: self.height,
            paragraphs: self
                .paragraphs
                .into_iter()
                .map(|p| OcrParagraph {
                    lines: p.lines.into_iter().map(LineInput::into_line).collect(),
                })
                .collect(),
        }
    }
}

impl LineInput {
    fn into_line(self) -> OcrLine {
        OcrLine {
            left: self.left,
            top: self.top,
            width: self.width,
            height: self.height,
            words: self
                .words
                .into_iter()
                .filter(|w| !w.text.trim().is_empty())
                .map(|w| OcrWord {
                    text: w.text.trim().to_string(),
                    left: w.left,
                    top: w.top,
                    width: w.width,
                    height: w.height,
                    confidence: Confidence::new(w.confidence / 100.0),
                })
                .collect(),
        }
    }
}

/// Builds a document from pages of positioned text through the core's layout analysis, the same
/// path a PDF or a scan takes on the desktop. A page with OCR results is scaled from image pixels
/// to the page's own units, so a scanned page and a text page in one PDF line up.
///
/// Returns `{ document, pages_needing_ocr, pages_recognised }`.
pub fn document_from_pages(json: &str) -> Result<String> {
    let input: PagesInput =
        serde_json::from_str(json).map_err(|e| format!("The pages could not be read: {e}"))?;
    let (format, unit) = match input.format.as_str() {
        "pdf" => (SourceFormat::Pdf, Unit::Pt),
        "image" => (SourceFormat::Image, Unit::Px),
        other => return Err(format!("{other} is not a format read as pages.")),
    };
    if input.pages.is_empty() {
        return Err("The file has no pages.".into());
    }
    let usable = |v: f32| v.is_finite() && v > 0.0;
    let mut pages = Vec::with_capacity(input.pages.len());
    let mut fragments: Vec<Fragment> = Vec::new();
    let mut recognised = Vec::new();
    let mut all_links = Vec::new();
    for page in input.pages {
        if !usable(page.width) || !usable(page.height) {
            return Err(format!("Page {} has no size.", page.number));
        }
        pages.push(Page {
            number: page.number,
            width: page.width,
            height: page.height,
            unit,
        });
        let page_links: Vec<links::LinkInput> = page
            .links
            .into_iter()
            .filter_map(|l| {
                let usable = [l.x, l.y, l.width, l.height].iter().all(|v| v.is_finite())
                    && l.width > 0.0
                    && l.height > 0.0;
                let url = links::web_link(&l.url).filter(|_| usable)?;
                Some(links::LinkInput { url, ..l })
            })
            .collect();
        let items: Vec<TextItem> = page
            .items
            .into_iter()
            .flat_map(|item| links::split_at_links(item, &page_links))
            .collect();
        all_links.extend(page_links.into_iter().map(|l| (page.number, l)));
        for item in items {
            if item.text.is_empty() || !item.x.is_finite() || !item.baseline.is_finite() {
                continue;
            }
            fragments.push(Fragment {
                page: page.number,
                x: item.x,
                baseline: item.baseline,
                width: if item.width.is_finite() {
                    item.width.max(0.0)
                } else {
                    0.0
                },
                size: if usable(item.size) { item.size } else { 1.0 },
                bold: item.bold,
                text: item.text,
                confidence: Confidence::CERTAIN,
            });
        }
        if let Some(found) = page.ocr {
            let scale = if usable(found.width) {
                page.width / found.width
            } else {
                1.0
            };
            fragments.extend(ocr::fragments(&found.into_page(), page.number, scale));
            recognised.push(page.number);
        }
    }
    let title = if input.title.trim().is_empty() {
        stem(&input.file_name).to_string()
    } else {
        input.title
    };
    let mut imported = PdfText {
        title,
        file_name: input.file_name,
        pages,
        fragments,
        pages_without_text: Vec::new(),
    }
    .into_imported();
    imported.document.source.format = format;
    links::attach_links(&mut imported.document, &all_links);
    // a page OCR ran on is finished even when it was blank
    imported
        .pages_needing_ocr
        .retain(|p| !recognised.contains(p));
    let output = PagesOutput {
        document: imported.document,
        pages_needing_ocr: imported.pages_needing_ocr,
        pages_recognised: recognised,
    };
    serde_json::to_string(&output).map_err(text_error)
}

// --- reading and exporting ---------------------------------------------------------------------

/// Counts the reading view shows beside the document.
#[derive(Debug, Serialize)]
struct Summary {
    title: String,
    blocks: usize,
    pages: usize,
    uncertain: usize,
    images_without_alt: usize,
}

/// What the document holds, for the status line: block and page counts, how many pieces of text
/// need checking and how many images still need a description.
pub fn summary(document_json: &str) -> Result<String> {
    let document = read_document(document_json)?;
    let summary = Summary {
        title: document.title.clone(),
        blocks: document
            .blocks
            .iter()
            .filter(|b| !matches!(b.kind, BlockKind::PageBreak))
            .count(),
        pages: document.pages.len(),
        uncertain: uncertain_runs(&document).count(),
        images_without_alt: document
            .blocks
            .iter()
            .filter(|b| matches!(&b.kind, BlockKind::Image { alt_text: None, .. }))
            .count(),
    };
    serde_json::to_string(&summary).map_err(text_error)
}

/// The document as HTML for the reading view inside the app: the export's own markup, in an
/// `<article>` rather than `<main>` (the app has its main landmark already) and with every heading
/// one level lower, so the app's title stays the only `<h1>`.
pub fn reading_html(document_json: &str, profile_json: &str) -> Result<String> {
    let document = read_document(document_json)?;
    let profile = read_profile(profile_json)?;
    let options = ExportOptions {
        profile: Some(&profile),
        ..ExportOptions::default()
    };
    let body = render_body(&document, &options, false).body;
    Ok(demote_headings(
        &body
            .replacen("<main class=\"zc-doc\">", "<article class=\"zc-doc\">", 1)
            .replace("</main>", "</article>"),
    ))
}

/// Moves every heading down a level, `<h6>` staying `<h6>`. Text is escaped by the renderer, so a
/// `<h` followed by a digit in this string is always markup.
fn demote_headings(html: &str) -> String {
    let mut out = html.to_string();
    for level in (1..=5).rev() {
        out = out
            .replace(&format!("<h{level}"), &format!("<h{}", level + 1))
            .replace(&format!("</h{level}>"), &format!("</h{}>", level + 1));
    }
    out
}

fn export_options<'a>(
    profile: &'a ReadingProfile,
    language: &str,
    modified: Option<u64>,
) -> ExportOptions<'a> {
    ExportOptions {
        profile: Some(profile),
        language: language.to_string(),
        modified,
        ..ExportOptions::default()
    }
}

/// A standalone accessible HTML page in the profile.
pub fn export_html(document_json: &str, profile_json: &str, language: &str) -> Result<String> {
    let document = read_document(document_json)?;
    let profile = read_profile(profile_json)?;
    Ok(to_html(
        &document,
        &export_options(&profile, language, None),
    ))
}

/// An EPUB 3 book in the profile, as the bytes of the file. `modified` is the time in Unix
/// seconds, which the page supplies because WebAssembly in a browser cannot read the clock.
pub fn export_epub(
    document_json: &str,
    profile_json: &str,
    language: &str,
    modified: u64,
) -> Result<Vec<u8>> {
    let document = read_document(document_json)?;
    let profile = read_profile(profile_json)?;
    to_epub(
        &document,
        &export_options(&profile, language, Some(modified)),
    )
    .map_err(text_error)
}

#[cfg(test)]
mod tests;
