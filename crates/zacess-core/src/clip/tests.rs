//! Capture tests: web pages from the fixtures in `tests/fixtures/clip`, text, links, limits,
//! repeats and the inbox.

use super::html::{ClipOptions, html_to_document, resolve_link};
use super::*;
use crate::import::ocr::{OcrEngine, OcrLine, OcrPage, OcrParagraph, OcrWord};
use crate::model::{BlockKind, Confidence, Script, TextRole};
use std::path::Path;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/clip")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn page(name: &str, url: &str) -> Document {
    html_to_document(
        &fixture(name),
        &ClipOptions {
            url: Some(url),
            title: None,
            selection_only: false,
        },
    )
    .unwrap()
    .unwrap()
}

fn kinds(doc: &Document) -> Vec<String> {
    doc.blocks
        .iter()
        .map(|b| match &b.kind {
            BlockKind::Heading { level, .. } => format!("h{level}"),
            BlockKind::Paragraph { role, .. } => match role {
                TextRole::Body => "p".into(),
                TextRole::Quote => "quote".into(),
                TextRole::Code => "code".into(),
            },
            BlockKind::List { ordered: true, .. } => "ol".into(),
            BlockKind::List { ordered: false, .. } => "ul".into(),
            BlockKind::Table { .. } => "table".into(),
            BlockKind::Image { .. } => "image".into(),
            BlockKind::Equation { .. } => "equation".into(),
            BlockKind::PageBreak => "page_break".into(),
        })
        .collect()
}

fn links(doc: &Document) -> Vec<(String, String)> {
    doc.runs()
        .filter_map(|r| r.link.clone().map(|l| (r.text.trim().to_string(), l)))
        .collect()
}

fn request(content: CaptureContent, at: i64) -> CaptureRequest {
    CaptureRequest {
        version: CAPTURE_REQUEST_VERSION,
        via: CaptureVia::Extension,
        captured_at: at,
        content,
    }
}

fn web(url: &str, html: &str) -> CaptureContent {
    CaptureContent::WebClip {
        url: url.into(),
        title: String::new(),
        html: html.into(),
        selection_only: false,
    }
}

fn text(text: &str) -> CaptureContent {
    CaptureContent::Text { text: text.into() }
}

#[test]
fn an_article_keeps_its_content_and_drops_the_page_furniture() {
    let doc = page(
        "article.html",
        "https://signals.example/guides/low-pass?ref=home#top",
    );
    let text = doc.plain_text();
    assert_eq!(doc.title, "Filters explained | Signals Weekly");
    assert_eq!(doc.source.format, SourceFormat::Html);
    assert_eq!(
        kinds(&doc),
        [
            "h1", "p", "p", "image", "p", "image", "h2", "p", "quote", "code", "p"
        ],
        "{text}"
    );
    for kept in [
        "How a low-pass filter works",
        "By the Signals Weekly team",
        "A low-pass filter lets slow changes through",
        "The response of a first order filter.",
        "The capacitor is the memory of the circuit.",
        "Read aloud as f sub c.",
        "Unsafe script link text stays as text.",
    ] {
        assert!(text.contains(kept), "missing {kept:?} in\n{text}");
    }
    for dropped in [
        "cookies",
        "Accept all",
        "News",
        "Share this page",
        "oscilloscope",
        "hidden",
        "Neither is this one",
        "injected",
        "tracking",
        "Tags:",
        "Related articles",
        "amplifier myths",
        "newsletter",
        "Copyright",
        "font-family",
        "Signals Weekly home",
        "tracker",
    ] {
        assert!(!text.contains(dropped), "kept {dropped:?} in\n{text}");
    }
}

