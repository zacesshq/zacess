# Document model

Every importer produces a Zacess document and every view and export reads one. It lives in
`crates/zacess-core/src/model.rs`. This page documents its JSON form, which is how documents are saved,
exported and passed between parts of Zacess.

A complete example is in [`examples/document.json`](examples/document.json). A unit test parses that
file, so the example always matches the code.

## Versioning

| Rule | Detail |
| --- | --- |
| Current version | `schema_version` is `1` |
| Adding an optional field | Allowed without a new version. Older builds ignore fields they do not know |
| Renaming, removing or changing the meaning of a field | Needs a new version and an upgrade step in `Document::from_json` |
| Reading a newer version | Refused with an error rather than misread |
| Reading an older version | Upgraded on read and written back as the current version |

Optional fields left at their default are not written, which keeps files small. A reader must treat a
missing optional field as its default.

## Coordinates

Every page has a size and a unit. Every region on a page uses the same unit, measured from the
top-left corner of the page with `y` increasing downwards.

| Unit | Meaning |
| --- | --- |
| `pt` | PDF points, 1/72 of an inch |
| `px` | Image pixels |

## Document

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | The version of this JSON form |
| `title` | string | The document title |
| `source` | [Source](#source) | Where the document came from |
| `pages` | array of [Page](#page) | Every page of the original, in order |
| `blocks` | array of [Block](#block) | Every block, in reading order |
| `annotations` | array of [Annotation](#annotation) | Help a helper added beside blocks. Missing when there is none |
| `translations` | array of [Translation](bilingual.md#the-data) | Translations kept beside the original, block by block. Missing when there is none |
| `clip` | [Clip](#clip) | Where a captured clip came from. Missing for a document imported from a file |

### Source

| Field | Type | Meaning |
| --- | --- | --- |
| `file_name` | string | The original file name |
| `format` | string | One of `pdf`, `pptx`, `docx`, `image`, `html` and `other` |

### Clip

Set on a document made by [universal capture](universal-capture.md), so the reader can always show
where it came from.

| Field | Type | Meaning |
| --- | --- | --- |
| `via` | string | Which shell sent it: `extension`, `share_sheet`, `clipboard` or `shortcut` |
| `captured_at` | integer | When it was taken, in Unix seconds |
| `url` | string | The page address. Missing when there was none |
| `page_title` | string | The page title the shell sent. Missing when there was none |
| `file_name` | string | The image or file name. Missing for a page or text |
| `selection_only` | boolean | True when only a selection was sent. Missing when false |

A captured document has no pages. Its blocks and runs point at page 1 with an empty region.

### Page

| Field | Type | Meaning |
| --- | --- | --- |
| `number` | integer | Page number, counted from 1 |
| `width`, `height` | number | Page size in `unit` |
| `unit` | string | `pt` or `px` |

### Origin

The link from the model back to the original. Every block has one and every imported run has one.

| Field | Type | Meaning |
| --- | --- | --- |
| `page` | integer | The page number |
| `region` | object | `x`, `y`, `width` and `height` of the area on that page |

## Block

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | integer | Unique within the document. Ids are never reused while a higher id exists |
| `type` | string | The block type, which decides the other fields |
| `origin` | [Origin](#origin) | The page and region the block came from |

| `type` | Extra fields |
| --- | --- |
| `heading` | `level` (1 to 6) and `runs` |
| `paragraph` | `runs`, plus optional `role`: `body` (the default), `quote` for a quotation or `code` for code whose spacing and line breaks are kept. Older builds read any role as an ordinary paragraph |
| `list` | `ordered` (true or false) and `items`, each with `runs` plus optional nested `children` |
| `table` | `rows`, each with `cells`; optional `caption` runs |
| `image` | Optional `alt_text` (missing means it still needs one) and optional `asset`, a path to the stored image |
| `equation` | `runs` as recognised, optional `latex`, optional `mathml` and `display` (true when set on its own line) |
| `page_break` | None |

A table cell has `runs`, plus optional `header` (true for a header cell), `col_span` and `row_span`
(both 1 when missing).

## Run

A run is a piece of inline text.

| Field | Type | Meaning |
| --- | --- | --- |
| `text` | string | The text as it now reads |
| `confidence` | number | How sure the recogniser was, from 0 to 1. Native text is 1 |
| `origin` | [Origin](#origin) | Where the text sits on the original page. Missing only for text the user typed |
| `status` | string | `recognised` (the default), `confirmed` once checked or `corrected` once changed |
| `original_text` | string | The recognised text before a correction, so the original stays one step away |
| `script` | string | `superscript` or `subscript` for raised or lowered text, such as an exponent or the `out` in `V_out`. Missing for ordinary text |
| `link` | string | The address the text links to, as written on the page. It is never fetched. Missing for text that is not a link |

Runs carry their own spacing. When two runs would join two words together, views add a single space
between them. No space is added before a run that starts with closing punctuation (`.`, `,`, `;`,
`:`, `!`, `?`, `)`, `]` or `}`), such as the full stop after a link. No space is added after an
opening bracket (`(`, `[` or `{`) either, such as the one before a link written in brackets.

## Annotation

Help added beside a block by a helper, described in [assist.md](assist.md). It is never part of
the content: the block keeps its own text and the annotation keeps a copy of it, so the original is
always shown alongside.

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | integer | Unique within the document |
| `block` | integer | The block the help belongs to |
| `kind` | string | `diagram_description` or `simple_explanation` |
| `text` | string | The help itself |
| `original_text` | string | The block's text when the help was made |
| `provenance.helper` | string | The helper's name |
| `provenance.model` | string | The model it used, when it used one |
| `provenance.ran_on` | string | `device` or `cloud` |
| `provenance.at` | integer | When it ran, in Unix seconds |
