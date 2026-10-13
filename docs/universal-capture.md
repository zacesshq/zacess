# Universal capture

How a web page, a piece of shared or copied text, an image or a file goes straight into the
library's inbox. The browser extension, the share sheet, the clipboard watcher and the desktop
shortcut all send the same request to the core. The code is in `crates/zacess-core/src/clip` and
the inbox in `crates/zacess-core/src/library/clips.rs`. See the
[roadmap](https://zacess.com/roadmap/).

## Done when

The core of the issue is done when all of these hold. The shells that send captures are the items
that cannot be checked here.

1. Every shell sends one versioned JSON request. A newer version is refused rather than misread.
   A request, page, text, image or file over its limit is refused with the size and the limit.
2. A web page becomes a document that keeps headings, paragraphs, lists with their nesting, tables
   with their header cells, links with their text and address, image descriptions, code and
   quotations. Page furniture (menus, headers, footers, side panels, forms, cookie notices,
   advertisements and hidden elements) is left out.
3. The main content is found in a `main` element when there is one, otherwise in the largest
   `article`, otherwise in the block holding the most paragraph text. A selection is kept whole.
4. Every clip records where it came from: the page address, the page title, the time it was taken
   and which shell sent it. The record is kept inside the document as well, so the reader can
   always show it.
5. Plain text becomes paragraphs split on blank lines, with list markers kept as lists. A text that
   is only a web address is kept as a link note.
6. Images go through OCR when an engine is available. Files go through the importer for their
   type. A type that cannot be read is refused with an error that names it.
7. Captures wait in the library's inbox, marked new until they are opened. The same content
   captured twice within ten minutes is kept once.
8. A library from before the inbox is carried over with every item, version and bin entry kept.
9. Nothing is fetched and nothing leaves the device.
10. The browser extension, the share sheet, the clipboard watcher and the desktop shortcut send
    real captures from each platform (built in the app).

## The request

Each shell sends a `CaptureRequest` as JSON:

```json
{
  "version": 1,
  "via": "extension",
  "captured_at": 1760000000,
  "content": {
    "kind": "web_clip",
    "url": "https://example.org/notes/filters",
    "title": "How a low-pass filter works",
    "html": "<main><h1>How a low-pass filter works</h1><p>Slow changes pass.</p></main>",
    "selection_only": false
  }
}
```

A unit test parses this example, so it always matches the code.

| Field | Meaning |
| --- | --- |
| `version` | The request format version, currently `1` |
| `via` | Which shell sent it: `extension`, `share_sheet`, `clipboard` or `shortcut` |
| `captured_at` | When it was taken, in Unix seconds, as the shell saw it |
| `content` | What was captured, with a `kind` that decides its other fields |

| `kind` | Fields |
| --- | --- |
| `web_clip` | `url` (an `http`, `https` or `file` address), optional `title`, `html` (the whole page or the selection) and optional `selection_only` (true when `html` is only what the person selected) |
| `text` | `text` |
| `image` | One of `path` (an image file on the device) and `data_base64` (its bytes as base64, where a data URL also works), with an optional `name` such as the screenshot's file name |
| `file` | `path`, a file on the device |

## From a web page to a document

The page is read with a small, forgiving HTML reader. Broken markup reads as a browser would read
it: unclosed paragraphs, list items and table cells close themselves and stray end tags are
ignored. Character references such as `&amp;` and `&#8217;` become the characters they stand for.

What is kept:

| On the page | In the document |
| --- | --- |
| `h1` to `h6` | Headings at the same level |
| Paragraphs and loose text | Paragraphs. Two line breaks in a row end a paragraph |
| `ul`, `ol` and nested lists | Lists, nested as on the page |
| Data tables | Tables. A `th` cell is a header cell, as is any cell in `thead`. Column and row spans and the caption are kept. A table with one cell or the role `presentation` only places things on the page, so its content is read as ordinary blocks |
| Links | The link text with its address, made whole against the page address. Only web, mail and phone addresses are kept as links. A `javascript:` or `data:` link keeps its text but not its address |
| Images | An image block holding only the `alt` text. An image with no `alt` is kept as one that still needs a description. An image with an empty `alt` is decorative and a 1 pixel image is a tracker, so both are left out. The picture itself is never fetched |
| `pre` | A code paragraph whose spacing and line breaks are kept exactly |
| `blockquote` | Quotation paragraphs |
| `sub` and `sup` | Subscript and superscript text |
| Text hidden only visually, such as an `sr-only` label | Kept, since it was written for screen reader users |

What is left out when the whole page was sent:

- Scripts, styles, `noscript`, templates, embedded frames, objects, canvases, media and every form
  control.
- `nav`, `aside`, `footer`, `form` and `menu` elements. Elements with a navigation, complementary,
  content information, search, dialog, menu or toolbar role.
- A `header` or banner outside the article or main content. An article's own header, which usually
  holds its title and byline, is kept.
- Hidden elements: the `hidden` attribute, `aria-hidden="true"` and `display: none` or
  `visibility: hidden` styles.
- Elements whose class, id or label names page furniture, such as `cookie`, `consent`, `ad`,
  `advert`, `sponsored`, `promo`, `newsletter`, `share`, `social`, `related`, `sidebar`,
  `breadcrumb`, `popup`, `modal` and `comments`. Elements with a `data-ad` attribute go too. A
  block of 400 characters or more that also names itself content (`article`, `content`, `main`,
  `post`, `entry`, `story`) is kept.

A selection keeps everything except scripts, styles, form controls and hidden elements, because the
person chose it.

The title is the one the shell sent, otherwise the page's `title`, otherwise its first heading,
otherwise its address.

## Text, images and files

**Text** from the share sheet or the clipboard splits into paragraphs on blank lines. The lines of
a paragraph join with a space. Lines starting with `-`, `*`, `+`, a bullet or a number followed by
`.` or `)` become a list, nested by indentation. An indented line straight after an item continues
that item. A line starting with one to six `#` and a space is a heading. The title is the first
line, cut to 80 characters.

A text that is only one `http` or `https` address, optionally in angle brackets, is kept as a link
note holding the address. A web page with nothing readable left is kept the same way. The page is
never fetched.

**Images** go through the [OCR importer](ocr.md). An image sent as bytes is recognised by its first
bytes (PNG, JPEG, GIF, WebP, BMP or TIFF). Without an OCR engine on the device the capture is
refused with "no OCR engine is available on this device", so the shell can keep the image and say
why.

**Files** go through the [importer](import.md) for their type. Saved web pages (`.html`, `.htm`,
`.xhtml`) and text files (`.txt`, `.md`) are read as above. Any other type the importer cannot read
is refused by name, for example "docx files cannot be imported yet". A file in another encoding is
read with unreadable bytes marked rather than dropped.

## Where it came from

Every clip carries a source record. A clip holding a document carries the same record in the
document's `clip` field (see [the document model](document-model.md#clip)):

| Field | Meaning |
| --- | --- |
| `via` | Which shell sent it |
| `captured_at` | When it was taken, in Unix seconds |
| `url` | The page address, also set for a link note |
| `page_title` | The title the shell sent |
| `file_name` | The image or file name |
| `selection_only` | True when only a selection was sent |

## The inbox

Captures are kept in the [library](library.md) as `clip` items. Each holds its title, its state
(`new` or `opened`), its source record, a fingerprint of its content and either a document or a link
note. A clip is new until it is opened: reading it for a preview leaves it new and opening it saves
it once as opened. The inbox lists every clip not in the bin, with how many are still new. Clips
have the same version history, bin and export as every other item.

The fingerprint is a hash of the kind of capture, the page address or file name and the captured
text. A capture whose fingerprint matches a clip taken within ten minutes of it is not stored again,
whichever shell sent it. The earlier clip's id is returned instead. A clip in the bin does not
count, so capturing something again after deleting it keeps it.

## Privacy and limits

Nothing is fetched. The core reads only what the shell passed in: no linked page, image, style sheet
or script is downloaded. A link note keeps an address without visiting it. Nothing leaves the
device. Captures are written to the local library like every other item and go no further unless the
person syncs or exports them.

| Limit | Size |
| --- | --- |
| Request JSON | 40 MiB |
| Page or selection HTML | 10 MiB |
| Text | 2 MiB |
| Image, once decoded | 25 MiB |
| File | 200 MiB |
| Page address | 8 KiB |
| Page title | 4 KiB |
| Elements and text pieces in one page | 400,000 |
| Element nesting read | 128 levels, deeper content joins the deepest level kept |

The browser extension, the share sheet, the clipboard watcher and the desktop shortcut are built in
the app.
