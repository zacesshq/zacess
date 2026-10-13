//! Semantic HTML for a document seen through a reading profile.
//!
//! The same body serves the standalone HTML export and the XHTML content document inside an
//! EPUB, so the markup is always well-formed XML: void elements are self-closed and only the five
//! XML entities are used.

use super::mathml::to_mathml;
use super::{ExportOptions, escape};
use crate::confidence::flags_for;
use crate::model::{Block, BlockKind, Document, ListItem, Run, TableRow, TextRole};
use crate::profile::{ReadingProfile, profile_css};
use std::fmt::Write;

/// What a rendered body contains, for building navigation around it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outline {
    /// Headings in order: level, element id and text.
    pub headings: Vec<(u8, String, String)>,
    /// Page markers in order: page number and element id.
    pub pages: Vec<(u32, String)>,
    pub has_mathml: bool,
    pub has_images: bool,
    pub images_without_alt: usize,
    /// Image assets the body refers to, relative to the document.
    pub assets: Vec<String>,
}

/// A rendered body plus its outline.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub body: String,
    pub outline: Outline,
}

/// The style for a profile plus the rules the export markup needs on top.
pub fn stylesheet(profile: &ReadingProfile) -> String {
    let mut css = profile_css(profile);
    css.push_str(
        ".zc-doc figure { margin: 0 0 1em; }\n\
         .zc-doc figcaption { font-style: italic; }\n\
         .zc-equation { overflow-x: auto; margin: 0 0 1em; }\n\
         .zc-doc math { font-size: 1.1em; }\n\
         .zc-page-break { border: 0; height: 0; margin: 0; padding: 0; }\n\
         .zc-ai { border-left: 3px solid currentColor; padding: 0.25em 0 0.25em 0.75em; margin: 0 0 1em; }\n",
    );
    css
}

/// A complete standalone HTML document.
pub fn to_html(document: &Document, options: &ExportOptions<'_>) -> String {
    let rendered = render_body(document, options, false);
    let profile = options.profile();
    let title = options
        .title
        .clone()
        .unwrap_or_else(|| document.title.clone());
    // the ruler is a fixed band the style sheet positions; it is decoration, so it is hidden from
    // assistive technology
    let ruler = if profile.reading_ruler.enabled {
        "<div class=\"zc-ruler\" aria-hidden=\"true\"></div>\n"
    } else {
        ""
    };
    format!(
        "<!DOCTYPE html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\"/>\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"/>\n\
         <title>{title}</title>\n<style>\n{css}</style>\n</head>\n<body>\n{ruler}{body}</body>\n</html>\n",
        lang = escape(options.language()),
        title = escape(&title),
        css = stylesheet(&profile),
        body = rendered.body,
    )
}

/// The body of a document as `<main>` plus everything inside it.
///
/// With `xhtml` set, page markers also carry `epub:type`, which only an EPUB content document
/// declares a namespace for.
pub fn render_body(document: &Document, options: &ExportOptions<'_>, xhtml: bool) -> Rendered {
    let mut out = String::new();
    let mut outline = Outline::default();
    let mut previous_level = 0u8;
    out.push_str("<main class=\"zc-doc\">\n");

    if let Some(first) = document.pages.first() {
        page_marker(&mut out, &mut outline, first.number, xhtml);
    }
    for block in &document.blocks {
        render_block(
            &mut out,
            &mut outline,
            &mut previous_level,
            block,
            options,
            xhtml,
        );
        for annotation in document.annotations_for(block.id) {
            // help sits beside the block, labelled. It never replaces the block
            let _ = writeln!(
                out,
                "<aside class=\"zc-ai\" role=\"note\" aria-label=\"{label}\"><p><strong>{label}:</strong> {text}</p></aside>",
                label = escape(&annotation.label()),
                text = escape(&annotation.text),
            );
        }
    }
    out.push_str("</main>\n");
    Rendered { body: out, outline }
}

fn page_marker(out: &mut String, outline: &mut Outline, page: u32, xhtml: bool) {
    let id = format!("page{page}");
    let epub = if xhtml {
        " epub:type=\"pagebreak\""
    } else {
        ""
    };
    // an hr already has the separator role, which doc-pagebreak refines, so the label is allowed
    let _ = writeln!(
        out,
        "<hr class=\"zc-page-break\" id=\"{id}\" role=\"doc-pagebreak\"{epub} aria-label=\"Page {page}\"/>"
    );
    outline.pages.push((page, id));
}

