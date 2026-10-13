//! The Zacess engine: document model, reading profiles, import and export.

pub mod ask;
pub mod assist;
pub mod bilingual;
pub mod capture;
pub mod clip;
pub mod confidence;
pub mod export;
pub mod import;
pub mod library;
pub mod maths;
pub mod model;
pub mod profile;
pub mod recording;
pub mod speech;
pub mod study;
pub mod sync;
pub mod timeline;

#[cfg(feature = "library")]
pub use clip::capture_into;
pub use clip::{CaptureOutcome, CaptureRequest, ClipEntry, ClipError};
pub use confidence::{NUMERIC_THRESHOLD, WORD_THRESHOLD, flags_for, needs_checking};
pub use import::{ImportError, Imported, import_file};
#[cfg(feature = "library")]
pub use library::Library;
pub use library::{ItemId, ItemKind, LibraryError, Note};
pub use model::{
    Annotation, AnnotationKind, Block, BlockId, BlockKind, Confidence, Document, ListItem,
    ModelError, Origin, Page, Provenance, RanOn, Rect, Run, RunStatus, SCHEMA_VERSION, Script,
    Source, SourceFormat, TableCell, TableRow, TextRole, Unit,
};
pub use profile::{
    ContrastTheme, CursorSize, Font, FontFallback, PROFILE_SCHEMA_VERSION, Preset, ProfileError,
    ProfileStore, ProfiledDocument, ReadingProfile, ReadingRuler, SettingChange,
};
