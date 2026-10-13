//! A sequence CRDT for note text, so edits made on two devices at once merge without conflicts.
//!
//! Every character carries an id of the device that typed it and that device's counter. An insert
//! names the character it follows; a delete names the character it removes. Two replicas that have
//! seen the same operations, in any order, show the same text. This is the replicated growable
//! array (RGA) approach, kept small on purpose.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Identifies a device in a sync group.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub String);

/// The unique id of one character: who typed it and when, by that device's counter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CharId {
    pub counter: u64,
    pub device: DeviceId,
}

/// One edit to the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TextOp {
    /// Insert `ch` with id `id` just after the character `after`. `None` means at the start.
    Insert {
        id: CharId,
        after: Option<CharId>,
        ch: char,
    },
    Delete {
        id: CharId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Cell {
    id: CharId,
    ch: char,
    deleted: bool,
}

/// The text with its history of characters, deleted ones included.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextCrdt {
    cells: Vec<Cell>,
    /// Operations whose `after` character has not arrived yet.
    #[serde(default)]
    waiting: Vec<TextOp>,
    /// The largest counter seen, so a new character always gets a larger one than anything it
    /// follows. That is what lets concurrent inserts order the same way on every replica.
    #[serde(default)]
    clock: u64,
}

impl TextCrdt {
    pub fn new() -> Self {
        Self::default()
    }

    /// The visible text.
    pub fn text(&self) -> String {
        self.cells
            .iter()
            .filter(|c| !c.deleted)
            .map(|c| c.ch)
            .collect()
    }

    /// The ids of the visible characters, in order.
    fn visible_ids(&self) -> Vec<CharId> {
        self.cells
            .iter()
            .filter(|c| !c.deleted)
            .map(|c| c.id.clone())
            .collect()
    }

    /// Applies an operation from any device. Returns `false` when it had to wait for an earlier
    /// operation, in which case it is applied as soon as that one arrives.
    pub fn apply(&mut self, op: &TextOp) -> bool {
        let id = match op {
            TextOp::Insert { id, .. } | TextOp::Delete { id } => id,
        };
        self.clock = self.clock.max(id.counter);
        let done = match op {
            TextOp::Insert { id, after, ch } => {
                if self.cells.iter().any(|c| &c.id == id) {
                    true
                } else {
                    self.insert_cell(id.clone(), after.as_ref(), *ch)
                }
            }
            TextOp::Delete { id } => match self.cells.iter_mut().find(|c| &c.id == id) {
                Some(cell) => {
                    cell.deleted = true;
                    true
                }
                None => false,
            },
        };
        if !done {
            self.waiting.push(op.clone());
            return false;
        }
        self.drain_waiting();
        true
    }

    fn drain_waiting(&mut self) {
        loop {
            let waiting = std::mem::take(&mut self.waiting);
            let before = waiting.len();
            for op in waiting {
                let ok = match &op {
                    TextOp::Insert { id, after, ch } => {
                        self.cells.iter().any(|c| &c.id == id)
                            || self.insert_cell(id.clone(), after.as_ref(), *ch)
                    }
                    TextOp::Delete { id } => self
                        .cells
                        .iter_mut()
                        .find(|c| &c.id == id)
                        .map(|c| c.deleted = true)
                        .is_some(),
                };
                if !ok {
                    self.waiting.push(op);
                }
            }
            if self.waiting.len() == before {
                break;
            }
        }
    }

    /// Places a character after `after`, skipping past any characters inserted there at the same
    /// time by other devices with a larger id, so every replica orders them the same way.
    fn insert_cell(&mut self, id: CharId, after: Option<&CharId>, ch: char) -> bool {
        let mut index = match after {
            Some(after) => match self.cells.iter().position(|c| &c.id == after) {
                Some(i) => i + 1,
                None => return false,
            },
            None => 0,
        };
        while index < self.cells.len() && self.cells[index].id > id {
            index += 1;
        }
        self.cells.insert(
            index,
            Cell {
                id,
                ch,
                deleted: false,
            },
        );
        true
    }

