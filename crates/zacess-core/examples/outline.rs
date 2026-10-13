//! Prints the outline of an imported file, marking every run that needs checking.
//!
//! ```text
//! cargo run -p zacess-core --example outline -- tests/fixtures/handout-scan.png
//! ```

use zacess_core::flags_for;
use zacess_core::import::{Tools, import_file_with};
use zacess_core::model::{BlockKind, Run, join_runs};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: outline <file>");
        std::process::exit(2);
    };
    let tools = Tools::detect();
    let imported = match import_file_with(std::path::Path::new(&path), &tools.as_tools()) {
        Ok(imported) => imported,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let doc = &imported.document;
    println!(
        "{} ({} pages, {} blocks)",
        doc.title,
        doc.pages.len(),
        doc.blocks.len()
    );
    if !imported.pages_recognised.is_empty() {
        println!("pages recognised by OCR: {:?}", imported.pages_recognised);
    }
    if !imported.pages_needing_ocr.is_empty() {
        println!("pages still needing OCR: {:?}", imported.pages_needing_ocr);
    }
    let flagged = |runs: &[Run]| {
        let refs: Vec<&Run> = runs.iter().collect();
        let flags = flags_for(&refs);
        let mut text = String::new();
        for (run, flagged) in runs.iter().zip(flags) {
            if !text.is_empty()
                && !text.ends_with(char::is_whitespace)
                && !run.text.starts_with(char::is_whitespace)
            {
                text.push(' ');
            }
            if flagged {
                text.push_str(&format!(
                    "[{} {:.0}%]",
                    run.text.trim(),
                    run.confidence.value() * 100.0
                ));
                if run.text.ends_with(char::is_whitespace) {
                    text.push(' ');
                }
            } else {
                text.push_str(&run.text);
            }
        }
        text
    };
    for block in &doc.blocks {
        let page = block.origin.page;
        match &block.kind {
            BlockKind::Heading { level, runs } => println!("p{page} h{level}: {}", flagged(runs)),
            BlockKind::Paragraph { runs, .. } => println!("p{page} p: {}", flagged(runs)),
            BlockKind::List { ordered, items } => {
                println!("p{page} {}:", if *ordered { "ol" } else { "ul" });
                for item in items {
                    println!("      - {}", flagged(&item.runs));
                    for child in &item.children {
                        println!("          - {}", flagged(&child.runs));
                    }
                }
            }
            BlockKind::PageBreak => println!("---"),
            other => println!(
                "p{page} {}",
                join_runs(&block.runs().into_iter().cloned().collect::<Vec<_>>())
                    .chars()
                    .take(60)
                    .collect::<String>()
                    + &format!(" ({})", kind_name(other))
            ),
        }
    }
}

fn kind_name(kind: &BlockKind) -> &'static str {
    match kind {
        BlockKind::Table { .. } => "table",
        BlockKind::Image { .. } => "image",
        BlockKind::Equation { .. } => "equation",
        _ => "block",
    }
}
