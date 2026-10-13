# Study sessions, revision paths and spaced repetition

The core logic and data model for studying with Zacess: flashcards scheduled with FSRS, reading
goals with progress, revision paths through a document, reminders without streak pressure and
exports to Anki and Notion. The code is in `crates/zacess-core/src/study`.

## Done when

The core of the issue is done when all of these hold. The screens, the notification and checking
an import in the real Anki and Notion are the items that cannot be checked here.

1. Cards are scheduled with FSRS-5 and keep the place in a document they came from. A card can be
   made straight from a block of a document, with the title and page filled in from it.
2. Decks and study plans are kept in the [local library](library.md) like every other item, with a
   version for every change, restore and the bin. A plan keeps its goals, its sessions, the
   position of each revision path and its local day. All of it survives closing and reopening the
   library. A library from before study items existed is carried over when it is opened.
3. Goals measure today or a rolling week. Today starts at the person's local midnight, not at
   midnight UTC. The one reminder a day follows the same local day.
4. A card reviewed during a session counts towards a cards goal.
5. A deck exports as Anki text that imports into the right deck and note type, keeps answers on
   several lines, shows its source under the answer and keeps the same GUID on every export, so
   importing again updates the notes rather than adding copies.
6. A deck exports as a Notion CSV with tags and the deck name as columns. A revision path exports
   as a Notion checklist with the steps already done ticked.
7. The library export writes each deck as JSON, Anki text and Notion CSV and each study plan as
   JSON. It reads them back unchanged.
8. Review screens, notifications and checking imports in Anki and Notion are done in the app.

## Flashcards and FSRS

A card has a front, a back, optional tags and the place it came from: the document, page and
block, so a card can always open its source. `Deck::add_card_from` takes a document and one of its
blocks and fills the place from them. A deck holds cards and a scheduler.

The scheduler is FSRS-5 (the Free Spaced Repetition Scheduler) with its published formulas and
default weights. Each card carries a memory state: a stability, the days until the chance of recall
falls to 90%, plus a difficulty from 1 to 10. A review with a rating of Again, Hard, Good or Easy
updates both and gives the next interval for the retention the person wants, 90% by default. The
first ratings give intervals of 1, 1, 3 and 16 days; good reviews lengthen them and forgetting
shortens them. A card can be suspended. Its chance of recall right now is always available.

## Reading goals and sessions

A session is one sitting with a document: when it started and ended, the blocks and pages read and
the cards reviewed. Reviewing a card with `Deck::review_in` records the review and counts it in the
session, so a cards goal moves as cards are reviewed. A goal is a target in minutes, blocks, pages
or cards, measured either today or over the last seven days. Progress is what was done, the
fraction of the target and whether it was reached. The weekly window rolls, so a quiet day never
resets anything.

## The local day

A plan carries the person's offset from UTC in minutes (`utc_offset_minutes`, 60 for British
Summer Time). Today runs from local midnight to local midnight: `local_day` turns a time into a
local day number and both the daily goal window and the once-a-day reminder use it. A plan saved
before the offset existed reads it as zero, which is UTC, as before. The app sets the offset from
the device and updates it when the clocks change; the core does not follow time zone rules itself.

## Revision paths

A path is built from a document's headings: read each section, review the cards made from it and,
when there is more than one section, a recap of the headings at the end. The path keeps the
document's title and its position and reports how far along it is. A plan holds its paths, so the
position is kept with everything else.

## Reminders without streak pressure

A reminder says what is ready: how many cards are due and what would complete a goal, with "Any
amount counts." It is given at most once a local day and never when there is nothing to say. The
words streak, missed, broke, lost, behind, failed, must and late are never used. A test checks
that.

## Storage

Decks and study plans are library items of the kinds `deck` and `study_plan`, stored as JSON. Every
save that changes one adds a version, an earlier version can be restored and a deleted one waits in
the bin, exactly as for documents and notes. A deck's title in the library is its name. A plan's
title is its name when it has one and "Study plan" when it has none. Adding the two kinds raised
the library schema to version 3; [the library](library.md) describes how an earlier library is
carried over.

Plans and paths saved before a field existed still read: a plan's name, paths and offset and a
path's title all have defaults.

## Exports

| Format | What it is |
| --- | --- |
| Anki | Tab-separated text for Anki's File, Import, described below |
| Notion database | CSV with Name, Answer, Source, Due, Reviews, Recall, Tags and Deck columns. Tags are one comma-separated cell, which Notion reads as a multi-select. A comma inside a tag becomes a space, so it never splits one tag into two. Tabs become spaces |
| Notion page | `RevisionPath::to_notion_markdown` gives a Markdown checklist headed by the document's title. Each step is a `- [x]` or `- [ ]` line, ticked when done. Each review step lists the fronts of its cards beneath it |

The Anki text starts with the header lines Anki reads before the cards. The last one names the
columns, separated by tabs:

```text
#separator:tab
#html:true
#notetype:Basic
#deck:<deck name>
#guid column:1
#tags column:4
#columns:GUID Front Back Tags
```

The `#deck` line is written only when a deck name is given; the library export always gives one.
Each card is one line with four fields: a GUID, the front, the back and the tags. The GUID is
`zacess-<hash of the deck name>-<card id>`, the same on every export, so Anki updates a note it
already has. The card's source goes under the answer as `<br><small>Source: ...</small>`. As HTML,
`&`, `<`, `>` and `"` are escaped, line breaks become `<br>` and tabs become spaces, so every line
holds exactly four fields. Tags are separated by spaces, so a space inside a tag becomes `_`.

`to_anki_text` writes HTML. `to_anki_text_with` takes `AnkiOptions` to name the deck or to write
plain text instead. In plain text each field is quoted where it holds a tab, a line break or a
quote, as Anki's reader expects. Anki escapes the text itself on import. A plain field can run over
several lines, so the four fields to a line hold only for HTML.

## Limits

- Renaming a deck changes its GUIDs, so a renamed deck imported again into Anki adds new notes
  instead of updating the old ones.
- Only text files are written for Anki. An `.apkg` package with scheduling history is not made, so
  Anki schedules imported cards from new.
- Notion takes the CSV and the Markdown through its own import screen. Nothing is sent to Notion
  directly.
- The local day is a fixed offset. A move between time zones or a change of the clocks needs the
  app to update the plan's offset.

Review screens, notifications and checking imports in Anki and Notion are done in the app.
