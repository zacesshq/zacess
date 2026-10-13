use super::journal::{self, Entry};
use super::*;
use crate::library::{ItemKind, Library};
use crate::speech::Audio;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use tempfile::TempDir;

const RATE: u32 = 8000;
const START: i64 = 1_760_000_000;

fn config(chunk_secs: f32) -> RecordingConfig {
    RecordingConfig {
        chunk_secs,
        reserve_bytes: 0,
        keep_room_to_finish: false,
        max_bytes: 1 << 30,
    }
}

fn store(dir: &TempDir, chunk_secs: f32) -> RecordingStore {
    RecordingStore::open(&dir.path().join("store"), config(chunk_secs))
        .unwrap()
        .with_space(FixedSpace(u64::MAX))
}

/// Samples that depend on where they fall, so a misplaced one is noticed.
fn pattern(from: u64, frames: usize) -> Vec<i16> {
    (0..frames as u64)
        .map(|i| (((from + i) % 251) as i16 - 125) * 100)
        .collect()
}

fn audio_of(store: &RecordingStore, id: &str) -> Vec<i16> {
    let bytes = std::fs::read(store.audio_path(id).unwrap()).unwrap();
    let (rate, declared) = read_wav_header(&bytes).unwrap();
    assert_eq!(rate, RATE);
    assert_eq!(
        declared as usize,
        bytes.len() - WAV_HEADER,
        "header is true"
    );
    bytes[WAV_HEADER..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect()
}

fn chunk(dir: &Path, index: u32) -> PathBuf {
    dir.join("chunks").join(format!("{index:06}.wav"))
}

/// Records `secs` of the pattern and drops the recorder without stopping it: the app was killed.
fn crash_after(store: &RecordingStore, secs: f64) -> (String, PathBuf) {
    let mut rec = store.start("Signals", START, RATE).unwrap();
    let frames = (secs * RATE as f64) as usize;
    let mut sent = 0;
    while sent < frames {
        let n = 777.min(frames - sent);
        rec.push_pcm(&pattern(sent as u64, n)).unwrap();
        sent += n;
    }
    let id = rec.id();
    let dir = store.root().join(&id);
    (id, dir)
}

#[test]
fn a_clean_recording_is_one_file_with_chunks_placed_on_its_clock() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let mut rec = store.start("Circuits 3", START, RATE).unwrap();
    let samples: Vec<f32> = pattern(0, 20_500)
        .iter()
        .map(|s| *s as f32 / 32767.0)
        .collect();
    assert_eq!(rec.push(&samples[..9_000]).unwrap(), Status::Recording);
    rec.push(&samples[9_000..]).unwrap();
    let progress = rec.progress();
    assert_eq!(progress.chunks, 2);
    assert!((progress.audio_secs - 20_500.0 / 8000.0).abs() < 1e-9);

    let done = rec.stop(START + 3).unwrap();
    assert!(done.report.is_clean(), "{:?}", done.report);
    assert_eq!(done.manifest.ended, EndReason::Stopped);
    assert_eq!(done.manifest.chunks.len(), 3);
    assert_eq!(done.manifest.chunks[2].secs, 4500.0 / 8000.0);
    assert_eq!(done.manifest.chunks[1].audio_start_secs, 1.0);
    assert!(
        done.manifest
            .chunks
            .iter()
            .all(|c| c.state == ChunkState::Ok)
    );

    let id = rec.id();
    let heard = audio_of(&store, &id);
    assert_eq!(heard.len(), 20_500);
    for (got, want) in heard.iter().zip(samples.iter()) {
        assert_eq!(*got, quantise(*want));
    }
    // the chunks are gone once the manifest is and the file opens like any WAV
    assert!(!tmp.path().join("store").join(&id).join("chunks").exists());
    let audio = Audio::from_wav(&store.audio_path(&id).unwrap()).unwrap();
    assert_eq!(audio.sample_rate, RATE);
    assert_eq!(audio.samples.len(), 20_500);
    assert!(store.unfinished().unwrap().is_empty());
    assert!(matches!(
        store.recover(&id, 0),
        Err(RecordingError::Finished(_))
    ));
}

#[test]
fn a_crash_in_the_middle_of_a_chunk_loses_only_that_chunk() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let (id, dir) = crash_after(&store, 2.5);
    // the chunk being written when the app died: a .part with half its bytes
    let half = std::fs::read(chunk(&dir, 2)).unwrap();
    std::fs::write(
        chunk(&dir, 3).with_extension("wav.part"),
        &half[..half.len() / 2],
    )
    .unwrap();

    assert_eq!(store.unfinished().unwrap(), [id.as_str()]);
    let done = store.recover(&id, START + 99).unwrap();
    assert_eq!(done.report.ended, EndReason::Recovered);
    assert_eq!(done.manifest.audio_secs, 2.0);
    assert_eq!(done.report.dropped_chunks, 0);
    assert_eq!(audio_of(&store, &id), pattern(0, 16_000));
    assert!(store.unfinished().unwrap().is_empty());
    assert!(
        !dir.join("chunks").exists(),
        "the half-written file went too"
    );
}

