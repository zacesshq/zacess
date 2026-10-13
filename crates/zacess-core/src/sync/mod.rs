//! Local-first sync with the conflict rules agreed in the plan, before any server exists.
//!
//! Every device keeps a [`Replica`]: the items it knows, a log of the changes it made and the
//! changes it has received. Devices exchange changes; how two changes to the same item combine
//! follows the rule table in "Data, storage and portability":
//!
//! | Situation | Behaviour |
//! | --- | --- |
//! | Same note edited on two devices | Text edits merge automatically through a CRDT |
//! | Note deleted on one device, edited offline on another | The edit wins and the note is restored, with a notice |
//! | Same reading-profile setting changed on two devices | The most recent change wins and the older one is kept in history |
//! | Large file changed on two devices | Both copies are kept and the person chooses |
//! | Storage full | Sync pauses with a clear message; nothing is deleted |
//!
//! Every item carries a status for the interface: saved on this device, syncing, synced or needs
//! attention. The transport between devices is not here; the server comes later and will carry
//! [`Change`]s as they are.

pub mod text;

use crate::library::{ItemId, ItemKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
pub use text::{CharId, DeviceId, TextCrdt, TextOp};

/// The id of a change: the device that made it and that device's running counter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChangeId {
    pub device: DeviceId,
    pub counter: u64,
}

/// What a change does to an item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Operation {
    /// A note's text edits, which merge with everyone else's.
    NoteEdit {
        ops: Vec<TextOp>,
    },
    /// A whole new value for a setting-like item (a profile or a document), where the most recent
    /// change wins. `base` is the change this one was made on top of.
    Replace {
        content: String,
        base: Option<ChangeId>,
    },
    /// A new copy of a large file, identified by its hash. Concurrent copies are both kept.
    LargeFile {
        hash: String,
        size: u64,
        base: Option<ChangeId>,
    },
    Delete,
    Restore,
}

/// One change to one item, as devices exchange them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub id: ChangeId,
    pub item: ItemId,
    pub kind: ItemKind,
    /// Wall-clock time on the device that made it, Unix seconds.
    pub at: i64,
    pub op: Operation,
}

/// What the interface shows next to an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SyncStatus {
    /// Changed here and not yet sent.
    SavedOnThisDevice,
    /// Sent, not yet confirmed by another device or the server.
    Syncing,
    Synced,
    /// Something needs the person: the message says what.
    NeedsAttention {
        message: String,
    },
}

/// Something the person should hear about after a merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub item: ItemId,
    pub message: String,
}

/// A copy of a large file, kept until the person chooses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCopy {
    pub change: ChangeId,
    pub hash: String,
    pub size: u64,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplicaItem {
    pub kind: ItemKind,
    pub deleted: bool,
    /// Note text, for notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TextCrdt>,
    /// The current value, for setting-like items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Earlier values that lost to a more recent change, newest last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<(ChangeId, String)>,
    /// Copies of a large file waiting for a choice. Otherwise the single current copy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copies: Vec<FileCopy>,
    /// The change the current value came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_change: Option<ChangeId>,
    /// The time of the change the current value came from.
    pub last_at: i64,
    /// The last delete applied, so a later concurrent edit can restore the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_by: Option<ChangeId>,
    pub status: SyncStatus,
}

/// One device's view of the sync group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replica {
    pub device: DeviceId,
    counter: u64,
    items: BTreeMap<ItemId, ReplicaItem>,
    /// Changes made here, in order. Sent ones stay until acknowledged.
    outbox: Vec<Change>,
    acknowledged: HashSet<ChangeId>,
    seen: HashSet<ChangeId>,
    storage_full: bool,
    notices: Vec<Notice>,
}

const STORAGE_FULL: &str = "Sync is paused because storage is full. Nothing has been deleted.";

/// Why changes were not applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SyncError {
    #[error("sync is paused because this device's storage is full; nothing has been deleted")]
    StorageFull,
    #[error("no item {0}")]
    NoSuchItem(ItemId),
    #[error("item {0} is a {1}, which this operation does not apply to")]
    WrongKind(ItemId, ItemKind),
}

impl Replica {
    pub fn new(device: DeviceId) -> Self {
        Replica {
            device,
            counter: 0,
            items: BTreeMap::new(),
            outbox: Vec::new(),
            acknowledged: HashSet::new(),
            seen: HashSet::new(),
            storage_full: false,
            notices: Vec::new(),
        }
    }

