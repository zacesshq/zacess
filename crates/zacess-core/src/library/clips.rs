//! The capture inbox: clips kept as library items, each new until it is opened.

use super::{ItemId, ItemInfo, ItemKind, Library, LibraryError, Saved};
use crate::clip::{ClipContent, ClipEntry, ClipSource, InboxState};
use crate::model::{ModelError, SCHEMA_VERSION};
use rusqlite::{OptionalExtension, params};

type Result<T> = std::result::Result<T, LibraryError>;

/// One clip as the inbox lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct InboxItem {
    pub info: ItemInfo,
    pub state: InboxState,
    pub source: ClipSource,
}

fn read_entry(content: &str) -> Result<ClipEntry> {
    let entry: ClipEntry = serde_json::from_str(content)?;
    if let ClipContent::Document { document } = &entry.content
        && document.schema_version > SCHEMA_VERSION
    {
        return Err(ModelError::UnsupportedVersion {
            found: document.schema_version,
        }
        .into());
    }
    Ok(entry)
}

impl Library {
    pub fn create_clip(&mut self, entry: &ClipEntry) -> Result<ItemId> {
        self.create_item(ItemKind::Clip, &entry.title, &serde_json::to_string(entry)?)
    }

    pub fn save_clip(&mut self, id: &ItemId, entry: &ClipEntry) -> Result<Saved> {
        self.save_item(
            id,
            ItemKind::Clip,
            &entry.title,
            &serde_json::to_string(entry)?,
        )
    }

    /// Reads a clip without changing whether it is new.
    pub fn clip(&self, id: &ItemId) -> Result<ClipEntry> {
        read_entry(&self.current_content(id, ItemKind::Clip)?)
    }

    /// Reads a clip for the person to look at, marking it opened. Opening it again changes nothing.
    pub fn open_clip(&mut self, id: &ItemId) -> Result<ClipEntry> {
        let mut entry = self.clip(id)?;
        if entry.state == InboxState::New {
            entry.state = InboxState::Opened;
            self.save_clip(id, &entry)?;
        }
        Ok(entry)
    }

    /// Every clip not in the bin, most recently changed first.
    pub fn inbox(&self) -> Result<Vec<InboxItem>> {
        self.items(Some(ItemKind::Clip))?
            .into_iter()
            .map(|info| {
                let entry = read_entry(&self.version_content(&info.id, info.current_version)?)?;
                Ok(InboxItem {
                    info,
                    state: entry.state,
                    source: entry.source,
                })
            })
            .collect()
    }

    /// How many clips have not been opened yet.
    pub fn new_clip_count(&self) -> Result<usize> {
        Ok(self
            .inbox()?
            .iter()
            .filter(|i| i.state == InboxState::New)
            .count())
    }

    /// A clip not in the bin with the same fingerprint, captured within `window` seconds of
    /// `captured_at`. SQLite reads the two fields from each clip's JSON, so no clip is loaded.
    pub fn find_duplicate_clip(
        &self,
        fingerprint: &str,
        captured_at: i64,
        window: i64,
    ) -> Result<Option<ItemId>> {
        let found: Option<String> = self
            .conn
            .query_row(
                "SELECT items.id FROM items
                 JOIN versions ON versions.item_id = items.id
                     AND versions.number = items.current_version
                 WHERE items.kind = 'clip' AND items.deleted_at IS NULL
                     AND json_valid(versions.content)
                     AND json_extract(versions.content, '$.fingerprint') = ?1
                     AND abs(json_extract(versions.content, '$.source.captured_at') - ?2) <= ?3
                 ORDER BY items.created_at DESC, items.id DESC LIMIT 1",
                params![fingerprint, captured_at, window.saturating_abs()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.map(ItemId))
    }
}
