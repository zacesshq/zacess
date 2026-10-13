//! Recording a lecture on the device: the storage core.
//!
//! The microphone is the app's. This module takes the samples it hands over and keeps them safe.
//! Audio is written in short chunks, each a file that is written whole and moved into place, with
//! a journal beside them. A crash, a killed app or a dead battery then loses at most the audio not
//! yet in a chunk and the next start can find the recording and close it. A finished recording is
//! one WAV file with a manifest that says where every chunk sits on the recording's clock, so the
//! [timeline](crate::timeline) can line the boards and slides up with the audio.
//!
//! See `docs/recording.md`.

mod finish;
mod journal;
mod recorder;
mod store;

pub use finish::{Finished, Recovery};
pub use recorder::{Progress, Recorder, Status};
pub use store::RecordingStore;

use crate::library::LibraryError;
use crate::timeline::{Pause, Recording};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use std::path::Path;

pub const MANIFEST_VERSION: u32 = 1;

/// The bytes in the header of every WAV file this module writes.
pub(crate) const WAV_HEADER: usize = 44;

/// The most audio a WAV file can describe, leaving room for the header.
const WAV_MAX_AUDIO: u64 = u32::MAX as u64 - WAV_HEADER as u64;

#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    #[error("recording storage failed: {0}")]
    Io(#[from] io::Error),
    #[error("a recording file is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Config(String),
    #[error("the recording cannot do that now: {0}")]
    State(&'static str),
    #[error("there is no recording called {0}")]
    Unknown(String),
    #[error("{0} is already finished")]
    Finished(String),
    #[error("{0} holds no audio, so there is nothing to keep")]
    Empty(String),
    #[error("{needed} bytes are needed to finish the recording and {available} are free")]
    NoRoomToFinish { needed: u64, available: u64 },
    #[error("the manifest is version {0}, this build reads up to {MANIFEST_VERSION}")]
    UnsupportedManifest(u32),
    #[error(transparent)]
    Library(#[from] LibraryError),
}

pub type Result<T> = std::result::Result<T, RecordingError>;

/// Settings for a recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingConfig {
    /// Seconds of audio in each chunk. This is the most a crash can lose.
    pub chunk_secs: f32,
    /// Free space always left for the rest of the device.
    pub reserve_bytes: u64,
    /// Also keep free as much as has been recorded so far, so the chunks can always be joined
    /// into the finished file.
    pub keep_room_to_finish: bool,
    /// The most audio in one recording.
    pub max_bytes: u64,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        RecordingConfig {
            chunk_secs: 10.0,
            reserve_bytes: 64 << 20,
            keep_room_to_finish: true,
            max_bytes: 2 << 30,
        }
    }
}

impl RecordingConfig {
    pub fn validate(&self) -> Result<()> {
        if !self.chunk_secs.is_finite() || !(0.1..=60.0).contains(&self.chunk_secs) {
            return Err(RecordingError::Config(
                "a chunk must hold between 0.1 and 60 seconds".into(),
            ));
        }
        if self.max_bytes < 1 << 16 || self.max_bytes > WAV_MAX_AUDIO {
            return Err(RecordingError::Config(format!(
                "the size limit must be between 64 KiB and {WAV_MAX_AUDIO} bytes, which is all a WAV file holds"
            )));
        }
        Ok(())
    }
}

/// How a recording came to an end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// The person stopped it.
    Stopped,
    /// The disk had no room for more.
    LowSpace,
    /// It reached the size limit.
    Limit,
    /// The app stopped without closing it and the next start closed it.
    Recovered,
}

impl fmt::Display for EndReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EndReason::Stopped => "stopped",
            EndReason::LowSpace => "out of space",
            EndReason::Limit => "size limit reached",
            EndReason::Recovered => "recovered",
        })
    }
}

/// What became of a chunk when the recording was finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkState {
    /// Whole and checked.
    Ok,
    /// Whole, but the journal never heard of it: the app stopped between the two writes.
    Adopted,
    /// Damaged at the end and cut back to the whole samples that survived.
    Salvaged,
    /// Damaged or missing and replaced with silence of the same length.
    Silenced,
}

/// One chunk of the finished recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkInfo {
    pub index: u32,
    /// Seconds into the audio file.
    pub audio_start_secs: f64,
    /// Seconds from the moment the recording started, counting the time paused.
    pub wall_start_secs: f64,
    pub secs: f64,
    pub state: ChunkState,
}

/// A finished recording described.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub id: String,
    pub title: String,
    /// Unix seconds.
    pub started_at: i64,
    pub ended_at: i64,
    pub sample_rate: u32,
    /// Length of the audio file.
    pub audio_secs: f64,
    /// Time from start to end, counting the time paused.
    pub wall_secs: f64,
    pub ended: EndReason,
    /// Audio replaced with silence or dropped because it was damaged.
    pub lost_secs: f64,
    pub file: String,
    pub size_bytes: u64,
    pub fingerprint: u64,
    pub pauses: Vec<Pause>,
    pub chunks: Vec<ChunkInfo>,
}

impl Manifest {
    /// The recording as the timeline wants it, pauses included, so a photo or slide taken after a
    /// pause lands where it belongs in the audio. `path` is the audio file.
    pub fn timeline_recording(
        &self,
        path: &Path,
    ) -> std::result::Result<Recording, crate::timeline::TimelineError> {
        let mut recording = Recording::from_file(path, self.started_at, self.audio_secs as f32)?;
        recording.pauses = self.pauses.clone();
        Ok(recording)
    }

