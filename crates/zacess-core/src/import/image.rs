//! Importing images and scanned pages through OCR.

use super::layout::{self, PageSize};
use super::ocr::{self, OcrEngine};
use super::{ImportError, Imported, compose};
use crate::model::{Page, Source, SourceFormat, Unit};
use std::path::Path;

/// File extensions treated as images.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff", "bmp", "webp", "gif"];

/// Recognises the text in an image and lays it out as a one-page document measured in pixels.
pub fn import_image(path: &Path, engine: &dyn OcrEngine) -> Result<Imported, ImportError> {
    if !path.is_file() {
        return Err(ImportError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            path.display().to_string(),
        )));
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("image")
        .to_string();
    let title = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Image")
        .to_string();

    let recognised = engine.recognise(path)?;
    let pages = vec![Page {
        number: 1,
        width: recognised.width,
        height: recognised.height,
        unit: Unit::Px,
    }];
    let sizes = [PageSize {
        number: 1,
        width: recognised.width,
        height: recognised.height,
    }];
    let fragments = ocr::fragments(&recognised, 1, 1.0);
    let layouts = layout::analyse(&sizes, &fragments);

    let mut imported = compose(
        title,
        Source {
            file_name,
            format: SourceFormat::Image,
        },
        pages,
        layouts,
    );
    // the engine ran, so an empty page is a blank image rather than a page still waiting for OCR
    imported.pages_recognised = vec![1];
    imported.pages_needing_ocr.clear();
    Ok(imported)
}
