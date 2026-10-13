//! Rendering a PDF page to an image, for pages that have no text layer and need OCR.

use super::ImportError;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Dots per inch to render at. Tesseract's models are trained on roughly 300 dpi text.
pub const RENDER_DPI: u32 = 300;

/// Poppler's `pdftoppm`, run as a command.
#[derive(Debug, Clone)]
pub struct PdfRasteriser {
    command: PathBuf,
    pub dpi: u32,
}

impl PdfRasteriser {
    /// Finds `pdftoppm` on the path. Returns `None` when it is not installed.
    pub fn detect() -> Option<Self> {
        let candidate = PdfRasteriser {
            command: PathBuf::from("pdftoppm"),
            dpi: RENDER_DPI,
        };
        let ok = Command::new(&candidate.command)
            .arg("-v")
            .output()
            .is_ok_and(|o| o.status.success());
        ok.then_some(candidate)
    }

    pub fn new(command: impl Into<PathBuf>, dpi: u32) -> Self {
        PdfRasteriser {
            command: command.into(),
            dpi,
        }
    }

    /// Renders one page, numbered from 1, to a PNG inside `out_dir` and returns its path.
    pub fn render_page(
        &self,
        pdf: &Path,
        page: u32,
        out_dir: &Path,
    ) -> Result<PathBuf, ImportError> {
        let prefix = out_dir.join(format!("page-{page}"));
        let output = Command::new(&self.command)
            .args([
                "-r",
                &self.dpi.to_string(),
                "-f",
                &page.to_string(),
                "-l",
                &page.to_string(),
                "-singlefile",
                "-png",
            ])
            .arg(pdf)
            .arg(&prefix)
            .output()
            .map_err(|e| ImportError::Ocr(format!("running {}: {e}", self.command.display())))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ImportError::Ocr(format!(
                "pdftoppm failed on page {page}: {}",
                stderr.trim()
            )));
        }
        let path = prefix.with_extension("png");
        if !path.is_file() {
            return Err(ImportError::Ocr(format!(
                "pdftoppm produced no image for page {page}"
            )));
        }
        Ok(path)
    }
}
