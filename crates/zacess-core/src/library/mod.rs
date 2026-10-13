//! The local library: documents, notes, reading profiles, recordings, study decks and plans and
//! captured clips kept in one SQLite file, with a version for every change and a bin that holds deleted items for
//! thirty days.
//!
//! Students must never lose work. Every save that changes an item adds a version rather than
//! overwriting the last one, so any earlier state can be read back or restored. Deleting an item
//! moves it to the bin. It is gone only after thirty days or once the person empties the bin.
//! The whole library exports to open formats, see [`export`] and `docs/library.md`.

#[cfg(feature = "library")]
pub mod clips;
#[cfg(feature = "library")]
pub mod export;

use crate::model::BlockId;
#[cfg(feature = "library")]
use crate::model::Document;
#[cfg(feature = "library")]
use crate::profile::ReadingProfile;
#[cfg(feature = "library")]
use crate::recording::RecordingEntry;
#[cfg(feature = "library")]
use crate::study::{Deck, Plan};
#[cfg(feature = "library")]
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::fmt;
#[cfg(feature = "library")]
use std::path::Path;
#[cfg(feature = "library")]
use std::time::{SystemTime, UNIX_EPOCH};

/// The version of the database layout written by this build.
pub const LIBRARY_SCHEMA_VERSION: u32 = 4;
/// How long a deleted item stays in the bin before it is removed for good.
pub const BIN_RETENTION_SECS: i64 = 30 * 24 * 60 * 60;

#[cfg(feature = "library")]
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS items (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('document', 'note', 'profile', 'recording', 'deck', 'study_plan', 'clip')),
    title TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    current_version INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS versions (
    item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    number INTEGER NOT NULL,
    saved_at INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    content TEXT NOT NULL,
    PRIMARY KEY (item_id, number)
);
CREATE INDEX IF NOT EXISTS items_by_kind ON items(kind, deleted_at, updated_at);
";

/// Makes the items table again with every current kind allowed, keeping every row. Version 1 lacked
/// the recording kind, version 2 the deck and study plan kinds and version 3 the clip kind.
#[cfg(feature = "library")]
const MIGRATE_TO_4: &str = "
BEGIN;
CREATE TABLE items_new (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('document', 'note', 'profile', 'recording', 'deck', 'study_plan', 'clip')),
    title TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    current_version INTEGER NOT NULL
);
INSERT INTO items_new SELECT id, kind, title, created_at, updated_at, deleted_at, current_version FROM items;
DROP TABLE items;
ALTER TABLE items_new RENAME TO items;
CREATE INDEX IF NOT EXISTS items_by_kind ON items(kind, deleted_at, updated_at);
UPDATE meta SET value = '4' WHERE key = 'schema_version';
COMMIT;
";

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[cfg(feature = "library")]
    #[error("library database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("library content: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Model(#[from] crate::model::ModelError),
    #[error(transparent)]
    Profile(#[from] crate::profile::ProfileError),
    #[error("library files: {0}")]
    Io(#[from] std::io::Error),
    #[error("no item {0} in the library")]
    NotFound(ItemId),
    #[error("item {item} has no version {number}")]
    NoSuchVersion { item: ItemId, number: u32 },
    #[error("item {item} is a {found}, not a {expected}")]
    WrongKind {
        item: ItemId,
        expected: ItemKind,
        found: ItemKind,
    },
    #[error("the library uses schema version {0}, this build reads up to {LIBRARY_SCHEMA_VERSION}")]
    UnsupportedSchema(u32),
    #[error("library export: {0}")]
    Export(String),
}

#[cfg(feature = "library")]
type Result<T> = std::result::Result<T, LibraryError>;

/// The identity of an item, stable for its whole life including exports and imports.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemId(pub String);

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Document,
    Note,
    Profile,
    Recording,
    Deck,
    StudyPlan,
    /// Something captured from a web page, the share sheet, the clipboard or the desktop shortcut,
    /// waiting in the inbox.
    Clip,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemKind::Document => "document",
            ItemKind::Note => "note",
            ItemKind::Profile => "profile",
            ItemKind::Recording => "recording",
            ItemKind::Deck => "deck",
            ItemKind::StudyPlan => "study_plan",
            ItemKind::Clip => "clip",
        }
    }

    /// The start of every id of this kind, so an id says what it is.
    #[cfg_attr(not(feature = "library"), allow(dead_code))]
    fn id_prefix(self) -> &'static str {
        match self {
            ItemKind::Document => "d",
            ItemKind::Note => "n",
            ItemKind::Profile => "p",
            ItemKind::Recording => "r",
            ItemKind::Deck => "k",
            ItemKind::StudyPlan => "s",
            ItemKind::Clip => "c",
        }
    }

    #[cfg_attr(not(feature = "library"), allow(dead_code))]
    fn parse(text: &str) -> Option<Self> {
        match text {
            "document" => Some(ItemKind::Document),
            "note" => Some(ItemKind::Note),
            "profile" => Some(ItemKind::Profile),
            "recording" => Some(ItemKind::Recording),
            "deck" => Some(ItemKind::Deck),
            "study_plan" => Some(ItemKind::StudyPlan),
            "clip" => Some(ItemKind::Clip),
            _ => None,
        }
    }
}

