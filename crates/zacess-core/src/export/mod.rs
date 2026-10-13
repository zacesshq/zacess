//! Turning a document plus a reading profile into accessible output.

pub mod braille;
pub mod epub;
pub mod html;
pub mod mathml;
mod zip;

use crate::profile::ReadingProfile;
use std::path::Path;

pub use braille::{
    BrailleCode, BrailleOptions, BrailleTranslator, LibLouis, to_brf, to_unicode_braille, write_brf,
};
pub use epub::{to_epub, write_epub};
pub use html::{Outline, Rendered, to_html};

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("writing the export: {0}")]
    Io(#[from] std::io::Error),
    #[error("braille translation: {0}")]
    Braille(String),
    #[error("no braille translator is available on this device")]
    NoBrailleTranslator,
}

/// How a document is exported.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions<'a> {
    /// The reading profile whose settings style the output. The standard profile when `None`.
    pub profile: Option<&'a ReadingProfile>,
    /// The BCP 47 language tag of the text. English when empty.
    pub language: String,
    /// A title to use instead of the document's own.
    pub title: Option<String>,
    /// Where image assets named by the document live, so an export can bundle or check them.
    pub asset_dir: Option<&'a Path>,
    /// When the export was made, in Unix seconds. The system clock when `None`. WebAssembly in a
    /// browser has no clock of its own, so the browser build passes the page's.
    pub modified: Option<u64>,
}

impl ExportOptions<'_> {
    pub fn profile(&self) -> ReadingProfile {
        self.profile.cloned().unwrap_or_default()
    }

    pub fn language(&self) -> &str {
        if self.language.trim().is_empty() {
            "en"
        } else {
            self.language.trim()
        }
    }

    /// Whether an asset the document names can be found.
    pub fn asset_exists(&self, asset: &str) -> bool {
        self.asset_dir.is_some_and(|dir| dir.join(asset).is_file())
    }
}

/// Escapes text for XML and HTML content and attribute values.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}
