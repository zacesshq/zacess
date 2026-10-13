//! Turning files into documents.
//!
//! Every importer ends in the same place: positioned text goes through [`layout`] and comes out as
//! a [`Document`]. Which engine recognises text in an image is behind [`ocr::OcrEngine`].

pub mod image;
pub mod layout;
pub mod ocr;
pub mod pdf;
pub mod render;

#[cfg(test)]
mod tests;

use crate::model::{BlockKind, Document, Origin, Page, Rect, Source};
use layout::PageLayout;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("{0} files cannot be imported yet")]
    Unsupported(String),
    #[error("reading the file: {0}")]
    Io(#[from] std::io::Error),
    #[error("reading the PDF: {0}")]
    Pdf(String),
    #[error("the PDF is encrypted")]
    Encrypted,
    #[error("recognising text: {0}")]
    Ocr(String),
    #[error("no OCR engine is available on this device")]
    NoOcrEngine,
}

/// A document plus what the importer could not finish on its own.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub document: Document,
    /// Pages with no text layer, numbered from 1, that still need OCR before they have content.
    pub pages_needing_ocr: Vec<u32>,
    /// Pages whose text came from OCR, so every run on them carries a measured confidence.
    pub pages_recognised: Vec<u32>,
}

/// The tools an import can use beyond the file itself.
#[derive(Default)]
pub struct Tools<'a> {
    pub ocr: Option<&'a dyn ocr::OcrEngine>,
    pub rasteriser: Option<&'a render::PdfRasteriser>,
}

impl Tools<'_> {
    /// Whatever is installed on this device.
    pub fn detect() -> OwnedTools {
        OwnedTools {
            ocr: ocr::Tesseract::detect(),
            rasteriser: render::PdfRasteriser::detect(),
        }
    }
}

/// Detected tools, owned so they can be borrowed as [`Tools`].
#[derive(Debug, Default, Clone)]
pub struct OwnedTools {
    pub ocr: Option<ocr::Tesseract>,
    pub rasteriser: Option<render::PdfRasteriser>,
}

impl OwnedTools {
    pub fn as_tools(&self) -> Tools<'_> {
        Tools {
            ocr: self.ocr.as_ref().map(|t| t as &dyn ocr::OcrEngine),
            rasteriser: self.rasteriser.as_ref(),
        }
    }
}

/// Imports a file with whatever tools are installed on this device.
pub fn import_file(path: &Path) -> Result<Imported, ImportError> {
    let owned = Tools::detect();
    import_file_with(path, &owned.as_tools())
}

/// Imports a file, choosing the importer from its extension.
pub fn import_file_with(path: &Path, tools: &Tools<'_>) -> Result<Imported, ImportError> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match extension.as_str() {
        "pdf" => pdf::import_pdf_with(path, tools),
        ext if image::IMAGE_EXTENSIONS.contains(&ext) => {
            let engine = tools.ocr.ok_or(ImportError::NoOcrEngine)?;
            image::import_image(path, engine)
        }
        "" => Err(ImportError::Unsupported("extensionless".into())),
        other => Err(ImportError::Unsupported(other.to_string())),
    }
}

/// Builds a document from laid-out pages, with a page break between pages.
pub(crate) fn compose(
    title: String,
    source: Source,
    pages: Vec<Page>,
    layouts: Vec<PageLayout>,
) -> Imported {
    let mut document = Document::new(title, source);
    document.pages = pages;
    let mut pages_needing_ocr = Vec::new();
    for (i, page) in layouts.into_iter().enumerate() {
        if i > 0 {
            document.push(
                BlockKind::PageBreak,
                Origin::new(page.number, Rect::default()),
            );
        }
        if page.blocks.is_empty() {
            pages_needing_ocr.push(page.number);
        }
        for (origin, kind) in page.blocks {
            document.push(kind, origin);
        }
    }
    Imported {
        document,
        pages_needing_ocr,
        pages_recognised: Vec::new(),
    }
}