impl fmt::Display for ItemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A note in Markdown, on its own or attached to a place in a document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub title: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<ItemId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<BlockId>,
}

/// What the library knows about an item without loading its content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemInfo {
    pub id: ItemId,
    pub kind: ItemKind,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<i64>,
    pub current_version: u32,
}

/// One saved state of an item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionInfo {
    pub number: u32,
    pub saved_at: i64,
    pub size: u64,
    pub content_hash: String,
}

/// What a save did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Saved {
    pub version: u32,
    /// `false` when the content was identical to the current version, so nothing was written.
    pub changed: bool,
}

/// How much the library holds, for the storage view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorageUsage {
    pub items: u64,
    pub versions: u64,
    pub content_bytes: u64,
    pub bin_items: u64,
}

#[cfg(feature = "library")]
pub struct Library {
    conn: Connection,
    clock: Box<dyn Fn() -> i64 + Send>,
    counter: u32,
}

#[cfg(feature = "library")]
impl fmt::Debug for Library {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Library")
            .field("path", &self.conn.path())
            .finish()
    }
}

#[cfg(feature = "library")]
fn system_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A short hash of content, used to skip saves that change nothing.
pub(crate) fn content_hash(content: &str) -> String {
    let hash = content.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("{hash:016x}")
}

#[cfg(feature = "library")]
impl Library {
    /// Opens the library file, creating it when it does not exist.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        // a write-ahead log keeps reads working during a save and survives a crash mid-write
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::with_connection(conn)
    }

    /// A library that lives only in memory, for tests and previews.
    pub fn open_in_memory() -> Result<Self> {
        Self::with_connection(Connection::open_in_memory()?)
    }

    fn with_connection(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        let stored: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        match stored.and_then(|v| v.parse::<u32>().ok()) {
            Some(v) if v > LIBRARY_SCHEMA_VERSION => {
                return Err(LibraryError::UnsupportedSchema(v));
            }
            Some(1..=3) => {
                // earlier tables forbid the newer kinds, which SQLite can only change by making
                // the table again
                conn.pragma_update(None, "foreign_keys", "OFF")?;
                conn.execute_batch(MIGRATE_TO_4)?;
                conn.pragma_update(None, "foreign_keys", "ON")?;
            }
            Some(_) => {}
            None => {
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
                    params![LIBRARY_SCHEMA_VERSION.to_string()],
                )?;
            }
        }
        let mut library = Library {
            conn,
            clock: Box::new(system_now),
            counter: 0,
        };
        library.purge_bin()?;
        Ok(library)
    }

    /// Replaces the clock, so tests can move time forward.
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    fn new_id(&mut self, kind: ItemKind) -> ItemId {
        self.counter = self.counter.wrapping_add(1);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        ItemId(format!(
            "{}-{:x}-{:04x}",
            kind.id_prefix(),
            nanos,
            self.counter
        ))
    }

    // --- documents ---------------------------------------------------------------------------

    pub fn create_document(&mut self, document: &Document) -> Result<ItemId> {
        self.create_item(ItemKind::Document, &document.title, &document.to_json()?)
    }

    /// Saves a document. Called on every change: a new version is written only when the content
    /// differs from the current one.
    pub fn save_document(&mut self, id: &ItemId, document: &Document) -> Result<Saved> {
        self.save_item(
            id,
            ItemKind::Document,
            &document.title,
            &document.to_json()?,
        )
    }

    pub fn document(&self, id: &ItemId) -> Result<Document> {
        Ok(Document::from_json(
            &self.current_content(id, ItemKind::Document)?,
        )?)
    }

    pub fn document_version(&self, id: &ItemId, number: u32) -> Result<Document> {
        self.expect_kind(id, ItemKind::Document)?;
        Ok(Document::from_json(&self.version_content(id, number)?)?)
    }

    // --- notes -------------------------------------------------------------------------------

    pub fn create_note(&mut self, note: &Note) -> Result<ItemId> {
        self.create_item(ItemKind::Note, &note.title, &serde_json::to_string(note)?)
    }

    pub fn save_note(&mut self, id: &ItemId, note: &Note) -> Result<Saved> {
        self.save_item(
            id,
            ItemKind::Note,
            &note.title,
            &serde_json::to_string(note)?,
        )
    }

    pub fn note(&self, id: &ItemId) -> Result<Note> {
        Ok(serde_json::from_str(
            &self.current_content(id, ItemKind::Note)?,
        )?)
    }

    pub fn note_version(&self, id: &ItemId, number: u32) -> Result<Note> {
        self.expect_kind(id, ItemKind::Note)?;
        Ok(serde_json::from_str(&self.version_content(id, number)?)?)
    }

    // --- recordings ---------------------------------------------------------------------------

    /// Holds a finished recording: its title, times and files. The audio stays in the recording
    /// store, as an item is text.
    pub fn create_recording(&mut self, entry: &RecordingEntry) -> Result<ItemId> {
        self.create_item(
            ItemKind::Recording,
            &entry.title,
            &serde_json::to_string(entry)?,
        )
    }

    pub fn save_recording(&mut self, id: &ItemId, entry: &RecordingEntry) -> Result<Saved> {
        self.save_item(
            id,
            ItemKind::Recording,
            &entry.title,
            &serde_json::to_string(entry)?,
        )
    }

    pub fn recording(&self, id: &ItemId) -> Result<RecordingEntry> {
        Ok(serde_json::from_str(
            &self.current_content(id, ItemKind::Recording)?,
        )?)
    }

    // --- profiles ----------------------------------------------------------------------------

    pub fn create_profile(&mut self, profile: &ReadingProfile) -> Result<ItemId> {
        profile.validate()?;
        self.create_item(ItemKind::Profile, &profile.name, &profile.to_json()?)
    }

    pub fn save_profile(&mut self, id: &ItemId, profile: &ReadingProfile) -> Result<Saved> {
        profile.validate()?;
        self.save_item(id, ItemKind::Profile, &profile.name, &profile.to_json()?)
    }

    pub fn profile(&self, id: &ItemId) -> Result<ReadingProfile> {
        Ok(ReadingProfile::from_json(
            &self.current_content(id, ItemKind::Profile)?,
        )?)
    }

    // --- study --------------------------------------------------------------------------------

    /// Holds a deck of cards with their schedule and review history.
    pub fn create_deck(&mut self, deck: &Deck) -> Result<ItemId> {
        self.create_item(ItemKind::Deck, &deck.name, &serde_json::to_string(deck)?)
    }

    pub fn save_deck(&mut self, id: &ItemId, deck: &Deck) -> Result<Saved> {
        self.save_item(
            id,
            ItemKind::Deck,
            &deck.name,
            &serde_json::to_string(deck)?,
        )
    }

    pub fn deck(&self, id: &ItemId) -> Result<Deck> {
        Ok(serde_json::from_str(
            &self.current_content(id, ItemKind::Deck)?,
        )?)
    }

    /// Holds a study plan: its goals, sessions, revision paths and local day.
    pub fn create_study_plan(&mut self, plan: &Plan) -> Result<ItemId> {
        self.create_item(
            ItemKind::StudyPlan,
            &plan_title(plan),
            &serde_json::to_string(plan)?,
        )
    }

    pub fn save_study_plan(&mut self, id: &ItemId, plan: &Plan) -> Result<Saved> {
        self.save_item(
            id,
            ItemKind::StudyPlan,
            &plan_title(plan),
            &serde_json::to_string(plan)?,
        )
    }

    pub fn study_plan(&self, id: &ItemId) -> Result<Plan> {
        Ok(serde_json::from_str(
            &self.current_content(id, ItemKind::StudyPlan)?,
        )?)
    }

    // --- every kind --------------------------------------------------------------------------

    /// Items that are not in the bin, most recently changed first.
    pub fn items(&self, kind: Option<ItemKind>) -> Result<Vec<ItemInfo>> {
        let mut statement = self.conn.prepare(
            "SELECT id, kind, title, created_at, updated_at, deleted_at, current_version FROM items
             WHERE deleted_at IS NULL AND (?1 IS NULL OR kind = ?1)
             ORDER BY updated_at DESC, id",
        )?;
        let rows = statement.query_map(params![kind.map(ItemKind::as_str)], row_to_info)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn item(&self, id: &ItemId) -> Result<ItemInfo> {
        self.conn
            .query_row(
                "SELECT id, kind, title, created_at, updated_at, deleted_at, current_version FROM items WHERE id = ?1",
                params![id.0],
                row_to_info,
            )
            .optional()?
            .ok_or_else(|| LibraryError::NotFound(id.clone()))
    }

    /// Every version of an item, oldest first.
    pub fn versions(&self, id: &ItemId) -> Result<Vec<VersionInfo>> {
        self.item(id)?;
        let mut statement = self.conn.prepare(
            "SELECT number, saved_at, length(content), content_hash FROM versions WHERE item_id = ?1 ORDER BY number",
        )?;
        let rows = statement.query_map(params![id.0], |r| {
            Ok(VersionInfo {
                number: r.get(0)?,
                saved_at: r.get(1)?,
                size: r.get::<_, i64>(2)? as u64,
                content_hash: r.get(3)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// The stored content of one version, as the JSON it was saved as.
    pub fn version_content(&self, id: &ItemId, number: u32) -> Result<String> {
        self.conn
            .query_row(
                "SELECT content FROM versions WHERE item_id = ?1 AND number = ?2",
                params![id.0, number],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| LibraryError::NoSuchVersion {
                item: id.clone(),
                number,
            })
    }

    /// Makes an earlier version current again by saving it as a new version, so the history is
    /// never rewritten.
    pub fn restore_version(&mut self, id: &ItemId, number: u32) -> Result<Saved> {
        let info = self.item(id)?;
        let content = self.version_content(id, number)?;
        let title = match info.kind {
            ItemKind::Document => Document::from_json(&content)?.title,
            ItemKind::Note => serde_json::from_str::<Note>(&content)?.title,
            ItemKind::Profile => ReadingProfile::from_json(&content)?.name,
            ItemKind::Recording => serde_json::from_str::<RecordingEntry>(&content)?.title,
            ItemKind::Deck => serde_json::from_str::<Deck>(&content)?.name,
            ItemKind::StudyPlan => plan_title(&serde_json::from_str::<Plan>(&content)?),
            ItemKind::Clip => serde_json::from_str::<crate::clip::ClipEntry>(&content)?.title,
        };
        self.save_item(id, info.kind, &title, &content)
    }

    /// Moves an item to the bin.
    pub fn delete(&mut self, id: &ItemId) -> Result<()> {
        let now = self.now();
        let changed = self.conn.execute(
            "UPDATE items SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![id.0, now],
        )?;
        if changed == 0 {
            self.item(id)?;
        }
        Ok(())
    }

    /// Brings an item back from the bin.
    pub fn restore(&mut self, id: &ItemId) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE items SET deleted_at = NULL WHERE id = ?1",
            params![id.0],
        )?;
        if changed == 0 {
            return Err(LibraryError::NotFound(id.clone()));
        }
        Ok(())
    }

    /// Items in the bin, most recently deleted first, with when each will be removed for good.
    pub fn bin(&self) -> Result<Vec<(ItemInfo, i64)>> {
        let mut statement = self.conn.prepare(
            "SELECT id, kind, title, created_at, updated_at, deleted_at, current_version FROM items
             WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC, id",
        )?;
        let rows = statement.query_map([], row_to_info)?;
        rows.map(|r| {
            let info = r?;
            let expires = info.deleted_at.unwrap_or(0) + BIN_RETENTION_SECS;
            Ok((info, expires))
        })
        .collect()
    }

    /// Removes items whose thirty days in the bin are over. Runs on open and after each delete.
    pub fn purge_bin(&mut self) -> Result<usize> {
        let cutoff = self.now() - BIN_RETENTION_SECS;
        Ok(self.conn.execute(
            "DELETE FROM items WHERE deleted_at IS NOT NULL AND deleted_at <= ?1",
            params![cutoff],
        )?)
    }

    /// Removes an item from the bin now, at the person's request.
    pub fn erase(&mut self, id: &ItemId) -> Result<()> {
        let changed = self.conn.execute(
            "DELETE FROM items WHERE id = ?1 AND deleted_at IS NOT NULL",
            params![id.0],
        )?;
        if changed == 0 {
            return Err(LibraryError::NotFound(id.clone()));
        }
        Ok(())
    }

    pub fn usage(&self) -> Result<StorageUsage> {
        let (items, bin_items): (i64, i64) = self.conn.query_row(
            "SELECT count(*) FILTER (WHERE deleted_at IS NULL), count(*) FILTER (WHERE deleted_at IS NOT NULL) FROM items",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let (versions, bytes): (i64, Option<i64>) = self.conn.query_row(
            "SELECT count(*), sum(length(content)) FROM versions",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(StorageUsage {
            items: items as u64,
            versions: versions as u64,
            content_bytes: bytes.unwrap_or(0) as u64,
            bin_items: bin_items as u64,
        })
    }

    /// Writes a complete copy of the library file, for backups.
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        self.conn
            .execute("VACUUM INTO ?1", params![path.to_string_lossy()])?;
        Ok(())
    }

    // --- internals ---------------------------------------------------------------------------

    fn create_item(&mut self, kind: ItemKind, title: &str, content: &str) -> Result<ItemId> {
        let id = self.new_id(kind);
        let now = self.now();
        let info = ItemInfo {
            id: id.clone(),
            kind,
            title: title.to_string(),
            created_at: now,
            updated_at: now,
            deleted_at: None,
            current_version: 1,
        };
        self.insert_item(&info, &[(1, now, content.to_string())])?;
        Ok(id)
    }

    /// Inserts an item with its versions as given, which imports use to keep ids and times.
    pub(crate) fn insert_item(
        &mut self,
        info: &ItemInfo,
        versions: &[(u32, i64, String)],
    ) -> Result<()> {
        let (id, kind, title) = (&info.id, info.kind, &info.title);
        let (created_at, updated_at, deleted_at) =
            (info.created_at, info.updated_at, info.deleted_at);
        let current = versions.iter().map(|v| v.0).max().unwrap_or(1);
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO items (id, kind, title, created_at, updated_at, deleted_at, current_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id.0, kind.as_str(), title, created_at, updated_at, deleted_at, current],
        )?;
        for (number, saved_at, content) in versions {
            tx.execute(
                "INSERT INTO versions (item_id, number, saved_at, content_hash, content) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id.0, number, saved_at, content_hash(content), content],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn save_item(
        &mut self,
        id: &ItemId,
        kind: ItemKind,
        title: &str,
        content: &str,
    ) -> Result<Saved> {
        let info = self.expect_kind(id, kind)?;
        let hash = content_hash(content);
        let current_hash: String = self.conn.query_row(
            "SELECT content_hash FROM versions WHERE item_id = ?1 AND number = ?2",
            params![id.0, info.current_version],
            |r| r.get(0),
        )?;
        if current_hash == hash {
            return Ok(Saved {
                version: info.current_version,
                changed: false,
            });
        }
        let number = info.current_version + 1;
        let now = self.now();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO versions (item_id, number, saved_at, content_hash, content) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.0, number, now, hash, content],
        )?;
        tx.execute(
            "UPDATE items SET title = ?2, updated_at = ?3, current_version = ?4 WHERE id = ?1",
            params![id.0, title, now, number],
        )?;
        tx.commit()?;
        Ok(Saved {
            version: number,
            changed: true,
        })
    }

    fn expect_kind(&self, id: &ItemId, expected: ItemKind) -> Result<ItemInfo> {
        let info = self.item(id)?;
        if info.kind != expected {
            return Err(LibraryError::WrongKind {
                item: id.clone(),
                expected,
                found: info.kind,
            });
        }
        Ok(info)
    }

    fn current_content(&self, id: &ItemId, kind: ItemKind) -> Result<String> {
        let info = self.expect_kind(id, kind)?;
        self.version_content(id, info.current_version)
    }

    /// Every item including the bin, for exports.
    pub(crate) fn all_items(&self) -> Result<Vec<ItemInfo>> {
        let mut statement = self.conn.prepare(
            "SELECT id, kind, title, created_at, updated_at, deleted_at, current_version FROM items ORDER BY created_at, id",
        )?;
        let rows = statement.query_map([], row_to_info)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

#[cfg(feature = "library")]
/// A plan's title in the library, as a plan need not have a name.
fn plan_title(plan: &Plan) -> String {
    if plan.name.trim().is_empty() {
        "Study plan".to_string()
    } else {
        plan.name.clone()
    }
}

#[cfg(feature = "library")]
fn row_to_info(r: &rusqlite::Row<'_>) -> rusqlite::Result<ItemInfo> {
    let kind: String = r.get(1)?;
    Ok(ItemInfo {
        id: ItemId(r.get(0)?),
        kind: ItemKind::parse(&kind).unwrap_or(ItemKind::Document),
        title: r.get(2)?,
        created_at: r.get(3)?,
        updated_at: r.get(4)?,
        deleted_at: r.get(5)?,
        current_version: r.get(6)?,
    })
}

#[cfg(all(test, feature = "library"))]
mod tests {
    use super::*;
    use crate::model::{BlockKind, Origin, Rect, Run, Source, SourceFormat};
    use crate::profile::Preset;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    fn document(title: &str, text: &str) -> Document {
        let mut doc = Document::new(
            title,
            Source {
                file_name: "n.pdf".into(),
                format: SourceFormat::Pdf,
            },
        );
        let origin = Origin::new(1, Rect::new(0.0, 0.0, 10.0, 10.0));
        doc.push(
            BlockKind::paragraph(vec![Run::native(text, origin)]),
            origin,
        );
        doc
    }

    /// A library whose clock the test controls.
    fn library() -> (Library, Arc<AtomicI64>) {
        let clock = Arc::new(AtomicI64::new(1_000_000));
        let c = clock.clone();
        let library = Library::open_in_memory()
            .unwrap()
            .with_clock(move || c.load(Ordering::SeqCst));
        (library, clock)
    }

    #[test]
    fn documents_notes_and_profiles_are_stored_and_read_back() {
        let (mut lib, _) = library();
        let doc = document("Week 3", "Fourier");
        let doc_id = lib.create_document(&doc).unwrap();
        let note = Note {
            title: "Check this".into(),
            text: "The 2.5 looks wrong".into(),
            document: Some(doc_id.clone()),
            block: Some(BlockId(1)),
        };
        let note_id = lib.create_note(&note).unwrap();
        let profile_id = lib
            .create_profile(&ReadingProfile::preset(Preset::LowVision))
            .unwrap();

        assert_eq!(lib.document(&doc_id).unwrap(), doc);
        assert_eq!(lib.note(&note_id).unwrap(), note);
        assert_eq!(lib.profile(&profile_id).unwrap().name, "Low Vision");
        assert_eq!(lib.items(None).unwrap().len(), 3);
        assert_eq!(
            lib.items(Some(ItemKind::Note)).unwrap()[0].title,
            "Check this"
        );
        assert!(matches!(
            lib.note(&doc_id),
            Err(LibraryError::WrongKind { .. })
        ));
        assert!(matches!(
            lib.document(&ItemId("missing".into())),
            Err(LibraryError::NotFound(_))
        ));
    }

    #[test]
    fn every_change_adds_a_version_and_identical_saves_do_not() {
        let (mut lib, clock) = library();
        let mut doc = document("Week 3", "first");
        let id = lib.create_document(&doc).unwrap();
        assert_eq!(
            lib.save_document(&id, &doc).unwrap(),
            Saved {
                version: 1,
                changed: false
            }
        );

        clock.store(1_000_060, Ordering::SeqCst);
        doc.block_mut(crate::model::BlockId(1)).unwrap().runs_mut()[0].correct("second");
        assert_eq!(
            lib.save_document(&id, &doc).unwrap(),
            Saved {
                version: 2,
                changed: true
            }
        );
        clock.store(1_000_120, Ordering::SeqCst);
        doc.title = "Week 3 revised".into();
        assert_eq!(
            lib.save_document(&id, &doc).unwrap(),
            Saved {
                version: 3,
                changed: true
            }
        );

        let versions = lib.versions(&id).unwrap();
        assert_eq!(
            versions
                .iter()
                .map(|v| (v.number, v.saved_at))
                .collect::<Vec<_>>(),
            [(1, 1_000_000), (2, 1_000_060), (3, 1_000_120)]
        );
        assert!(versions[0].size > 0);
        assert_eq!(lib.document_version(&id, 1).unwrap().plain_text(), "first");
        assert_eq!(lib.item(&id).unwrap().title, "Week 3 revised");
        assert_eq!(lib.item(&id).unwrap().updated_at, 1_000_120);

        // restoring writes a new version rather than deleting the later ones
        assert_eq!(
            lib.restore_version(&id, 1).unwrap(),
            Saved {
                version: 4,
                changed: true
            }
        );
        assert_eq!(lib.document(&id).unwrap().plain_text(), "first");
        assert_eq!(lib.item(&id).unwrap().title, "Week 3");
        assert_eq!(lib.versions(&id).unwrap().len(), 4);
        assert!(matches!(
            lib.document_version(&id, 9),
            Err(LibraryError::NoSuchVersion { number: 9, .. })
        ));
    }

    #[test]
    fn deleted_items_wait_in_the_bin_for_thirty_days() {
        let (mut lib, clock) = library();
        let id = lib.create_document(&document("Old", "text")).unwrap();
        let keep = lib.create_document(&document("Keep", "text")).unwrap();
        lib.delete(&id).unwrap();
        assert_eq!(lib.items(None).unwrap().len(), 1);
        let bin = lib.bin().unwrap();
        assert_eq!(bin.len(), 1);
        assert_eq!(bin[0].1, 1_000_000 + BIN_RETENTION_SECS);
        // still readable and restorable while in the bin
        assert_eq!(lib.document(&id).unwrap().title, "Old");
        lib.restore(&id).unwrap();
        assert_eq!(lib.items(None).unwrap().len(), 2);

        lib.delete(&id).unwrap();
        clock.store(1_000_000 + BIN_RETENTION_SECS - 1, Ordering::SeqCst);
        assert_eq!(lib.purge_bin().unwrap(), 0);
        clock.store(1_000_000 + BIN_RETENTION_SECS, Ordering::SeqCst);
        assert_eq!(lib.purge_bin().unwrap(), 1);
        assert!(matches!(lib.document(&id), Err(LibraryError::NotFound(_))));
        assert!(matches!(lib.versions(&id), Err(LibraryError::NotFound(_))));
        assert_eq!(lib.document(&keep).unwrap().title, "Keep");

        // erasing from the bin is immediate, erasing something not in the bin is refused
        lib.delete(&keep).unwrap();
        lib.erase(&keep).unwrap();
        assert!(lib.bin().unwrap().is_empty());
        let fresh = lib.create_document(&document("Fresh", "text")).unwrap();
        assert!(matches!(lib.erase(&fresh), Err(LibraryError::NotFound(_))));
    }

    #[test]
    fn usage_counts_items_versions_and_bytes() {
        let (mut lib, _) = library();
        let id = lib.create_document(&document("A", "one")).unwrap();
        lib.save_document(&id, &document("A", "two")).unwrap();
        let other = lib
            .create_note(&Note {
                title: "n".into(),
                text: "t".into(),
                document: None,
                block: None,
            })
            .unwrap();
        lib.delete(&other).unwrap();
        let usage = lib.usage().unwrap();
        assert_eq!((usage.items, usage.versions, usage.bin_items), (1, 3, 1));
        assert!(usage.content_bytes > 100);
    }

    #[test]
    fn the_library_persists_on_disk_and_backs_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data").join("library.sqlite");
        let id = {
            let mut lib = Library::open(&path).unwrap();
            lib.create_document(&document("Persisted", "text")).unwrap()
        };
        let lib = Library::open(&path).unwrap();
        assert_eq!(lib.document(&id).unwrap().title, "Persisted");
        let backup = dir.path().join("backup.sqlite");
        lib.backup_to(&backup).unwrap();
        let restored = Library::open(&backup).unwrap();
        assert_eq!(restored.document(&id).unwrap().title, "Persisted");
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', '99')",
            [],
        )
        .unwrap();
        assert!(matches!(
            Library::with_connection(conn),
            Err(LibraryError::UnsupportedSchema(99))
        ));
    }

    fn study_deck() -> Deck {
        let mut deck = Deck::new("Signals");
        deck.add_card("Third harmonic of 50 Hz?", "150 Hz", None, 1_000);
        deck.add_card("A periodic signal is a sum of?", "Sinusoids", None, 1_000);
        deck
    }

    #[test]
    fn a_deck_round_trips_through_save_version_and_restore() {
        let (mut lib, clock) = library();
        let mut deck = study_deck();
        let id = lib.create_deck(&deck).unwrap();
        assert!(id.0.starts_with("k-"));
        assert_eq!(lib.deck(&id).unwrap(), deck);
        assert_eq!(lib.items(Some(ItemKind::Deck)).unwrap()[0].title, "Signals");

        clock.store(1_000_060, Ordering::SeqCst);
        deck.review(1, crate::study::Rating::Good, 1_000_060)
            .unwrap();
        deck.name = "Signals and systems".into();
        assert_eq!(lib.save_deck(&id, &deck).unwrap().version, 2);
        assert_eq!(
            lib.save_deck(&id, &deck).unwrap(),
            Saved {
                version: 2,
                changed: false
            }
        );
        assert_eq!(lib.deck(&id).unwrap(), deck);
        assert_eq!(lib.item(&id).unwrap().title, "Signals and systems");

        assert_eq!(lib.restore_version(&id, 1).unwrap().version, 3);
        assert_eq!(lib.deck(&id).unwrap(), study_deck());
        assert_eq!(lib.item(&id).unwrap().title, "Signals");
        assert!(matches!(
            lib.study_plan(&id),
            Err(LibraryError::WrongKind { .. })
        ));
    }

    #[test]
    fn plan_sessions_and_path_position_survive_reopening() {
        use crate::study::{Goal, Measure, Period, RevisionPath, Session};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite");
        let doc = document("Week 3", "Fourier");
        let mut plan = Plan {
            name: "Exam revision".into(),
            goals: vec![Goal {
                measure: Measure::Cards,
                target: 3,
                period: Period::Day,
            }],
            utc_offset_minutes: 60,
            ..Default::default()
        };
        let (plan_id, deck_id) = {
            let mut lib = Library::open(&path).unwrap();
            let doc_id = lib.create_document(&doc).unwrap();
            let mut deck = study_deck();
            let mut session = Session::start(doc_id.clone(), 1_000);
            deck.review_in(1, crate::study::Rating::Good, 1_000, &mut session)
                .unwrap();
            session.end(1_600);
            plan.sessions.push(session);
            let mut revision = RevisionPath::from_document(doc_id, &doc, &deck);
            revision.complete_step().unwrap();
            plan.paths.push(revision);
            let deck_id = lib.create_deck(&deck).unwrap();
            let plan_id = lib.create_study_plan(&plan).unwrap();
            (plan_id, deck_id)
        };
        let lib = Library::open(&path).unwrap();
        let read = lib.study_plan(&plan_id).unwrap();
        assert_eq!(read, plan);
        assert_eq!(read.sessions[0].cards_reviewed, 1);
        assert_eq!(read.paths[0].position, 1);
        assert_eq!(read.utc_offset_minutes, 60);
        assert_eq!(lib.item(&plan_id).unwrap().title, "Exam revision");
        assert_eq!(lib.deck(&deck_id).unwrap().cards[0].reviews.len(), 1);
    }

    #[test]
    fn a_version_two_library_is_carried_over_and_takes_study_items() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v2.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE items (
                    id TEXT PRIMARY KEY,
                    kind TEXT NOT NULL CHECK (kind IN ('document', 'note', 'profile', 'recording')),
                    title TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                    deleted_at INTEGER, current_version INTEGER NOT NULL);
                 CREATE TABLE versions (
                    item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
                    number INTEGER NOT NULL, saved_at INTEGER NOT NULL,
                    content_hash TEXT NOT NULL, content TEXT NOT NULL,
                    PRIMARY KEY (item_id, number));
                 CREATE INDEX items_by_kind ON items(kind, deleted_at, updated_at);
                 INSERT INTO meta VALUES ('schema_version', '2');
                 INSERT INTO items VALUES ('n-1', 'note', 'Old note', 5, 6, NULL, 2);
                 INSERT INTO versions VALUES ('n-1', 1, 5, 'h1', '{\"title\":\"Old note\",\"text\":\"one\"}');
                 INSERT INTO versions VALUES ('n-1', 2, 6, 'h2', '{\"title\":\"Old note\",\"text\":\"two\"}');",
            )
            .unwrap();
        }
        let mut lib = Library::open(&path).unwrap().with_clock(|| 8);
        let id = ItemId("n-1".into());
        assert_eq!(lib.note(&id).unwrap().text, "two");
        assert_eq!(lib.versions(&id).unwrap().len(), 2, "history kept");
        let deck = lib.create_deck(&study_deck()).unwrap();
        let plan = lib.create_study_plan(&Plan::default()).unwrap();
        assert_eq!(lib.item(&plan).unwrap().title, "Study plan");
        assert_eq!(lib.items(Some(ItemKind::Deck)).unwrap()[0].id, deck);
        let version: String = lib
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, LIBRARY_SCHEMA_VERSION.to_string());
        drop(lib);
        // opening the carried-over file again changes nothing
        let lib = Library::open(&path).unwrap();
        assert_eq!(lib.deck(&deck).unwrap(), study_deck());
    }

    #[test]
    fn a_version_three_library_is_carried_over_and_takes_clips() {
        use crate::clip::{CaptureContent, CaptureRequest, CaptureVia, capture_into};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v3.sqlite");
        let doc = document("Week 3", "Fourier");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE items (
                    id TEXT PRIMARY KEY,
                    kind TEXT NOT NULL CHECK (kind IN ('document', 'note', 'profile', 'recording', 'deck', 'study_plan')),
                    title TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                    deleted_at INTEGER, current_version INTEGER NOT NULL);
                 CREATE TABLE versions (
                    item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
                    number INTEGER NOT NULL, saved_at INTEGER NOT NULL,
                    content_hash TEXT NOT NULL, content TEXT NOT NULL,
                    PRIMARY KEY (item_id, number));
                 CREATE INDEX items_by_kind ON items(kind, deleted_at, updated_at);
                 INSERT INTO meta VALUES ('schema_version', '3');
                 INSERT INTO items VALUES ('k-1', 'deck', 'Signals', 5, 6, NULL, 1);
                 INSERT INTO items VALUES ('n-1', 'note', 'Binned note', 5, 6, 4000000000, 1);",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO versions VALUES ('k-1', 1, 5, 'h1', ?1)",
                params![serde_json::to_string(&study_deck()).unwrap()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO versions VALUES ('n-1', 1, 5, 'h2', '{\"title\":\"Binned note\",\"text\":\"x\"}')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO items VALUES ('d-1', 'document', 'Week 3', 5, 6, NULL, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO versions VALUES ('d-1', 1, 5, 'h3', ?1)",
                params![doc.to_json().unwrap()],
            )
            .unwrap();
        }
        let mut lib = Library::open(&path).unwrap().with_clock(|| 8);
        assert_eq!(lib.deck(&ItemId("k-1".into())).unwrap(), study_deck());
        assert_eq!(lib.document(&ItemId("d-1".into())).unwrap(), doc);
        assert_eq!(
            lib.bin().unwrap().len(),
            1,
            "an item in the bin stays there"
        );
        let request = CaptureRequest {
            version: 1,
            via: CaptureVia::Clipboard,
            captured_at: 8,
            content: CaptureContent::Text {
                text: "Read chapter 4".into(),
            },
        };
        let clip = capture_into(&mut lib, &request, &crate::import::Tools::default())
            .unwrap()
            .id()
            .clone();
        assert_eq!(lib.inbox().unwrap()[0].info.id, clip);
        let version: String = lib
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, "4");
        drop(lib);
        let lib = Library::open(&path).unwrap();
        assert_eq!(lib.clip(&clip).unwrap().title, "Read chapter 4");
        assert_eq!(lib.items(None).unwrap().len(), 3);
    }
}
