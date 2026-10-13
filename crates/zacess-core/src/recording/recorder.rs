//! The recorder: takes samples as they come and writes them as chunks.

use super::journal::{Entry, encode, hex};
use super::{
    EndReason, FreeSpace, RecordingConfig, RecordingError, Result, Storage, WAV_HEADER, finish,
    fnv64, quantise, wav_header,
};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Recording,
    Paused,
    /// The recording ended by itself, because the disk or the size limit ran out. Nothing more
    /// is kept. It still has to be stopped to be finished.
    Ended(EndReason),
}

/// How far a recording has got.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub status: Status,
    /// Seconds of audio, including what waits for the next chunk.
    pub audio_secs: f64,
    pub chunks: u32,
    /// Bytes of audio in chunks.
    pub bytes: u64,
}

enum Step {
    Written,
    Stop(EndReason),
}

fn is_full(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded
    )
}

/// A recording in progress. Made by [`super::RecordingStore::start`].
#[derive(Debug)]
pub struct Recorder {
    pub(crate) dir: PathBuf,
    pub(crate) sample_rate: u32,
    pub(crate) config: RecordingConfig,
    pub(crate) space: Arc<dyn FreeSpace>,
    pub(crate) storage: Arc<dyn Storage>,
    pub(crate) chunk_frames: usize,
    buffer: Vec<i16>,
    next_index: u32,
    frames_written: u64,
    status: Status,
    paused_since: Option<i64>,
    last_at: i64,
    closed: bool,
    finished: bool,
}

impl Recorder {
    pub(crate) fn new(
        dir: PathBuf,
        sample_rate: u32,
        started_at: i64,
        config: RecordingConfig,
        space: Arc<dyn FreeSpace>,
        storage: Arc<dyn Storage>,
    ) -> Self {
        let chunk_frames = ((config.chunk_secs as f64 * sample_rate as f64) as usize).max(1);
        Recorder {
            dir,
            sample_rate,
            config,
            space,
            storage,
            chunk_frames,
            buffer: Vec::new(),
            next_index: 1,
            frames_written: 0,
            status: Status::Recording,
            paused_since: None,
            last_at: started_at,
            closed: false,
            finished: false,
        }
    }

