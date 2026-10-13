# Braille-ready output

How a document becomes a file a braille display or embosser can read. The code is in
`crates/zacess-core/src/export/braille.rs`.

## Done when

The core of the issue is done when all of these hold. A real display or embosser is the one item
that cannot be checked here.

1. A document becomes Grade 1 or Grade 2 Unified English Braille through a translator trait, with
   liblouis as the adapter. The adapter is found at run time and detection returns nothing when
   liblouis is absent, so the caller can say so instead of writing an empty file.
2. The `.brf` file is North American Braille Computer Code, the characters a BRF reader expects,
   with lines of the chosen length, pages of the chosen size, CRLF line endings and a form feed
   between pages. It holds nothing but printable ASCII.
3. No line is longer than the cell width, at any width a device uses, whatever the text holds.
   That includes deeply nested lists on the narrowest display.
4. A page never opens with a blank line. A title or heading, every line of it, stays on a page
   with the line after it.
5. Pages can carry braille page numbers.
6. Settings no device uses are refused with a reason instead of being changed quietly. A
   character that cannot be shown is counted and reported, not dropped without a word.
7. Known translations from the liblouis tables are checked against written-out cells. A
   translation reads back as the text it came from.
8. Reading the output on a physical braille display and embossing it are checked by a person with
   the hardware (issue 9).

## Translation

Translation is liblouis's job. It runs as the `lou_translate` command behind the `BrailleTranslator`
trait, detected at run time the same way as the OCR tools, so the mobile shells can put a bundled
liblouis build behind the same trait later. Without it an export says that no braille translator is
available rather than producing nothing quietly.

| Code | liblouis tables | Used for |
| --- | --- | --- |
| UEB grade 2 (the default) | `en-ueb-g2.ctb` | Ordinary text, contracted as fluent readers expect |
| UEB grade 1 | `en-ueb-g1.ctb` | Ordinary text spelt out letter by letter |
| UEB technical | `en-ueb-g1.ctb,en-ueb-math.ctb` | Equations, always uncontracted |

## Formats

**BRF** is the file embossers and displays read: ASCII braille (North American Braille Computer
Code) in lines of 40 cells and pages of 25 lines, with a form feed between pages and CRLF line
endings. Both sizes can be changed. Line length can be 10 to 255 cells. Lines per page can be 0 for
no page breaks, otherwise at least 3 (4 with page numbers). Anything else is an error.

liblouis is run with its `en-us-brf.dis` display table, so cells come out as uppercase letters and
the symbols NABCC uses. Without it dot 4 is printed as a backtick, which no embosser reads as a
cell.

Pages follow transcriber practice. A page never opens with a blank line. The title and each
heading stay on a page with the line after them. With page numbers on, every page after the first
starts with its braille number (the numeric indicator and the digits as the letters A to J) at the
right margin. The number takes one of the page's lines.

Whatever the text holds is made safe before it reaches the file. All whitespace inside a run, such
as a newline from OCR, becomes one space before translation, so runs of spaces are not kept. The
cells that come back must be printable ASCII, because a tab or a stray character would shift every
cell after it. A translator that returns dot patterns (U+2800 to U+283F) is read back to ASCII
braille. Anything else is replaced with a question mark and counted: `to_brf_with_report` returns
the count with the file. A caller should tell the person when it is not zero.

**Unicode braille** is the same lines as dot patterns (`U+2800` to `U+28FF`), for software that
shows braille on screen or feeds a display directly.

## Layout

The layout follows the conventions braille readers know, so a document reads the same way it would
from a transcriber.

| Block | Braille |
| --- | --- |
| Title | Centred on the first line |
| Heading | Cell 5 for a major heading, cell 7 for a minor one, with a blank line before |
| Paragraph | Starts at cell 3 and runs over at cell 1 |
| List | Items at cell 1 running over at cell 3; each nested level steps in two cells; ordered items keep their numbers |
| Table | Listed row by row after a transcriber's note giving the number of data rows, with the cells of a row separated by a translated semicolon. Each cell is preceded by its column heading, so a row makes sense on its own on a one-line display |
| Image | Its description in a transcriber's note. When it has none yet, the note says so |
| Equation | The equation, then its natural reading in a transcriber's note for readers who have not met the technical code. The equation is a plain-text rendering of the source put through the UEB technical tables: a fraction becomes `(a)/(b)`, subscripts and powers lose their braces and any other LaTeX is left as it is. Full UEB technical structure (radicals, matrices) needs a maths braille translator and is not done here |
| Page break | A blank line. A page break in the source does not start a new braille page; braille pages follow the length setting |

Text that [needs checking](ocr.md#confidence-thresholds) is preceded by a transcriber's note reading
"unclear", framed by the UEB transcriber's note indicators, so a braille reader gets the same warning a
sighted reader gets from the highlight.

## Checking it

The tests that need liblouis run when `lou_translate` is installed. CI installs it (see the
workflow), so they run there; on a machine without it they report that they were skipped.

- **Known translations.** Written-out cells, uppercase as a BRF has them, for the strong contractions (the, and, for, with, of),
  the alphabetic wordsigns (knowledge, children, braille), groupsigns, capitals, numbers and a
  decimal, in both grades. A change in a table or in how it is called shows up here.
- **Read back.** Sentences are translated and back-translated by liblouis in both grades and must
  match the original. Grade 2 must come out shorter than grade 1.
- **Every width.** The example document is exported at 20, 32, 40 and 80 cells with and without
  page breaks and page numbers. No line is too long, no page too tall and nothing but printable
  ASCII appears.

The unit tests with a stand-in translator cover layout, pagination, page numbers, settings and
text that would corrupt a line. The output has not yet been read on a physical braille display or
embossed; that test is outstanding and needs the hardware.