#[test]
fn a_chunk_written_but_never_journalled_is_adopted() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let (id, dir) = crash_after(&store, 2.5);
    // the app died between the chunk and its journal line
    let log = std::fs::read_to_string(dir.join("journal.log")).unwrap();
    let kept: Vec<&str> = log.lines().collect();
    std::fs::write(
        dir.join("journal.log"),
        kept[..kept.len() - 1].join("\n") + "\n",
    )
    .unwrap();
    let done = store.recover(&id, START).unwrap();
    assert_eq!(done.report.adopted_chunks, 1);
    assert_eq!(done.manifest.chunks[1].state, ChunkState::Adopted);
    assert_eq!(audio_of(&store, &id), pattern(0, 16_000));
}

#[test]
fn a_truncated_last_chunk_is_cut_back_to_the_samples_that_survived() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let (id, dir) = crash_after(&store, 3.5);
    let path = chunk(&dir, 3);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..WAV_HEADER + 5_001]).unwrap();
    let done = store.recover(&id, START).unwrap();
    assert_eq!(done.report.salvaged_chunks, 1);
    assert_eq!(done.manifest.chunks[2].state, ChunkState::Salvaged);
    // an odd byte is half a sample and goes
    assert_eq!(audio_of(&store, &id), pattern(0, 16_000 + 2_500));
    assert!((done.report.lost_secs - 5_500.0 / 8000.0).abs() < 1e-9);
}

#[test]
fn a_garbled_last_chunk_is_dropped_and_what_came_before_is_kept() {
    let tmp = TempDir::new().unwrap();
    for damage in ["garbage", "flipped", "empty", "missing"] {
        let store = store(&tmp, 1.0);
        let (id, dir) = crash_after(&store, 3.5);
        let path = chunk(&dir, 3);
        match damage {
            "garbage" => std::fs::write(&path, vec![0x5au8; 16_044]).unwrap(),
            "flipped" => {
                let mut bytes = std::fs::read(&path).unwrap();
                bytes[9_000] ^= 0x40;
                std::fs::write(&path, bytes).unwrap();
            }
            "empty" => std::fs::write(&path, b"").unwrap(),
            _ => std::fs::remove_file(&path).unwrap(),
        }
        let done = store.recover(&id, START).unwrap();
        assert_eq!(done.report.dropped_chunks, 1, "{damage}");
        assert_eq!(done.report.silenced_chunks, 0, "{damage}");
        assert_eq!(audio_of(&store, &id), pattern(0, 16_000), "{damage}");
        assert_eq!(done.report.lost_secs, 1.0, "{damage}");
    }
}

#[test]
fn a_damaged_chunk_in_the_middle_becomes_silence_and_the_rest_keeps_its_time() {
    let tmp = TempDir::new().unwrap();
    for damage in ["flipped", "missing", "truncated"] {
        let store = store(&tmp, 1.0);
        let (id, dir) = crash_after(&store, 4.5);
        let path = chunk(&dir, 2);
        match damage {
            "flipped" => {
                let mut bytes = std::fs::read(&path).unwrap();
                bytes[400] ^= 0xff;
                std::fs::write(&path, bytes).unwrap();
            }
            "truncated" => {
                let bytes = std::fs::read(&path).unwrap();
                std::fs::write(&path, &bytes[..2_000]).unwrap();
            }
            _ => std::fs::remove_file(&path).unwrap(),
        }
        let done = store.recover(&id, START).unwrap();
        assert_eq!(done.report.silenced_chunks, 1, "{damage}");
        assert_eq!(done.manifest.chunks[1].state, ChunkState::Silenced);
        assert_eq!(done.manifest.audio_secs, 4.0, "{damage}");
        let heard = audio_of(&store, &id);
        assert_eq!(heard.len(), 32_000);
        assert_eq!(&heard[..8_000], &pattern(0, 8_000)[..], "{damage}");
        assert!(heard[8_000..16_000].iter().all(|s| *s == 0), "{damage}");
        assert_eq!(&heard[16_000..], &pattern(16_000, 16_000)[..], "{damage}");
        assert_eq!(done.report.lost_secs, 1.0);
        // the good chunks are cleared away and the damaged one is kept, as it may still play
        let folder = store.root().join(&id);
        assert!(!folder.join("chunks").exists(), "{damage}");
        assert_eq!(
            folder.join("damaged").exists(),
            damage != "missing",
            "{damage}"
        );
        std::fs::remove_dir_all(store.root()).unwrap();
    }
}

#[test]
fn a_torn_or_damaged_journal_line_costs_only_itself() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let (id, dir) = crash_after(&store, 2.5);
    // torn: the last line ends half way
    let log = std::fs::read(dir.join("journal.log")).unwrap();
    std::fs::write(dir.join("journal.log"), &log[..log.len() - 9]).unwrap();
    let done = store.recover(&id, START).unwrap();
    assert_eq!(done.report.damaged_journal_lines, 1);
    assert_eq!(done.report.adopted_chunks, 1);
    assert_eq!(audio_of(&store, &id), pattern(0, 16_000));
    // the close was written on a line of its own, not onto the torn one
    let replay = journal::replay(&dir.join("journal.log")).unwrap();
    assert_eq!(replay.closed, Some((START, EndReason::Recovered)));

    // damaged in the middle: a character changed in the first chunk's line
    let (id, dir) = crash_after(&store, 2.5);
    let mut log = std::fs::read(dir.join("journal.log")).unwrap();
    let at = log.windows(7).position(|w| w == b"\"chunk\"").unwrap();
    log[at + 2] = b'X';
    std::fs::write(dir.join("journal.log"), log).unwrap();
    let done = store.recover(&id, START).unwrap();
    assert_eq!(done.report.damaged_journal_lines, 1);
    assert_eq!(audio_of(&store, &id), pattern(0, 16_000));
}

