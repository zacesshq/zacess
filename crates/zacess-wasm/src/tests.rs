use super::*;
use serde_json::Value;

const TABLE_PAGE: &str = include_str!("../../../tests/fixtures/clip/table-list.html");

fn standard() -> String {
    default_profile()
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("valid JSON")
}

#[test]
fn every_preset_is_listed_with_a_valid_profile() {
    let list = json(&presets());
    let list = list.as_array().expect("an array");
    assert_eq!(list.len(), Preset::ALL.len());
    assert_eq!(list[0]["id"], "low_vision");
    assert_eq!(list[0]["name"], "Low Vision");
    for entry in list {
        let profile = entry["profile"].to_string();
        check_profile(&profile).expect("every preset passes the core's checks");
    }
}

#[test]
fn impossible_profile_settings_are_refused_with_a_reason() {
    let mut profile = json(&standard());
    profile["font_size_pt"] = 400.into();
    let err = check_profile(&profile.to_string()).unwrap_err();
    assert!(err.contains("size"), "{err}");
    assert!(check_profile("not json").is_err());
}

#[test]
fn reading_css_is_scoped_to_the_view() {
    let css = reading_css(&standard(), ".zc-view").unwrap();
    for line in css.lines().filter(|l| l.contains(" {")) {
        assert!(line.starts_with(".zc-view"), "unscoped rule: {line}");
    }
    assert!(css.contains(".zc-view { margin: 0; background: #ffffff; color: #111111; }"));
    assert!(css.contains(".zc-view .zc-uncertain"));
    // reduced motion reaches the view and everything inside it, nothing else
    assert!(css.contains(".zc-view, .zc-view * { animation: none"));
}

#[test]
fn scoping_rewrites_every_selector_in_a_list() {
    let css = scope_css(".a p, .a ul { margin: 0; }\n", "#v");
    assert_eq!(css, "#v .a p, #v .a ul { margin: 0; }\n");
}

#[test]
fn text_and_markdown_become_structured_documents() {
    let out = document_from_text(
        "# Signals\n\nA first paragraph.\n\n- one\n- two\n",
        "notes.md",
    )
    .unwrap();
    let doc = Document::from_json(&out).unwrap();
    assert_eq!(doc.source.file_name, "notes.md");
    assert!(matches!(
        doc.blocks[0].kind,
        BlockKind::Heading { level: 1, .. }
    ));
    assert!(
        doc.blocks
            .iter()
            .any(|b| matches!(b.kind, BlockKind::List { ordered: false, .. }))
    );
    assert!(document_from_text("   \n", "empty.txt").is_err());
}

#[test]
fn a_web_page_keeps_its_table_and_list_and_drops_the_menu() {
    let out = document_from_html(TABLE_PAGE, "results.html", "html", false).unwrap();
    let doc = Document::from_json(&out).unwrap();
    assert_eq!(doc.title, "Lab 4 results");
    assert_eq!(doc.source.format, SourceFormat::Html);
    assert!(
        doc.blocks
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Table { .. }))
    );
    assert!(!doc.plain_text().contains("Home"), "the menu was kept");
}

#[test]
fn a_word_file_fragment_is_kept_whole_and_marked_as_docx() {
    let html = "<h1>Week 3</h1><p>Read <strong>chapter 2</strong>.</p>\
                <p><img src=\"data:image/png;base64,AA\" alt=\"A sine wave\"></p>";
    let out = document_from_html(html, "week3.docx", "docx", true).unwrap();
    let doc = Document::from_json(&out).unwrap();
    assert_eq!(doc.source.format, SourceFormat::Docx);
    assert_eq!(doc.source.file_name, "week3.docx");
    assert_eq!(doc.title, "Week 3");
    assert!(doc.blocks.iter().any(|b| matches!(
        &b.kind,
        BlockKind::Image { alt_text: Some(alt), .. } if alt == "A sine wave"
    )));
    assert!(document_from_html(html, "x.docx", "pptx", true).is_err());
}

fn item(x: f32, baseline: f32, size: f32, bold: bool, text: &str) -> Value {
    serde_json::json!({
        "x": x, "baseline": baseline, "width": text.len() as f32 * size * 0.5,
        "size": size, "bold": bold, "text": text
    })
}

