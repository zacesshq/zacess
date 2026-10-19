//! Exports a file as accessible HTML, EPUB 3 and, when liblouis is installed, a BRF braille file.
//!
//! ```text
//! cargo run -p zacess-core --example export -- tests/fixtures/slides-signals.pdf out/ [preset]
//! ```
//!
//! The input is imported. A document JSON file is read directly. The preset is one of
//! `low-vision`, `dyslexia`, `clutter` and `screen-reader`; the standard profile is used otherwise.
//! `high-contrast-dark` and `high-contrast-light` give the standard profile in that theme.

use std::path::Path;
use zacess_core::export::{
    BrailleOptions, ExportOptions, LibLouis, to_html, write_brf, write_epub,
};
use zacess_core::import::{Tools, import_file_with};
use zacess_core::model::Document;
use zacess_core::profile::{ContrastTheme, Preset, ReadingProfile};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(input), Some(out_dir)) = (args.first(), args.get(1)) else {
        eprintln!(
            "usage: export <file> <out-dir> [low-vision|dyslexia|clutter|screen-reader|high-contrast-dark|high-contrast-light]"
        );
        std::process::exit(2);
    };
    let input = Path::new(input);
    let out_dir = Path::new(out_dir);
    let profile = args.get(2).map(|name| match name.as_str() {
        "low-vision" => ReadingProfile::preset(Preset::LowVision),
        "dyslexia" => ReadingProfile::preset(Preset::DyslexiaFriendly),
        "clutter" => ReadingProfile::preset(Preset::ReducedVisualClutter),
        "screen-reader" => ReadingProfile::preset(Preset::ScreenReader),
        "high-contrast-dark" => ReadingProfile {
            contrast: ContrastTheme::HighContrastDark,
            ..ReadingProfile::default()
        },
        "high-contrast-light" => ReadingProfile {
            contrast: ContrastTheme::HighContrastLight,
            ..ReadingProfile::default()
        },
        other => {
            eprintln!("unknown preset {other}");
            std::process::exit(2);
        }
    });

    let document = match load(input) {
        Ok(document) => document,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let options = ExportOptions {
        profile: profile.as_ref(),
        asset_dir: input.parent(),
        ..ExportOptions::default()
    };
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document");
    if let Err(e) = std::fs::create_dir_all(out_dir) {
        eprintln!("error: creating {}: {e}", out_dir.display());
        std::process::exit(1);
    }
    let html_path = out_dir.join(format!("{stem}.html"));
    let epub_path = out_dir.join(format!("{stem}.epub"));
    let result = std::fs::write(&html_path, to_html(&document, &options))
        .map_err(|e| e.to_string())
        .and_then(|_| write_epub(&document, &options, &epub_path).map_err(|e| e.to_string()));
    let brf_path = out_dir.join(format!("{stem}.brf"));
    let result = result.and_then(|_| match LibLouis::detect() {
        Some(louis) => write_brf(&document, &louis, &BrailleOptions::default(), &brf_path)
            .map(|_| println!("{}", brf_path.display()))
            .map_err(|e| e.to_string()),
        None => {
            eprintln!("note: liblouis is not installed, so no braille file was written");
            Ok(())
        }
    });
    match result {
        Ok(()) => println!("{}\n{}", html_path.display(), epub_path.display()),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn load(input: &Path) -> Result<Document, String> {
    if input.extension().is_some_and(|e| e == "json") {
        let json = std::fs::read_to_string(input).map_err(|e| e.to_string())?;
        return Document::from_json(&json).map_err(|e| e.to_string());
    }
    let tools = Tools::detect();
    let imported = import_file_with(input, &tools.as_tools()).map_err(|e| e.to_string())?;
    if !imported.pages_needing_ocr.is_empty() {
        eprintln!(
            "note: pages {:?} have no text layer and no OCR engine was available",
            imported.pages_needing_ocr
        );
    }
    Ok(imported.document)
}