#[test]
fn links_are_made_whole_and_unsafe_ones_are_left_as_text() {
    let doc = page(
        "article.html",
        "https://signals.example/guides/low-pass?ref=home#top",
    );
    assert_eq!(
        links(&doc),
        [
            (
                "guide to RC circuits".to_string(),
                "https://signals.example/guides/rc-circuits".to_string()
            ),
            (
                "reference tables".to_string(),
                "https://example.org/reference".to_string()
            ),
        ]
    );
    // a full stop after a link joins the link without a space
    let first = doc.blocks[2].text();
    assert!(first.ends_with("see the reference tables."), "{first}");
    let html = crate::export::to_html(&doc, &crate::export::ExportOptions::default());
    assert!(html.contains("<a href=\"https://signals.example/guides/rc-circuits\">"));
    assert!(!html.contains("javascript:"));
}

#[test]
fn images_become_descriptions_and_are_never_fetched() {
    let doc = page("article.html", "https://signals.example/guides/low-pass");
    let images: Vec<(Option<String>, Option<String>)> = doc
        .blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::Image { alt_text, asset } => Some((alt_text.clone(), asset.clone())),
            _ => None,
        })
        .collect();
    // the decorative image and the tracking pixel are left out, the unlabelled one still needs a
    // description and no picture is stored
    assert_eq!(
        images,
        [
            (
                Some("Bode plot falling at 20 dB per decade above the cut-off".to_string()),
                None
            ),
            (None, None),
        ]
    );
}

#[test]
fn code_quotes_and_scripts_keep_their_meaning() {
    let doc = page("article.html", "https://signals.example/guides/low-pass");
    let code = doc
        .blocks
        .iter()
        .find(|b| {
            matches!(
                b.kind,
                BlockKind::Paragraph {
                    role: TextRole::Code,
                    ..
                }
            )
        })
        .unwrap();
    assert_eq!(
        code.text(),
        "fc = 1 / (2 * pi * R * C)\nprint(round(fc, 1))"
    );
    let cut_off = doc
        .blocks
        .iter()
        .find(|b| b.text().starts_with("The cut-off is"))
        .unwrap();
    let sub = cut_off
        .runs()
        .into_iter()
        .find(|r| r.script == Some(Script::Subscript))
        .unwrap();
    assert_eq!(sub.text, "c");
    assert!(cut_off.text().contains("1 / (2\u{3c0}RC)"));
    let html = crate::export::to_html(&doc, &crate::export::ExportOptions::default());
    assert!(html.contains("<blockquote"));
    assert!(html.contains("<code>fc = 1 / (2 * pi * R * C)\nprint"));
}

#[test]
fn a_table_keeps_its_header_cells_and_lists_keep_their_nesting() {
    let doc = page("table-list.html", "https://lab.example/lab4");
    assert_eq!(kinds(&doc), ["h1", "p", "table", "h2", "ol", "ul"]);
    assert!(!doc.plain_text().contains("Department"));
    assert!(!doc.plain_text().contains("Home"));

    let BlockKind::Table { rows, caption } = &doc.blocks[2].kind else {
        panic!("not a table")
    };
    assert_eq!(crate::model::join_runs(caption), "Measured gain");
    let shape: Vec<Vec<(String, bool)>> = rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| (crate::model::join_runs(&c.runs), c.header))
                .collect()
        })
        .collect();
    assert_eq!(
        shape,
        [
            vec![("Frequency (Hz)".into(), true), ("Gain".into(), true)],
            vec![("50".into(), true), ("0.98".into(), false)],
            vec![("500".into(), true), ("0.71".into(), false)],
            vec![("Readings taken at 21 \u{b0}C".into(), false)],
        ]
    );
    assert_eq!(rows[3].cells[0].col_span, 2);

    let BlockKind::List { items, .. } = &doc.blocks[4].kind else {
        panic!("not a list")
    };
    assert_eq!(items.len(), 3, "an unclosed item is closed by the next one");
    assert_eq!(
        crate::model::join_runs(&items[0].runs),
        "Set the generator to 1 V2 output"
    );
    assert_eq!(items[0].runs[1].script, Some(Script::Superscript));
    assert_eq!(crate::model::join_runs(&items[1].runs), "Connect the probe");
    assert_eq!(items[1].children.len(), 2);
    assert_eq!(
        crate::model::join_runs(&items[1].children[1].runs),
        "Channel two to the output"
    );
    assert_eq!(doc.blocks[5].text(), "Safety glasses\nLab book & pen");
}

