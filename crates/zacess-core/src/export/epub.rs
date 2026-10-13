//! EPUB 3 export: the HTML body as an XHTML content document, a navigation document with a table
//! of contents and a page list, the package file with accessibility metadata and the style sheet,
//! all in a stored ZIP container.

use super::html::{render_body, stylesheet};
use super::zip::ZipWriter;
use super::{ExportError, ExportOptions, escape};
use crate::model::Document;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";
const EPUB_NS: &str = "http://www.idpf.org/2007/ops";

/// Builds the EPUB as bytes.
pub fn to_epub(document: &Document, options: &ExportOptions<'_>) -> Result<Vec<u8>, ExportError> {
    let mut zip = ZipWriter::new();
    for (name, bytes) in epub_files(document, options)? {
        zip.add(&name, &bytes);
    }
    Ok(zip.finish())
}

pub fn write_epub(
    document: &Document,
    options: &ExportOptions<'_>,
    path: &Path,
) -> Result<(), ExportError> {
    Ok(std::fs::write(path, to_epub(document, options)?)?)
}

/// Every file of the container in order, `mimetype` first as the format requires.
pub fn epub_files(
    document: &Document,
    options: &ExportOptions<'_>,
) -> Result<Vec<(String, Vec<u8>)>, ExportError> {
    let rendered = render_body(document, options, true);
    let outline = &rendered.outline;
    let title = options
        .title
        .clone()
        .unwrap_or_else(|| document.title.clone());
    let lang = options.language();
    let profile = options.profile();

    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("mimetype".into(), b"application/epub+zip".to_vec()),
        (
            "META-INF/container.xml".into(),
            container_xml().into_bytes(),
        ),
    ];

    let mut assets = Vec::new();
    for asset in &outline.assets {
        if let Some(dir) = options.asset_dir {
            let bytes = std::fs::read(dir.join(asset))?;
            files.push((format!("EPUB/{asset}"), bytes));
            assets.push(asset.clone());
        }
    }

    files.push((
        "EPUB/package.opf".into(),
        package_opf(document, &title, lang, outline, &assets).into_bytes(),
    ));
    files.push((
        "EPUB/nav.xhtml".into(),
        nav_xhtml(&title, lang, outline).into_bytes(),
    ));
    files.push((
        "EPUB/content.xhtml".into(),
        content_xhtml(&title, lang, &rendered.body).into_bytes(),
    ));
    files.push(("EPUB/style.css".into(), stylesheet(&profile).into_bytes()));
    Ok(files)
}

fn container_xml() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
     <container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n\
     <rootfiles>\n\
     <rootfile full-path=\"EPUB/package.opf\" media-type=\"application/oebps-package+xml\"/>\n\
     </rootfiles>\n\
     </container>\n"
        .to_string()
}

