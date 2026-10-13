# Lecture timeline

The core of the lecture timeline: one lecture's recording, transcript, boards and pages,
bookmarks and chapters on one clock. The screens that show it are the app's. The code is in
`crates/zacess-core/src/timeline`.

## Done when

The core of the issue is done when all of these hold.

1. The transcript from the [speech module](speech.md) and the captures from the
   [capture module](capture.md) sit on one clock, seconds from the start of the recording. A
   photo's time is its Unix time less the recording's start. A photo taken outside the
   recording is moved to the nearest end and marked.
2. Bookmarks can be added, edited and removed. They refuse a time outside the lecture.
3. A search finds spoken sentences, board and page lines, bookmark notes and chapter titles in one
   list in time order. Each hit says what it is and the moment to jump to, which for speech is the
   first matching word.
4. Asking for a time gives what is being said, what is on the board, the chapter and the bookmarks
   either side. A time outside the lecture is moved to the nearest end. The next and previous
   event, of one kind or any, can be found from any time.
5. Chapters are found from long pauses in the speech and from new boards or pages. Signals close
   together are one change of topic, chapters that would be too short are not made and a chapter a
   person added is never moved or removed.
6. The timeline is saved beside the recording and loaded again. The file is written whole and moved
   into place. A damaged or newer file is an error. Loading says whether the recording beside
   it is still the one the timeline was made for.
7. Showing any of this on screen and playing the recording are the app's job.

## The clock

Everything is measured in seconds from the start of the recording. A timeline holds the recording's
file name, when it started (Unix seconds), its length and, once it has been read, its size and a
fingerprint. The lecture is as long as the longest of the recording, the transcript and the last
board seen, so a recording whose length is not known yet still has one.

| Record | Where it comes from | Time |
| --- | --- | --- |
| Sentence | The transcript, split at sentence punctuation using the word times | Its first word |
| Board or page | A capture session, one mark per kept version | When the photo was taken. The time it was last seen again is kept too |
| Bookmark | A person, with an optional note | The moment marked |
| Chapter | Found by detection or added by a person | Where it starts. It ends where the next begins |

A recording that was paused carries its pauses and a Unix time after one is moved back by the
length of the pauses before it, as the audio holds no sound for them. A time inside a pause is
placed at the start of it and marked as moved. See [recording](recording.md#the-clock).

A capture mark holds the board's title and the lines written on it, so the timeline can be searched
without the photos or the capture session. The transcript is held whole.

## Search, jump and step

A search needs every word of the query to match, as a whole word or the start of one, ignoring
case. Hits come in time order. A sentence hit jumps to its first matching word when word times are
known, so it lands on the word rather than the start of the sentence.

The position at a time is the sentence being spoken (or the last one before a pause), the latest
board or page at or before it, the chapter and the bookmarks either side. Stepping to the next or
previous event can be limited to one kind: chapters, sentences, boards or bookmarks.

## Chapters

Two things in a lecture mark a new chapter.

| Signal | Rule |
| --- | --- |
| Long pause | The lecturer is silent for at least 20 seconds. The chapter starts where speech resumes and is named from the first words that follow |
| New board | The first photo of a board or page. A new version of the same board is a change, not a new chapter. The chapter takes the board's title |

Signals within 45 seconds of each other are one change of topic. The earliest time is kept, a board
outranks a pause. A title that names the topic outranks one that only numbers the board. A
chapter shorter than two minutes is not made. Nor is one in the last minute of the lecture (half
the minimum length). Signals exactly 45 seconds apart still merge. Pauses are found between
segments of the transcript, not inside one. The
first chapter always starts at zero. A board that went up too early to make a chapter can still
name it. All three settings can be changed.

Detection replaces the chapters it found earlier and never touches one a person added. A
person's chapter is not counted when checking the minimum length of a detected one. A person's
chapter replaces a detected one within 45 seconds of it. Renaming the opening chapter keeps the
name.

## Keeping it

The timeline is saved as JSON next to the recording, in a file named after it with
`.zacess-timeline.json` added. It is written to a temporary file in the same folder and moved into
place, so a crash cannot leave half a timeline. It carries a format version. A file from a newer
version is refused rather than misread.

Loading says whether the recording beside it is still the one the timeline was made for, by size and
a fingerprint of its first and last megabyte. When it is not, the times may no longer line up and
the app should say so.

## Tests

The tests join the transcript of the speech sample with whiteboard photos taken at chosen moments
and check what is on the board at each time. They cover boards outside the recording, search across
every kind of record, bookmarks, stepping, chapter detection from pauses and boards and its merge,
length and manual rules. They also cover saving and loading with a changed recording, a missing
file and a newer format.

## Library and sync

The timeline is not yet registered as a library item and the recording is not a sync large file, so
it stays on the device that made it. That comes with the app. Events that share a time, such as a
chapter and the board that started it, are stepped over together by next and previous.

## Left for the interface

The timeline screen, playing the recording from a tap, adding bookmarks while recording and showing
the board photos. Slides captured from a projector are placed with `set_slides`, described in
[capture](capture.md#slide-capture) and sit beside the boards. Real lectures have not been run through chapter detection, so its thresholds are
starting points to tune.