    /// The status a change made here gets: saved locally, unless sync is paused for full storage.
    fn local_status(&self) -> SyncStatus {
        if self.storage_full {
            SyncStatus::NeedsAttention {
                message: STORAGE_FULL.to_string(),
            }
        } else {
            SyncStatus::SavedOnThisDevice
        }
    }

    fn next_id(&mut self) -> ChangeId {
        self.counter += 1;
        ChangeId {
            device: self.device.clone(),
            counter: self.counter,
        }
    }

    pub fn item(&self, id: &ItemId) -> Option<&ReplicaItem> {
        self.items.get(id)
    }

    pub fn items(&self) -> impl Iterator<Item = (&ItemId, &ReplicaItem)> {
        self.items.iter()
    }

    /// The current text of a note.
    pub fn note_text(&self, id: &ItemId) -> Option<String> {
        self.items
            .get(id)
            .and_then(|i| i.text.as_ref())
            .map(TextCrdt::text)
    }

    /// Notices raised by merges since they were last taken.
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }

    /// Marks storage as full or free again. While full, incoming changes are refused and local
    /// changes wait in the outbox; nothing is removed to make room.
    pub fn set_storage_full(&mut self, full: bool) {
        self.storage_full = full;
        let message =
            "Sync is paused because storage is full. Nothing has been deleted.".to_string();
        for item in self.items.values_mut() {
            match (&item.status, full) {
                (_, true) if item.status != SyncStatus::Synced => {
                    item.status = SyncStatus::NeedsAttention {
                        message: message.clone(),
                    }
                }
                (SyncStatus::NeedsAttention { message: m }, false) if *m == message => {
                    item.status = SyncStatus::SavedOnThisDevice
                }
                _ => {}
            }
        }
    }

    pub fn is_storage_full(&self) -> bool {
        self.storage_full
    }

    // --- local changes ----------------------------------------------------------------------

    fn record(&mut self, item: ItemId, kind: ItemKind, at: i64, op: Operation) -> ChangeId {
        let id = self.next_id();
        self.seen.insert(id.clone());
        self.outbox.push(Change {
            id: id.clone(),
            item,
            kind,
            at,
            op,
        });
        id
    }

    fn entry(&mut self, id: &ItemId, kind: ItemKind, at: i64) -> &mut ReplicaItem {
        self.items.entry(id.clone()).or_insert_with(|| ReplicaItem {
            kind,
            deleted: false,
            text: (kind == ItemKind::Note).then(TextCrdt::new),
            content: None,
            history: Vec::new(),
            copies: Vec::new(),
            last_change: None,
            last_at: at,
            deleted_by: None,
            status: SyncStatus::SavedOnThisDevice,
        })
    }

    /// Edits a note on this device. Returns the change to send.
    pub fn edit_note(&mut self, id: &ItemId, new_text: &str, at: i64) -> Change {
        let device = self.device.clone();
        let mut counter = self.counter;
        let item = self.entry(id, ItemKind::Note, at);
        let text = item.text.get_or_insert_with(TextCrdt::new);
        let ops = text.edit_to(new_text, &device, &mut counter);
        self.counter = counter;
        let change_id = self.record(id.clone(), ItemKind::Note, at, Operation::NoteEdit { ops });
        let status = self.local_status();
        let item = self.items.get_mut(id).expect("just created");
        item.deleted = false;
        item.last_change = Some(change_id);
        item.last_at = at;
        item.status = status;
        self.outbox.last().expect("just recorded").clone()
    }

    /// Replaces a setting-like item's value on this device, such as a reading profile.
    pub fn replace(&mut self, id: &ItemId, kind: ItemKind, content: &str, at: i64) -> Change {
        let base = self.items.get(id).and_then(|i| i.last_change.clone());
        let change_id = self.record(
            id.clone(),
            kind,
            at,
            Operation::Replace {
                content: content.to_string(),
                base,
            },
        );
        let status = self.local_status();
        let item = self.entry(id, kind, at);
        item.content = Some(content.to_string());
        item.deleted = false;
        item.last_change = Some(change_id);
        item.last_at = at;
        item.status = status;
        self.outbox.last().expect("just recorded").clone()
    }

    /// Records a new copy of a large file made on this device.
    pub fn save_large_file(&mut self, id: &ItemId, hash: &str, size: u64, at: i64) -> Change {
        let base = self.items.get(id).and_then(|i| i.last_change.clone());
        let change_id = self.record(
            id.clone(),
            ItemKind::Document,
            at,
            Operation::LargeFile {
                hash: hash.into(),
                size,
                base,
            },
        );
        let status = self.local_status();
        let item = self.entry(id, ItemKind::Document, at);
        item.copies = vec![FileCopy {
            change: change_id.clone(),
            hash: hash.into(),
            size,
            at,
        }];
        item.deleted = false;
        item.last_change = Some(change_id);
        item.last_at = at;
        item.status = status;
        self.outbox.last().expect("just recorded").clone()
    }

    pub fn delete(&mut self, id: &ItemId, at: i64) -> Result<Change, SyncError> {
        let kind = self
            .items
            .get(id)
            .ok_or_else(|| SyncError::NoSuchItem(id.clone()))?
            .kind;
        let change_id = self.record(id.clone(), kind, at, Operation::Delete);
        let status = self.local_status();
        let item = self.items.get_mut(id).expect("checked above");
        item.deleted = true;
        item.deleted_by = Some(change_id);
        item.status = status;
        Ok(self.outbox.last().expect("just recorded").clone())
    }

    /// The person chooses one copy of a large file; the others are dropped from this item.
    pub fn choose_copy(
        &mut self,
        id: &ItemId,
        change: &ChangeId,
        at: i64,
    ) -> Result<Change, SyncError> {
        let item = self
            .items
            .get(id)
            .ok_or_else(|| SyncError::NoSuchItem(id.clone()))?;
        let chosen = item
            .copies
            .iter()
            .find(|c| &c.change == change)
            .cloned()
            .ok_or_else(|| SyncError::WrongKind(id.clone(), item.kind))?;
        Ok(self.save_large_file(id, &chosen.hash, chosen.size, at))
    }

    // --- exchanging changes -------------------------------------------------------------------

    /// Changes made here that other devices have not confirmed.
    pub fn pending(&self) -> Vec<Change> {
        self.outbox
            .iter()
            .filter(|c| !self.acknowledged.contains(&c.id))
            .cloned()
            .collect()
    }

    /// Marks changes as sent, so their items show as syncing.
    pub fn mark_sent(&mut self, ids: &[ChangeId]) {
        for change in &self.outbox {
            if ids.contains(&change.id)
                && let Some(item) = self.items.get_mut(&change.item)
                && item.status == SyncStatus::SavedOnThisDevice
            {
                item.status = SyncStatus::Syncing;
            }
        }
    }

    /// Records that other devices have the changes. Items with nothing outstanding show as synced.
    pub fn acknowledge(&mut self, ids: &[ChangeId]) {
        self.acknowledged.extend(ids.iter().cloned());
        let outstanding: HashSet<ItemId> = self
            .outbox
            .iter()
            .filter(|c| !self.acknowledged.contains(&c.id))
            .map(|c| c.item.clone())
            .collect();
        for (id, item) in self.items.iter_mut() {
            if !outstanding.contains(id)
                && matches!(
                    item.status,
                    SyncStatus::SavedOnThisDevice | SyncStatus::Syncing
                )
            {
                item.status = SyncStatus::Synced;
            }
        }
        self.outbox.retain(|c| !self.acknowledged.contains(&c.id));
    }

    /// Applies changes from another device, following the conflict rules. Returns the ids applied,
    /// which the other device wants acknowledged.
    pub fn apply(&mut self, changes: &[Change]) -> Result<Vec<ChangeId>, SyncError> {
        if self.storage_full {
            return Err(SyncError::StorageFull);
        }
        let mut applied = Vec::new();
        for change in changes {
            if !self.seen.insert(change.id.clone()) {
                applied.push(change.id.clone());
                continue;
            }
            self.apply_one(change);
            applied.push(change.id.clone());
        }
        Ok(applied)
    }

    fn apply_one(&mut self, change: &Change) {
        let item_id = change.item.clone();
        let existed = self.items.contains_key(&item_id);
        let item = self.entry(&item_id, change.kind, change.at);
        let local_edits_after_delete = |item: &ReplicaItem| {
            item.last_change
                .as_ref()
                .is_some_and(|c| Some(c) != item.deleted_by.as_ref())
        };
        match &change.op {
            Operation::NoteEdit { ops } => {
                let text = item.text.get_or_insert_with(TextCrdt::new);
                for op in ops {
                    text.apply(op);
                }
                let was_deleted = item.deleted;
                item.deleted = false;
                item.last_change = Some(change.id.clone());
                item.last_at = item.last_at.max(change.at);
                if was_deleted {
                    item.deleted_by = None;
                    self.notices.push(Notice {
                        item: item_id.clone(),
                        message: "This note was deleted on one device and edited on another. The edit won, so the note has been restored.".into(),
                    });
                }
            }
            Operation::Replace { content, .. } => {
                let newer = (change.at, &change.id)
                    > (
                        item.last_at,
                        item.last_change.as_ref().unwrap_or(&change.id),
                    );
                if item.content.is_none() || newer {
                    if let (Some(old), Some(old_id)) =
                        (item.content.take(), item.last_change.clone())
                    {
                        item.history.push((old_id, old));
                    }
                    item.content = Some(content.clone());
                    item.last_change = Some(change.id.clone());
                    item.last_at = change.at;
                } else {
                    // the older change loses but is kept in history
                    item.history.push((change.id.clone(), content.clone()));
                }
                if item.deleted {
                    item.deleted = false;
                    item.deleted_by = None;
                    self.notices.push(Notice {
                        item: item_id.clone(),
                        message: "This item was deleted on one device and changed on another. The change won, so it has been restored.".into(),
                    });
                }
            }
            Operation::LargeFile { hash, size, base } => {
                let copy = FileCopy {
                    change: change.id.clone(),
                    hash: hash.clone(),
                    size: *size,
                    at: change.at,
                };
                item.deleted = false;
                let known = item.copies.iter().any(|c| &c.hash == hash);
                let concurrent = existed
                    && !item.copies.is_empty()
                    && base.as_ref() != item.last_change.as_ref();
                if known || !concurrent {
                    // the person chose this copy or it follows on from what we already have
                    item.copies = vec![copy];
                    item.last_change = Some(change.id.clone());
                    item.last_at = change.at;
                    if matches!(item.status, SyncStatus::NeedsAttention { .. }) {
                        item.status = SyncStatus::Syncing;
                    }
                } else {
                    item.copies.push(copy);
                    item.status = SyncStatus::NeedsAttention {
                        message: "This file was changed on two devices. Both copies have been kept; choose the one to continue with.".into(),
                    };
                    self.notices.push(Notice {
                        item: item_id.clone(),
                        message: "A large file was changed on two devices. Both copies are kept until you choose.".into(),
                    });
                }
            }
            Operation::Delete => {
                if local_edits_after_delete(item) && item.last_at > change.at {
                    // this device edited the item after the other device deleted it: the edit wins
                    let edited_at = item.last_at;
                    self.notices.push(Notice {
                        item: item_id.clone(),
                        message: "This item was deleted on another device after you edited it here. Your edit won, so it stays.".into(),
                    });
                    self.counter += 1;
                    let id = ChangeId {
                        device: self.device.clone(),
                        counter: self.counter,
                    };
                    self.seen.insert(id.clone());
                    self.outbox.push(Change {
                        id,
                        item: item_id.clone(),
                        kind: change.kind,
                        at: edited_at,
                        op: Operation::Restore,
                    });
                } else {
                    item.deleted = true;
                    item.deleted_by = Some(change.id.clone());
                }
            }
            Operation::Restore => {
                item.deleted = false;
                item.deleted_by = None;
            }
        }
        if let Some(item) = self.items.get_mut(&item_id)
            && !matches!(item.status, SyncStatus::NeedsAttention { .. })
        {
            let outstanding = self
                .outbox
                .iter()
                .any(|c| c.item == item_id && !self.acknowledged.contains(&c.id));
            item.status = if outstanding {
                SyncStatus::SavedOnThisDevice
            } else {
                SyncStatus::Synced
            };
        }
    }
}

