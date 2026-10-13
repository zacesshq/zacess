//! Universal capture: web clips, shared text, clipboard text, images and files sent straight into
//! the library's inbox.
//!
//! The browser extension, the share sheet, the clipboard watcher and the desktop shortcut all send
//! the same [`CaptureRequest`]. This module turns it into a document or a link note and keeps it in
//! the library as a [`ClipEntry`] marked new until it is opened. Nothing is fetched: a clip holds
//! what the sender passed in and nothing leaves the device. The request format and the rules for
//! web pages are in `docs/universal-capture.md`.

mod base64;
pub mod html;
pub mod text;

#[cfg(all(test, feature = "library"))]
mod tests;

use crate::import::{self, ImportError, Tools};
#[cfg(feature = "library")]
use crate::library::Library;
use crate::library::{ItemId, LibraryError, content_hash};
use crate::model::{Document, Source, SourceFormat};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The version of the request format this build reads.
pub const CAPTURE_REQUEST_VERSION: u32 = 1;
/// The largest request accepted, as JSON. It leaves room for an image sent as base64.
pub const MAX_REQUEST_BYTES: usize = 40 * 1024 * 1024;
/// The largest page or selection accepted as HTML.
pub const MAX_HTML_BYTES: usize = 10 * 1024 * 1024;
/// The largest piece of plain text accepted.
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
/// The largest image accepted, once decoded.
pub const MAX_IMAGE_BYTES: usize = 25 * 1024 * 1024;
/// The largest file accepted by path.
pub const MAX_FILE_BYTES: u64 = 200 * 1024 * 1024;
/// The longest page address accepted.
pub const MAX_URL_BYTES: usize = 8 * 1024;
/// The longest page title accepted.
pub const MAX_TITLE_BYTES: usize = 4 * 1024;
/// Two captures of the same content this close together are one capture.
pub const DUPLICATE_WINDOW_SECS: i64 = 10 * 60;

#[derive(Debug, thiserror::Error)]
pub enum ClipError {
    #[error("the {what} is {size} bytes, more than the {limit} bytes a capture can hold")]
    TooLarge {
        what: &'static str,
        size: u64,
        limit: u64,
    },
    #[error(
        "the capture uses request version {0}, this build reads up to {CAPTURE_REQUEST_VERSION}"
    )]
    UnsupportedVersion(u32),
    #[error("invalid capture: {0}")]
    Invalid(String),
    #[error("the capture holds nothing to keep")]
    Empty,
    #[error("invalid capture JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Import(#[from] ImportError),
    #[error(transparent)]
    Library(#[from] LibraryError),
}

/// Where a capture was sent from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureVia {
    /// The browser extension.
    Extension,
    /// The share sheet on a phone, tablet or desktop.
    ShareSheet,
    /// The clipboard.
    Clipboard,
    /// The desktop shortcut.
    Shortcut,
}

/// What a shell sends to capture something.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureRequest {
    /// The request format version, [`CAPTURE_REQUEST_VERSION`] for this build.
    pub version: u32,
    pub via: CaptureVia,
    /// When the capture was taken, in Unix seconds, as the shell saw it.
    pub captured_at: i64,
    pub content: CaptureContent,
}

/// The content of a capture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureContent {
    /// A web page or the part of it the person selected.
    WebClip {
        url: String,
        #[serde(default)]
        title: String,
        html: String,
        /// The HTML is only what the person selected, so it is kept whole rather than searched
        /// for the main content.
        #[serde(default)]
        selection_only: bool,
    },
    /// Plain text, from the share sheet or the clipboard.
    Text { text: String },
    /// An image, either a file on the device or its bytes as base64. Exactly one is given.
    Image {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data_base64: Option<String>,
        /// A name for an image sent as bytes, such as the screenshot's file name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// A file on the device, imported by its type.
    File { path: PathBuf },
}