#[test]
fn a_journal_with_nothing_readable_still_recovers_from_the_chunks() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let (id, dir) = crash_after(&store, 2.5);
    std::fs::write(dir.join("journal.log"), b"\0\0\0 garbage").unwrap();
    let done = store.recover(&id, START).unwrap();
    assert_eq!(done.report.adopted_chunks, 2);
    assert_eq!(done.manifest.sample_rate, RATE);
    assert_eq!(
        done.manifest.started_at, START,
        "taken from the folder name"
    );
    assert_eq!(audio_of(&store, &id), pattern(0, 16_000));
}

#[test]
fn a_recording_with_no_audio_is_not_made_up_and_can_be_discarded() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let mut rec = store.start("Nothing", START, RATE).unwrap();
    let id = rec.id();
    assert!(matches!(rec.stop(START), Err(RecordingError::Empty(_))));
    // nothing was lost, so the empty folder goes
    assert!(store.unfinished().unwrap().is_empty());
    assert!(!store.root().join(&id).exists());

    // damaged files are not for the app to throw away: the folder stays until it is discarded
    let (id, dir) = crash_after(&store, 1.5);
    std::fs::write(chunk(&dir, 1), vec![7u8; 500]).unwrap();
    assert!(matches!(
        store.recover(&id, START),
        Err(RecordingError::Empty(_))
    ));
    assert_eq!(store.unfinished().unwrap(), [id.as_str()]);
    store.discard(&id).unwrap();
    assert!(store.unfinished().unwrap().is_empty());
    assert!(matches!(
        store.discard("../etc"),
        Err(RecordingError::Unknown(_))
    ));
}

#[test]
fn pausing_writes_what_waits_and_resuming_starts_a_new_chunk() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let mut rec = store.start("Pauses", START, RATE).unwrap();
    rec.push_pcm(&pattern(0, 12_000)).unwrap();
    assert_eq!(rec.pause(START + 2).unwrap(), Status::Paused);
    assert!(
        rec.push_pcm(&[1, 2, 3]).is_err(),
        "nothing is taken while paused"
    );
    assert!(rec.pause(START + 3).is_err());
    rec.resume(START + 62).unwrap();
    assert!(rec.resume(START + 63).is_err());
    rec.push_pcm(&pattern(12_000, 9_000)).unwrap();
    rec.pause(START + 64).unwrap();
    rec.resume(START + 64).unwrap();
    let done = rec.stop(START + 70).unwrap();
    let m = &done.manifest;
    // chunk 1 (1 s), chunk 2 (the 0.5 s waiting at the pause), chunk 3 and 4 after it
    let secs: Vec<f64> = m.chunks.iter().map(|c| c.secs).collect();
    assert_eq!(secs, [1.0, 0.5, 1.0, 0.125]);
    assert_eq!(m.chunks[2].audio_start_secs, 1.5);
    assert_eq!(
        m.chunks[2].wall_start_secs, 61.5,
        "the minute paused is counted"
    );
    assert_eq!(m.chunks[3].wall_start_secs, 62.5);
    assert_eq!(m.pauses.len(), 2);
    assert_eq!(m.pauses[0].at_secs, 1.5);
    assert_eq!(m.pauses[0].length_secs, 60.0);
    assert_eq!(m.audio_secs, 21_000.0 / 8000.0);
    assert_eq!(m.wall_secs, m.audio_secs + 60.0);
    assert_eq!(audio_of(&store, &rec.id()), pattern(0, 21_000));
}

#[test]
fn a_crash_while_paused_drops_that_pause_and_keeps_the_audio() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let mut rec = store.start("Paused", START, RATE).unwrap();
    rec.push_pcm(&pattern(0, 12_000)).unwrap();
    rec.pause(START + 2).unwrap();
    let id = rec.id();
    drop(rec);
    let done = store.recover(&id, START + 1000).unwrap();
    assert!(done.report.dropped_pause);
    assert!(done.manifest.pauses.is_empty());
    assert_eq!(
        done.manifest.audio_secs, 1.5,
        "what was waiting was written at the pause"
    );
}

