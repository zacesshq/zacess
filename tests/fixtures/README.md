# Document quality test suite

A fixed set of documents with hand-checked answers. Every pull request is scored against them in CI
and fails if any score goes down.

## Fixtures

| Fixture | Category | What it tests |
| --- | --- | --- |
| `slides-signals.pdf` | Slides | A four-slide 16:9 deck with large headings, bullet lists and a wrapped line |
| `textbook-two-column.pdf` | Textbook | A two-column page drawn right column first, so raw draw order disagrees with reading order. Its page number is left out of the answer |
| `notes-numbers.pdf` | Numbers | Values, units, a minus sign, a degree sign, `Ω`, `π`, `×` and `±` in a text layer |
| `notes-numbers-scan.png` | Numbers | The same notes as a slightly tilted scan with no text layer, for OCR |
| `handout-scan.png` | Scanned | A handout scanned as an image, with headings and a list |
| `handout-scan.pdf` | Scanned | The same scan inside a PDF with no text layer, which must be handed to OCR |

Each fixture has a `<name>.expected.json` answer listing its blocks in reading order, with the page,
type, heading level and text of each. `manifest.json` lists every fixture.

## Where the material comes from

Every document was written for this repository and is covered by its licence, so all of it may be
stored in a public repository. Nothing is copied from a textbook, a lecture or any other published
source. The only outside material is the DejaVu font, whose licence allows embedding.

`generate.py` builds every fixture and its answer from one copy of the content, so an answer can never
drift from its document. The output is deterministic. Each answer was then checked by hand against
the rendered page and the PDF text layer, including every number and symbol, before it was committed.

To regenerate, from the repository root:

```sh
python3 -m venv .venv && .venv/bin/pip install reportlab pillow
.venv/bin/python tests/fixtures/generate.py
```

## Scores

The scorer is the `zacess-quality` crate.

| Measure | How it is worked out |
| --- | --- |
| Character accuracy | One minus the edit distance between expected and actual text, divided by the expected length. Whitespace runs count as one space |
| Number and symbol accuracy | The share of expected tokens holding a digit or a maths symbol that appear exactly in the output. A hyphen read in place of a minus sign counts as wrong |
| Reading order | The share of neighbouring expected blocks that come out in the same order |
| Heading detection | The F1 score of detected headings against expected headings |

Scores are reported for every fixture, every category and overall, rounded to four decimal places.

```sh
cargo run -p zacess-quality -- score
cargo run -p zacess-quality -- check --baseline tests/fixtures/baseline.json
```

The scanned fixtures need `tesseract-ocr` and `poppler-utils` installed, which CI does. Without them
those fixtures score zero and `check` reports the drop.

## When a score changes

- **A score goes up:** run `cargo run -p zacess-quality -- score --out tests/fixtures/baseline.json`
  and commit the new baseline with the change, so the gain is protected from then on.
- **A score goes down:** CI fails. It compares against both the pull request's own baseline and the
  base branch's baseline, so lowering the baseline file does not get a drop through.
- **A new fixture:** add its content to `generate.py`, regenerate, check the answer by hand and update
  the baseline.

## Audio

`audio/lecture-sample.wav` is synthetic speech made with eSpeak NG (the `en-gb` voice at 150 words
a minute) from four sentences written for this repository, spoken as three utterances with
silences inserted between them. It is 22,050 Hz mono 16-bit, so the speech code has to resample
it. `audio/lecture-sample.json` records what each utterance says and exactly when it starts and
ends, which the voice activity and caption tests check against. Nothing in it was recorded from a
person or a lecture.

## Whiteboard captures

`whiteboard/` holds a board photographed through a lecture and a page of notes, all drawn with
Pillow by `generate.py` from text written for this repository: an off-white board with a lighting
gradient and noise, marker text in DejaVu Sans with a slight wobble and blur, plus lined paper for
the notes page. The boards are saved as JPEG at quality 85, as a phone camera would save them. The
notes page is saved as PNG. `whiteboard/captures.json` says which board each file belongs to, whether it
is new, a repeat (the same writing in different light with a small shift) or a change (a line added,
a line rubbed out and rewritten, a wiped board with the next topic) and what is written on it.
Nothing was photographed.

## Slide frames

`slides/` holds the frames of a short lecture as a phone would take them, drawn with Pillow by
`generate.py` from text written for this repository: two light slides and one dark one with a
diagram, photographed by a simulated camera that adds a wall around the projection, vignetting,
noise, shake, exposure changes and blur. The second slide is built in three steps. The frames come
in the order they arrive: the same photograph twice, a shaken one, a dim one, an out of focus one,
the three steps of the build each twice, a frame half way through a fade, the diagram twice, a
frame with the lens covered and a return to the first slide. `slides/frames.json` says what each
frame is. Nothing was photographed.

## Web pages

`clip/` holds three web pages written by hand for this repository for the
[universal capture](../../docs/universal-capture.md) tests. `article.html` is an article page
surrounded by a cookie notice, a site header and menu, a share bar, an advertisement, a side panel,
a newsletter box and a footer, with links, images, a quotation, code and hidden text inside the
article. `table-list.html` has a data table with header cells and nested lists inside `main`.
`no-main.html` has no `main` or `article` element, so its content has to be found by its text. The
tests check what each page keeps and what it leaves out. Nothing in them was copied from a real site.