impl CaptureRequest {
    /// Reads a request from JSON bytes, refusing one that is too large before parsing it.
    pub fn from_slice(json: &[u8]) -> Result<Self, ClipError> {
        check_size("request", json.len(), MAX_REQUEST_BYTES)?;
        #[derive(Deserialize)]
        struct VersionOnly {
            version: u32,
        }
        let VersionOnly { version } = serde_json::from_slice(json)?;
        if version > CAPTURE_REQUEST_VERSION || version == 0 {
            return Err(ClipError::UnsupportedVersion(version));
        }
        Ok(serde_json::from_slice(json)?)
    }

    pub fn from_json(json: &str) -> Result<Self, ClipError> {
        Self::from_slice(json.as_bytes())
    }

    pub fn to_json(&self) -> Result<String, ClipError> {
        Ok(serde_json::to_string(self)?)
    }
}

/// Where a clip came from, kept with the clip and inside its document so the reader can always
/// show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipSource {
    pub via: CaptureVia,
    /// Unix seconds.
    pub captured_at: i64,
    /// The page address, as the sender gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The page title, as the sender gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_title: Option<String>,
    /// The file name, for an image or a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub selection_only: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Whether a clip has been opened since it was captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxState {
    #[default]
    New,
    Opened,
}

/// What a clip holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClipContent {
    /// The content as a document.
    Document { document: Box<Document> },
    /// A bare address, kept as a link note. The page is never fetched.
    Link { url: String },
}

/// A capture as the library keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipEntry {
    pub title: String,
    #[serde(default)]
    pub state: InboxState,
    pub source: ClipSource,
    /// A hash of what was captured, which finds the same capture made twice.
    pub fingerprint: String,
    pub content: ClipContent,
}

/// What [`capture_into`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureOutcome {
    /// Kept as a new inbox item.
    Stored(ItemId),
    /// The same content was captured within [`DUPLICATE_WINDOW_SECS`], so the earlier item stands.
    Duplicate(ItemId),
}

impl CaptureOutcome {
    pub fn id(&self) -> &ItemId {
        match self {
            CaptureOutcome::Stored(id) | CaptureOutcome::Duplicate(id) => id,
        }
    }
}

fn check_size(what: &'static str, size: usize, limit: usize) -> Result<(), ClipError> {
    if size > limit {
        return Err(ClipError::TooLarge {
            what,
            size: size as u64,
            limit: limit as u64,
        });
    }
    Ok(())
}

