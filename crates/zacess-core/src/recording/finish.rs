//! Finishing a recording: reading the journal, checking the chunks, joining them into one file
//! and writing the manifest. Recovering after a crash runs exactly the same code, as a recording
//! that was never closed is a recording whose close is missing.

use super::journal::{self, Entry, encode};
use super::{
    ChunkInfo, ChunkState, EndReason, FreeSpace, MANIFEST_VERSION, Manifest, RecordingError,
    Result, Storage, WAV_HEADER, fnv64, read_wav_header, sync_dir, wav_header,
};
use crate::timeline::Pause;
use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// What finishing a recording found.
#[derive(Debug, Clone, PartialEq)]
pub struct Recovery {
    pub id: String,
    pub ended: EndReason,
    pub audio_secs: f64,
    /// Audio that was damaged or missing, whether it was replaced with silence or dropped.
    pub lost_secs: f64,
    /// Chunks that could not be used and were left out at the end.
    pub dropped_chunks: u32,
    /// Chunks replaced with silence.
    pub silenced_chunks: u32,
    /// Chunks cut back to the whole samples that survived.
    pub salvaged_chunks: u32,
    /// Chunks that were whole but missing from the journal.
    pub adopted_chunks: u32,
    /// Journal lines that failed their checksum.
    pub damaged_journal_lines: u32,
    /// The recording was paused when it stopped and that pause was dropped.
    pub dropped_pause: bool,
}

impl Recovery {
    /// Whether anything had to be repaired, lost or guessed.
    pub fn is_clean(&self) -> bool {
        self.ended != EndReason::Recovered
            && self.lost_secs == 0.0
            && self.dropped_chunks == 0
            && self.silenced_chunks == 0
            && self.salvaged_chunks == 0
            && self.adopted_chunks == 0
            && self.damaged_journal_lines == 0
    }
}

/// A finished recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    pub manifest: Manifest,
    pub report: Recovery,
}

enum Verdict {
    /// Whole and the journal's checksum matches.
    Ok(u64),
    /// Whole but the journal does not know it.
    Orphan(u64),
    /// The header is ours but the data stops short.
    Truncated(u64),
    Bad,
}

/// What goes into the finished file for one chunk.
enum Piece {
    Audio { path: PathBuf, frames: u64 },
    Silence { frames: u64 },
}

fn verify(path: &Path, rate: u32, entry: Option<&(u64, String)>) -> Verdict {
    let Ok(bytes) = std::fs::read(path) else {
        return Verdict::Bad;
    };
    let Some((file_rate, declared)) = read_wav_header(&bytes) else {
        return Verdict::Bad;
    };
    if file_rate != rate {
        return Verdict::Bad;
    }
    let actual = (bytes.len() - WAV_HEADER) as u64;
    if declared > actual {
        return Verdict::Truncated(actual / 2);
    }
    if declared < actual || declared % 2 != 0 || declared == 0 {
        return Verdict::Bad;
    }
    let frames = declared / 2;
    match entry {
        Some((expected, hash)) => {
            if *expected == frames && *hash == journal::hex(fnv64(&bytes)) {
                Verdict::Ok(frames)
            } else {
                Verdict::Bad
            }
        }
        None => Verdict::Orphan(frames),
    }
}

fn chunk_indices(dir: &Path) -> BTreeSet<u32> {
    let mut found = BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Some(index) = name
                .to_str()
                .and_then(|n| n.strip_suffix(".wav"))
                .and_then(|n| n.parse::<u32>().ok())
            {
                found.insert(index);
            }
        }
    }
    found
}

/// The start time out of an id such as `rec-1760000000-2`, for a journal that lost its first line.
fn start_from_id(id: &str) -> i64 {
    id.strip_prefix("rec-")
        .and_then(|rest| rest.split('-').next())
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0)
}

