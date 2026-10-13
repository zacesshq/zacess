# Lecture recording store

The storage core of recording a lecture on the device. The microphone, background modes and the
screens are the app's. The core takes samples from whatever the app captures and keeps them safe.
The code is in `crates/zacess-core/src/recording`.

## Done when

The core of the issue is done when all of these hold. A real microphone, a phone that is killed in
the background and a battery that dies are the items that cannot be checked here.

1. A recording is written in short chunks. Each chunk is a file that is written whole, synced and
   moved into place, so a chunk is either complete or not there. A crash, a killed app or a dead
   battery loses at most the audio not yet in a chunk, which is the chunk length (10 seconds by
   default, 1 to 60 as set).
2. A journal beside the chunks records the start, every chunk with a checksum, every pause and
   resume and the close. Each line carries its own checksum, so a torn last line or one damaged
   line costs only itself.
3. The next start finds each unfinished recording and closes it cleanly: the journal is read, the
   chunks are checked, a chunk written but not yet journalled is adopted and the recording is
   finished like any other. Nothing is deleted that could still be played.
4. A damaged chunk is handled by where it is. A damaged last chunk is cut back to the whole
   samples that survive or dropped. A damaged or missing chunk in the middle is replaced with
   silence of the same length so everything after it keeps its time. The report says what was lost.
5. A recording can be paused and resumed any number of times. Audio is not kept while paused.
   The time of every pause is kept so the recording knows where each chunk sits on the wall clock.
6. A manifest records, for every chunk, its start in the audio file and its start on the
   recording's clock, which counts the time paused. The [timeline](timeline.md) takes the pauses
   and places the photos and slides of the lecture at the right place in the audio, which is
   where the transcript's times are too.
7. The free space is checked before every chunk. When there is not enough the recording stops
   cleanly: the audio waiting for the next chunk, at most one chunk's worth, is let go, the journal
   is closed and the recording is kept and finished. A write that fails because the disk is full does the same. Room to finish
   the recording is kept back, so stopping never leaves audio that cannot be assembled.
8. A finished recording is one WAV file with a manifest beside it and it is stored in the
   [local library](library.md) as an item that points to them and carries the title, the times,
   the length and how the recording ended.
9. The tests cover a crash in the middle of a chunk, a damaged last chunk, a damaged chunk in the
   middle, a full disk, thousands of pauses and resumes and a recording of more than three hours.
10. Capturing from a real microphone, keeping going in the background and surviving a real power
    loss are checked by a person with the hardware (issue 9).

## Layout on disk

Each recording has a folder in the store named by its id, such as `rec-1760000000`.

| File | What it is |
| --- | --- |
| `journal.log` | One line per event, `checksum`, a space, then JSON. Only ever appended to and synced after each line |
| `chunks/000001.wav` | Chunk 1. A plain 16-bit mono WAV, so any player can open a single chunk. A file ending in `.part` is a chunk that was being written |
| `recording.wav` | The finished recording, the chunks joined |
| `manifest.json` | The finished recording described. It exists only once the recording is finished |
| `damaged/` | Chunks that could not be used, kept as they may still play in part. Made when the recording is finished |

A recording is open while it has no manifest. Finishing joins the chunks into `recording.wav.part`,
syncs it and moves it into place, writes the manifest the same way and only then clears the chunks
away: the ones that went into the file are removed and any damaged one is moved to `damaged`. A
crash at any point leaves the chunks, so finishing can be run again and the next start clears
away what a stopped finish left. A half-joined file from an earlier try is replaced and a whole
one counts as room already taken when the space is checked.

## How a recording is written

Samples are handed over as they come. They wait in memory until a chunk's worth is there, then the
chunk is written, then the journal says so. The order matters: a crash between the two leaves a
chunk the journal does not know, which recovery adopts if it is whole.

Pausing writes what is waiting as a short chunk first, so no chunk ever spans a pause. Resuming
starts a new chunk. A pause is kept as the place in the audio where it happened and how long it
lasted in seconds.

| Setting | Default | Meaning |
| --- | --- | --- |
| `chunk_secs` | 10 | Seconds of audio in each chunk. This is the most a crash can lose |
| `reserve_bytes` | 64 MiB | Free space always left for the rest of the device |
| `keep_room_to_finish` | true | Also keep free as much as has been recorded so far, so the chunks can be joined into the final file |
| `max_bytes` | 2 GiB | The most audio in one recording. A WAV file cannot hold more than 4 GiB |

The free space comes from a probe the app supplies, as the platform decides how to ask. The core
has one that asks `df` where that exists and one with a fixed answer for tests.

## The clock

There are two ways to count time in a recording and the manifest keeps both.

- **Audio time** is seconds from the start of `recording.wav`. Pauses take no time in it. The
  transcript from the [speech module](speech.md) is measured in it.
- **Wall time** is seconds from the moment the recording started, with the pauses counted. A photo
  is taken at a Unix time, which is wall time once the start is taken off.

The timeline works in audio time. Its recording carries the pauses and a photo taken after a pause
is moved back by the pauses before it. A photo taken during a pause is placed at the start of that
pause and marked as moved. `Manifest::timeline_recording` builds the timeline's recording with the
pauses in it.

## When the app starts again

`RecordingStore::unfinished` lists the recordings with no manifest. `RecordingStore::recover` closes
and finishes one and returns a report.

| Found | Done |
| --- | --- |
| The journal has no close | It is closed at the end of the last chunk that could be used, as recovered |
| Paused when it stopped | If the person stopped it, the pause ends at the stop. If the app was killed, the pause is dropped, as nothing more can be known about it |
| A resume line that fails its checksum | That pause is kept with no length. The pauses after it are not affected |
| A journal line that fails its checksum | It is skipped and counted |
| A chunk the journal does not know, whole and next in order | It is adopted |
| A last chunk that fails its checksum or is short | It is cut back to whole samples if the header is readable. Otherwise it is dropped |
| A middle chunk that fails its checksum or is missing | It is replaced with silence of the length the journal gave. If the journal line is gone too the usual chunk length is used |
| Nothing usable at all | A folder with no chunk file is removed, as there is nothing to lose. One with damaged files is kept until the app discards it. Recovery says it holds no audio |

A chunk that fails its checksum is treated as lost even if most of it may still play. This is
deliberate: a recording that says what it lost is more useful than one that quietly holds damaged
audio. The damaged file itself is not thrown away.

## The library

A finished recording is stored in the library as an item of kind recording. The item holds the
title, when it started, its length, how it ended (complete, recovered, out of space or at the size
limit), the audio lost, the file names and the size and fingerprint of the audio. The audio stays
in the store, as a library item is text. The library schema moved to version 2 to allow the new
kind and a version 1 library is carried over when it is opened.

## Limits

- Mono only. The app brings a stereo or multi-channel source to one channel first.
- The wall time of a pause is taken to the second, so a recording with many short pauses can drift
  from the photos by up to a second a pause. The audio time itself is exact, as it is counted in
  samples.
- The core does not know the sample clock of the device drifts against the wall clock. Over three
  hours the two can differ by a second or two.
- Deleting a library item does not delete the audio. That is left to the app, which knows whether
  anything else uses it.

## Left for the interface

The microphone, recording in the background, the screen with its level meter and the pause button,
the prompt to free space when finishing fails for want of room and the list of recordings found on
start. The hardware checks are in issue 9.