/// Turns a request into a clip, without storing it. Images are read with the OCR engine in
/// `tools`, files with the importer for their type.
pub fn convert(request: &CaptureRequest, tools: &Tools<'_>) -> Result<ClipEntry, ClipError> {
    if request.version > CAPTURE_REQUEST_VERSION || request.version == 0 {
        return Err(ClipError::UnsupportedVersion(request.version));
    }
    let mut source = ClipSource {
        via: request.via,
        captured_at: request.captured_at,
        url: None,
        page_title: None,
        file_name: None,
        selection_only: false,
    };
    let (kind, content) = match &request.content {
        CaptureContent::WebClip {
            url,
            title,
            html,
            selection_only,
        } => {
            check_size("page address", url.len(), MAX_URL_BYTES)?;
            check_size("page title", title.len(), MAX_TITLE_BYTES)?;
            check_size("page", html.len(), MAX_HTML_BYTES)?;
            let url = url.trim();
            if !is_web_address(url) {
                return Err(ClipError::Invalid(
                    "a web clip needs an http, https or file address".into(),
                ));
            }
            source.url = Some(url.to_string());
            let title = collapse_spaces(title);
            source.page_title = (!title.is_empty()).then(|| title.clone());
            source.selection_only = *selection_only;
            let options = html::ClipOptions {
                url: Some(url),
                title: (!title.is_empty()).then_some(title.as_str()),
                selection_only: *selection_only,
            };
            match html::html_to_document(html, &options)? {
                Some(document) => (
                    "web",
                    ClipContent::Document {
                        document: Box::new(document),
                    },
                ),
                // a page with nothing readable left is still worth keeping as its address
                None => (
                    "link",
                    ClipContent::Link {
                        url: url.to_string(),
                    },
                ),
            }
        }
        CaptureContent::Text { text } => {
            check_size("text", text.len(), MAX_TEXT_BYTES)?;
            if let Some(url) = text::bare_url(text) {
                source.url = Some(url.clone());
                ("link", ClipContent::Link { url })
            } else {
                let document = text::text_to_document(text).ok_or(ClipError::Empty)?;
                (
                    "text",
                    ClipContent::Document {
                        document: Box::new(document),
                    },
                )
            }
        }
        CaptureContent::Image {
            path,
            data_base64,
            name,
        } => {
            let (document, file_name) =
                image_to_document(path.as_deref(), data_base64.as_deref(), name, tools)?;
            source.file_name = Some(file_name);
            (
                "image",
                ClipContent::Document {
                    document: Box::new(document),
                },
            )
        }
        CaptureContent::File { path } => {
            let (document, file_name) = file_to_document(path, tools)?;
            source.file_name = Some(file_name);
            (
                "file",
                ClipContent::Document {
                    document: Box::new(document),
                },
            )
        }
    };

    let (title, fingerprint_body) = match &content {
        ClipContent::Document { document } => (document.title.clone(), document.plain_text()),
        ClipContent::Link { url } => (
            source.page_title.clone().unwrap_or(url.clone()),
            url.clone(),
        ),
    };
    let fingerprint = content_hash(&format!(
        "{kind}\n{}\n{}\n{fingerprint_body}",
        source.url.as_deref().unwrap_or(""),
        source.file_name.as_deref().unwrap_or("")
    ));
    let content = match content {
        ClipContent::Document { mut document } => {
            document.clip = Some(source.clone());
            ClipContent::Document { document }
        }
        link => link,
    };
    Ok(ClipEntry {
        title,
        state: InboxState::New,
        source,
        fingerprint,
        content,
    })
}

/// Converts a request and keeps it in the library's inbox, unless the same content was captured
/// within [`DUPLICATE_WINDOW_SECS`].
#[cfg(feature = "library")]
pub fn capture_into(
    library: &mut Library,
    request: &CaptureRequest,
    tools: &Tools<'_>,
) -> Result<CaptureOutcome, ClipError> {
    let entry = convert(request, tools)?;
    if let Some(existing) = library.find_duplicate_clip(
        &entry.fingerprint,
        entry.source.captured_at,
        DUPLICATE_WINDOW_SECS,
    )? {
        return Ok(CaptureOutcome::Duplicate(existing));
    }
    Ok(CaptureOutcome::Stored(library.create_clip(&entry)?))
}

/// Whether an address can be a web clip's source.
fn is_web_address(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["http://", "https://", "file://"]
        .iter()
        .any(|scheme| lower.starts_with(scheme) && lower.len() > scheme.len())
        && !url.chars().any(char::is_whitespace)
}

/// Every stretch of whitespace as one space, with none at either end.
pub(crate) fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// At most `max` characters, cut at a word where one is near.
pub(crate) fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let cut = match cut.rfind(' ') {
        Some(space) if space > max / 2 => cut[..space].to_string(),
        _ => cut,
    };
    format!("{}...", cut.trim_end())
}