fn render_block(
    out: &mut String,
    outline: &mut Outline,
    previous_level: &mut u8,
    block: &Block,
    options: &ExportOptions<'_>,
    xhtml: bool,
) {
    match &block.kind {
        BlockKind::Heading { level, runs } => {
            // levels never skip, so assistive technology can walk the outline
            let level = (*level).clamp(1, 6).min(*previous_level + 1).max(1);
            *previous_level = level;
            let id = format!("b{}", block.id);
            let text = runs_text(runs);
            let _ = writeln!(
                out,
                "<h{level} id=\"{id}\">{}</h{level}>",
                render_runs(runs)
            );
            outline.headings.push((level, id, text));
        }
        BlockKind::Paragraph { runs, role } => match role {
            TextRole::Body => {
                let _ = writeln!(out, "<p id=\"b{}\">{}</p>", block.id, render_runs(runs));
            }
            TextRole::Quote => {
                let _ = writeln!(
                    out,
                    "<blockquote id=\"b{}\"><p>{}</p></blockquote>",
                    block.id,
                    render_runs(runs)
                );
            }
            TextRole::Code => {
                // spacing and line breaks are meaning in code, so the text is kept exactly
                let text: String = runs.iter().map(|r| r.text.as_str()).collect();
                let _ = writeln!(
                    out,
                    "<pre id=\"b{}\"><code>{}</code></pre>",
                    block.id,
                    escape(&text)
                );
            }
        },
        BlockKind::List { ordered, items } => {
            let tag = if *ordered { "ol" } else { "ul" };
            let _ = writeln!(out, "<{tag} id=\"b{}\">", block.id);
            render_items(out, items, tag);
            let _ = writeln!(out, "</{tag}>");
        }
        BlockKind::Table { rows, caption } => render_table(out, block, rows, caption),
        BlockKind::Image { alt_text, asset } => {
            outline.has_images = true;
            let page = block.origin.page;
            let alt = match alt_text {
                Some(alt) if !alt.trim().is_empty() => alt.clone(),
                _ => {
                    outline.images_without_alt += 1;
                    format!("Image from page {page} that has no description yet")
                }
            };
            // an image is only embedded when its file can be found, since an EPUB must carry it
            let available = asset.as_deref().is_some_and(|a| {
                if xhtml {
                    options.asset_exists(a)
                } else {
                    options.asset_dir.is_none() || options.asset_exists(a)
                }
            });
            match asset {
                Some(src) if available => {
                    outline.assets.push(src.clone());
                    let _ = writeln!(
                        out,
                        "<figure id=\"b{}\"><img src=\"{}\" alt=\"{}\"/></figure>",
                        block.id,
                        escape(src),
                        escape(&alt)
                    );
                }
                _ => {
                    let _ = writeln!(
                        out,
                        "<figure id=\"b{}\"><figcaption>{}</figcaption></figure>",
                        block.id,
                        escape(&alt)
                    );
                }
            }
        }
        BlockKind::Equation {
            runs,
            latex,
            mathml,
            display,
        } => {
            outline.has_mathml = true;
            let inner = match mathml {
                Some(m)
                    if m.trim_start().starts_with("<m") && !m.trim_start().starts_with("<math") =>
                {
                    m.clone()
                }
                Some(m) if m.trim_start().starts_with("<math") => strip_math_wrapper(m),
                _ => {
                    let source = latex.clone().unwrap_or_else(|| runs_text(runs));
                    format!(
                        "<semantics>{}<annotation encoding=\"application/x-tex\">{}</annotation></semantics>",
                        to_mathml(&source),
                        escape(&source)
                    )
                }
            };
            // alttext carries the natural reading for assistive technology without MathML support
            let spoken = match mathml {
                Some(_) => None,
                None => Some(crate::maths::speech::speak(
                    &latex.clone().unwrap_or_else(|| runs_text(runs)),
                )),
            };
            let math = format!(
                "<math xmlns=\"http://www.w3.org/1998/Math/MathML\"{}{}>{inner}</math>",
                if *display { " display=\"block\"" } else { "" },
                spoken
                    .map(|t| format!(" alttext=\"{}\"", escape(&t)))
                    .unwrap_or_default()
            );
            let flagged = flags_for(&runs.iter().collect::<Vec<_>>())
                .iter()
                .any(|f| *f);
            let math = if flagged {
                uncertain(&math, lowest(runs))
            } else {
                math
            };
            if *display {
                let _ = writeln!(
                    out,
                    "<div class=\"zc-equation\" id=\"b{}\">{math}</div>",
                    block.id
                );
            } else {
                let _ = writeln!(out, "<p id=\"b{}\">{math}</p>", block.id);
            }
        }
        BlockKind::PageBreak => page_marker(out, outline, block.origin.page, xhtml),
    }
}