#[test]
fn a_thousand_pauses_and_resumes_keep_every_chunk_on_the_clock() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 10.0);
    let mut rec = store.start("Interrupted", START, RATE).unwrap();
    let mut wall = START;
    for cycle in 0..1000u64 {
        rec.push_pcm(&pattern(cycle * 80, 80)).unwrap();
        rec.pause(wall).unwrap();
        wall += 1 + (cycle % 7) as i64;
        rec.resume(wall).unwrap();
    }
    rec.push_pcm(&pattern(80_000, 4_000)).unwrap();
    let done = rec.stop(wall + 1).unwrap();
    let m = &done.manifest;
    assert_eq!(m.pauses.len(), 1000);
    assert_eq!(m.chunks.len(), 1001);
    let paused: f64 = (0..1000).map(|c| 1.0 + (c % 7) as f64).sum();
    assert_eq!(m.wall_secs, m.audio_secs + paused);
    let mut running = 0.0;
    for (i, c) in m.chunks.iter().enumerate() {
        assert_eq!(c.wall_start_secs, c.audio_start_secs + running, "chunk {i}");
        if i < 1000 {
            running += 1.0 + (i % 7) as f64;
        }
    }
    assert_eq!(audio_of(&store, &rec.id()), pattern(0, 84_000));
    assert!(done.report.is_clean());
}

#[test]
fn photos_and_slides_after_a_pause_land_where_they_belong_in_the_audio() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let mut rec = store.start("Lecture", START, RATE).unwrap();
    rec.push_pcm(&pattern(0, 80_000)).unwrap(); // 10 s
    rec.pause(START + 10).unwrap();
    rec.resume(START + 610).unwrap(); // a ten minute break
    rec.push_pcm(&pattern(80_000, 80_000)).unwrap();
    let done = rec.stop(START + 620).unwrap();
    let m = &done.manifest;
    assert_eq!(m.audio_secs_at(START + 4), (4.0, false));
    assert_eq!(m.audio_secs_at(START + 300), (10.0, true), "in the break");
    assert_eq!(m.audio_secs_at(START + 615), (15.0, false));
    assert_eq!(
        m.audio_secs_at(START + 9_999),
        (20.0, true),
        "after the end"
    );

    let path = store.audio_path(&rec.id()).unwrap();
    let recording = m.timeline_recording(&path).unwrap();
    assert_eq!(recording.pauses.len(), 1);
    assert_eq!(recording.duration_secs, 20.0);
    assert_eq!(recording.size_bytes, Some(m.size_bytes));
    assert_eq!(recording.fingerprint, Some(m.fingerprint));
}

#[derive(Debug)]
struct Quota {
    capacity: u64,
    root: PathBuf,
}

