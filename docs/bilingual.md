# Bilingual reading

The original alongside its translation, with language switching that never loses the place. This
is the core logic and data model; the reading view that shows it is built in the app. The code is
in `crates/zacess-core/src/bilingual`.

## Done when

The core of the issue is done when all of these hold. The reading view, read aloud in the chosen
language and real translation engines are the items that belong to the app.

1. A translation is kept beside the original block by block, labelled with who made it, where it
   ran and when. The original is never replaced. Equations and page breaks are left as they are.
2. Blocks whose original changed since translation are flagged stale, blocks with no translation
   are flagged missing and running the translation again fills both without touching the rest. A
   block that fails is reported and the others still translate.
3. Switching language keeps the block and the sentence. The offset is mapped within the matching
   sentence when both sides have the same number of sentences and over the whole block when they
   do not. It is always snapped to a word start. Offsets count characters, not bytes.
4. Moving between two translations, such as French to German, goes through the original and lands
   on the same sentence.
5. A block with no translation shows the original on both sides, so switching keeps the same
   offset, clamped to the text and snapped to a word, instead of jumping to the start or the end.
6. Machine translation follows the AI rules: nothing runs while helpers are off, a device
   translator is used before a cloud one and the cloud only when the person allows it and has the
   paid tier. Otherwise the reason and an alternative are given in the same words as the other
   helpers.
7. A translated block whose numbers do not match the original's is flagged, reading commas and
   full stops as decimal points, until a person corrects it. Documents saved before the flag
   existed still load.
8. A translation a person already has can be imported paragraph by paragraph. It counts as checked
   and is labelled as theirs. When the paragraphs and blocks do not pair up, nothing is changed and
   the counts are reported.
9. Each side of the view carries its language and which way it runs, so Arabic, Hebrew, Urdu and
   other right-to-left text can be laid out right to left.
10. The reading view, read aloud and real translation engines are done in the app.

## The data

A translation is kept on the document, one per pair of languages, block by block. The original is
never replaced.

| Field | Meaning |
| --- | --- |
| `from`, `to` | BCP 47 language tags, such as `en` and `fr` |
| `translator` | Who made it: the name, the model if any, device or cloud and the time |
| `blocks` | For each block id: the translated `text`, the `source_text` it was made from, whether a person has `checked` it and whether its `numbers_differ` from the original |

Equations and page breaks are not translated; they read the same in every language. A block whose
original has changed since it was translated is **stale**. A block with no translation yet is
**missing**. Re-running the translation fills both and leaves everything else as it is, so one
failed block never loses the rest. A person can mark a block's translation as checked or replace it
with their own wording.

## Translating under the AI rules

Translation sits behind a `Translator` trait, the same shape as the helpers in
[assist.md](assist.md). Each translator says whether it runs on the device or in the cloud.
`translate_with_policy` applies the rules in [our privacy commitments](https://zacess.com/privacy/) to it:

| Situation | Result |
| --- | --- |
| Helpers are off (the default) | `Off`; nothing runs |
| A device translator handles the pair | It runs, even when a cloud one is also available |
| Only a cloud translator handles it, the cloud is allowed and the person has the paid tier | The cloud translator runs |
| Only a cloud translator handles it otherwise | `NeedsCloud`, saying whether cloud use is turned off or part of the paid tier, with an alternative |
| Nothing handles the pair | `NeedsCloud`, saying no helper can translate it |

The alternative offered is to import a translation the person already has.

## The number guard

A translation must never quietly change a number. When a block is translated or corrected, every
number in the original is looked for in the translation and none may be added. Commas and full
stops are both read as decimal points, so `2.5 ms` and `2,5 ms` match. Thin, narrow and
non-breaking spaces inside a number group thousands and are ignored, so `1,000` and `1 000` match.
Arabic-Indic, Devanagari and full-width digits count as the same digits. Units are not compared,
since a translation may write them its own way.

A block that fails the check has `numbers_differ` set and the view shows a warning on it. The flag
clears when a person corrects the block so that the numbers match.

## Supplied translations

`import_parallel` takes a translation a person already has, such as a translated handout. The text
is split into paragraphs at blank lines and the paragraphs fill the blocks that take a translation,
in reading order, one paragraph per block. Lines inside a paragraph are joined with a space, since
pasted text wraps wherever its page ended.

The result is labelled as supplied by that person on the device, counts as checked and replaces any
earlier translation between the same languages. Numbers are checked as above and the blocks that
differ are listed in the report. When the number of paragraphs and blocks differ the import is
refused with both counts and the document is left exactly as it was, because pairing them up
regardless would put text beside the wrong block.

## The view

`pairs` gives the document as pairs of original and translation in reading order, with the missing,
stale, checked and number flags, for a side-by-side view or a swapped one where the translation
leads. A block with no translation shows its original in both places rather than a gap.

`view` gives the same pairs for a named pair of languages along with each side's language and
direction.

## Right to left

`direction` reads a BCP 47 tag. A script subtag decides when there is one: Arabic, Hebrew, Thaana,
Syriac, N'Ko, Adlam, Hanifi Rohingya, Mandaic and Samaritan run right to left, so `pa-Arab` is right
to left while `sd-Deva` is not. Without a script subtag the language decides: Arabic, Hebrew,
Persian, Urdu, Pashto, Sindhi, Uyghur, Yiddish, Dhivehi and Central Kurdish run right to left. Every
other tag runs left to right.

## Keeping the place

A reading position is a block, a character offset into its text and the language the reader is in.
Switching language keeps the block, then keeps the sentence:

1. The block's text on each side is split into sentences at `.`, `!`, `?`, `؟` and `।` followed by a
   space or the end of the text. The full-width `。`, `！` and `？` end a sentence wherever they
   fall. A decimal point such as the one in `2.5 ms` does not end a sentence.
2. When both sides have the same number of sentences, the offset stays in the matching sentence and
   is scaled by that sentence's length on each side. A long first sentence in one language cannot
   pull the reader back from the second.
3. When the counts differ, as when a translator joins or splits sentences, the offset is scaled by
   the two whole texts' lengths instead.
4. Either way the result is snapped back to the start of the word it lands in.

Switching to a language and back returns to where the reader started. Moving between two
translations goes through the original: French to English, then English to German. A block with no
translation shows the original on both sides, so its offset is kept as it is, clamped to the text
and snapped to a word start.

The reading view, read aloud and real translation engines are done in the app.