/// Exchanges every pending change between two devices, as a server would relay them.
pub fn exchange(a: &mut Replica, b: &mut Replica) -> Result<(), SyncError> {
    let from_a = a.pending();
    let from_b = b.pending();
    a.mark_sent(&from_a.iter().map(|c| c.id.clone()).collect::<Vec<_>>());
    b.mark_sent(&from_b.iter().map(|c| c.id.clone()).collect::<Vec<_>>());
    let got_by_b = b.apply(&from_a)?;
    let got_by_a = a.apply(&from_b)?;
    a.acknowledge(&got_by_b);
    b.acknowledge(&got_by_a);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Replica, Replica) {
        (
            Replica::new(DeviceId("phone".into())),
            Replica::new(DeviceId("laptop".into())),
        )
    }

    fn note() -> ItemId {
        ItemId("n-1".into())
    }

    #[test]
    fn a_note_edited_on_two_devices_merges_automatically() {
        let (mut phone, mut laptop) = pair();
        phone.edit_note(&note(), "The period is 2.5 ms", 100);
        exchange(&mut phone, &mut laptop).unwrap();
        assert_eq!(
            laptop.note_text(&note()).as_deref(),
            Some("The period is 2.5 ms")
        );
        assert_eq!(phone.item(&note()).unwrap().status, SyncStatus::Synced);

        phone.edit_note(&note(), "The period is 2.6 ms", 200);
        laptop.edit_note(&note(), "The period is 2.5 ms (checked)", 201);
        assert_eq!(
            phone.item(&note()).unwrap().status,
            SyncStatus::SavedOnThisDevice
        );
        exchange(&mut phone, &mut laptop).unwrap();
        assert_eq!(
            phone.note_text(&note()).as_deref(),
            Some("The period is 2.6 ms (checked)")
        );
        assert_eq!(laptop.note_text(&note()), phone.note_text(&note()));
        assert!(phone.take_notices().is_empty());
        assert_eq!(laptop.item(&note()).unwrap().status, SyncStatus::Synced);
    }

    #[test]
    fn a_note_deleted_on_one_device_and_edited_on_another_is_restored_with_a_notice() {
        let (mut phone, mut laptop) = pair();
        phone.edit_note(&note(), "Keep this", 100);
        exchange(&mut phone, &mut laptop).unwrap();

        phone.delete(&note(), 200).unwrap();
        laptop.edit_note(&note(), "Keep this, it matters", 210);
        exchange(&mut phone, &mut laptop).unwrap();

        for replica in [&mut phone, &mut laptop] {
            let item = replica.item(&note()).unwrap();
            assert!(!item.deleted, "{}", replica.device.0);
            assert_eq!(
                replica.note_text(&note()).as_deref(),
                Some("Keep this, it matters")
            );
            let notices = replica.take_notices();
            assert_eq!(notices.len(), 1, "{}: {notices:?}", replica.device.0);
            assert!(
                notices[0].message.contains("restored") || notices[0].message.contains("stays")
            );
        }
    }

    #[test]
    fn a_plain_delete_still_deletes_everywhere() {
        let (mut phone, mut laptop) = pair();
        phone.edit_note(&note(), "Old", 100);
        exchange(&mut phone, &mut laptop).unwrap();
        laptop.delete(&note(), 200).unwrap();
        exchange(&mut phone, &mut laptop).unwrap();
        assert!(phone.item(&note()).unwrap().deleted);
        assert!(phone.take_notices().is_empty());
    }

    #[test]
    fn the_most_recent_profile_change_wins_and_the_older_one_is_kept() {
        let (mut phone, mut laptop) = pair();
        let profile = ItemId("p-1".into());
        phone.replace(&profile, ItemKind::Profile, r#"{"font_size_pt":16}"#, 100);
        exchange(&mut phone, &mut laptop).unwrap();

        laptop.replace(&profile, ItemKind::Profile, r#"{"font_size_pt":20}"#, 300);
        phone.replace(&profile, ItemKind::Profile, r#"{"font_size_pt":24}"#, 250);
        exchange(&mut phone, &mut laptop).unwrap();

        for replica in [&phone, &laptop] {
            let item = replica.item(&profile).unwrap();
            assert_eq!(
                item.content.as_deref(),
                Some(r#"{"font_size_pt":20}"#),
                "{}",
                replica.device.0
            );
            let kept: Vec<&str> = item.history.iter().map(|(_, c)| c.as_str()).collect();
            assert!(
                kept.contains(&r#"{"font_size_pt":24}"#),
                "{}: {kept:?}",
                replica.device.0
            );
        }
    }

    #[test]
    fn a_large_file_changed_on_two_devices_keeps_both_copies_until_the_person_chooses() {
        let (mut phone, mut laptop) = pair();
        let recording = ItemId("r-1".into());
        phone.save_large_file(&recording, "aaa", 1_000, 100);
        exchange(&mut phone, &mut laptop).unwrap();

        phone.save_large_file(&recording, "bbb", 1_200, 200);
        laptop.save_large_file(&recording, "ccc", 900, 205);
        exchange(&mut phone, &mut laptop).unwrap();

        let item = phone.item(&recording).unwrap();
        assert_eq!(item.copies.len(), 2);
        assert!(matches!(item.status, SyncStatus::NeedsAttention { .. }));
        let hashes: HashSet<&str> = item.copies.iter().map(|c| c.hash.as_str()).collect();
        assert_eq!(hashes, HashSet::from(["bbb", "ccc"]));
        let chosen = item
            .copies
            .iter()
            .find(|c| c.hash == "ccc")
            .unwrap()
            .change
            .clone();
        assert_eq!(laptop.item(&recording).unwrap().copies.len(), 2);
        assert!(!phone.take_notices().is_empty());

        phone.choose_copy(&recording, &chosen, 300).unwrap();
        exchange(&mut phone, &mut laptop).unwrap();
        for replica in [&phone, &laptop] {
            let item = replica.item(&recording).unwrap();
            assert_eq!(item.copies.len(), 1, "{}", replica.device.0);
            assert_eq!(item.copies[0].hash, "ccc");
        }
    }

    #[test]
    fn full_storage_pauses_sync_and_deletes_nothing() {
        let (mut phone, mut laptop) = pair();
        phone.edit_note(&note(), "Before", 100);
        exchange(&mut phone, &mut laptop).unwrap();

        laptop.set_storage_full(true);
        phone.edit_note(&note(), "After", 200);
        let incoming = phone.pending();
        assert_eq!(laptop.apply(&incoming), Err(SyncError::StorageFull));
        assert_eq!(laptop.note_text(&note()).as_deref(), Some("Before"));
        laptop.edit_note(&note(), "Before, locally", 210);
        assert!(
            matches!(laptop.item(&note()).unwrap().status, SyncStatus::NeedsAttention { ref message } if message.contains("storage is full"))
        );
        assert_eq!(
            laptop.pending().len(),
            1,
            "local changes wait rather than vanish"
        );

        laptop.set_storage_full(false);
        assert_eq!(
            laptop.item(&note()).unwrap().status,
            SyncStatus::SavedOnThisDevice
        );
        exchange(&mut phone, &mut laptop).unwrap();
        assert_eq!(phone.note_text(&note()), laptop.note_text(&note()));
        assert_eq!(laptop.item(&note()).unwrap().status, SyncStatus::Synced);
    }

    #[test]
    fn statuses_follow_the_change_through_sending_and_confirmation() {
        let (mut phone, mut laptop) = pair();
        let change = phone.edit_note(&note(), "Hello", 100);
        assert_eq!(
            phone.item(&note()).unwrap().status,
            SyncStatus::SavedOnThisDevice
        );
        phone.mark_sent(std::slice::from_ref(&change.id));
        assert_eq!(phone.item(&note()).unwrap().status, SyncStatus::Syncing);
        let got = laptop.apply(std::slice::from_ref(&change)).unwrap();
        phone.acknowledge(&got);
        assert_eq!(phone.item(&note()).unwrap().status, SyncStatus::Synced);
        assert!(phone.pending().is_empty());
        // applying the same change twice changes nothing
        laptop.apply(&[change]).unwrap();
        assert_eq!(laptop.note_text(&note()).as_deref(), Some("Hello"));
    }

    #[test]
    fn a_replica_survives_being_saved_as_json() {
        let (mut phone, mut laptop) = pair();
        phone.edit_note(&note(), "Persist me", 100);
        exchange(&mut phone, &mut laptop).unwrap();
        let json = serde_json::to_string(&laptop).unwrap();
        let back: Replica = serde_json::from_str(&json).unwrap();
        assert_eq!(back.note_text(&note()).as_deref(), Some("Persist me"));
        assert_eq!(back.device, laptop.device);
    }
}
