//! The journal: one line per event, each with its own checksum, only ever appended to.

use super::{EndReason, fnv64};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum Entry {
    Started {
        version: u32,
        id: String,
        title: String,
        started_at: i64,
        sample_rate: u32,
        chunk_secs: f32,
    },
    Chunk {
        index: u32,
        frames: u64,
        /// The checksum of the whole chunk file, as hex.
        hash: String,
    },
    Paused {
        at: i64,
        /// Samples recorded before the pause.
        frames: u64,
    },
    Resumed {
        at: i64,
    },
    Closed {
        at: i64,
        reason: EndReason,
    },
}

pub(crate) fn hex(hash: u64) -> String {
    format!("{hash:016x}")
}

/// A line ready to append: the checksum of the JSON, a space, the JSON and a newline.
pub(crate) fn encode(entry: &Entry) -> Vec<u8> {
    let json = serde_json::to_string(entry).unwrap_or_default();
    let mut line = format!("{} {json}\n", hex(fnv64(json.as_bytes()))).into_bytes();
    line.shrink_to_fit();
    line
}

fn decode(line: &[u8]) -> Option<Entry> {
    if line.len() < 18 || line[16] != b' ' {
        return None;
    }
    let (sum, json) = (&line[..16], &line[17..]);
    if sum != hex(fnv64(json)).as_bytes() {
        return None;
    }
    serde_json::from_slice(json).ok()
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StartInfo {
    pub chunk_secs: f32,
    pub id: String,
    pub title: String,
    pub started_at: i64,
    pub sample_rate: u32,
}

/// A pause as journalled. `resumed_at` is empty when the app stopped while paused.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PauseRecord {
    pub frames: u64,
    pub at: i64,
    pub resumed_at: Option<i64>,
}

/// What the journal says happened.
#[derive(Debug, Default)]
pub(crate) struct Replay {
    pub started: Option<StartInfo>,
    /// Chunk index to the frames and checksum the journal gave.
    pub chunks: BTreeMap<u32, (u64, String)>,
    pub pauses: Vec<PauseRecord>,
    pub closed: Option<(i64, EndReason)>,
    /// Lines that failed their checksum or did not parse. A torn last line is one.
    pub damaged_lines: u32,
}

/// Reads a journal. A line that fails its checksum is skipped and counted, as each line stands
/// alone. A missing journal reads as empty.
pub(crate) fn replay(path: &Path) -> io::Result<Replay> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Replay::default()),
        Err(e) => return Err(e),
    };
    let mut replay = Replay::default();
    for line in bytes.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Some(entry) = decode(line) else {
            replay.damaged_lines += 1;
            continue;
        };
        match entry {
            Entry::Started {
                id,
                title,
                started_at,
                sample_rate,
                chunk_secs,
                ..
            } => {
                if replay.started.is_none() && sample_rate > 0 {
                    replay.started = Some(StartInfo {
                        chunk_secs,
                        id,
                        title,
                        started_at,
                        sample_rate,
                    });
                }
            }
            Entry::Chunk {
                index,
                frames,
                hash,
            } => {
                replay.chunks.insert(index, (frames, hash));
            }
            Entry::Paused { at, frames } => {
                // a pause still open here lost its resume line to damage, so its length is not
                // known and it is closed at no length, which keeps the pauses after it right
                if let Some(open) = replay.pauses.last_mut()
                    && open.resumed_at.is_none()
                {
                    open.resumed_at = Some(open.at);
                }
                replay.pauses.push(PauseRecord {
                    frames,
                    at,
                    resumed_at: None,
                });
            }
            Entry::Resumed { at } => {
                if let Some(open) = replay.pauses.last_mut()
                    && open.resumed_at.is_none()
                {
                    open.resumed_at = Some(at);
                }
            }
            Entry::Closed { at, reason } => {
                if replay.closed.is_none() {
                    replay.closed = Some((at, reason));
                }
            }
        }
    }
    Ok(replay)
}
