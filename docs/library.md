# Local library

Where documents, notes, reading profiles, recordings, study decks and plans and captured clips live
on the device. The code is in
`crates/zacess-core/src/library`. See [how it works](https://zacess.com/how-it-works/).

## Storage

One SQLite file, opened in write-ahead-log mode so a crash during a save never corrupts it and reads
keep working while a save runs. SQLite is bundled with the engine, so nothing has to be installed.

| Table | Columns |
| --- | --- |
| `meta` | `key`, `value`. Holds `schema_version`, currently `4`. A newer schema is refused rather than misread |
| `items` | `id`, `kind` (`document`, `note`, `profile`, `recording`, `deck`, `study_plan` or `clip`), `title`, `created_at`, `updated_at`, `deleted_at` (set while the item is in the bin) and `current_version` |
| `versions` | `item_id`, `number`, `saved_at`, `content_hash` and `content`, the item's JSON at that version |

Times are Unix seconds. Documents are stored as [document model JSON](document-model.md), profiles
as [profile JSON](reading-profiles.md) and notes as JSON with a title, Markdown text and, when the
note is attached to a place in a document, that document's id and block id. A recording item is
the JSON entry of a [finished lecture recording](recording.md): title, times, length, how it ended
and where its audio is. The audio itself stays in the recording store. Decks and study plans are
the JSON of a [study deck or plan](study.md). A clip is an inbox entry from
[universal capture](universal-capture.md#the-inbox): its title, whether it is new, where it came
from, a fingerprint for spotting repeats and either a document or a link note. A version 1, 2 or 3
library gains the kinds it lacks when it is opened. The table has to be made again to allow them and
every row is kept.

## Autosave and version history

The app calls save on every change. A save that changes nothing writes nothing. A save that changes
the content writes a new version with the time. The item's title and `updated_at` follow it.
Every version is kept, so the history can be browsed and any version read back.

Restoring an earlier version saves it as a new version. History is never rewritten, so the state
before the restore can itself be restored.

## The bin

Deleting an item sets `deleted_at` and hides it from the library. For thirty days it can be read and
restored. Items whose thirty days are over are removed, with every version, when the library opens
and after each delete. A person can also erase an item from the bin at once. Storage usage (items,
versions, bytes and items in the bin) is always available for the storage view.

A backup is a complete copy of the library file, written with SQLite's own `VACUUM INTO`, so it opens
like any library.

## Full export

Everything leaves in open formats, the bin included, with the whole history:

```text
library.json                 manifest
documents/<id>.json          current document as document model JSON
documents/<id>.html          the same document as an accessible HTML page
notes/<id>.md                current note as Markdown, headed by its title
profiles/<id>.json           current profile as profile JSON
recordings/<id>.json         current recording entry as JSON
decks/<id>.json              current deck as JSON
decks/<id>.anki.txt          the same deck as Anki text
decks/<id>.notion.csv        the same deck as a Notion CSV
study_plans/<id>.json        current study plan as JSON
clips/<id>.json              current clip as JSON
clips/<id>.html              the clip's document as an accessible HTML page, when it holds one
history/<id>/<n>.json        every version of every item, as stored
```

`library.json` has `format` (`zacess-library`), `format_version` (currently `1`), `exported_at` and an
`items` list. Each item carries its `id`, `kind`, `title`, `created_at`, `updated_at`, `deleted_at`
when in the bin, `current_version`, the `path` of its current content and a `history` list of
`number`, `saved_at` and `path` for every version.

An export can be read back into any library. Ids, times and versions are kept. Items that already
exist are left alone, so the same export can be imported twice without duplicates.
