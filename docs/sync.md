# Sync

The local-first sync engine, built to agreed conflict rules before any server exists (sync arrives in v3 on
[the roadmap](https://zacess.com/roadmap/)). The code is in `crates/zacess-core/src/sync`.

## Shape

Every device keeps a replica: the items it knows, the changes it made (its outbox) and the changes
it has received. Devices exchange changes; a change names the device and counter that made it, the
item, the time on that device and what it does. The transport is not here yet. The server will relay
changes exactly as they are. Two replicas can already be exchanged in tests with `exchange`.

A replica serialises to JSON, so it can be kept alongside the [library](library.md).

## The rules, as implemented

| Situation | Behaviour | How |
| --- | --- | --- |
| Same note edited on two devices | Text edits merge automatically | Note text is a sequence CRDT: every character has an id, an insert names the character it follows and a delete names the character it removes. Replicas that have the same operations show the same text whatever the order |
| Note deleted on one device, edited offline on another | The edit wins and the note is restored, with a notice | An edit arriving for a deleted note restores it; a delete arriving for a note edited here later is refused and a restore is sent back. Both devices get a notice |
| Same reading-profile setting changed on two devices | The most recent change wins and the older one is kept in history | Setting-like items replace whole values. The later time wins, with ties broken by change id. The losing value goes into the item's history |
| Large file changed on two devices | Both copies kept, the person chooses | A copy made on a different base is kept beside the current one and the item needs attention until `choose_copy` picks one |
| Storage full | Sync pauses with a clear message; nothing is deleted | With storage marked full, incoming changes are refused with a clear error, local changes wait in the outbox and items show the message. Marking it free resumes |

## Status on every item

| Status | Meaning |
| --- | --- |
| Saved on this device | Changed here, not yet sent |
| Syncing | Sent, not yet confirmed |
| Synced | Every change to the item is confirmed |
| Needs attention | A message says what: two copies of a file or storage that is full |

## What is not here yet

The server, authentication and end-to-end encryption are v3 work. Documents are treated as
setting-like whole values for now; once a document can be edited in place, its runs will go through
the same CRDT as notes.
