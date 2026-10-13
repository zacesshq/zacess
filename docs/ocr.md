# OCR

How images and scanned pages become text and how a doubtful word, number or symbol is flagged. The
code lives in `crates/zacess-core/src/import/ocr.rs` and `crates/zacess-core/src/confidence.rs`.

## The engine

Every engine sits behind the `OcrEngine` trait: it takes an image and returns words with their
position and a confidence from 0 to 1. The rest of the pipeline never knows which engine ran.

The engine for native builds is **Tesseract 5**.

| Requirement | How Tesseract meets it |
| --- | --- |
| Runs on the device | A local command or library. Nothing leaves the machine |
| Open source | Apache-2.0, maintained since 2006, packaged for Windows, macOS and Linux |
| Native or WebAssembly | Native through the `tesseract` command or `libtesseract`. In the browser the same engine runs as WebAssembly through tesseract.js, with the same models and the same word-level confidences, so the web app can share the thresholds and the layout code below |
| Confidence per word | The TSV output carries a confidence for every word, which is what the model needs |
| Languages | Over 100 trained languages |

Alternatives considered:

- **ocrs**, a pure Rust engine that also compiles to WebAssembly. Promising for a single binary, but
  it reports text per line without a confidence per word. Its models are fetched at run time.
  Worth revisiting once it exposes word confidences.
- **PaddleOCR** has the best accuracy on messy photos but is a Python stack with large models. It is
  the right engine for the heavy processing worker, not for the device.
- **Apple Vision and Google ML Kit** are the native engines on iOS and Android. They will be plugged
  in behind the same trait by the mobile shells, as the plan sets out.

The native build calls `tesseract <image> stdout --psm 3 tsv`. Pages of a PDF that have no text
layer are rendered at 300 dpi with Poppler's `pdftoppm` first. Both tools are detected at run time;
without them an image import reports that no OCR engine is available rather than failing quietly.

## From words to a document

1. Tesseract's lines and paragraphs give each word a baseline and a font size. The size is taken
   from the paragraph's median line height, except for a line that clearly differs from it, such as
   a heading the engine grouped with the paragraph below.
2. A lone one-character word with a confidence under 60% at the start of an indented line is read as
   a bullet, because that is what the engine does with one. It keeps its low confidence, so it stays
   flagged.
3. The words go through the same [layout analysis](import.md#layout-analysis) as a PDF text layer,
   so an OCR document has the same headings, lists and reading order and feeds the same document
   model.

For a PDF, image pixels are scaled back to the page's points, so every run still links to its
place on the original page.

## Confidence thresholds

| Text | Flagged for checking below | Why |
| --- | --- | --- |
| An ordinary word | 80% | A misread word is usually obvious from its context |
| A number, operator, unit or symbol | 95% | A misread digit, sign or unit changes the meaning and nothing in the sentence gives it away |

A token counts as numeric when it holds a digit, a maths or unit symbol (`=`, `+`, `−`, `×`, `±`,
`%`, `°`, `Ω`, `π` and others) or is a unit written in letters (`V`, `mA`, `kHz`, `rad` and others).
A short token straight after a number is read as its unit as well, so `kQ` read for `kΩ` after
`4.7` is held to the stricter threshold even though the engine was 86% sure of it.

A run that a person has confirmed or corrected is never flagged again, whatever its confidence.

## What the fixtures show

On the scanned number-heavy notes in `tests/fixtures`, Tesseract reads `kΩ` as `kQ`, `−` as an em
dash and `π` as `n`. Every one of those is flagged by the rules above, which is the behaviour the
test suite protects: a miss is acceptable only when it is flagged. The handout scan is read without
error.