fn used(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| {
                    let path = e.path();
                    if path.is_dir() {
                        used(&path)
                    } else {
                        e.metadata().map(|m| m.len()).unwrap_or(0)
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

impl FreeSpace for Quota {
    fn available(&self, _: &Path) -> io::Result<u64> {
        Ok(self.capacity.saturating_sub(used(&self.root)))
    }
}

fn small_disk(tmp: &TempDir, capacity: u64, keep_room: bool) -> RecordingStore {
    let root = tmp.path().join("store");
    RecordingStore::open(
        &root,
        RecordingConfig {
            keep_room_to_finish: keep_room,
            ..config(1.0)
        },
    )
    .unwrap()
    .with_space(Quota { capacity, root })
}

#[test]
fn a_recording_stops_cleanly_before_the_disk_fills_and_is_kept() {
    let tmp = TempDir::new().unwrap();
    // room for two and a bit chunks of 16 044 bytes once the chunks are joined too
    let store = small_disk(&tmp, 100_000, true);
    let mut rec = store.start("Full", START, RATE).unwrap();
    let mut status = Status::Recording;
    for second in 0..30u64 {
        status = rec.push_pcm(&pattern(second * 8_000, 8_000)).unwrap();
        if status != Status::Recording {
            break;
        }
    }
    let Status::Ended(EndReason::LowSpace) = status else {
        panic!("{status:?}");
    };
    let kept = rec.progress().chunks;
    assert!((2..=3).contains(&kept), "{kept}");
    // later samples are dropped without an error
    assert_eq!(
        rec.push_pcm(&pattern(0, 8_000)).unwrap(),
        Status::Ended(EndReason::LowSpace)
    );
    assert_eq!(rec.progress().chunks, kept);
    // and because room was held back the chunks can still be joined
    let done = rec.stop(START + 40).unwrap();
    assert_eq!(done.manifest.ended, EndReason::LowSpace);
    assert_eq!(done.manifest.audio_secs, kept as f64);
    assert_eq!(
        audio_of(&store, &rec.id()),
        pattern(0, kept as usize * 8_000)
    );
    assert!(done.report.is_clean() || done.report.lost_secs == 0.0);
}

#[test]
fn without_room_held_back_the_chunks_are_kept_until_there_is_room_to_join_them() {
    let tmp = TempDir::new().unwrap();
    let store = small_disk(&tmp, 40_000, false);
    let mut rec = store.start("Tight", START, RATE).unwrap();
    for second in 0..30u64 {
        if rec.push_pcm(&pattern(second * 8_000, 8_000)).unwrap() != Status::Recording {
            break;
        }
    }
    assert_eq!(rec.status(), Status::Ended(EndReason::LowSpace));
    match rec.stop(START + 40) {
        Err(RecordingError::NoRoomToFinish { needed, available }) => {
            assert!(available < needed);
        }
        other => panic!("{other:?}"),
    }
    // nothing was lost: the recording is still there to finish
    let id = rec.id();
    assert_eq!(store.unfinished().unwrap(), [id.as_str()]);
    let roomy = RecordingStore::open(store.root(), config(1.0))
        .unwrap()
        .with_space(FixedSpace(u64::MAX));
    let done = roomy.recover(&id, START + 50).unwrap();
    assert_eq!(
        done.manifest.ended,
        EndReason::LowSpace,
        "it still says why it ended"
    );
    assert_eq!(
        audio_of(&roomy, &id),
        pattern(0, done.manifest.audio_secs as usize * 8_000)
    );
}

#[test]
fn space_is_checked_before_every_chunk_and_a_probe_that_fails_does_not_stop_the_recording() {
    #[derive(Debug)]
    struct Shrinking(Arc<AtomicU64>);
    impl FreeSpace for Shrinking {
        fn available(&self, _: &Path) -> io::Result<u64> {
            Ok(self.0.load(Ordering::SeqCst))
        }
    }
    #[derive(Debug)]
    struct Broken;
    impl FreeSpace for Broken {
        fn available(&self, _: &Path) -> io::Result<u64> {
            Err(io::Error::other("no answer"))
        }
    }
    let tmp = TempDir::new().unwrap();
    let free = Arc::new(AtomicU64::new(10_000_000));
    let store = RecordingStore::open(&tmp.path().join("a"), config(1.0))
        .unwrap()
        .with_space(Shrinking(free.clone()));
    let mut rec = store.start("Shrink", START, RATE).unwrap();
    rec.push_pcm(&pattern(0, 8_000)).unwrap();
    assert_eq!(rec.progress().chunks, 1);
    free.store(1_000, Ordering::SeqCst);
    assert_eq!(
        rec.push_pcm(&pattern(8_000, 8_000)).unwrap(),
        Status::Ended(EndReason::LowSpace)
    );
    assert_eq!(rec.progress().chunks, 1);

    let store = RecordingStore::open(&tmp.path().join("b"), config(1.0))
        .unwrap()
        .with_space(Broken);
    let mut rec = store.start("Blind", START, RATE).unwrap();
    rec.push_pcm(&pattern(0, 16_000)).unwrap();
    assert_eq!(rec.progress().chunks, 2);
}

#[derive(Debug)]
struct FailingDisk {
    chunks_before_full: u32,
    seen: AtomicU32,
}

impl Storage for FailingDisk {
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if path.extension().is_some_and(|e| e == "wav")
            && self.seen.fetch_add(1, Ordering::SeqCst) >= self.chunks_before_full
        {
            // the disk fills part way through the write and leaves the half file behind
            std::fs::write(part_path(path), &bytes[..bytes.len() / 3])?;
            return Err(io::Error::from(io::ErrorKind::StorageFull));
        }
        RealDisk.write_atomic(path, bytes)
    }

    fn append_line(&self, path: &Path, line: &[u8]) -> io::Result<()> {
        RealDisk.append_line(path, line)
    }
}

#[test]
fn a_write_that_fails_because_the_disk_is_full_ends_the_recording_the_same_way() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0).with_storage(FailingDisk {
        chunks_before_full: 2,
        seen: AtomicU32::new(0),
    });
    let mut rec = store.start("Full mid write", START, RATE).unwrap();
    let status = rec.push_pcm(&pattern(0, 40_000)).unwrap();
    assert_eq!(status, Status::Ended(EndReason::LowSpace));
    assert_eq!(rec.progress().chunks, 2);
    let done = rec.stop(START + 9).unwrap();
    assert_eq!(done.manifest.ended, EndReason::LowSpace);
    assert_eq!(audio_of(&store, &rec.id()), pattern(0, 16_000));
}

#[test]
fn the_size_limit_ends_a_recording_at_a_whole_sample() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("store");
    let store = RecordingStore::open(
        &root,
        RecordingConfig {
            max_bytes: 100_001,
            ..config(1.0)
        },
    )
    .unwrap()
    .with_space(FixedSpace(u64::MAX));
    let mut rec = store.start("Long", START, RATE).unwrap();
    let status = rec.push_pcm(&pattern(0, 80_000)).unwrap();
    assert_eq!(status, Status::Ended(EndReason::Limit));
    let done = rec.stop(START + 20).unwrap();
    assert_eq!(done.manifest.ended, EndReason::Limit);
    assert_eq!(audio_of(&store, &rec.id()).len(), 50_000);
}

#[test]
fn a_recording_can_only_be_used_in_its_own_order() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let mut rec = store.start("Order", START, RATE).unwrap();
    assert!(rec.resume(START).is_err());
    rec.push_pcm(&pattern(0, 9_000)).unwrap();
    rec.stop(START + 1).unwrap();
    assert!(rec.push_pcm(&[1]).is_err());
    assert!(rec.pause(START).is_err());
    assert!(rec.stop(START).is_err());
    // a second recording at the same second gets a folder of its own
    let again = store.start("Same second", START, RATE).unwrap();
    assert_ne!(again.id(), rec.id());
    assert!(store.start("Bad rate", START, 100).is_err());
}

