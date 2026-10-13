//! Exporting the whole library to open formats and reading such an export back.
//!
//! The layout is documented in `docs/library.md`: a `library.json` manifest, each document as
//! model JSON plus a standalone HTML page, each note as Markdown, each profile as JSON, each deck as
//! JSON plus Anki text and Notion CSV, each study plan as JSON, each clip as JSON plus an HTML page
//! when it holds a document and every earlier version under `history/`.

use super::{ItemId, ItemInfo, ItemKind, Library, LibraryError, Note};
use crate::clip::{ClipContent, ClipEntry};
use crate::export::{ExportOptions, to_html};
use crate::model::Document;
use crate::recording::RecordingEntry;
use crate::study::{AnkiOptions, Deck, Plan};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The format of the manifest written by this build.
pub const EXPORT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub exported_at: i64,
    pub items: Vec<ManifestItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestItem {
    #[serde(flatten)]
    pub info: ItemInfo,
    /// The current content, relative to the export directory.
    pub path: String,
    /// Every version including the current one, relative to the export directory.
    pub history: Vec<ManifestVersion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestVersion {
    pub number: u32,
    pub saved_at: i64,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSummary {
    pub directory: PathBuf,
    pub items: usize,
    pub versions: usize,
}

type Result<T> = std::result::Result<T, LibraryError>;

impl Library {
    /// Writes every item, including the bin, with its full history.
    pub fn export_all(&self, dir: &Path) -> Result<ExportSummary> {
        std::fs::create_dir_all(dir)?;
        let mut manifest = Manifest {
            format: "zacess-library".into(),
            format_version: EXPORT_FORMAT_VERSION,
            exported_at: (self.clock)(),
            items: Vec::new(),
        };
        let mut version_count = 0;
        for info in self.all_items()? {
            let (folder, extension) = match info.kind {
                ItemKind::Document => ("documents", "json"),
                ItemKind::Note => ("notes", "md"),
                ItemKind::Profile => ("profiles", "json"),
                ItemKind::Recording => ("recordings", "json"),
                ItemKind::Deck => ("decks", "json"),
                ItemKind::StudyPlan => ("study_plans", "json"),
                ItemKind::Clip => ("clips", "json"),
            };
            std::fs::create_dir_all(dir.join(folder))?;
            let current = self.version_content(&info.id, info.current_version)?;
            let path = format!("{folder}/{}.{extension}", info.id);
            std::fs::write(dir.join(&path), readable(info.kind, &current)?)?;
            if info.kind == ItemKind::Document {
                let document = Document::from_json(&current)?;
                std::fs::write(
                    dir.join(format!("{folder}/{}.html", info.id)),
                    to_html(&document, &ExportOptions::default()),
                )?;
            }
            if info.kind == ItemKind::Clip
                && let ClipContent::Document { document } =
                    serde_json::from_str::<ClipEntry>(&current)?.content
            {
                std::fs::write(
                    dir.join(format!("{folder}/{}.html", info.id)),
                    to_html(&document, &ExportOptions::default()),
                )?;
            }
            if info.kind == ItemKind::Deck {
                let deck: Deck = serde_json::from_str(&current)?;
                let anki = AnkiOptions {
                    deck: Some(deck.name.clone()),
                    ..AnkiOptions::default()
                };
                std::fs::write(
                    dir.join(format!("{folder}/{}.anki.txt", info.id)),
                    deck.to_anki_text_with(&anki),
                )?;
                std::fs::write(
                    dir.join(format!("{folder}/{}.notion.csv", info.id)),
                    deck.to_notion_csv(manifest.exported_at),
                )?;
            }

            let history_dir = dir.join("history").join(info.id.to_string());
            std::fs::create_dir_all(&history_dir)?;
            let mut history = Vec::new();
            for version in self.versions(&info.id)? {
                let relative = format!("history/{}/{}.json", info.id, version.number);
                std::fs::write(
                    dir.join(&relative),
                    self.version_content(&info.id, version.number)?,
                )?;
                history.push(ManifestVersion {
                    number: version.number,
                    saved_at: version.saved_at,
                    path: relative,
                });
                version_count += 1;
            }
            manifest.items.push(ManifestItem {
                info,
                path,
                history,
            });
        }
        std::fs::write(
            dir.join("library.json"),
            serde_json::to_string_pretty(&manifest)? + "\n",
        )?;
        Ok(ExportSummary {
            directory: dir.to_path_buf(),
            items: manifest.items.len(),
            versions: version_count,
        })
    }

    /// Reads an export back, keeping every id, time and version. Items already in the library are
    /// left alone.
    pub fn import_all(&mut self, dir: &Path) -> Result<usize> {
        let manifest: Manifest =
            serde_json::from_str(&std::fs::read_to_string(dir.join("library.json"))?)?;
        if manifest.format != "zacess-library" {
            return Err(LibraryError::Export(format!(
                "{} is not a Zacess library export",
                manifest.format
            )));
        }
        if manifest.format_version > EXPORT_FORMAT_VERSION {
            return Err(LibraryError::Export(format!(
                "export format {} is newer than this build reads",
                manifest.format_version
            )));
        }
        let mut imported = 0;
        for item in manifest.items {
            if self.item(&item.info.id).is_ok() {
                continue;
            }
            let mut versions = Vec::new();
            for version in &item.history {
                let content = std::fs::read_to_string(dir.join(&version.path))?;
                validate(item.info.kind, &content)?;
                versions.push((version.number, version.saved_at, content));
            }
            if versions.is_empty() {
                return Err(LibraryError::Export(format!(
                    "item {} has no versions",
                    item.info.id
                )));
            }
            self.insert_item(&item.info, &versions)?;
            imported += 1;
        }
        Ok(imported)
    }
}

/// The readable form of an item's content: pretty JSON for every kind but notes, Markdown with
/// the title as its heading for notes.
fn readable(kind: ItemKind, content: &str) -> Result<String> {
    Ok(match kind {
        ItemKind::Document => Document::from_json(content)?.to_json_pretty()?,
        ItemKind::Profile => content.to_string(),
        ItemKind::Recording => {
            serde_json::to_string_pretty(&serde_json::from_str::<RecordingEntry>(content)?)? + "\n"
        }
        ItemKind::Deck => {
            serde_json::to_string_pretty(&serde_json::from_str::<Deck>(content)?)? + "\n"
        }
        ItemKind::StudyPlan => {
            serde_json::to_string_pretty(&serde_json::from_str::<Plan>(content)?)? + "\n"
        }
        ItemKind::Clip => {
            serde_json::to_string_pretty(&serde_json::from_str::<ClipEntry>(content)?)? + "\n"
        }
        ItemKind::Note => {
            let note: Note = serde_json::from_str(content)?;
            let mut md = format!("# {}\n\n", note.title.trim());
            if let Some(document) = &note.document {
                md.push_str(&format!("Attached to document `{document}`"));
                if let Some(block) = note.block {
                    md.push_str(&format!(", block `{block}`"));
                }
                md.push_str("\n\n");
            }
            md.push_str(note.text.trim_end());
            md.push('\n');
            md
        }
    })
}

fn validate(kind: ItemKind, content: &str) -> Result<()> {
    match kind {
        ItemKind::Document => {
            Document::from_json(content)?;
        }
        ItemKind::Note => {
            serde_json::from_str::<Note>(content)?;
        }
        ItemKind::Profile => {
            crate::profile::ReadingProfile::from_json(content)?;
        }
        ItemKind::Recording => {
            serde_json::from_str::<RecordingEntry>(content)?;
        }
        ItemKind::Deck => {
            serde_json::from_str::<Deck>(content)?;
        }
        ItemKind::StudyPlan => {
            serde_json::from_str::<Plan>(content)?;
        }
        ItemKind::Clip => {
            let entry = serde_json::from_str::<ClipEntry>(content)?;
            if let ClipContent::Document { document } = entry.content {
                // the document inside is checked against its own schema version too
                Document::from_json(&serde_json::to_string(&*document)?)?;
            }
        }
    }
    Ok(())
}

impl ItemId {
    /// Whether an id is safe to use as a file name in an export.
    pub fn is_file_safe(&self) -> bool {
        !self.0.is_empty()
            && self
                .0
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlockKind, Origin, Rect, Run, Source, SourceFormat};
    use crate::profile::{Preset, ReadingProfile};

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

    #[test]
    fn an_export_holds_every_item_in_open_formats_and_imports_back_whole() {
        let mut lib = Library::open_in_memory().unwrap();
        let doc_id = lib.create_document(&document("Week 3", "first")).unwrap();
        lib.save_document(&doc_id, &document("Week 3", "second"))
            .unwrap();
        let note_id = lib
            .create_note(&Note {
                title: "Check".into(),
                text: "Looks wrong".into(),
                document: Some(doc_id.clone()),
                block: Some(crate::model::BlockId(1)),
            })
            .unwrap();
        let profile_id = lib
            .create_profile(&ReadingProfile::preset(Preset::DyslexiaFriendly))
            .unwrap();
        let binned = lib.create_document(&document("Binned", "gone")).unwrap();
        lib.delete(&binned).unwrap();
        assert!(doc_id.is_file_safe());

        let dir = tempfile::tempdir().unwrap();
        let summary = lib.export_all(dir.path()).unwrap();
        assert_eq!((summary.items, summary.versions), (4, 5));

        let manifest: Manifest = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("library.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest.format, "zacess-library");
        assert_eq!(manifest.items.len(), 4);
        let doc_item = manifest.items.iter().find(|i| i.info.id == doc_id).unwrap();
        assert_eq!(doc_item.history.len(), 2);
        assert!(dir.path().join(&doc_item.path).is_file());
        assert!(
            dir.path()
                .join(format!("documents/{doc_id}.html"))
                .is_file()
        );
        let note_md =
            std::fs::read_to_string(dir.path().join(format!("notes/{note_id}.md"))).unwrap();
        assert!(note_md.starts_with("# Check\n\nAttached to document `"));
        assert!(note_md.ends_with("Looks wrong\n"));
        assert!(
            dir.path()
                .join(format!("profiles/{profile_id}.json"))
                .is_file()
        );
        let binned_item = manifest.items.iter().find(|i| i.info.id == binned).unwrap();
        assert!(binned_item.info.deleted_at.is_some());

        let mut fresh = Library::open_in_memory().unwrap();
        assert_eq!(fresh.import_all(dir.path()).unwrap(), 4);
        assert_eq!(fresh.document(&doc_id).unwrap().plain_text(), "second");
        assert_eq!(
            fresh.document_version(&doc_id, 1).unwrap().plain_text(),
            "first"
        );
        assert_eq!(fresh.note(&note_id).unwrap().text, "Looks wrong");
        assert_eq!(
            fresh.profile(&profile_id).unwrap().name,
            "Dyslexia-Friendly"
        );
        assert_eq!(fresh.bin().unwrap().len(), 1);
        assert_eq!(
            fresh.versions(&doc_id).unwrap(),
            lib.versions(&doc_id).unwrap()
        );
        // importing again changes nothing
        assert_eq!(fresh.import_all(dir.path()).unwrap(), 0);
    }

    #[test]
    fn foreign_or_newer_exports_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("library.json"),
            r#"{"format":"other","format_version":1,"exported_at":0,"items":[]}"#,
        )
        .unwrap();
        let mut lib = Library::open_in_memory().unwrap();
        assert!(matches!(
            lib.import_all(dir.path()),
            Err(LibraryError::Export(_))
        ));
        std::fs::write(
            dir.path().join("library.json"),
            r#"{"format":"zacess-library","format_version":99,"exported_at":0,"items":[]}"#,
        )
        .unwrap();
        assert!(matches!(
            lib.import_all(dir.path()),
            Err(LibraryError::Export(_))
        ));
    }

    #[test]
    fn decks_and_plans_export_with_anki_and_notion_files_and_read_back_equal() {
        use crate::study::{Deck, Plan, Rating, Session};
        let mut lib = Library::open_in_memory()
            .unwrap()
            .with_clock(|| 86_400 * 100);
        let mut deck = Deck::new("Signals");
        deck.add_card("Is x < y?", "Yes,\nalways", None, 0);
        deck.review(1, Rating::Good, 10).unwrap();
        let deck_id = lib.create_deck(&deck).unwrap();
        let mut session = Session::start(ItemId("d-1".into()), 10);
        session.cards_reviewed = 4;
        let plan = Plan {
            name: "Exams".into(),
            sessions: vec![session],
            utc_offset_minutes: -300,
            ..Default::default()
        };
        let plan_id = lib.create_study_plan(&plan).unwrap();

        let dir = tempfile::tempdir().unwrap();
        lib.export_all(dir.path()).unwrap();
        let read = |p: String| std::fs::read_to_string(dir.path().join(p)).unwrap();
        let json: Deck = serde_json::from_str(&read(format!("decks/{deck_id}.json"))).unwrap();
        assert_eq!(json, deck);
        let anki = read(format!("decks/{deck_id}.anki.txt"));
        assert_eq!(
            anki,
            deck.to_anki_text_with(&crate::study::AnkiOptions {
                deck: Some("Signals".into()),
                html: true,
            })
        );
        assert!(
            anki.contains("#deck:Signals\n") && anki.contains("Is x &lt; y?\tYes,<br>always\t")
        );
        let notion = read(format!("decks/{deck_id}.notion.csv"));
        assert!(notion.starts_with("Name,Answer,Source,Due,Reviews,Recall,Tags,Deck\n"));
        assert!(notion.contains("Is x < y?,\"Yes,\nalways\","));
        let plan_json: Plan =
            serde_json::from_str(&read(format!("study_plans/{plan_id}.json"))).unwrap();
        assert_eq!(plan_json, plan);

        let mut fresh = Library::open_in_memory().unwrap();
        assert_eq!(fresh.import_all(dir.path()).unwrap(), 2);
        assert_eq!(fresh.deck(&deck_id).unwrap(), deck);
        assert_eq!(fresh.study_plan(&plan_id).unwrap(), plan);
        assert_eq!(fresh.item(&plan_id).unwrap().title, "Exams");
    }
}