    /// Works out the operations that turn the current text into `target`, numbering new
    /// characters from `counter` on. The operations are applied here as well as returned.
    pub fn edit_to(&mut self, target: &str, device: &DeviceId, counter: &mut u64) -> Vec<TextOp> {
        let current: Vec<char> = self.text().chars().collect();
        let wanted: Vec<char> = target.chars().collect();
        let ids = self.visible_ids();
        *counter = (*counter).max(self.clock);

        let prefix = current
            .iter()
            .zip(&wanted)
            .take_while(|(a, b)| a == b)
            .count();
        let max_suffix = current.len().min(wanted.len()) - prefix;
        let suffix = current
            .iter()
            .rev()
            .zip(wanted.iter().rev())
            .take(max_suffix)
            .take_while(|(a, b)| a == b)
            .count();

        let mut ops = Vec::new();
        for id in &ids[prefix..current.len() - suffix] {
            ops.push(TextOp::Delete { id: id.clone() });
        }
        let mut after = if prefix == 0 {
            None
        } else {
            Some(ids[prefix - 1].clone())
        };
        for &ch in &wanted[prefix..wanted.len() - suffix] {
            *counter += 1;
            let id = CharId {
                counter: *counter,
                device: device.clone(),
            };
            ops.push(TextOp::Insert {
                id: id.clone(),
                after,
                ch,
            });
            after = Some(id);
        }
        for op in &ops {
            self.apply(op);
        }
        ops
    }

    /// Whether every operation has found its place.
    pub fn is_settled(&self) -> bool {
        self.waiting.is_empty()
    }

    /// The ids of operations this replica has applied, for telling another replica what is known.
    pub fn known(&self) -> HashSet<CharId> {
        self.cells.iter().map(|c| c.id.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str) -> DeviceId {
        DeviceId(name.into())
    }

    #[test]
    fn edits_on_one_device_round_trip() {
        let mut text = TextCrdt::new();
        let mut counter = 0;
        text.edit_to("hello world", &device("phone"), &mut counter);
        assert_eq!(text.text(), "hello world");
        text.edit_to("hello big world", &device("phone"), &mut counter);
        assert_eq!(text.text(), "hello big world");
        text.edit_to("big world", &device("phone"), &mut counter);
        assert_eq!(text.text(), "big world");
        text.edit_to("", &device("phone"), &mut counter);
        assert_eq!(text.text(), "");
    }

    #[test]
    fn concurrent_edits_on_two_devices_converge() {
        let mut phone = TextCrdt::new();
        let mut phone_counter = 0;
        let setup = phone.edit_to("The period is 2.5 ms", &device("phone"), &mut phone_counter);
        let mut laptop = TextCrdt::new();
        for op in &setup {
            laptop.apply(op);
        }
        let mut laptop_counter = 0;

        // offline, both devices change different parts of the same note
        let from_phone =
            phone.edit_to("The period is 2.6 ms", &device("phone"), &mut phone_counter);
        let from_laptop = laptop.edit_to(
            "The period is 2.5 ms (checked)",
            &device("laptop"),
            &mut laptop_counter,
        );

        for op in &from_laptop {
            phone.apply(op);
        }
        for op in &from_phone {
            laptop.apply(op);
        }
        assert_eq!(phone.text(), "The period is 2.6 ms (checked)");
        assert_eq!(laptop.text(), phone.text());
    }

    #[test]
    fn inserts_at_the_same_place_land_in_the_same_order_everywhere() {
        let mut a = TextCrdt::new();
        let mut ac = 0;
        let base = a.edit_to("ab", &device("a"), &mut ac);
        let mut b = TextCrdt::new();
        let mut bc = 0;
        for op in &base {
            b.apply(op);
        }
        let from_a = a.edit_to("aXb", &device("a"), &mut ac);
        let from_b = b.edit_to("aYb", &device("b"), &mut bc);
        // delivered in opposite orders
        for op in &from_b {
            a.apply(op);
        }
        for op in &from_a {
            b.apply(op);
        }
        assert_eq!(a.text(), b.text());
        assert!(["aXYb", "aYXb"].contains(&a.text().as_str()));
    }

    #[test]
    fn operations_arriving_early_wait_for_their_predecessor() {
        let mut source = TextCrdt::new();
        let mut counter = 0;
        let ops = source.edit_to("abc", &device("a"), &mut counter);
        let mut late = TextCrdt::new();
        // the third character arrives before the first two
        assert!(!late.apply(&ops[2]));
        assert!(!late.is_settled());
        late.apply(&ops[0]);
        late.apply(&ops[1]);
        assert!(late.is_settled());
        assert_eq!(late.text(), "abc");
        assert_eq!(late.known().len(), 3);
    }
}