#[test]
fn pdf_text_items_go_through_the_core_layout() {
    let input = serde_json::json!({
        "file_name": "slides.pdf",
        "format": "pdf",
        "pages": [{
            "number": 1, "width": 612.0, "height": 792.0,
            "items": [
                item(72.0, 100.0, 24.0, true, "Fourier series"),
                item(72.0, 160.0, 12.0, false, "Any periodic signal can be written as a sum."),
                item(72.0, 175.0, 12.0, false, "Each term is a sine or a cosine."),
            ]
        }, {
            "number": 2, "width": 612.0, "height": 792.0, "items": []
        }]
    });
    let out = json(&document_from_pages(&input.to_string()).unwrap());
    let doc = Document::from_json(&out["document"].to_string()).unwrap();
    assert_eq!(doc.title, "slides");
    assert_eq!(doc.pages.len(), 2);
    assert!(matches!(doc.blocks[0].kind, BlockKind::Heading { .. }));
    assert_eq!(out["pages_needing_ocr"], serde_json::json!([2]));
    assert_eq!(out["pages_recognised"], serde_json::json!([]));
}

fn word(text: &str, left: f32, confidence: f32) -> Value {
    serde_json::json!({
        "text": text, "left": left, "top": 100.0, "width": 60.0, "height": 30.0,
        "confidence": confidence
    })
}

#[test]
fn ocr_words_keep_their_confidence_and_doubtful_ones_are_flagged() {
    let input = serde_json::json!({
        "file_name": "board.png",
        "format": "image",
        "pages": [{
            "number": 1, "width": 1200.0, "height": 800.0,
            "ocr": {
                "width": 1200.0, "height": 800.0,
                "paragraphs": [{ "lines": [{
                    "left": 100.0, "top": 100.0, "width": 400.0, "height": 30.0,
                    "words": [word("Resistance", 100.0, 96.0), word("4.7", 180.0, 62.0), word("kQ", 260.0, 70.0)]
                }]}]
            }
        }]
    });
    let out = json(&document_from_pages(&input.to_string()).unwrap());
    let document = out["document"].to_string();
    let doc = Document::from_json(&document).unwrap();
    assert_eq!(doc.source.format, SourceFormat::Image);
    assert_eq!(out["pages_recognised"], serde_json::json!([1]));
    assert_eq!(out["pages_needing_ocr"], serde_json::json!([]));
    let counts = json(&summary(&document).unwrap());
    assert_eq!(
        counts["uncertain"], 2,
        "the number and its unit are both doubtful"
    );
    let html = reading_html(&document, &standard()).unwrap();
    assert!(html.contains("<span class=\"zc-sr\">unclear: </span>4.7"));
}

#[test]
fn pages_without_a_size_or_an_unknown_format_are_refused() {
    let bad = serde_json::json!({
        "file_name": "a.pdf", "format": "pdf",
        "pages": [{ "number": 1, "width": 0.0, "height": 792.0 }]
    });
    assert!(document_from_pages(&bad.to_string()).is_err());
    let unknown = serde_json::json!({ "file_name": "a.x", "format": "x", "pages": [] });
    assert!(document_from_pages(&unknown.to_string()).is_err());
    let empty = serde_json::json!({ "file_name": "a.pdf", "format": "pdf", "pages": [] });
    assert!(document_from_pages(&empty.to_string()).is_err());
}

#[test]
fn the_reading_view_leaves_the_only_h1_to_the_app() {
    let out = document_from_html(TABLE_PAGE, "results.html", "html", false).unwrap();
    let html = reading_html(&out, &standard()).unwrap();
    assert!(html.starts_with("<article class=\"zc-doc\">"));
    assert!(html.trim_end().ends_with("</article>"));
    assert!(!html.contains("<h1") && !html.contains("<main"));
    assert!(html.contains("<h2") && html.contains("<h3"));
    assert!(html.contains("<caption>"));
}

#[test]
fn heading_demotion_stops_at_h6() {
    assert_eq!(
        demote_headings("<h1 id=\"a\">A</h1><h5>E</h5><h6>F</h6>"),
        "<h2 id=\"a\">A</h2><h6>E</h6><h6>F</h6>"
    );
}

#[test]
fn exports_are_whole_files_in_the_profile() {
    let doc = document_from_text("# Notes\n\nSome text.", "notes.txt").unwrap();
    let low_vision = serde_json::to_string(&ReadingProfile::preset(Preset::LowVision)).unwrap();
    let html = export_html(&doc, &low_vision, "en-GB").unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("<html lang=\"en-GB\">"));
    assert!(
        html.contains("#ffeb3b"),
        "the low vision theme styles the export"
    );
    assert!(html.contains("<main class=\"zc-doc\">"));

    let epub = export_epub(&doc, &low_vision, "en-GB", 1_791_883_805).unwrap();
    assert!(epub.starts_with(b"PK"));
    // the mimetype entry comes first and is stored, as EPUB readers require
    assert_eq!(&epub[30..38], b"mimetype");
    assert!(
        epub.windows(b"application/epub+zip".len())
            .any(|w| w == b"application/epub+zip")
    );
    // the container is stored uncompressed, so the time the page supplied is readable as it is
    let modified = b"2026-10-13T09:30:05Z";
    assert!(epub.windows(modified.len()).any(|w| w == modified));
}