#[test]
fn a_page_with_no_main_element_is_read_from_its_largest_block_of_text() {
    let doc = page("no-main.html", "https://hub.example/tips/spacing");
    let text = doc.plain_text();
    assert_eq!(doc.title, "Revision tips");
    assert_eq!(kinds(&doc), ["h2", "p", "p", "h3", "p", "p", "p"], "{text}");
    assert!(text.contains("Short sessions spread over several days"));
    assert!(text.contains("Lines separated by breaks stay in one paragraph"));
    assert!(text.ends_with("until a blank line ends it."));
    for dropped in ["Study Hub", "Log in", "Exam timetable", "STUDY20"] {
        assert!(!text.contains(dropped), "kept {dropped:?} in\n{text}");
    }
}

#[test]
fn a_selection_is_kept_whole() {
    let html = "<nav>Chapter 3</nav><p>Picked <b>this</b> part.</p><script>x()</script>";
    let doc = html_to_document(
        html,
        &ClipOptions {
            url: Some("https://book.example/c3"),
            title: Some("  Chapter   three "),
            selection_only: true,
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(doc.title, "Chapter three");
    assert_eq!(doc.plain_text(), "Chapter 3\n\nPicked this part.");
}

#[test]
fn the_source_page_title_and_time_are_recorded() {
    let req = CaptureRequest {
        version: 1,
        via: CaptureVia::Extension,
        captured_at: 1_760_000_000,
        content: CaptureContent::WebClip {
            url: " https://signals.example/guides/low-pass ".into(),
            title: "How a low-pass filter works".into(),
            html: fixture("article.html"),
            selection_only: false,
        },
    };
    let entry = convert(&req, &Tools::default()).unwrap();
    assert_eq!(entry.title, "How a low-pass filter works");
    assert_eq!(entry.state, InboxState::New);
    let expected = ClipSource {
        via: CaptureVia::Extension,
        captured_at: 1_760_000_000,
        url: Some("https://signals.example/guides/low-pass".into()),
        page_title: Some("How a low-pass filter works".into()),
        file_name: None,
        selection_only: false,
    };
    assert_eq!(entry.source, expected);
    let ClipContent::Document { document } = &entry.content else {
        panic!("not a document")
    };
    assert_eq!(document.clip.as_ref(), Some(&expected));
    assert_eq!(
        document.source.file_name,
        "https://signals.example/guides/low-pass"
    );
    // the provenance survives the document's own JSON form
    let json = document.to_json().unwrap();
    let back = Document::from_json(&json).unwrap();
    assert_eq!(back.clip, Some(expected));
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["clip"]["via"], "extension");
    assert_eq!(value["clip"]["captured_at"], 1_760_000_000);
}

#[test]
fn plain_text_becomes_paragraphs_lists_and_headings() {
    let input = "\u{feff}Shopping for the lab\r\nand the library\r\n\r\n- resistors\r\n- capacitors\r\n  - ceramic\r\n  - electrolytic\r\n- wire that wraps\r\n  onto a second line\r\n\r\n1. Solder\r\n2) Test\r\n\r\n## Notes\r\nThe end.";
    let doc = text::text_to_document(input).unwrap();
    assert_eq!(kinds(&doc), ["p", "ul", "ol", "h2", "p"]);
    assert_eq!(doc.title, "Shopping for the lab");
    assert_eq!(doc.blocks[0].text(), "Shopping for the lab and the library");
    let BlockKind::List { items, .. } = &doc.blocks[1].kind else {
        panic!("not a list")
    };
    assert_eq!(items.len(), 3);
    assert_eq!(items[1].children.len(), 2);
    assert_eq!(
        crate::model::join_runs(&items[2].runs),
        "wire that wraps onto a second line"
    );
    assert_eq!(doc.blocks[2].text(), "Solder\nTest");
    assert_eq!(doc.blocks[4].text(), "The end.");
    assert!(text::text_to_document(" \n\t\n").is_none());
}

#[test]
fn a_bare_address_is_kept_as_a_link_note_and_not_fetched() {
    let entry = convert(
        &request(text("  https://example.org/paper.pdf \n"), 5),
        &Tools::default(),
    )
    .unwrap();
    assert_eq!(
        entry.content,
        ClipContent::Link {
            url: "https://example.org/paper.pdf".into()
        }
    );
    assert_eq!(entry.title, "https://example.org/paper.pdf");
    assert_eq!(
        entry.source.url.as_deref(),
        Some("https://example.org/paper.pdf")
    );

    assert_eq!(
        text::bare_url("<http://a.example>"),
        Some("http://a.example".into())
    );
    for not_bare in [
        "see https://example.org",
        "https://",
        "ftp://example.org",
        "https:///path",
        "example.org",
    ] {
        assert_eq!(text::bare_url(not_bare), None, "{not_bare}");
    }
    // a page with nothing readable left is kept as its address too
    let entry = convert(
        &request(web("https://empty.example/", "<nav>Menu</nav>"), 5),
        &Tools::default(),
    )
    .unwrap();
    assert!(matches!(entry.content, ClipContent::Link { .. }));
}

#[test]
fn oversized_and_malformed_requests_are_refused_clearly() {
    let big = "a".repeat(MAX_TEXT_BYTES + 1);
    let err = convert(&request(text(&big), 1), &Tools::default()).unwrap_err();
    assert!(
        matches!(err, ClipError::TooLarge { what: "text", .. }),
        "{err}"
    );
    assert!(err.to_string().contains("more than"));

    let page = format!("<p>{}</p>", "x".repeat(MAX_HTML_BYTES));
    let err = convert(
        &request(web("https://a.example/", &page), 1),
        &Tools::default(),
    )
    .unwrap_err();
    assert!(matches!(err, ClipError::TooLarge { what: "page", .. }));

    let long_url = format!("https://a.example/{}", "p".repeat(MAX_URL_BYTES));
    let err = convert(&request(web(&long_url, "<p>x</p>"), 1), &Tools::default()).unwrap_err();
    assert!(matches!(err, ClipError::TooLarge { .. }));

    let huge = vec![b' '; MAX_REQUEST_BYTES + 1];
    assert!(matches!(
        CaptureRequest::from_slice(&huge),
        Err(ClipError::TooLarge {
            what: "request",
            ..
        })
    ));
    let not_utf8: &[u8] = b"{\"version\":1,\"via\":\"extension\",\"captured_at\":1,\"content\":{\"kind\":\"text\",\"text\":\"\xff\"}}";
    assert!(matches!(
        CaptureRequest::from_slice(not_utf8),
        Err(ClipError::Json(_))
    ));
    assert!(matches!(
        CaptureRequest::from_json(
            r#"{"version":2,"via":"extension","captured_at":1,"content":{"kind":"text","text":"a"}}"#
        ),
        Err(ClipError::UnsupportedVersion(2))
    ));
    assert!(matches!(
        convert(
            &request(web("javascript:alert(1)", "<p>x</p>"), 1),
            &Tools::default()
        ),
        Err(ClipError::Invalid(_))
    ));
    assert!(matches!(
        convert(&request(text("   "), 1), &Tools::default()),
        Err(ClipError::Empty)
    ));
}

#[test]
fn malformed_and_deeply_nested_pages_still_read() {
    let broken = "<p>One <b>bold <i>both</b> after</i><p>Two &amp; &#x1F600; &bogus; &#0; <<>> a < b\
                  <div><table><tr><td>cell<td>next</table><ul><li>a<li>b</ul><script>never";
    let doc = html_to_document(broken, &ClipOptions::default())
        .unwrap()
        .unwrap();
    let text = doc.plain_text();
    assert!(text.starts_with("One bold both after"), "{text}");
    assert!(text.contains("Two & \u{1f600} &bogus; \u{fffd}"), "{text}");
    assert!(text.contains("a < b"), "{text}");
    assert!(!text.contains("never"));

    let deep = format!(
        "{}<p>deep text</p>{}",
        "<div>".repeat(50_000),
        "</div>".repeat(50_000)
    );
    let doc = html_to_document(&deep, &ClipOptions::default())
        .unwrap()
        .unwrap();
    assert_eq!(doc.plain_text(), "deep text");

    let lists = format!(
        "{}x{}",
        "<ul><li>".repeat(5_000),
        "</li></ul>".repeat(5_000)
    );
    let doc = html_to_document(&lists, &ClipOptions::default())
        .unwrap()
        .unwrap();
    assert!(doc.plain_text().contains('x'));
    // the converted document still writes and reads as JSON at that depth
    Document::from_json(&doc.to_json().unwrap()).unwrap();

    // junk that would be slow to read naively still reads quickly
    let attributes = format!("<p {}>kept</p>", "a ".repeat(500_000));
    let doc = html_to_document(&attributes, &ClipOptions::default())
        .unwrap()
        .unwrap();
    assert_eq!(doc.plain_text(), "kept");
    let paragraphs = "<p>line</p>".repeat(100_000);
    let doc = html_to_document(&paragraphs, &ClipOptions::default())
        .unwrap()
        .unwrap();
    assert_eq!(doc.blocks.len(), 100_000);
    assert_eq!(doc.blocks[99_999].id.0, 100_000);

    let many = "<b></b>".repeat(super::html::MAX_NODES + 1);
    assert!(matches!(
        html_to_document(&many, &ClipOptions::default()),
        Err(ClipError::TooLarge { .. })
    ));
    for odd in [
        "",
        "<",
        "</",
        "<!--",
        "<a href=\"x",
        "<p",
        "&",
        "&#",
        "&#x;",
        "<title>t",
    ] {
        html_to_document(odd, &ClipOptions::default()).unwrap();
    }
}

#[test]
fn relative_links_resolve_against_the_page() {
    let base = Some("https://site.example/docs/guide/page.html?q=1#frag");
    let cases = [
        ("other.html", "https://site.example/docs/guide/other.html"),
        ("../up.html", "https://site.example/docs/up.html"),
        ("../../../../top", "https://site.example/top"),
        ("/root?x=1", "https://site.example/root?x=1"),
        ("//cdn.example/a", "https://cdn.example/a"),
        (
            "#part",
            "https://site.example/docs/guide/page.html?q=1#part",
        ),
        (
            "?page=2",
            "https://site.example/docs/guide/page.html?page=2",
        ),
        ("mailto:help@site.example", "mailto:help@site.example"),
    ];
    for (href, expected) in cases {
        assert_eq!(
            resolve_link(base, href).as_deref(),
            Some(expected),
            "{href}"
        );
    }
    for unsafe_link in [
        "javascript:alert(1)",
        " JavaScript:x",
        "data:text/html,x",
        "vbscript:x",
    ] {
        assert_eq!(resolve_link(base, unsafe_link), None, "{unsafe_link}");
    }
    assert_eq!(resolve_link(None, "relative.html"), None);
    assert_eq!(
        resolve_link(Some("https://site.example"), "a"),
        Some("https://site.example/a".into())
    );
}

/// An OCR engine that reads the same words from any image.
struct FakeOcr;

impl OcrEngine for FakeOcr {
    fn name(&self) -> String {
        "fake".into()
    }

    fn recognise(&self, _image: &Path) -> Result<OcrPage, ImportError> {
        let word = |text: &str, left: f32| OcrWord {
            text: text.into(),
            left,
            top: 10.0,
            width: 40.0,
            height: 12.0,
            confidence: Confidence::new(0.95),
        };
        Ok(OcrPage {
            width: 200.0,
            height: 100.0,
            paragraphs: vec![OcrParagraph {
                lines: vec![OcrLine {
                    left: 10.0,
                    top: 10.0,
                    width: 100.0,
                    height: 12.0,
                    words: vec![word("Screenshot", 10.0), word("text", 60.0)],
                }],
            }],
        })
    }
}

/// A PNG signature followed by padding, enough for the type to be recognised, as base64.
const PNG_START_BASE64: &str = "iVBORw0KGgowMDAw";

#[test]
fn images_go_through_ocr_and_unknown_types_are_refused() {
    let engine = FakeOcr;
    let tools = Tools {
        ocr: Some(&engine),
        rasteriser: None,
    };
    let image = CaptureContent::Image {
        path: None,
        data_base64: Some(PNG_START_BASE64.into()),
        name: Some("Lecture slide.png".into()),
    };
    let entry = convert(&request(image, 9), &tools).unwrap();
    assert_eq!(entry.title, "Lecture slide");
    assert_eq!(entry.source.file_name.as_deref(), Some("Lecture slide.png"));
    let ClipContent::Document { document } = &entry.content else {
        panic!("not a document")
    };
    assert_eq!(document.plain_text(), "Screenshot text");
    assert!(document.clip.is_some());

    // without an OCR engine the image is refused with the reason
    let image = CaptureContent::Image {
        path: None,
        data_base64: Some(PNG_START_BASE64.into()),
        name: None,
    };
    assert!(matches!(
        convert(&request(image, 9), &Tools::default()),
        Err(ClipError::Import(ImportError::NoOcrEngine))
    ));
    let not_image = CaptureContent::Image {
        path: None,
        data_base64: Some("aGVsbG8gd29ybGQ=".into()),
        name: None,
    };
    assert!(matches!(
        convert(&request(not_image, 9), &tools),
        Err(ClipError::Import(ImportError::Unsupported(_)))
    ));
    let neither = CaptureContent::Image {
        path: None,
        data_base64: None,
        name: None,
    };
    assert!(matches!(
        convert(&request(neither, 9), &tools),
        Err(ClipError::Invalid(_))
    ));
    let wrong_extension = CaptureContent::Image {
        path: Some("scan.heic".into()),
        data_base64: None,
        name: None,
    };
    assert!(matches!(
        convert(&request(wrong_extension, 9), &tools),
        Err(ClipError::Import(ImportError::Unsupported(ref e))) if e == "heic"
    ));
}

#[test]
fn files_import_by_type_and_unsupported_types_say_so() {
    let dir = tempfile::tempdir().unwrap();
    let docx = dir.path().join("Essay.docx");
    std::fs::write(&docx, b"PK\x03\x04").unwrap();
    let err = convert(
        &request(CaptureContent::File { path: docx }, 1),
        &Tools::default(),
    )
    .unwrap_err();
    assert!(
        matches!(err, ClipError::Import(ImportError::Unsupported(ref e)) if e == "docx"),
        "{err}"
    );
    assert_eq!(err.to_string(), "docx files cannot be imported yet");

    let missing = convert(
        &request(
            CaptureContent::File {
                path: dir.path().join("gone.pdf"),
            },
            1,
        ),
        &Tools::default(),
    )
    .unwrap_err();
    assert!(matches!(missing, ClipError::Import(ImportError::Io(_))));

    // a saved page is read like a clip, in whatever encoding it was saved
    let saved = dir.path().join("Saved page.html");
    let mut bytes = b"<html><body><main><h1>Saved</h1><p>Caf".to_vec();
    bytes.extend_from_slice(b"\xe9 notes</p></main></body></html>");
    std::fs::write(&saved, bytes).unwrap();
    let entry = convert(
        &request(CaptureContent::File { path: saved }, 1),
        &Tools::default(),
    )
    .unwrap();
    assert_eq!(entry.title, "Saved");
    assert_eq!(entry.source.file_name.as_deref(), Some("Saved page.html"));
    let ClipContent::Document { document } = &entry.content else {
        panic!("not a document")
    };
    assert_eq!(document.plain_text(), "Saved\n\nCaf\u{fffd} notes");
    assert_eq!(document.source.file_name, "Saved page.html");

    let notes = dir.path().join("todo.md");
    std::fs::write(&notes, "# Todo\n\n- read chapter 2\n- try the quiz\n").unwrap();
    let entry = convert(
        &request(CaptureContent::File { path: notes }, 1),
        &Tools::default(),
    )
    .unwrap();
    assert_eq!(entry.title, "todo");
    let ClipContent::Document { document } = &entry.content else {
        panic!("not a document")
    };
    assert_eq!(kinds(document), ["h1", "ul"]);
}

#[test]
fn the_documented_example_request_parses() {
    let doc = include_str!("../../../../docs/universal-capture.md");
    let start = doc.find("```json\n").expect("example in the doc") + 8;
    let end = start + doc[start..].find("```").unwrap();
    let req = CaptureRequest::from_json(&doc[start..end]).unwrap();
    assert_eq!(req.via, CaptureVia::Extension);
    assert!(matches!(req.content, CaptureContent::WebClip { .. }));
    assert_eq!(
        CaptureRequest::from_json(&req.to_json().unwrap()).unwrap(),
        req
    );
    convert(&req, &Tools::default()).unwrap();

    for via in ["share_sheet", "clipboard", "shortcut"] {
        let json = format!(
            r#"{{"version":1,"via":"{via}","captured_at":1,"content":{{"kind":"text","text":"hi"}}}}"#
        );
        CaptureRequest::from_json(&json).unwrap();
    }
    let file = r#"{"version":1,"via":"shortcut","captured_at":1,"content":{"kind":"file","path":"/tmp/a.pdf"}}"#;
    assert!(matches!(
        CaptureRequest::from_json(file).unwrap().content,
        CaptureContent::File { .. }
    ));
}

fn library_at(clock: std::sync::Arc<std::sync::atomic::AtomicI64>) -> Library {
    Library::open_in_memory()
        .unwrap()
        .with_clock(move || clock.load(std::sync::atomic::Ordering::SeqCst))
}

#[test]
fn the_same_capture_twice_within_the_window_is_kept_once() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};
    let clock = Arc::new(AtomicI64::new(1_000));
    let mut lib = library_at(clock.clone());
    let html = fixture("article.html");
    let url = "https://signals.example/guides/low-pass";

    let first = capture_into(
        &mut lib,
        &request(web(url, &html), 1_000),
        &Tools::default(),
    )
    .unwrap();
    let CaptureOutcome::Stored(id) = first.clone() else {
        panic!("first capture not stored")
    };
    assert!(id.0.starts_with("c-"));
    clock.store(1_060, Ordering::SeqCst);
    let again = capture_into(
        &mut lib,
        &request(web(url, &html), 1_060),
        &Tools::default(),
    )
    .unwrap();
    assert_eq!(again, CaptureOutcome::Duplicate(id.clone()));

    // the same text from the clipboard and then the share sheet is still one capture
    let mut clipboard = request(text("Remember the lab"), 1_100);
    clipboard.via = CaptureVia::Clipboard;
    let note = capture_into(&mut lib, &clipboard, &Tools::default()).unwrap();
    let mut shared = clipboard.clone();
    shared.via = CaptureVia::ShareSheet;
    assert_eq!(
        capture_into(&mut lib, &shared, &Tools::default()).unwrap(),
        CaptureOutcome::Duplicate(note.id().clone())
    );

    // outside the window it is stored again, as is different content
    let later = capture_into(
        &mut lib,
        &request(web(url, &html), 1_000 + DUPLICATE_WINDOW_SECS + 1),
        &Tools::default(),
    )
    .unwrap();
    assert!(matches!(later, CaptureOutcome::Stored(_)));
    let other = capture_into(
        &mut lib,
        &request(web(url, "<p>Changed page</p>"), 1_070),
        &Tools::default(),
    )
    .unwrap();
    assert!(matches!(other, CaptureOutcome::Stored(_)));

    // a clip moved to the bin no longer counts, so capturing it again keeps it
    lib.delete(note.id()).unwrap();
    assert!(matches!(
        capture_into(&mut lib, &clipboard, &Tools::default()).unwrap(),
        CaptureOutcome::Stored(_)
    ));
    assert_eq!(lib.inbox().unwrap().len(), 4);
}