fn strip_math_wrapper(markup: &str) -> String {
    let start = markup.find('>').map(|i| i + 1).unwrap_or(0);
    let end = markup.rfind("</math>").unwrap_or(markup.len());
    markup[start..end.max(start)].to_string()
}

fn lowest(runs: &[Run]) -> f32 {
    runs.iter()
        .map(|r| r.confidence.value())
        .fold(1.0, f32::min)
}

fn render_items(out: &mut String, items: &[ListItem], tag: &str) {
    for item in items {
        let _ = write!(out, "<li>{}", render_runs(&item.runs));
        if !item.children.is_empty() {
            let _ = writeln!(out, "\n<{tag}>");
            render_items(out, &item.children, tag);
            let _ = writeln!(out, "</{tag}>");
        }
        let _ = writeln!(out, "</li>");
    }
}

fn render_table(out: &mut String, block: &Block, rows: &[TableRow], caption: &[Run]) {
    let _ = writeln!(out, "<table id=\"b{}\">", block.id);
    if !caption.is_empty() {
        let _ = writeln!(out, "<caption>{}</caption>", render_runs(caption));
    }
    let header_rows = rows
        .iter()
        .take_while(|r| !r.cells.is_empty() && r.cells.iter().all(|c| c.header))
        .count();
    let (head, body) = rows.split_at(header_rows.min(rows.len()));
    if !head.is_empty() {
        out.push_str("<thead>\n");
        for row in head {
            render_row(out, row, "col");
        }
        out.push_str("</thead>\n");
    }
    if !body.is_empty() {
        out.push_str("<tbody>\n");
        for row in body {
            render_row(out, row, "row");
        }
        out.push_str("</tbody>\n");
    }
    out.push_str("</table>\n");
}

fn render_row(out: &mut String, row: &TableRow, header_scope: &str) {
    out.push_str("<tr>");
    for cell in &row.cells {
        let tag = if cell.header { "th" } else { "td" };
        let mut attrs = String::new();
        if cell.header {
            let _ = write!(attrs, " scope=\"{header_scope}\"");
        }
        if cell.col_span > 1 {
            let _ = write!(attrs, " colspan=\"{}\"", cell.col_span);
        }
        if cell.row_span > 1 {
            let _ = write!(attrs, " rowspan=\"{}\"", cell.row_span);
        }
        let _ = write!(out, "<{tag}{attrs}>{}</{tag}>", render_runs(&cell.runs));
    }
    out.push_str("</tr>\n");
}

/// Runs as inline markup. A run that needs checking is wrapped so it is both seen and announced.
pub fn render_runs(runs: &[Run]) -> String {
    let refs: Vec<&Run> = runs.iter().collect();
    let flags = flags_for(&refs);
    let mut out = String::new();
    for (run, flagged) in runs.iter().zip(flags) {
        let needs_space = !out.is_empty()
            && !out.ends_with(char::is_whitespace)
            && !out.ends_with(crate::model::opens_text)
            && !out.ends_with("</span>")
            && !run.text.starts_with(char::is_whitespace)
            && !run.text.starts_with(crate::model::closes_text);
        if needs_space
            || (out.ends_with("</span>")
                && !run.text.starts_with(char::is_whitespace)
                && !run.text.starts_with(crate::model::closes_text)
                && !out.ends_with("</span> "))
        {
            // the uncertain wrapper ends with its tag, so a space before the next word is kept outside it
            if !out.ends_with(char::is_whitespace) {
                out.push(' ');
            }
        }
        let text = escape(run.text.trim_end());
        let text = match run.script {
            Some(crate::model::Script::Subscript) => format!("<sub>{text}</sub>"),
            Some(crate::model::Script::Superscript) => format!("<sup>{text}</sup>"),
            None => text,
        };
        let text = match run.link.as_deref().filter(|l| is_safe_link(l)) {
            Some(href) => format!("<a href=\"{}\">{text}</a>", escape(href)),
            None => text,
        };
        let trailing = run.text.len() - run.text.trim_end().len();
        if flagged {
            out.push_str(&uncertain(&text, run.confidence.value()));
        } else {
            out.push_str(&text);
        }
        if trailing > 0 {
            out.push(' ');
        }
    }
    out.trim_end().to_string()
}

