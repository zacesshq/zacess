# Ask questions of your own lectures

Answers drawn only from the student's own material, each citing where it came from. The code is in
`crates/zacess-core/src/ask`.

## How it works

There is no text generation. The index holds the blocks of the documents the person chose to
include. A question is broken into terms, the blocks that match them best are found and the answer
is the sentences from those blocks that carry the terms, in their original words.

| Step | Detail |
| --- | --- |
| Terms | Lowercase tokens with light stemming, so "harmonics" matches "harmonic". Numbers keep their decimal points and units stay as they are. Common words such as "the" and "what" are ignored |
| Ranking | Okapi BM25 over the blocks, which favours rare terms and short blocks that mention them more than once |
| Answer | The sentences of each top block with the most matched terms, plus the whole block for context |
| Citation | The document title, page and block id, so the reader can open the source at the right place |
| Not covered | When no block matches two terms or scores strongly on one, the answer says the material does not cover the question. Nothing is invented |

Every answer carries the note "Drawn only from your own material. Every passage says where it came
from."

## Keeping the index current

Documents are added by their library id and title and removed by id. Adding a document that is
already there replaces it, so re-indexing after an edit is one call. The index lives in memory and
is rebuilt from the library when the app starts.

## What comes later

The plan's v4 "ask with sources" can add an on-device model that writes a short answer from these
passages, under the same rules as the other helpers: labelled, optional and always showing the
passages it drew on. The retrieval and citations above are what such a model would be held to.