fn package_opf(
    document: &Document,
    title: &str,
    lang: &str,
    outline: &super::Outline,
    assets: &[String],
) -> String {
    let mut metadata = String::new();
    let mut meta = |property: &str, value: &str| {
        metadata.push_str(&format!(
            "<meta property=\"{property}\">{}</meta>\n",
            escape(value)
        ));
    };
    meta("dcterms:modified", &iso_now());
    meta("schema:accessMode", "textual");
    if outline.has_images {
        meta("schema:accessMode", "visual");
    }
    meta("schema:accessModeSufficient", "textual");
    for feature in ["structuralNavigation", "readingOrder", "tableOfContents"] {
        meta("schema:accessibilityFeature", feature);
    }
    if !outline.pages.is_empty() {
        meta("schema:accessibilityFeature", "pageBreakMarkers");
    }
    if outline.has_mathml {
        meta("schema:accessibilityFeature", "MathML");
    }
    if outline.has_images && outline.images_without_alt == 0 {
        meta("schema:accessibilityFeature", "alternativeText");
    }
    meta("schema:accessibilityHazard", "none");
    let summary = if outline.images_without_alt > 0 {
        format!(
            "Structured text with headings, lists and page markers in reading order. {} image(s) still need a description.",
            outline.images_without_alt
        )
    } else {
        "Structured text with headings, lists and page markers in reading order. Text the recogniser was unsure of is marked and announced.".to_string()
    };
    meta("schema:accessibilitySummary", &summary);

    let mut manifest = String::from(
        "<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n",
    );
    let content_properties = if outline.has_mathml {
        " properties=\"mathml\""
    } else {
        ""
    };
    manifest.push_str(&format!(
        "<item id=\"content\" href=\"content.xhtml\" media-type=\"application/xhtml+xml\"{content_properties}/>\n"
    ));
    manifest.push_str("<item id=\"style\" href=\"style.css\" media-type=\"text/css\"/>\n");
    for (i, asset) in assets.iter().enumerate() {
        manifest.push_str(&format!(
            "<item id=\"asset{i}\" href=\"{}\" media-type=\"{}\"/>\n",
            escape(asset),
            media_type(asset)
        ));
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"pub-id\" xml:lang=\"{lang}\">\n\
         <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n\
         <dc:identifier id=\"pub-id\">{id}</dc:identifier>\n\
         <dc:title>{title}</dc:title>\n\
         <dc:language>{lang}</dc:language>\n\
         {metadata}\
         </metadata>\n\
         <manifest>\n{manifest}</manifest>\n\
         <spine>\n<itemref idref=\"content\"/>\n</spine>\n\
         </package>\n",
        lang = escape(lang),
        id = identifier(document),
        title = escape(title),
    )
}

fn media_type(asset: &str) -> &'static str {
    match asset
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

fn nav_xhtml(title: &str, lang: &str, outline: &super::Outline) -> String {
    let toc = if outline.headings.is_empty() {
        format!(
            "<ol>\n<li><a href=\"content.xhtml\">{}</a></li>\n</ol>\n",
            escape(title)
        )
    } else {
        render_toc(&toc_tree(&outline.headings))
    };

    let mut pages = String::new();
    if !outline.pages.is_empty() {
        pages.push_str("<nav epub:type=\"page-list\" role=\"doc-pagelist\" hidden=\"\">\n<h1>Pages</h1>\n<ol>\n");
        for (number, id) in &outline.pages {
            pages.push_str(&format!(
                "<li><a href=\"content.xhtml#{id}\">{number}</a></li>\n"
            ));
        }
        pages.push_str("</ol>\n</nav>\n");
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n\
         <html xmlns=\"{XHTML_NS}\" xmlns:epub=\"{EPUB_NS}\" lang=\"{lang}\" xml:lang=\"{lang}\">\n\
         <head>\n<meta charset=\"utf-8\"/>\n<title>Contents</title>\n</head>\n<body>\n\
         <nav epub:type=\"toc\" role=\"doc-toc\" id=\"toc\">\n<h1>Contents</h1>\n{toc}</nav>\n{pages}</body>\n</html>\n",
        lang = escape(lang),
    )
}

/// A heading and the headings nested under it.
#[derive(Debug, Default)]
struct TocNode {
    id: String,
    text: String,
    children: Vec<TocNode>,
}

/// Nests headings by level. A heading nests under the nearest heading above it with a lower
/// level, so a document that skips a level still gives a well-formed tree.
fn toc_tree(headings: &[(u8, String, String)]) -> Vec<TocNode> {
    let mut roots: Vec<TocNode> = Vec::new();
    let mut levels: Vec<u8> = Vec::new();
    for (level, id, text) in headings {
        while levels.last().is_some_and(|l| l >= level) {
            levels.pop();
        }
        let node = TocNode {
            id: id.clone(),
            text: text.clone(),
            children: Vec::new(),
        };
        let mut siblings = &mut roots;
        for _ in 0..levels.len() {
            siblings = &mut siblings
                .last_mut()
                .expect("every level on the stack has a node")
                .children;
        }
        siblings.push(node);
        levels.push(*level);
    }
    roots
}