#[test]
fn settings_that_cannot_work_are_refused() {
    let tmp = TempDir::new().unwrap();
    for bad in [
        RecordingConfig {
            chunk_secs: 0.0,
            ..config(1.0)
        },
        RecordingConfig {
            chunk_secs: 600.0,
            ..config(1.0)
        },
        RecordingConfig {
            chunk_secs: f32::NAN,
            ..config(1.0)
        },
        RecordingConfig {
            max_bytes: 10,
            ..config(1.0)
        },
        RecordingConfig {
            max_bytes: u64::MAX,
            ..config(1.0)
        },
    ] {
        assert!(RecordingStore::open(&tmp.path().join("x"), bad).is_err());
    }
    assert!(RecordingConfig::default().validate().is_ok());
}

#[test]
fn df_output_is_read_and_a_wav_header_survives_a_round_trip() {
    let out = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/vda 264212084 19456568 21387044 48% /\n";
    assert_eq!(parse_df(out), Some(21_387_044 * 1024));
    assert_eq!(parse_df(""), None);
    assert_eq!(parse_df("Filesystem\nx y z"), None);
    assert!(DfSpace.available(Path::new(".")).is_ok());

    let header = wav_header(1234, 44_100);
    assert_eq!(read_wav_header(&header), Some((44_100, 2468)));
    assert_eq!(read_wav_header(&header[..40]), None);
    let mut other = header;
    other[22] = 2;
    assert_eq!(read_wav_header(&other), None, "stereo is not ours");
    assert_eq!(quantise(f32::NAN), 0);
    assert_eq!(quantise(5.0), 32767);
    assert_eq!(quantise(-5.0), -32767);
}

#[test]
fn a_finished_recording_is_stored_in_the_library_once() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let mut rec = store.start("Signals 4", START, RATE).unwrap();
    rec.push_pcm(&pattern(0, 20_000)).unwrap();
    let done = rec.stop(START + 5).unwrap();
    let id = rec.id();

    let mut library = Library::open_in_memory().unwrap();
    let item = store.store_in_library(&mut library, &id).unwrap();
    let info = library.item(&item).unwrap();
    assert_eq!(info.kind, ItemKind::Recording);
    assert_eq!(info.title, "Signals 4");
    let entry = library.recording(&item).unwrap();
    assert_eq!(entry.audio_secs, 2.5);
    assert_eq!(entry.ended, EndReason::Stopped);
    assert_eq!(entry.file, format!("{id}/recording.wav"));
    assert_eq!(entry.fingerprint, done.manifest.fingerprint);
    assert!(tmp.path().join("store").join(&entry.file).exists());

    // storing again finds the same item
    assert_eq!(store.store_in_library(&mut library, &id).unwrap(), item);
    assert_eq!(library.items(Some(ItemKind::Recording)).unwrap().len(), 1);
    assert_eq!(library.item(&item).unwrap().current_version, 1);
    // an unfinished recording is not stored
    let open = store.start("Open", START + 1, RATE).unwrap();
    assert!(store.store_in_library(&mut library, &open.id()).is_err());
    // the other kinds refuse a recording and the reverse
    assert!(library.note(&item).is_err());

    // it survives an export and an import
    let out = tmp.path().join("export");
    library.export_all(&out).unwrap();
    let mut other = Library::open_in_memory().unwrap();
    assert_eq!(other.import_all(&out).unwrap(), 1);
    assert_eq!(other.recording(&item).unwrap(), entry);
}

#[test]
fn a_version_one_library_is_carried_over_with_everything_in_it() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("old.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE items (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL CHECK (kind IN ('document', 'note', 'profile')),
                title TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                deleted_at INTEGER, current_version INTEGER NOT NULL);
             CREATE TABLE versions (
                item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
                number INTEGER NOT NULL, saved_at INTEGER NOT NULL,
                content_hash TEXT NOT NULL, content TEXT NOT NULL,
                PRIMARY KEY (item_id, number));
             CREATE INDEX items_by_kind ON items(kind, deleted_at, updated_at);
             INSERT INTO meta VALUES ('schema_version', '1');
             INSERT INTO items VALUES ('n-1', 'note', 'Old note', 5, 6, NULL, 2);
             INSERT INTO versions VALUES ('n-1', 1, 5, 'h1', '{\"title\":\"Old note\",\"text\":\"one\"}');
             INSERT INTO versions VALUES ('n-1', 2, 6, 'h2', '{\"title\":\"Old note\",\"text\":\"two\"}');",
        )
        .unwrap();
    }
    let mut library = Library::open(&path).unwrap();
    let notes = library.items(Some(ItemKind::Note)).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(library.note(&notes[0].id).unwrap().text, "two");
    assert_eq!(
        library.versions(&notes[0].id).unwrap().len(),
        2,
        "history kept"
    );
    let entry = RecordingEntry {
        id: "rec-1".into(),
        title: "New".into(),
        started_at: 1,
        audio_secs: 1.0,
        ended: EndReason::Stopped,
        lost_secs: 0.0,
        file: "rec-1/recording.wav".into(),
        manifest: "rec-1/manifest.json".into(),
        size_bytes: 1,
        fingerprint: 1,
    };
    library.create_recording(&entry).unwrap();
    drop(library);
    // and a second open reads it as current
    let library = Library::open(&path).unwrap();
    assert_eq!(library.items(None).unwrap().len(), 2);
}