pub(crate) fn finalise(
    dir: &Path,
    storage: &dyn Storage,
    space: &dyn FreeSpace,
    now: i64,
) -> Result<Finished> {
    let id = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    if dir.join("manifest.json").exists() {
        tidy_finished(dir);
        return Err(RecordingError::Finished(id));
    }
    let journal_path = dir.join("journal.log");
    let replay = journal::replay(&journal_path)?;
    let chunk_dir = dir.join("chunks");
    let chunk_path = |index: u32| chunk_dir.join(format!("{index:06}.wav"));

    let mut indices = chunk_indices(&chunk_dir);
    indices.extend(replay.chunks.keys().copied());

    // a journal that lost its first line still has chunks whose headers give the rate
    let rate = match &replay.started {
        Some(s) => s.sample_rate,
        None => indices
            .iter()
            .filter_map(|i| std::fs::read(chunk_path(*i)).ok())
            .find_map(|b| read_wav_header(&b).map(|h| h.0))
            .unwrap_or(0),
    };
    if rate == 0 {
        return Err(RecordingError::Empty(id));
    }
    let (title, started_at) = match &replay.started {
        Some(s) => (s.title.clone(), s.started_at),
        None => ("Recording".to_string(), start_from_id(&id)),
    };

    let mut report = Recovery {
        id: id.clone(),
        ended: EndReason::Recovered,
        audio_secs: 0.0,
        lost_secs: 0.0,
        dropped_chunks: 0,
        silenced_chunks: 0,
        salvaged_chunks: 0,
        adopted_chunks: 0,
        damaged_journal_lines: replay.damaged_lines,
        dropped_pause: false,
    };

    let unjournalled_frames = replay
        .started
        .as_ref()
        .map(|s| (s.chunk_secs as f64 * rate as f64) as u64)
        .filter(|f| *f > 0);
    let last = indices.last().copied();
    let mut pieces: Vec<(u32, Piece, ChunkState)> = Vec::new();
    let mut lost_frames = 0u64;
    for index in &indices {
        let entry = replay.chunks.get(index);
        let path = chunk_path(*index);
        let is_last = Some(*index) == last;
        let verdict = verify(&path, rate, entry);
        let (piece, state) = match verdict {
            Verdict::Ok(frames) => (Piece::Audio { path, frames }, ChunkState::Ok),
            Verdict::Orphan(frames) => {
                report.adopted_chunks += 1;
                (Piece::Audio { path, frames }, ChunkState::Adopted)
            }
            Verdict::Truncated(frames) if is_last && frames > 0 => {
                report.salvaged_chunks += 1;
                if let Some((expected, _)) = entry {
                    lost_frames += expected.saturating_sub(frames);
                }
                (Piece::Audio { path, frames }, ChunkState::Salvaged)
            }
            Verdict::Truncated(_) | Verdict::Bad => match entry {
                Some((frames, _)) if !is_last => {
                    report.silenced_chunks += 1;
                    lost_frames += frames;
                    (Piece::Silence { frames: *frames }, ChunkState::Silenced)
                }
                Some((frames, _)) => {
                    report.dropped_chunks += 1;
                    lost_frames += frames;
                    continue;
                }
                None => match unjournalled_frames {
                    // the length is lost with the journal line, so the usual chunk is assumed to
                    // keep everything after it in its place
                    Some(frames) if !is_last => {
                        report.silenced_chunks += 1;
                        lost_frames += frames;
                        (Piece::Silence { frames }, ChunkState::Silenced)
                    }
                    _ => {
                        report.dropped_chunks += 1;
                        continue;
                    }
                },
            },
        };
        pieces.push((*index, piece, state));
    }

    let total_frames: u64 = pieces
        .iter()
        .map(|(_, p, _)| match p {
            Piece::Audio { frames, .. } | Piece::Silence { frames } => *frames,
        })
        .sum();
    if total_frames == 0 {
        // a folder with no chunk file at all holds nothing to lose. One with damaged files is
        // left for the person to decide on
        if indices.is_empty() {
            let _ = std::fs::remove_dir_all(dir);
        }
        return Err(RecordingError::Empty(id));
    }

    let ended = match replay.closed {
        Some((_, reason)) => reason,
        None => {
            storage.append_line(
                &journal_path,
                &encode(&Entry::Closed {
                    at: now,
                    reason: EndReason::Recovered,
                }),
            )?;
            EndReason::Recovered
        }
    };
    report.ended = ended;
    report.lost_secs = lost_frames as f64 / rate as f64;
    report.audio_secs = total_frames as f64 / rate as f64;

    // the pauses, held to the audio that is left in case the end of it was dropped
    let rate_f = rate as f64;
    let mut pauses: Vec<(u64, f64)> = Vec::new();
    for pause in &replay.pauses {
        // stopped while paused: a close the person made ends the pause while a close made by
        // recovery says nothing about when it ended
        let resumed = pause.resumed_at.or(match replay.closed {
            Some((at, reason)) if reason != EndReason::Recovered && at >= pause.at => Some(at),
            _ => None,
        });
        match resumed {
            Some(resumed) => {
                let length = (resumed - pause.at).max(0) as f64;
                pauses.push((pause.frames.min(total_frames), length));
            }
            None => report.dropped_pause = true,
        }
    }
    pauses.sort_by_key(|p| p.0);

    let needed = WAV_HEADER as u64 + total_frames * 2;
    // what an earlier attempt left behind is in the way of the space check and is replaced
    // anyway: a half-joined file goes and a whole one counts as room already taken
    let part = dir.join("recording.wav.part");
    let target = dir.join("recording.wav");
    let _ = std::fs::remove_file(&part);
    let credit = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    if let Ok(available) = space.available(dir)
        && available.saturating_add(credit) < needed
    {
        return Err(RecordingError::NoRoomToFinish { needed, available });
    }

    if let Err(e) = assemble(&part, &target, rate, total_frames, &pieces) {
        let _ = std::fs::remove_file(&part);
        return Err(match e {
            AssembleError::Full => RecordingError::NoRoomToFinish {
                needed,
                available: 0,
            },
            AssembleError::Io(e) => RecordingError::Io(e),
        });
    }

    let mut chunks = Vec::with_capacity(pieces.len());
    let mut start = 0u64;
    let mut before = 0usize;
    let mut paused_before = 0.0f64;
    for (index, piece, state) in &pieces {
        while before < pauses.len() && pauses[before].0 <= start {
            paused_before += pauses[before].1;
            before += 1;
        }
        let frames = match piece {
            Piece::Audio { frames, .. } | Piece::Silence { frames } => *frames,
        };
        chunks.push(ChunkInfo {
            index: *index,
            audio_start_secs: start as f64 / rate_f,
            wall_start_secs: start as f64 / rate_f + paused_before,
            secs: frames as f64 / rate_f,
            state: *state,
        });
        start += frames;
    }
    let paused_total: f64 = pauses.iter().map(|p| p.1).sum();
    let audio_secs = total_frames as f64 / rate_f;
    let wall_secs = audio_secs + paused_total;
    let (size_bytes, fingerprint) = crate::timeline::fingerprint(&target)?;
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        id: id.clone(),
        title,
        started_at,
        ended_at: started_at + wall_secs.round() as i64,
        sample_rate: rate,
        audio_secs,
        wall_secs,
        ended,
        lost_secs: report.lost_secs,
        file: "recording.wav".into(),
        size_bytes,
        fingerprint,
        pauses: pauses
            .iter()
            .map(|(frames, length)| Pause {
                at_secs: (*frames as f64 / rate_f) as f32,
                length_secs: *length as f32,
            })
            .collect(),
        chunks,
    };
    storage.write_atomic(
        &dir.join("manifest.json"),
        (serde_json::to_string_pretty(&manifest)? + "\n").as_bytes(),
    )?;
    // the manifest is what says the recording is finished, so the chunks go only after it
    tidy_finished(dir);
    Ok(Finished { manifest, report })
}