fn render_toc(nodes: &[TocNode]) -> String {
    let mut out = String::from("<ol>\n");
    for node in nodes {
        let shown = if node.text.trim().is_empty() {
            "Untitled section".to_string()
        } else {
            escape(&node.text)
        };
        out.push_str(&format!(
            "<li><a href=\"content.xhtml#{}\">{shown}</a>",
            node.id
        ));
        if !node.children.is_empty() {
            out.push('\n');
            out.push_str(&render_toc(&node.children));
        }
        out.push_str("</li>\n");
    }
    out.push_str("</ol>\n");
    out
}

fn content_xhtml(title: &str, lang: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n\
         <html xmlns=\"{XHTML_NS}\" xmlns:epub=\"{EPUB_NS}\" lang=\"{lang}\" xml:lang=\"{lang}\">\n\
         <head>\n<meta charset=\"utf-8\"/>\n<title>{title}</title>\n\
         <link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>\n</head>\n<body>\n{body}</body>\n</html>\n",
        lang = escape(lang),
        title = escape(title),
    )
}

/// A stable identifier for the document, so re-exporting it gives the same publication.
fn identifier(document: &Document) -> String {
    let seed = format!("{}\u{0}{}", document.title, document.source.file_name);
    let a = fnv1a(seed.as_bytes(), 0xcbf2_9ce4_8422_2325);
    let b = fnv1a(seed.as_bytes(), 0x84222325_cbf29ce4);
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_be_bytes());
    bytes[8..].copy_from_slice(&b.to_be_bytes());
    // a version 4, variant 1 layout so readers that validate UUIDs accept it
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