#[test]
fn a_recording_of_more_than_three_hours_survives_a_crash_and_is_recovered_in_place() {
    let tmp = TempDir::new().unwrap();
    // minute-long chunks keep the number of files down, as a long lecture would
    let store = store(&tmp, 60.0);
    let mut rec = store.start("Long lecture", START, RATE).unwrap();
    let frames_per_minute = 60 * RATE as usize;
    let minutes = 193u64; // 3 h 13 min
    for minute in 0..minutes {
        let status = rec
            .push_pcm(&pattern(
                minute * frames_per_minute as u64,
                frames_per_minute,
            ))
            .unwrap();
        assert_eq!(status, Status::Recording);
        // a short break at the end of the first hour and another at the end of the second
        if minute == 59 || minute == 119 {
            rec.pause(START + (minute as i64 + 1) * 60).unwrap();
            rec.resume(START + (minute as i64 + 1) * 60 + 600).unwrap();
        }
    }
    // 40 seconds more that never reach a chunk, then the battery dies
    rec.push_pcm(&pattern(
        minutes * frames_per_minute as u64,
        40 * RATE as usize,
    ))
    .unwrap();
    let id = rec.id();
    assert_eq!(rec.progress().chunks as u64, minutes);
    drop(rec);

    assert_eq!(store.unfinished().unwrap(), [id.as_str()]);
    let done = store.recover(&id, START + 20_000).unwrap();
    let m = &done.manifest;
    assert_eq!(m.audio_secs, (minutes * 60) as f64);
    assert!(m.audio_secs > 3.0 * 3600.0);
    assert_eq!(m.wall_secs, m.audio_secs + 1200.0);
    assert_eq!(m.chunks.len(), minutes as usize);
    assert_eq!(m.pauses.len(), 2);
    assert_eq!(done.report.lost_secs, 0.0);
    assert_eq!(m.size_bytes, 44 + minutes * frames_per_minute as u64 * 2);

    // spot checks across the file: the start, the middle of an hour and the very end
    let path = store.audio_path(&id).unwrap();
    let file = std::fs::File::open(&path).unwrap();
    use std::os::unix::fs::FileExt;
    for frame in [
        0u64,
        3_600 * 8_000 + 12_345,
        7_200 * 8_000 - 1,
        minutes * 60 * 8_000 - 3,
    ] {
        let mut two = [0u8; 2];
        file.read_exact_at(&mut two, 44 + frame * 2).unwrap();
        assert_eq!(
            i16::from_le_bytes(two),
            pattern(frame, 1)[0],
            "frame {frame}"
        );
    }
    // and the timeline takes it
    let recording = m.timeline_recording(&path).unwrap();
    assert!(recording.duration_secs >= 11_580.0);
    assert_eq!(m.audio_secs_at(START + 3_600 + 300).0, 3_600.0);
}

#[test]
fn entries_are_written_one_to_a_line_with_their_own_checksum() {
    let line = journal::encode(&Entry::Resumed { at: 7 });
    assert!(line.ends_with(b"\n"));
    let text = String::from_utf8(line).unwrap();
    let (sum, json) = text.trim_end().split_once(' ').unwrap();
    assert_eq!(sum.len(), 16);
    assert_eq!(sum, journal::hex(fnv64(json.as_bytes())));
    assert!(json.contains("\"event\":\"resumed\""));
}

#[test]
fn leftovers_from_a_stopped_finish_are_cleared_away_on_the_next_start() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let mut rec = store.start("Leftovers", START, RATE).unwrap();
    rec.push_pcm(&pattern(0, 17_000)).unwrap();
    let id = rec.id();
    let folder = store.root().join(&id);
    let held = std::fs::read(chunk(&folder, 1)).unwrap();
    rec.stop(START + 3).unwrap();
    // the app stopped after the manifest and before the chunks were removed
    std::fs::create_dir(folder.join("chunks")).unwrap();
    std::fs::write(chunk(&folder, 1), held).unwrap();
    std::fs::write(chunk(&folder, 9).with_extension("wav.part"), b"half").unwrap();
    assert!(store.unfinished().unwrap().is_empty());
    assert!(!folder.join("chunks").exists());
    assert!(store.manifest(&id).is_ok());
}

#[test]
fn a_finish_that_was_cut_short_is_not_held_up_by_its_own_leftovers() {
    let tmp = TempDir::new().unwrap();
    // 70 000 bytes: the chunks and the joined file fit together only with the room held back
    for leftover in ["part", "whole"] {
        let store = small_disk(&tmp, 70_000, true);
        let mut rec = store.start("Stale", START, RATE).unwrap();
        for second in 0..30u64 {
            if rec.push_pcm(&pattern(second * 8_000, 8_000)).unwrap() != Status::Recording {
                break;
            }
        }
        let id = rec.id();
        let folder = store.root().join(&id);
        let audio = rec.progress().bytes;
        drop(rec);
        match leftover {
            "part" => std::fs::write(folder.join("recording.wav.part"), vec![0u8; 25_000]).unwrap(),
            _ => std::fs::write(folder.join("recording.wav"), vec![0u8; audio as usize + 44])
                .unwrap(),
        }
        let done = store.recover(&id, START + 9).unwrap();
        assert_eq!(
            done.manifest.audio_secs,
            audio as f64 / 16_000.0,
            "{leftover}"
        );
        assert_eq!(
            audio_of(&store, &id),
            pattern(0, audio as usize / 2),
            "{leftover}"
        );
        std::fs::remove_dir_all(store.root()).unwrap();
    }
}