    pub fn id(&self) -> String {
        self.dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string()
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn progress(&self) -> Progress {
        Progress {
            status: self.status,
            audio_secs: (self.frames_written + self.buffer.len() as u64) as f64
                / self.sample_rate as f64,
            chunks: self.next_index - 1,
            bytes: self.frames_written * 2,
        }
    }

    /// Adds samples from -1 to 1. Returns the status: when the recording has ended by itself
    /// the samples are dropped and the status says why. Pushing while paused is an error, as the
    /// app should have stopped the microphone.
    pub fn push(&mut self, samples: &[f32]) -> Result<Status> {
        self.guard()?;
        for block in samples.chunks(self.chunk_frames) {
            let pcm: Vec<i16> = block.iter().map(|s| quantise(*s)).collect();
            if let Status::Ended(r) = self.feed(&pcm)? {
                return Ok(Status::Ended(r));
            }
        }
        Ok(self.status)
    }

    /// Adds 16-bit samples as the microphone gives them.
    pub fn push_pcm(&mut self, samples: &[i16]) -> Result<Status> {
        self.guard()?;
        for block in samples.chunks(self.chunk_frames) {
            if let Status::Ended(r) = self.feed(block)? {
                return Ok(Status::Ended(r));
            }
        }
        Ok(self.status)
    }

    fn guard(&self) -> Result<()> {
        if self.finished {
            return Err(RecordingError::State("the recording is finished"));
        }
        if self.status == Status::Paused {
            return Err(RecordingError::State("the recording is paused"));
        }
        Ok(())
    }

    fn feed(&mut self, block: &[i16]) -> Result<Status> {
        if let Status::Ended(r) = self.status {
            return Ok(Status::Ended(r));
        }
        self.buffer.extend_from_slice(block);
        while self.buffer.len() >= self.chunk_frames {
            if let Step::Stop(reason) = self.write_chunk(self.chunk_frames)? {
                self.end(reason);
                break;
            }
        }
        Ok(self.status)
    }

    /// Pauses at a Unix time. What waits for the next chunk is written first, so no chunk ever
    /// spans a pause.
    pub fn pause(&mut self, at: i64) -> Result<Status> {
        if self.finished || self.status != Status::Recording {
            return Err(RecordingError::State("only a recording can be paused"));
        }
        self.last_at = at;
        if let Step::Stop(reason) = self.flush()? {
            self.end(reason);
            return Ok(self.status);
        }
        self.storage.append_line(
            &self.dir.join("journal.log"),
            &encode(&Entry::Paused {
                at,
                frames: self.frames_written,
            }),
        )?;
        self.paused_since = Some(at);
        self.status = Status::Paused;
        Ok(self.status)
    }

    /// Resumes at a Unix time.
    pub fn resume(&mut self, at: i64) -> Result<Status> {
        if self.finished || self.status != Status::Paused {
            return Err(RecordingError::State("only a paused recording can resume"));
        }
        self.last_at = at;
        self.storage.append_line(
            &self.dir.join("journal.log"),
            &encode(&Entry::Resumed { at }),
        )?;
        self.paused_since = None;
        self.status = Status::Recording;
        Ok(self.status)
    }

    /// Stops the recording, writes what waits and finishes it: the chunks are joined into one
    /// file and a manifest is written. When there is not enough room to join them the error says
    /// so and the chunks are kept. Call this again once there is room.
    pub fn stop(&mut self, now: i64) -> Result<finish::Finished> {
        if self.finished {
            return Err(RecordingError::State("the recording is finished"));
        }
        self.last_at = now;
        if !self.closed {
            let mut reason = match self.status {
                Status::Ended(r) => r,
                _ => EndReason::Stopped,
            };
            if self.status == Status::Recording
                && let Step::Stop(r) = self.flush()?
            {
                reason = r;
            }
            self.buffer.clear();
            self.close(reason, true)?;
            self.status = Status::Ended(reason);
        }
        let done = finish::finalise(&self.dir, &*self.storage, &*self.space, now)?;
        self.finished = true;
        Ok(done)
    }

    fn flush(&mut self) -> Result<Step> {
        if self.buffer.is_empty() {
            return Ok(Step::Written);
        }
        self.write_chunk(self.buffer.len())
    }

    fn end(&mut self, reason: EndReason) {
        self.buffer.clear();
        self.status = Status::Ended(reason);
        // the journal may be the very thing that cannot be written and recovery closes it anyway
        let _ = self.close(reason, false);
    }

    fn close(&mut self, reason: EndReason, must: bool) -> Result<()> {
        let result = self.storage.append_line(
            &self.dir.join("journal.log"),
            &encode(&Entry::Closed {
                at: self.last_at,
                reason,
            }),
        );
        match result {
            Ok(()) => {
                self.closed = true;
                Ok(())
            }
            Err(e) if must => Err(e.into()),
            Err(_) => Ok(()),
        }
    }

    fn write_chunk(&mut self, frames: usize) -> Result<Step> {
        let fit = (self
            .config
            .max_bytes
            .saturating_sub(self.frames_written * 2)
            / 2) as usize;
        if fit < frames {
            if fit > 0 {
                self.store_chunk(fit)?;
            }
            return Ok(Step::Stop(EndReason::Limit));
        }
        self.store_chunk(frames)
    }

    fn store_chunk(&mut self, frames: usize) -> Result<Step> {
        let audio = frames as u64 * 2;
        let mut required = WAV_HEADER as u64 + audio + self.config.reserve_bytes;
        if self.config.keep_room_to_finish {
            required += self.frames_written * 2;
        }
        // a probe that cannot answer is no reason to stop recording
        if let Ok(free) = self.space.available(&self.dir)
            && free < required
        {
            return Ok(Step::Stop(EndReason::LowSpace));
        }
        let mut data = Vec::with_capacity(WAV_HEADER + audio as usize);
        data.extend_from_slice(&wav_header(frames as u64, self.sample_rate));
        for sample in &self.buffer[..frames] {
            data.extend_from_slice(&sample.to_le_bytes());
        }
        let index = self.next_index;
        let path = self.dir.join("chunks").join(format!("{index:06}.wav"));
        match self.storage.write_atomic(&path, &data) {
            Ok(()) => {}
            Err(e) if is_full(&e) => return Ok(Step::Stop(EndReason::LowSpace)),
            Err(e) => return Err(e.into()),
        }
        let entry = Entry::Chunk {
            index,
            frames: frames as u64,
            hash: hex(fnv64(&data)),
        };
        let journalled = self
            .storage
            .append_line(&self.dir.join("journal.log"), &encode(&entry));
        match journalled {
            Ok(()) => {}
            Err(e) if is_full(&e) => {
                // the chunk is on disk and recovery adopts it
                self.counted(frames);
                return Ok(Step::Stop(EndReason::LowSpace));
            }
            Err(e) => return Err(e.into()),
        }
        self.counted(frames);
        Ok(Step::Written)
    }

    fn counted(&mut self, frames: usize) {
        self.buffer.drain(..frames);
        self.next_index += 1;
        self.frames_written += frames as u64;
    }
}