fn fnv1a(bytes: &[u8], seed: u64) -> u64 {
    bytes.iter().fold(seed, |hash, b| {
        (hash ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The current time as `dcterms:modified` wants it, without a date dependency.
fn iso_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (h, m, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    // days since 1970-01-01 to a civil date, per Howard Hinnant's algorithm
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        Document::from_json(include_str!("../../../../docs/examples/document.json")).unwrap()
    }

    fn text_of<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> &'a str {
        let (_, bytes) = files
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name} missing"));
        std::str::from_utf8(bytes).unwrap()
    }

    #[test]
    fn the_container_has_every_required_file_in_order() {
        let files = epub_files(&doc(), &ExportOptions::default()).unwrap();
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "mimetype",
                "META-INF/container.xml",
                "EPUB/package.opf",
                "EPUB/nav.xhtml",
                "EPUB/content.xhtml",
                "EPUB/style.css"
            ]
        );
        assert_eq!(files[0].1, b"application/epub+zip");
        assert!(
            text_of(&files, "META-INF/container.xml").contains("full-path=\"EPUB/package.opf\"")
        );
    }

    #[test]
    fn the_package_declares_mathml_and_accessibility_metadata() {
        let files = epub_files(&doc(), &ExportOptions::default()).unwrap();
        let opf = text_of(&files, "EPUB/package.opf");
        assert!(opf.contains("<dc:title>Signals week 3</dc:title>"));
        assert!(opf.contains("<dc:language>en</dc:language>"));
        assert!(opf.contains(
            "href=\"content.xhtml\" media-type=\"application/xhtml+xml\" properties=\"mathml\""
        ));
        assert!(
            opf.contains("<meta property=\"schema:accessibilityFeature\">pageBreakMarkers</meta>")
        );
        assert!(opf.contains("<meta property=\"schema:accessibilityFeature\">MathML</meta>"));
        assert!(opf.contains("<meta property=\"schema:accessMode\">visual</meta>"));
        assert!(
            !opf.contains(">alternativeText<"),
            "the example image has no description"
        );
        assert!(opf.contains("1 image(s) still need a description"));
        let modified = opf.split("dcterms:modified\">").nth(1).unwrap();
        assert_eq!(modified.as_bytes()[4], b'-');
        assert_eq!(modified.as_bytes()[10], b'T');
        assert_eq!(modified.as_bytes()[19], b'Z');
    }

    #[test]
    fn the_nav_has_a_toc_and_a_page_list() {
        let files = epub_files(&doc(), &ExportOptions::default()).unwrap();
        let nav = text_of(&files, "EPUB/nav.xhtml");
        assert!(nav.contains("<nav epub:type=\"toc\" role=\"doc-toc\""));
        assert!(nav.contains("<li><a href=\"content.xhtml#b1\">Fourier series</a></li>"));
        assert!(nav.contains("<nav epub:type=\"page-list\" role=\"doc-pagelist\" hidden=\"\">"));
        assert!(nav.contains("<li><a href=\"content.xhtml#page2\">2</a></li>"));
        let content = text_of(&files, "EPUB/content.xhtml");
        assert!(content.contains("xmlns:epub=\"http://www.idpf.org/2007/ops\""));
        assert!(content.contains("epub:type=\"pagebreak\""));
        assert!(
            content.contains("<link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>")
        );
    }

    #[test]
    fn nested_headings_nest_in_the_toc() {
        let outline = super::super::Outline {
            headings: vec![
                (1, "a".into(), "One".into()),
                (2, "b".into(), "One point one".into()),
                (3, "c".into(), "Deep".into()),
                (2, "d".into(), "One point two".into()),
                (1, "e".into(), "Two".into()),
            ],
            ..Default::default()
        };
        let nav = nav_xhtml("t", "en", &outline);
        let toc = nav
            .split("<h1>Contents</h1>\n")
            .nth(1)
            .unwrap()
            .split("</nav>")
            .next()
            .unwrap();
        assert_eq!(toc.matches("<ol>").count(), toc.matches("</ol>").count());
        assert_eq!(toc.matches("<li>").count(), toc.matches("</li>").count());
        assert_eq!(
            toc,
            "<ol>\n<li><a href=\"content.xhtml#a\">One</a>\n<ol>\n<li><a href=\"content.xhtml#b\">One point one</a>\n<ol>\n<li><a href=\"content.xhtml#c\">Deep</a></li>\n</ol>\n</li>\n<li><a href=\"content.xhtml#d\">One point two</a></li>\n</ol>\n</li>\n<li><a href=\"content.xhtml#e\">Two</a></li>\n</ol>\n"
        );
    }

    #[test]
    fn identifiers_are_stable_and_uuid_shaped() {
        let id = identifier(&doc());
        assert_eq!(id, identifier(&doc()));
        assert!(id.starts_with("urn:uuid:"));
        let parts: Vec<&str> = id["urn:uuid:".len()..].split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('4'));
    }

    #[test]
    fn the_epub_is_a_zip_starting_with_the_mimetype() {
        let bytes = to_epub(&doc(), &ExportOptions::default()).unwrap();
        assert_eq!(&bytes[0..2], b"PK");
        assert_eq!(&bytes[30..38], b"mimetype");
        assert_eq!(&bytes[38..58], b"application/epub+zip");
    }

    #[test]
    fn epubcheck_passes_when_it_is_installed() {
        let Ok(jar) = std::env::var("EPUBCHECK_JAR") else {
            eprintln!("skipped: set EPUBCHECK_JAR to the epubcheck.jar path to run EPUBCheck");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("example.epub");
        write_epub(&doc(), &ExportOptions::default(), &path).unwrap();
        let output = std::process::Command::new("java")
            .args(["-jar", &jar, "--quiet", "--failonwarnings"])
            .arg(&path)
            .output()
            .unwrap();
        let report = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success(),
            "EPUBCheck reported problems:\n{report}"
        );
    }
}