enum AssembleError {
    Full,
    Io(io::Error),
}

impl From<io::Error> for AssembleError {
    fn from(e: io::Error) -> Self {
        if matches!(
            e.kind(),
            io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded
        ) {
            AssembleError::Full
        } else {
            AssembleError::Io(e)
        }
    }
}

fn assemble(
    part: &Path,
    target: &Path,
    rate: u32,
    total_frames: u64,
    pieces: &[(u32, Piece, ChunkState)],
) -> std::result::Result<(), AssembleError> {
    let file = std::fs::File::create(part)?;
    let mut out = io::BufWriter::with_capacity(1 << 20, file);
    out.write_all(&wav_header(total_frames, rate))?;
    let zeros = vec![0u8; 1 << 16];
    for (_, piece, _) in pieces {
        match piece {
            Piece::Audio { path, frames } => {
                let bytes = std::fs::read(path)?;
                out.write_all(&bytes[WAV_HEADER..WAV_HEADER + (*frames as usize) * 2])?;
            }
            Piece::Silence { frames } => {
                let mut left = *frames as usize * 2;
                while left > 0 {
                    let step = left.min(zeros.len());
                    out.write_all(&zeros[..step])?;
                    left -= step;
                }
            }
        }
    }
    out.flush()?;
    out.get_ref().sync_all()?;
    drop(out);
    std::fs::rename(part, target)?;
    if let Some(parent) = target.parent() {
        sync_dir(parent);
    }
    Ok(())
}

/// Clears up after a recording that has a manifest. The chunks that went into the file are
/// removed. A chunk that did not, because it was damaged, is moved to a `damaged` folder as it
/// may still hold audio worth playing. Safe to run again.
pub(crate) fn tidy_finished(dir: &Path) {
    let chunk_dir = dir.join("chunks");
    if !chunk_dir.is_dir() {
        return;
    }
    let good: BTreeSet<u32> = std::fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Manifest>(&text).ok())
        .map(|m| {
            m.chunks
                .iter()
                .filter(|c| matches!(c.state, ChunkState::Ok | ChunkState::Adopted))
                .map(|c| c.index)
                .collect()
        })
        .unwrap_or_default();
    // without a readable manifest nothing is known to be safe, so nothing is removed
    if good.is_empty() && !dir.join("manifest.json").exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&chunk_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let index = name
            .strip_suffix(".wav")
            .and_then(|n| n.parse::<u32>().ok());
        match index {
            Some(i) if good.contains(&i) => {
                let _ = std::fs::remove_file(&path);
            }
            Some(_) => {
                let kept = dir.join("damaged");
                if std::fs::create_dir_all(&kept).is_ok() {
                    let _ = std::fs::rename(&path, kept.join(&name));
                }
            }
            None if name.ends_with(".part") => {
                let _ = std::fs::remove_file(&path);
            }
            None => {}
        }
    }
    // only an empty folder goes, so anything unexpected is left where it is
    let _ = std::fs::remove_dir(&chunk_dir);
}