#[test]
fn a_damaged_resume_line_costs_only_that_pause() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let mut rec = store.start("Breaks", START, RATE).unwrap();
    rec.push_pcm(&pattern(0, 8_000)).unwrap();
    rec.pause(START + 1).unwrap();
    rec.resume(START + 101).unwrap();
    rec.push_pcm(&pattern(8_000, 16_000)).unwrap();
    rec.pause(START + 103).unwrap();
    rec.resume(START + 163).unwrap();
    rec.push_pcm(&pattern(24_000, 8_000)).unwrap();
    let id = rec.id();
    let dir = store.root().join(&id);
    drop(rec);
    let mut log = std::fs::read(dir.join("journal.log")).unwrap();
    let at = log.windows(9).position(|w| w == b"\"resumed\"").unwrap();
    log[at + 2] = b'X';
    std::fs::write(dir.join("journal.log"), log).unwrap();
    let done = store.recover(&id, START + 500).unwrap();
    let pauses = &done.manifest.pauses;
    assert_eq!(pauses.len(), 2);
    assert_eq!(
        pauses[0].length_secs, 0.0,
        "its length was lost with the line"
    );
    assert_eq!(pauses[1].at_secs, 3.0);
    assert_eq!(pauses[1].length_secs, 60.0);
    assert_eq!(done.report.damaged_journal_lines, 1);
}

#[test]
fn stopping_while_paused_keeps_the_pause() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let mut rec = store.start("Ended on a break", START, RATE).unwrap();
    rec.push_pcm(&pattern(0, 12_000)).unwrap();
    rec.pause(START + 2).unwrap();
    let done = rec.stop(START + 1_202).unwrap();
    assert!(!done.report.dropped_pause);
    assert_eq!(done.manifest.pauses.len(), 1);
    assert_eq!(done.manifest.pauses[0].length_secs, 1_200.0);
    assert_eq!(done.manifest.wall_secs, 1.5 + 1_200.0);
    assert_eq!(done.manifest.ended_at, START + 1_202);
}

#[test]
fn an_unjournalled_damaged_chunk_in_the_middle_still_holds_its_place() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let (id, dir) = crash_after(&store, 4.5);
    // chunk 2 is damaged and its journal line is gone too, so its length is not on record
    let log = std::fs::read_to_string(dir.join("journal.log")).unwrap();
    let kept: Vec<&str> = log.lines().filter(|l| !l.contains("\"index\":2")).collect();
    assert_eq!(kept.len(), log.lines().count() - 1);
    std::fs::write(dir.join("journal.log"), kept.join("\n") + "\n").unwrap();
    std::fs::write(chunk(&dir, 2), vec![3u8; 90]).unwrap();
    let done = store.recover(&id, START).unwrap();
    assert_eq!(done.report.silenced_chunks, 1);
    let heard = audio_of(&store, &id);
    assert_eq!(heard.len(), 32_000);
    assert_eq!(
        &heard[16_000..],
        &pattern(16_000, 16_000)[..],
        "the rest kept its time"
    );
}

#[derive(Debug)]
struct BrokenJournal;

impl Storage for BrokenJournal {
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        RealDisk.write_atomic(path, bytes)
    }

    fn append_line(&self, _: &Path, _: &[u8]) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::StorageFull))
    }
}

#[test]
fn a_recording_that_could_not_be_set_up_leaves_no_folder_behind() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0).with_storage(BrokenJournal);
    assert!(store.start("No room", START, RATE).is_err());
    assert!(store.unfinished().unwrap().is_empty());
    assert_eq!(std::fs::read_dir(store.root()).unwrap().count(), 0);
}

#[test]
fn a_recording_whose_item_is_in_the_bin_is_not_stored_twice() {
    let tmp = TempDir::new().unwrap();
    let store = store(&tmp, 1.0);
    let mut rec = store.start("Binned", START, RATE).unwrap();
    rec.push_pcm(&pattern(0, 9_000)).unwrap();
    rec.stop(START + 2).unwrap();
    let id = rec.id();
    let mut library = Library::open_in_memory().unwrap();
    let item = store.store_in_library(&mut library, &id).unwrap();
    library.delete(&item).unwrap();
    assert_eq!(store.store_in_library(&mut library, &id).unwrap(), item);
    assert_eq!(library.bin().unwrap().len(), 1);
    assert!(library.items(Some(ItemKind::Recording)).unwrap().is_empty());
    library.restore(&item).unwrap();
    assert_eq!(library.items(Some(ItemKind::Recording)).unwrap().len(), 1);
}