/// Whether a link may be written into a page: web, mail and in-page addresses only, so a captured
/// `javascript:` or `data:` address can never run when the export is opened.
fn is_safe_link(href: &str) -> bool {
    let lower = href.trim_start().to_ascii_lowercase();
    lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || lower.starts_with('#')
}

/// Wraps markup that a person should check: highlighted by the style sheet and announced to a
/// screen reader by the hidden text.
fn uncertain(markup: &str, confidence: f32) -> String {
    format!(
        "<span class=\"zc-uncertain\" data-confidence=\"{:.2}\"><span class=\"zc-sr\">unclear: </span>{markup}</span>",
        confidence
    )
}

fn runs_text(runs: &[Run]) -> String {
    crate::model::join_runs(runs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Confidence, Origin, Rect, Source, SourceFormat, TableCell};

    fn origin() -> Origin {
        Origin::new(1, Rect::new(0.0, 0.0, 10.0, 10.0))
    }

    fn doc() -> Document {
        Document::from_json(include_str!("../../../../docs/examples/document.json")).unwrap()
    }

    #[test]
    fn the_example_document_renders_every_block_type_semantically() {
        let html = to_html(&doc(), &ExportOptions::default());
        assert!(html.starts_with("<!DOCTYPE html>\n<html lang=\"en\">"));
        assert!(html.contains("<title>Signals week 3</title>"));
        assert!(html.contains("<h1 id=\"b1\">Fourier series</h1>"));
        assert!(html.contains("<ol id=\"b3\">\n<li>Sample</li>\n<li>Transform\n<ol>\n<li>Window first</li>\n</ol>\n</li>"));
        assert!(html.contains("<caption>Measured gain</caption>"));
        assert!(html.contains("<th scope=\"col\">f (Hz)</th>"));
        assert!(html.contains("<td>0.7</td>"));
        assert!(html.contains(
            "<img src=\"images/p2-1.png\" alt=\"Image from page 2 that has no description yet\"/>"
        ));
        assert!(html.contains("<math xmlns=\"http://www.w3.org/1998/Math/MathML\" display=\"block\" alttext=\"V equals I R\"><semantics><mrow><mi>V</mi><mo>=</mo><mi>I</mi><mi>R</mi></mrow>"));
        assert!(html.contains("<annotation encoding=\"application/x-tex\">V = IR</annotation>"));
        assert!(html.contains("role=\"doc-pagebreak\" aria-label=\"Page 2\""));
        assert!(
            !html.contains("epub:type"),
            "plain HTML carries no EPUB attributes"
        );
    }

    #[test]
    fn low_confidence_text_is_marked_and_announced() {
        let html = to_html(&doc(), &ExportOptions::default());
        // 2.6 was corrected, so it is no longer flagged even at 62%
        assert!(
            html.contains("<p id=\"b2\">The period is 2.6 ms.</p>"),
            "{html}"
        );

        let mut d = doc();
        let run = &mut d.block_mut(crate::model::BlockId(2)).unwrap().runs_mut()[1];
        run.status = crate::model::RunStatus::Recognised;
        run.original_text = None;
        let html = to_html(&d, &ExportOptions::default());
        assert!(html.contains(
            "The period is <span class=\"zc-uncertain\" data-confidence=\"0.62\"><span class=\"zc-sr\">unclear: </span>2.6</span> ms."
        ), "{html}");
        // the equation at 91% is a numeric run, so it is flagged too
        assert!(html.contains("<span class=\"zc-sr\">unclear: </span><math"));
    }

    #[test]
    fn heading_levels_never_skip() {
        let mut d = Document::new(
            "t",
            Source {
                file_name: "t.pdf".into(),
                format: SourceFormat::Pdf,
            },
        );
        d.push(
            BlockKind::heading(2, vec![Run::native("Second", origin())]).unwrap(),
            origin(),
        );
        d.push(
            BlockKind::heading(5, vec![Run::native("Fifth", origin())]).unwrap(),
            origin(),
        );
        d.push(
            BlockKind::heading(1, vec![Run::native("First", origin())]).unwrap(),
            origin(),
        );
        let html = to_html(&d, &ExportOptions::default());
        assert!(html.contains("<h1 id=\"b1\">Second</h1>"));
        assert!(html.contains("<h2 id=\"b2\">Fifth</h2>"));
        assert!(html.contains("<h1 id=\"b3\">First</h1>"));
    }

    #[test]
    fn markup_in_text_is_escaped_and_images_without_assets_still_describe_themselves() {
        let mut d = Document::new(
            "a < b & c",
            Source {
                file_name: "t.pdf".into(),
                format: SourceFormat::Pdf,
            },
        );
        d.push(
            BlockKind::paragraph(vec![Run::native(
                "<script>alert(1)</script> & \"quotes\"",
                origin(),
            )]),
            origin(),
        );
        d.push(
            BlockKind::Image {
                alt_text: Some("A circuit".into()),
                asset: None,
            },
            origin(),
        );
        d.push(
            BlockKind::Table {
                rows: vec![TableRow {
                    cells: vec![
                        TableCell::header(vec![Run::native("Row head", origin())]),
                        TableCell::new(vec![Run::native("1", origin())]),
                    ],
                }],
                caption: vec![],
            },
            origin(),
        );
        let html = to_html(&d, &ExportOptions::default());
        assert!(html.contains("<title>a &lt; b &amp; c</title>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;quotes&quot;"));
        assert!(html.contains("<figure id=\"b2\"><figcaption>A circuit</figcaption></figure>"));
        assert!(html.contains("<th scope=\"row\">Row head</th>"));
    }

    #[test]
    fn runs_keep_single_spaces_around_flagged_words() {
        let runs = vec![
            Run::native("Supply: ", origin()),
            Run::recognised("12", Confidence::new(0.9), origin()),
            Run::native(" V", origin()),
        ];
        let html = render_runs(&runs);
        assert_eq!(
            html,
            "Supply: <span class=\"zc-uncertain\" data-confidence=\"0.90\"><span class=\"zc-sr\">unclear: </span>12</span> V"
        );
    }

    #[test]
    fn the_outline_lists_headings_pages_and_media() {
        let rendered = render_body(&doc(), &ExportOptions::default(), true);
        assert_eq!(
            rendered.outline.headings,
            vec![(1, "b1".to_string(), "Fourier series".to_string())]
        );
        assert_eq!(
            rendered.outline.pages,
            vec![(1, "page1".to_string()), (2, "page2".to_string())]
        );
        assert!(rendered.outline.has_mathml);
        assert!(rendered.outline.has_images);
        assert_eq!(rendered.outline.images_without_alt, 1);
        assert!(rendered.body.contains("epub:type=\"pagebreak\""));
        // the example names an image file that does not exist, so the EPUB body describes it instead
        assert!(rendered.outline.assets.is_empty());
        assert!(
            rendered
                .body
                .contains("<figcaption>Image from page 2 that has no description yet</figcaption>")
        );
    }

    #[test]
    fn the_reading_ruler_is_present_only_when_the_profile_turns_it_on() {
        let html = to_html(&doc(), &ExportOptions::default());
        assert!(!html.contains("zc-ruler\" aria-hidden"));
        let low_vision = crate::profile::ReadingProfile::preset(crate::profile::Preset::LowVision);
        let options = ExportOptions {
            profile: Some(&low_vision),
            ..ExportOptions::default()
        };
        let html = to_html(&doc(), &options);
        assert!(
            html.contains("<body>\n<div class=\"zc-ruler\" aria-hidden=\"true\"></div>\n<main")
        );
        assert!(html.contains("font-size: 24pt"));
        assert!(html.contains("mix-blend-mode: screen"));
    }

    #[test]
    fn annotations_render_as_labelled_notes_beside_their_block() {
        let mut d = doc();
        let annotation = crate::model::Annotation {
            id: 0,
            block: crate::model::BlockId(2),
            kind: crate::model::AnnotationKind::SimpleExplanation,
            text: "The period is 2.6 ms.".into(),
            original_text: String::new(),
            provenance: crate::model::Provenance {
                helper: "rule-based simplifier".into(),
                model: None,
                ran_on: crate::model::RanOn::Device,
                at: 0,
            },
        };
        d.annotate(annotation).unwrap();
        let html = to_html(&d, &ExportOptions::default());
        let paragraph = html.find("<p id=\"b2\">").unwrap();
        let note = html.find("<aside class=\"zc-ai\" role=\"note\"").unwrap();
        assert!(note > paragraph, "the note follows the original");
        assert!(html.contains("aria-label=\"Simpler explanation generated by rule-based simplifier on this device, not part of the original\""));
        assert!(
            html.contains("not part of the original:</strong> The period is 2.6 ms.</p></aside>")
        );
    }
}