/// File name without the folders, for titles and provenance.
fn file_name_of(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string()
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

fn file_size(path: &Path) -> Result<u64, ClipError> {
    let metadata = std::fs::metadata(path).map_err(ImportError::Io)?;
    if !metadata.is_file() {
        return Err(ClipError::Invalid(format!(
            "{} is not a file",
            path.display()
        )));
    }
    Ok(metadata.len())
}

fn image_to_document(
    path: Option<&Path>,
    data_base64: Option<&str>,
    name: &Option<String>,
    tools: &Tools<'_>,
) -> Result<(Document, String), ClipError> {
    match (path, data_base64) {
        (Some(path), None) => {
            let extension = extension_of(path);
            if !import::image::IMAGE_EXTENSIONS.contains(&extension.as_str()) {
                return Err(ImportError::Unsupported(if extension.is_empty() {
                    "extensionless".into()
                } else {
                    extension
                })
                .into());
            }
            let size = file_size(path)?;
            if size > MAX_IMAGE_BYTES as u64 {
                return Err(ClipError::TooLarge {
                    what: "image",
                    size,
                    limit: MAX_IMAGE_BYTES as u64,
                });
            }
            let engine = tools.ocr.ok_or(ImportError::NoOcrEngine)?;
            let document = import::image::import_image(path, engine)?.document;
            Ok((document, file_name_of(path)))
        }
        (None, Some(data)) => {
            // base64 is four characters for every three bytes, so the text says the size first
            check_size("image", data.len() / 4 * 3, MAX_IMAGE_BYTES)?;
            let bytes = base64::decode(data)
                .ok_or_else(|| ClipError::Invalid("the image is not valid base64".into()))?;
            check_size("image", bytes.len(), MAX_IMAGE_BYTES)?;
            let extension = image_type(&bytes)
                .ok_or_else(|| ImportError::Unsupported("unrecognised image".into()))?;
            let engine = tools.ocr.ok_or(ImportError::NoOcrEngine)?;
            let file = tempfile::Builder::new()
                .suffix(&format!(".{extension}"))
                .tempfile()
                .map_err(ImportError::Io)?;
            std::fs::write(file.path(), &bytes).map_err(ImportError::Io)?;
            let mut document = import::image::import_image(file.path(), engine)?.document;
            // the temporary file's name means nothing to the reader
            let name = name
                .as_deref()
                .map(collapse_spaces)
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| format!("Captured image.{extension}"));
            document.title = Path::new(&name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Captured image")
                .to_string();
            document.source.file_name = name.clone();
            Ok((document, name))
        }
        _ => Err(ClipError::Invalid(
            "an image needs either a path or base64 data, not both".into(),
        )),
    }
}

/// The file extension that matches an image's first bytes.
fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some("tif")
    } else {
        None
    }
}

fn file_to_document(path: &Path, tools: &Tools<'_>) -> Result<(Document, String), ClipError> {
    let size = file_size(path)?;
    if size > MAX_FILE_BYTES {
        return Err(ClipError::TooLarge {
            what: "file",
            size,
            limit: MAX_FILE_BYTES,
        });
    }
    let file_name = file_name_of(path);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("File")
        .to_string();
    let read_text = |limit: usize| -> Result<String, ClipError> {
        check_size("file", size as usize, limit)?;
        let bytes = std::fs::read(path).map_err(ImportError::Io)?;
        // a file in another encoding still reads, with unreadable bytes marked rather than lost
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    };
    let mut document = match extension_of(path).as_str() {
        "html" | "htm" | "xhtml" => {
            let html = read_text(MAX_HTML_BYTES)?;
            let options = html::ClipOptions {
                url: None,
                title: None,
                selection_only: false,
            };
            let mut document = html::html_to_document(&html, &options)?.ok_or(ClipError::Empty)?;
            if document.title.is_empty() {
                document.title = stem.clone();
            }
            document
        }
        "txt" | "text" | "md" | "markdown" => {
            let text = read_text(MAX_TEXT_BYTES)?;
            let mut document = text::text_to_document(&text).ok_or(ClipError::Empty)?;
            document.title = stem.clone();
            document
        }
        _ => import::import_file_with(path, tools)?.document,
    };
    if document.source.format == SourceFormat::Html || document.source.format == SourceFormat::Other
    {
        document.source = Source {
            file_name: file_name.clone(),
            format: document.source.format,
        };
    }
    Ok((document, file_name))
}