#[test]
fn a_damaged_document_is_refused_rather_than_misread() {
    let err = export_html("{\"schema_version\": 99}", &standard(), "en").unwrap_err();
    assert!(err.starts_with("The document could not be read"), "{err}");
}

#[test]
fn a_title_falls_back_to_the_file_name() {
    assert_eq!(stem("notes.final.txt"), "notes.final");
    assert_eq!(stem(".hidden"), ".hidden");
    assert_eq!(stem("plain"), "plain");
}

fn links_in(document: &Document) -> Vec<(String, String)> {
    document
        .blocks
        .iter()
        .flat_map(|b| b.runs())
        .filter_map(|r| r.link.clone().map(|l| (r.text.trim().to_string(), l)))
        .collect()
}

#[test]
fn a_bare_address_with_a_full_stop_links_only_the_address() {
    let out = document_from_text(
        "Notes are at https://example.org/week-3. See (https://en.wikipedia.org/wiki/Fourier_(series)) too.",
        "pasted.txt",
    )
    .unwrap();
    let doc = Document::from_json(&out).unwrap();
    assert_eq!(
        links_in(&doc),
        vec![
            (
                "https://example.org/week-3".to_string(),
                "https://example.org/week-3".to_string()
            ),
            (
                "https://en.wikipedia.org/wiki/Fourier_(series)".to_string(),
                "https://en.wikipedia.org/wiki/Fourier_(series)".to_string()
            ),
        ]
    );
    // the text reads exactly as it was pasted
    assert!(
        doc.plain_text()
            .contains("https://example.org/week-3. See (https://")
    );
    let html = reading_html(&out, &standard()).unwrap();
    assert!(
        html.contains("<a href=\"https://example.org/week-3\">https://example.org/week-3</a>.")
    );
}

#[test]
fn addresses_inside_words_or_without_a_host_are_left_alone() {
    assert!(links::find_addresses("nohttps://example.org").is_empty());
    assert!(links::find_addresses("just https:// and nothing").is_empty());
    assert_eq!(links::find_addresses("<http://a.b>"), vec![(1, 11)]);
}

#[test]
fn only_web_and_mail_addresses_become_links() {
    assert_eq!(
        links::web_link(" https://a.b/c "),
        Some("https://a.b/c".to_string())
    );
    assert_eq!(
        links::web_link("mailto:hello@zacess.com"),
        Some("mailto:hello@zacess.com".to_string())
    );
    assert_eq!(links::web_link("javascript:alert(1)"), None);
    assert_eq!(links::web_link("data:text/html,x"), None);
    assert_eq!(links::web_link("#page2"), None);
    assert_eq!(links::web_link("https://"), None);
}

#[test]
fn a_pdf_link_area_links_the_words_inside_it() {
    let input = serde_json::json!({
        "file_name": "reading.pdf",
        "format": "pdf",
        "pages": [{
            "number": 1, "width": 612.0, "height": 792.0,
            "items": [
                { "x": 72.0, "baseline": 100.0, "width": 240.0, "size": 12.0, "text": "Read the course page before Monday." },
                { "x": 72.0, "baseline": 130.0, "width": 120.0, "size": 12.0, "text": "Unsafe link here" }
            ],
            "links": [
                // "course page" sits over characters 9 to 20 of the first line
                { "url": "https://example.org/course", "x": 135.0, "y": 88.0, "width": 72.0, "height": 15.0 },
                { "url": "javascript:alert(1)", "x": 72.0, "y": 118.0, "width": 120.0, "height": 15.0 }
            ]
        }]
    });
    let out = json(&document_from_pages(&input.to_string()).unwrap());
    let document = out["document"].to_string();
    let doc = Document::from_json(&document).unwrap();
    assert_eq!(
        links_in(&doc),
        vec![(
            "course page".to_string(),
            "https://example.org/course".to_string()
        )]
    );
    assert!(
        doc.plain_text()
            .contains("Read the course page before Monday.")
    );
    let html = export_html(&document, &standard(), "en").unwrap();
    assert!(html.contains("<a href=\"https://example.org/course\">course page</a>"));
    assert!(!html.contains("javascript:"));
}

#[test]
fn links_survive_both_exports() {
    let doc = document_from_text("Slides: https://example.org/slides", "n.txt").unwrap();
    let html = export_html(&doc, &standard(), "en").unwrap();
    assert!(html.contains("<a href=\"https://example.org/slides\">"));
    let epub = export_epub(&doc, &standard(), "en", 0).unwrap();
    let needle = b"<a href=\"https://example.org/slides\">";
    assert!(epub.windows(needle.len()).any(|w| w == needle));
}
