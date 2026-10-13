//! End-to-end import tests against the fixtures. The OCR tests run only where Tesseract and
//! pdftoppm are installed. They say so when they are skipped.

use super::ocr::Tesseract;
use super::render::PdfRasteriser;
use super::*;
use crate::confidence::{flags_for, uncertain_runs};
use crate::model::{BlockKind, SourceFormat, Unit};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn tesseract() -> Option<Tesseract> {
    let found = Tesseract::detect();
    if found.is_none() {
        eprintln!("skipped: tesseract is not installed");
    }
    found
}

fn headings(doc: &Document) -> Vec<String> {
    doc.blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::Heading { .. } => Some(b.text()),
            _ => None,
        })
        .collect()
}

#[test]
fn unknown_formats_and_missing_engines_are_clear_errors() {
    let err = import_file_with(Path::new("notes.xyz"), &Tools::default()).unwrap_err();
    assert!(
        matches!(err, ImportError::Unsupported(ref e) if e == "xyz"),
        "{err}"
    );
    let err = import_file_with(&fixture("handout-scan.png"), &Tools::default()).unwrap_err();
    assert!(matches!(err, ImportError::NoOcrEngine), "{err}");
    assert!(matches!(
        import_file_with(Path::new("README"), &Tools::default()).unwrap_err(),
        ImportError::Unsupported(_)
    ));
}

#[test]
fn a_scanned_handout_image_is_read_with_its_structure() {
    let Some(engine) = tesseract() else { return };
    let imported = image::import_image(&fixture("handout-scan.png"), &engine).unwrap();
    let doc = &imported.document;
    assert_eq!(imported.pages_recognised, [1]);
    assert!(imported.pages_needing_ocr.is_empty());
    assert_eq!(doc.pages[0].unit, Unit::Px);
    assert_eq!(doc.source.format, SourceFormat::Image);

    let found = headings(doc);
    assert_eq!(
        found.first().map(String::as_str),
        Some("Laboratory safety handout"),
        "{found:?}"
    );
    assert!(found.iter().any(|h| h == "Before you start"), "{found:?}");
    let list = doc.blocks.iter().find_map(|b| match &b.kind {
        BlockKind::List { items, .. } => Some(items.len()),
        _ => None,
    });
    assert_eq!(list, Some(3));
    assert!(doc.plain_text().contains("Press the red isolation button"));

    // every run sits inside the image and carries a measured confidence
    for run in doc.runs() {
        let origin = run.origin.unwrap();
        assert_eq!(origin.page, 1);
        assert!(origin.region.x + origin.region.width <= doc.pages[0].width + 1.0);
        assert!(run.confidence.value() <= 1.0);
    }
}

#[test]
fn numbers_in_a_scan_are_flagged_unless_the_engine_is_sure() {
    let Some(engine) = tesseract() else { return };
    let imported = image::import_image(&fixture("notes-numbers-scan.png"), &engine).unwrap();
    let doc = &imported.document;
    assert_eq!(headings(doc), ["Potential divider notes"]);

    let flagged: Vec<&str> = uncertain_runs(doc).map(|(_, r)| r.text.trim()).collect();
    assert!(flagged.len() >= 5, "{flagged:?}");
    // a numeric run below 95% is flagged even though a word at that confidence would not be
    assert!(
        doc.runs()
            .any(|r| r.text.chars().any(|c| c.is_ascii_digit())
                && (0.80..0.95).contains(&r.confidence.value())),
        "the fixture should hold a number between 80% and 95%"
    );
    for block in &doc.blocks {
        let runs = block.runs();
        for (run, flag) in runs.iter().zip(flags_for(&runs)) {
            let numeric = run.text.chars().any(|c| c.is_ascii_digit());
            if numeric && run.confidence.value() < 0.95 {
                assert!(
                    flag,
                    "{} at {:.0}% should be flagged",
                    run.text,
                    run.confidence.value() * 100.0
                );
            }
            if !numeric && run.confidence.value() >= 0.95 {
                assert!(
                    !flag,
                    "{} at {:.0}% should not be flagged",
                    run.text,
                    run.confidence.value() * 100.0
                );
            }
        }
    }
    // the unit after 4.7 is held to the numeric threshold whatever the engine made of it
    let after_4_7 = doc.blocks.iter().find_map(|b| {
        let runs = b.runs();
        let flags = flags_for(&runs);
        runs.iter()
            .zip(flags)
            .skip_while(|(r, _)| r.text.trim() != "4.7")
            .nth(1)
            .map(|(r, f)| (r.text.clone(), f))
    });
    assert!(matches!(after_4_7, Some((_, true))), "{after_4_7:?}");
}

#[test]
fn a_scanned_pdf_is_recognised_when_the_tools_are_installed() {
    let Some(engine) = tesseract() else { return };
    let Some(rasteriser) = PdfRasteriser::detect() else {
        eprintln!("skipped: pdftoppm is not installed");
        return;
    };
    let tools = Tools {
        ocr: Some(&engine),
        rasteriser: Some(&rasteriser),
    };
    let imported = import_file_with(&fixture("handout-scan.pdf"), &tools).unwrap();
    let doc = &imported.document;
    assert_eq!(imported.pages_recognised, [1]);
    assert!(imported.pages_needing_ocr.is_empty());
    assert_eq!(doc.pages[0].unit, Unit::Pt);
    assert_eq!(
        headings(doc).first().map(String::as_str),
        Some("Laboratory safety handout")
    );
    // pixels were scaled back to points, so every region fits the page
    for run in doc.runs() {
        let region = run.origin.unwrap().region;
        assert!(
            region.x + region.width <= doc.pages[0].width + 1.0,
            "{region:?}"
        );
        assert!(
            region.y + region.height <= doc.pages[0].height + 1.0,
            "{region:?}"
        );
    }
    // a page with a text layer is never sent through OCR
    let text_pdf = import_file_with(&fixture("slides-signals.pdf"), &tools).unwrap();
    assert!(text_pdf.pages_recognised.is_empty());
}