    /// Seconds into the audio at a Unix time and whether that moment fell in a pause or outside
    /// the recording and was moved to the nearest edge of it.
    pub fn audio_secs_at(&self, unix: i64) -> (f64, bool) {
        let wall = (unix - self.started_at) as f64;
        let mut paused = 0.0f64;
        for pause in &self.pauses {
            let at = pause.at_secs as f64;
            let length = pause.length_secs.max(0.0) as f64;
            if wall < at + paused {
                break;
            }
            if wall < at + paused + length {
                return (at, true);
            }
            paused += length;
        }
        let secs = wall - paused;
        let clamped = secs.clamp(0.0, self.audio_secs);
        (clamped, clamped != secs)
    }
}

/// How a finished recording is held in the library. The audio stays in the store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingEntry {
    pub id: String,
    pub title: String,
    pub started_at: i64,
    pub audio_secs: f64,
    pub ended: EndReason,
    pub lost_secs: f64,
    /// The audio file, relative to the store.
    pub file: String,
    /// The manifest, relative to the store.
    pub manifest: String,
    pub size_bytes: u64,
    pub fingerprint: u64,
}

/// Where the free space comes from. The platform decides how to ask, so the app supplies this.
pub trait FreeSpace: Send + Sync + fmt::Debug {
    /// Bytes free for a new file under `path`.
    fn available(&self, path: &Path) -> io::Result<u64>;
}

/// A fixed answer, for tests and for a platform that has its own check beforehand.
#[derive(Debug, Clone, Copy)]
pub struct FixedSpace(pub u64);

impl FreeSpace for FixedSpace {
    fn available(&self, _: &Path) -> io::Result<u64> {
        Ok(self.0)
    }
}

/// Asks `df`, which is there on Linux and macOS.
#[derive(Debug, Clone, Copy)]
pub struct DfSpace;

impl FreeSpace for DfSpace {
    fn available(&self, path: &Path) -> io::Result<u64> {
        let out = std::process::Command::new("df")
            .arg("-Pk")
            .arg(path)
            .output()?;
        parse_df(&String::from_utf8_lossy(&out.stdout))
            .ok_or_else(|| io::Error::other("df gave no free space"))
    }
}

/// The free bytes from the output of `df -Pk`, whose fourth column is available kibibytes.
fn parse_df(output: &str) -> Option<u64> {
    let line = output.lines().rfind(|l| !l.trim().is_empty())?;
    let kib: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// The file operations the recorder needs, so a test can fail them the way a full disk does.
pub(crate) trait Storage: Send + Sync + fmt::Debug {
    /// Writes a file whole: to a `.part` beside it, synced, then moved into place.
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// Appends one line to a file and syncs it. A torn last line is ended first so the new one
    /// stands alone.
    fn append_line(&self, path: &Path, line: &[u8]) -> io::Result<()>;
}

#[derive(Debug)]
pub(crate) struct RealDisk;

pub(crate) fn part_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

/// Syncs a folder so a file moved into it is not lost with the power. Not every platform can open
/// a folder, so a failure is ignored.
pub(crate) fn sync_dir(dir: &Path) {
    if let Ok(handle) = std::fs::File::open(dir) {
        let _ = handle.sync_all();
    }
}

impl Storage for RealDisk {
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        use std::io::Write;
        let part = part_path(path);
        let result = (|| {
            let mut file = std::fs::File::create(&part)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            std::fs::rename(&part, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&part);
            return result;
        }
        if let Some(parent) = path.parent() {
            sync_dir(parent);
        }
        Ok(())
    }

    fn append_line(&self, path: &Path, line: &[u8]) -> io::Result<()> {
        use std::io::{Read, Seek, SeekFrom, Write};
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)?;
        let length = file.metadata()?.len();
        let mut out = Vec::with_capacity(line.len() + 1);
        if length > 0 {
            let mut last = [0u8; 1];
            file.seek(SeekFrom::Start(length - 1))?;
            file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                out.push(b'\n');
            }
        }
        out.extend_from_slice(line);
        file.write_all(&out)?;
        file.sync_all()?;
        if length == 0
            && let Some(parent) = path.parent()
        {
            sync_dir(parent);
        }
        Ok(())
    }
}

/// FNV-1a, 64 bits. A guard against damage and not against anyone.
pub(crate) fn fnv64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        hash = (hash ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The 44-byte header of a mono 16-bit WAV file holding `frames` samples.
pub(crate) fn wav_header(frames: u64, rate: u32) -> [u8; WAV_HEADER] {
    let data = (frames * 2) as u32;
    let mut h = [0u8; WAV_HEADER];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes());
    h[22..24].copy_from_slice(&1u16.to_le_bytes());
    h[24..28].copy_from_slice(&rate.to_le_bytes());
    h[28..32].copy_from_slice(&(rate * 2).to_le_bytes());
    h[32..34].copy_from_slice(&2u16.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

/// The sample rate and the data length a header declares, if it is one of ours.
pub(crate) fn read_wav_header(bytes: &[u8]) -> Option<(u32, u64)> {
    if bytes.len() < WAV_HEADER
        || &bytes[0..4] != b"RIFF"
        || &bytes[8..16] != b"WAVEfmt "
        || &bytes[36..40] != b"data"
    {
        return None;
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at =
        |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    if u32_at(16) != 16 || u16_at(20) != 1 || u16_at(22) != 1 || u16_at(34) != 16 {
        return None;
    }
    let rate = u32_at(24);
    if rate == 0 {
        return None;
    }
    Some((rate, u32_at(40) as u64))
}

/// A sample from -1 to 1 as 16 bits. Anything else, including NaN, is held to the range.
pub(crate) fn quantise(sample: f32) -> i16 {
    if sample.is_nan() {
        return 0;
    }
    (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

#[cfg(test)]
mod tests;