#[test]
fn clips_stay_new_until_opened() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicI64;
    let mut lib = library_at(Arc::new(AtomicI64::new(50)));
    let a = capture_into(
        &mut lib,
        &request(text("First note"), 10),
        &Tools::default(),
    )
    .unwrap()
    .id()
    .clone();
    let b = capture_into(
        &mut lib,
        &request(text("https://example.org/later"), 11),
        &Tools::default(),
    )
    .unwrap()
    .id()
    .clone();
    assert_eq!(lib.new_clip_count().unwrap(), 2);
    let inbox = lib.inbox().unwrap();
    assert!(inbox.iter().all(|i| i.state == InboxState::New));
    assert_eq!(inbox.len(), 2);

    // reading for a preview leaves it new, opening marks it
    assert_eq!(lib.clip(&a).unwrap().state, InboxState::New);
    let opened = lib.open_clip(&a).unwrap();
    assert_eq!(opened.state, InboxState::Opened);
    assert_eq!(lib.clip(&a).unwrap().state, InboxState::Opened);
    assert_eq!(lib.versions(&a).unwrap().len(), 2);
    lib.open_clip(&a).unwrap();
    assert_eq!(
        lib.versions(&a).unwrap().len(),
        2,
        "opening twice writes once"
    );
    assert_eq!(lib.new_clip_count().unwrap(), 1);
    let link = lib.clip(&b).unwrap();
    assert_eq!(link.title, "https://example.org/later");
    assert_eq!(lib.item(&b).unwrap().kind, crate::library::ItemKind::Clip);
    assert!(matches!(
        lib.document(&a),
        Err(LibraryError::WrongKind { .. })
    ));

    // a clip leaves in the full export with its page and comes back
    let dir = tempfile::tempdir().unwrap();
    lib.export_all(dir.path()).unwrap();
    assert!(dir.path().join(format!("clips/{a}.json")).exists());
    assert!(dir.path().join(format!("clips/{a}.html")).exists());
    assert!(!dir.path().join(format!("clips/{b}.html")).exists());
    let mut other = Library::open_in_memory().unwrap();
    assert_eq!(other.import_all(dir.path()).unwrap(), 2);
    assert_eq!(other.clip(&a).unwrap(), lib.clip(&a).unwrap());
    assert_eq!(other.restore_version(&a, 1).unwrap().version, 3);
    assert_eq!(other.clip(&a).unwrap().state, InboxState::New);
}

#[test]
fn a_saved_entry_without_a_state_reads_as_new() {
    let json = r#"{"title":"t","source":{"via":"clipboard","captured_at":3},"fingerprint":"f","content":{"type":"link","url":"https://a.example"}}"#;
    let entry: ClipEntry = serde_json::from_str(json).unwrap();
    assert_eq!(entry.state, InboxState::New);
    assert!(!entry.source.selection_only);
}
