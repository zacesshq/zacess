//! The store: a folder with one subfolder for each recording.

#[cfg(feature = "library")]
use super::RecordingEntry;
use super::journal::{Entry, encode};
use super::recorder::Recorder;
use super::{
    DfSpace, Finished, FreeSpace, MANIFEST_VERSION, Manifest, RealDisk, RecordingConfig,
    RecordingError, Result, Storage, finish, sync_dir,
};
#[cfg(feature = "library")]
use crate::library::{ItemId, ItemKind, Library};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Where recordings are kept and the settings they are made with.
#[derive(Debug)]
pub struct RecordingStore {
    root: PathBuf,
    config: RecordingConfig,
    space: Arc<dyn FreeSpace>,
    storage: Arc<dyn Storage>,
}

/// An id is a folder name, so nothing that could leave the store is one.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl RecordingStore {
    /// Opens the store, creating the folder. Free space is asked of `df` unless
    /// [`RecordingStore::with_space`] says otherwise.
    pub fn open(root: &Path, config: RecordingConfig) -> Result<Self> {
        config.validate()?;
        std::fs::create_dir_all(root)?;
        Ok(RecordingStore {
            root: root.to_path_buf(),
            config,
            space: Arc::new(DfSpace),
            storage: Arc::new(RealDisk),
        })
    }

    /// Replaces where free space comes from, as the app knows how to ask the platform.
    pub fn with_space(mut self, space: impl FreeSpace + 'static) -> Self {
        self.space = Arc::new(space);
        self
    }

    #[cfg(all(test, feature = "library"))]
    pub(crate) fn with_storage(mut self, storage: impl Storage + 'static) -> Self {
        self.storage = Arc::new(storage);
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config(&self) -> &RecordingConfig {
        &self.config
    }

    fn folder(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            return Err(RecordingError::Unknown(id.to_string()));
        }
        let dir = self.root.join(id);
        if !dir.is_dir() {
            return Err(RecordingError::Unknown(id.to_string()));
        }
        Ok(dir)
    }

    /// Starts a recording at a Unix time. `sample_rate` is that of the mono samples that will be
    /// pushed.
    pub fn start(&self, title: &str, started_at: i64, sample_rate: u32) -> Result<Recorder> {
        if !(4_000..=192_000).contains(&sample_rate) {
            return Err(RecordingError::Config(format!(
                "a sample rate of {sample_rate} Hz is outside 4000 to 192000"
            )));
        }
        let mut id = format!("rec-{started_at}");
        let mut n = 1;
        // create_dir fails if the folder is there, so two recordings can never share one
        loop {
            match std::fs::create_dir(self.root.join(&id)) {
                Ok(()) => break,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    n += 1;
                    id = format!("rec-{started_at}-{n}");
                }
                Err(e) => return Err(e.into()),
            }
        }
        let dir = self.root.join(&id);
        // a folder that could not be set up holds nothing and must not be left to be "recovered"
        let setup = (|| {
            std::fs::create_dir(dir.join("chunks"))?;
            self.storage.append_line(
                &dir.join("journal.log"),
                &encode(&Entry::Started {
                    version: MANIFEST_VERSION,
                    id: id.clone(),
                    title: title.to_string(),
                    started_at,
                    sample_rate,
                    chunk_secs: self.config.chunk_secs,
                }),
            )?;
            sync_dir(&dir);
            sync_dir(&self.root);
            Ok::<(), std::io::Error>(())
        })();
        if let Err(e) = setup {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e.into());
        }
        Ok(Recorder::new(
            dir,
            sample_rate,
            started_at,
            self.config.clone(),
            self.space.clone(),
            self.storage.clone(),
        ))
    }

    /// The recordings with no manifest: ones the app stopped in the middle of and ones that were
    /// closed but could not be joined for want of room. Call it when the app starts, before a new
    /// recording, as a recording in progress is unfinished too.
    pub fn unfinished(&self) -> Result<Vec<String>> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !valid_id(&name) || !entry.path().is_dir() {
                continue;
            }
            if entry.path().join("manifest.json").exists() {
                // finished, but the app may have stopped before the chunks were cleared away
                finish::tidy_finished(&entry.path());
            } else {
                found.push(name);
            }
        }
        found.sort();
        Ok(found)
    }

    /// Closes an unfinished recording cleanly and finishes it. Safe to run again after a failure.
    pub fn recover(&self, id: &str, now: i64) -> Result<Finished> {
        let dir = self.folder(id)?;
        finish::finalise(&dir, &*self.storage, &*self.space, now)
    }

    /// Recovers everything unfinished, one result each.
    pub fn recover_all(&self, now: i64) -> Result<Vec<(String, Result<Finished>)>> {
        Ok(self
            .unfinished()?
            .into_iter()
            .map(|id| {
                let result = self.recover(&id, now);
                (id, result)
            })
            .collect())
    }

    /// Removes a recording that could not be recovered, such as one with no usable audio.
    pub fn discard(&self, id: &str) -> Result<()> {
        let dir = self.folder(id)?;
        if dir.join("manifest.json").exists() {
            return Err(RecordingError::Finished(id.to_string()));
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    /// The manifest of a finished recording.
    pub fn manifest(&self, id: &str) -> Result<Manifest> {
        let dir = self.folder(id)?;
        let text = std::fs::read_to_string(dir.join("manifest.json"))
            .map_err(|_| RecordingError::State("the recording is not finished"))?;
        let manifest: Manifest = serde_json::from_str(&text)?;
        if manifest.version > MANIFEST_VERSION {
            return Err(RecordingError::UnsupportedManifest(manifest.version));
        }
        Ok(manifest)
    }

    /// The audio file of a finished recording.
    pub fn audio_path(&self, id: &str) -> Result<PathBuf> {
        let manifest = self.manifest(id)?;
        Ok(self.folder(id)?.join(manifest.file))
    }

    /// Stores a finished recording in the library and says which item it is. Storing the same
    /// recording again updates the item and does not make a second.
    #[cfg(feature = "library")]
    pub fn store_in_library(&self, library: &mut Library, id: &str) -> Result<ItemId> {
        let manifest = self.manifest(id)?;
        let entry = RecordingEntry {
            id: manifest.id.clone(),
            title: manifest.title.clone(),
            started_at: manifest.started_at,
            audio_secs: manifest.audio_secs,
            ended: manifest.ended,
            lost_secs: manifest.lost_secs,
            file: format!("{}/{}", manifest.id, manifest.file),
            manifest: format!("{}/manifest.json", manifest.id),
            size_bytes: manifest.size_bytes,
            fingerprint: manifest.fingerprint,
        };
        // the bin is searched too: a recording whose item was deleted is not stored a second time
        for info in library.all_items()? {
            if info.kind != ItemKind::Recording {
                continue;
            }
            if let Ok(held) = library.recording(&info.id)
                && held.id == entry.id
            {
                library.save_recording(&info.id, &entry)?;
                return Ok(info.id);
            }
        }
        Ok(library.create_recording(&entry)?)
    }
}
