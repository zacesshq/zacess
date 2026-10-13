//! Bilingual reading: the original alongside its translation, switching languages without losing
//! the place.
//!
//! A translation is kept beside the document block by block, labelled with who made it. The
//! original is never replaced. A reading position is a block plus a character offset into its
//! text. Mapping it between languages keeps the block, finds the same sentence and scales the
//! offset within it, snapped to a word, so a reader who switches language lands on the same
//! sentence. Moving between two translations goes through the original.
//!
//! Machine translation follows the same rules as the other helpers: off until the person turns it
//! on, on the device first and the cloud only when allowed and paid for. A translation that changes
//! a number is flagged rather than trusted.

use crate::assist::{self, Policy};
use crate::model::{BlockId, BlockKind, Document, Provenance, RanOn};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BilingualError {
    #[error("the translator cannot translate {from} to {to}")]
    Unsupported { from: String, to: String },
    #[error("translation failed: {0}")]
    Failed(String),
    #[error("no translation into {0}")]
    NoTranslation(String),
    #[error(
        "the supplied text has {paragraphs} paragraphs but the document has {blocks} blocks to translate, so nothing was changed"
    )]
    CountMismatch { blocks: usize, paragraphs: usize },
}

/// Something that translates text between two languages, on the device or in the cloud.
pub trait Translator {
    fn name(&self) -> String;
    fn provenance(&self, at: i64) -> Provenance;
    /// Where it runs, which decides whether the person's settings allow it.
    fn runs_on(&self) -> RanOn {
        self.provenance(0).ran_on
    }
    /// Which language pairs it handles. Empty means any.
    fn supports(&self, from: &str, to: &str) -> bool {
        let _ = (from, to);
        true
    }
    fn translate(&self, text: &str, from: &str, to: &str) -> Result<String, BilingualError>;
}

/// A translated block: machine translated until a person checks it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslatedBlock {
    pub text: String,
    /// The original text at the time of translation, so a later edit shows the translation is stale.
    pub source_text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub checked: bool,
    /// The numbers in the translation do not match the original's, so the view shows a warning.
    #[serde(default, skip_serializing_if = "is_false")]
    pub numbers_differ: bool,
}

impl TranslatedBlock {
    fn new(text: String, source_text: String, checked: bool) -> Self {
        TranslatedBlock {
            numbers_differ: !numbers_match(&source_text, &text),
            text,
            source_text,
            checked,
        }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A whole document in another language, block by block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Translation {
    /// BCP 47 tags, such as `en` and `fr`.
    pub from: String,
    pub to: String,
    pub translator: Provenance,
    pub blocks: BTreeMap<BlockId, TranslatedBlock>,
}

impl Translation {
    pub fn new(from: &str, to: &str, translator: Provenance) -> Self {
        Translation {
            from: from.into(),
            to: to.into(),
            translator,
            blocks: BTreeMap::new(),
        }
    }

    /// Blocks whose original changed since they were translated.
    pub fn stale(&self, document: &Document) -> Vec<BlockId> {
        self.blocks
            .iter()
            .filter(|(id, t)| {
                document
                    .block(**id)
                    .is_some_and(|b| b.text() != t.source_text)
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// Blocks with text that have no translation yet.
    pub fn missing(&self, document: &Document) -> Vec<BlockId> {
        translatable_blocks(document)
            .filter(|(id, _)| !self.blocks.contains_key(id))
            .map(|(id, _)| id)
            .collect()
    }

    pub fn mark_checked(&mut self, block: BlockId) -> bool {
        match self.blocks.get_mut(&block) {
            Some(t) => {
                t.checked = true;
                true
            }
            None => false,
        }
    }

    /// Replaces a block's translation with a person's own wording, which counts as checked.
    pub fn correct(&mut self, block: BlockId, text: &str) -> bool {
        match self.blocks.get_mut(&block) {
            Some(t) => {
                *t = TranslatedBlock::new(
                    text.to_string(),
                    std::mem::take(&mut t.source_text),
                    true,
                );
                true
            }
            None => false,
        }
    }
}

/// Equations and page breaks are the same in every language; images carry their descriptions.
fn translatable(kind: &BlockKind) -> bool {
    !matches!(kind, BlockKind::Equation { .. } | BlockKind::PageBreak)
}

/// The blocks a translation covers, in reading order, with their text.
fn translatable_blocks(document: &Document) -> impl Iterator<Item = (BlockId, String)> + '_ {
    document
        .blocks
        .iter()
        .filter(|b| translatable(&b.kind))
        .map(|b| (b.id, b.text()))
        .filter(|(_, text)| !text.trim().is_empty())
}

/// Translates every text block that has no current translation, keeping what is already there.
/// Blocks that fail are left out and reported, so one bad block never loses the rest.
///
/// This runs the translator it is given whatever the person's settings say; an app goes through
/// [`translate_with_policy`] instead.
pub fn translate_document(
    document: &mut Document,
    translator: &dyn Translator,
    from: &str,
    to: &str,
    now: i64,
) -> Result<Vec<(BlockId, BilingualError)>, BilingualError> {
    if !translator.supports(from, to) {
        return Err(BilingualError::Unsupported {
            from: from.into(),
            to: to.into(),
        });
    }
    Ok(translate_blocks(document, translator, from, to, now))
}

fn translate_blocks(
    document: &mut Document,
    translator: &dyn Translator,
    from: &str,
    to: &str,
    now: i64,
) -> Vec<(BlockId, BilingualError)> {
    let position = document
        .translations
        .iter()
        .position(|t| t.to == to && t.from == from);
    let mut translation = match position {
        Some(i) => document.translations.remove(i),
        None => Translation::new(from, to, translator.provenance(now)),
    };
    let mut failures = Vec::new();
    let todo: Vec<BlockId> = {
        let mut ids = translation.missing(document);
        ids.extend(translation.stale(document));
        ids.sort();
        ids.dedup();
        ids
    };
    for id in todo {
        let Some(block) = document.block(id) else {
            continue;
        };
        let source = block.text();
        match translator.translate(&source, from, to) {
            Ok(text) => {
                translation
                    .blocks
                    .insert(id, TranslatedBlock::new(text, source, false));
            }
            Err(e) => failures.push((id, e)),
        }
    }
    translation.translator = translator.provenance(now);
    // put it back where it was, so the order of languages a person sees does not shuffle
    match position {
        Some(i) => document.translations.insert(i, translation),
        None => document.translations.push(translation),
    }
    failures
}

/// What asking for a translation under the person's settings led to.
#[derive(Debug, Clone, PartialEq)]
pub enum TranslateOutcome {
    /// The document was translated, apart from the blocks listed with why they failed.
    Done(Vec<(BlockId, BilingualError)>),
    /// Helpers are turned off, so nothing ran.
    Off,
    /// No device translator handles the pair and the cloud is not available, with what the person
    /// can do instead.
    NeedsCloud { reason: String, alternative: String },
}

/// Translates the document under the person's settings: nothing while helpers are off, a device
/// translator before any cloud one and the cloud only when it is allowed and in the paid tier.
pub fn translate_with_policy(
    document: &mut Document,
    policy: Policy,
    translators: &[&dyn Translator],
    from: &str,
    to: &str,
    now: i64,
) -> TranslateOutcome {
    if !policy.enabled {
        return TranslateOutcome::Off;
    }
    let able: Vec<&dyn Translator> = translators
        .iter()
        .copied()
        .filter(|t| t.supports(from, to))
        .collect();
    let on_device = able.iter().find(|t| t.runs_on() == RanOn::Device);
    let in_cloud = able.iter().find(|t| t.runs_on() == RanOn::Cloud);
    let cloud_ok = policy.allow_cloud && policy.paid_tier;
    let alternative =
        "Import a translation you already have. Cloud helpers can be turned on in Settings.";
    match (on_device, in_cloud) {
        (Some(t), _) => TranslateOutcome::Done(translate_blocks(document, *t, from, to, now)),
        (None, Some(t)) if cloud_ok => {
            TranslateOutcome::Done(translate_blocks(document, *t, from, to, now))
        }
        (None, Some(_)) => TranslateOutcome::NeedsCloud {
            reason: assist::needs_cloud(&policy, &format!("translate {from} into {to}")),
            alternative: alternative.into(),
        },
        (None, None) => TranslateOutcome::NeedsCloud {
            reason: format!("No helper can translate {from} into {to}."),
            alternative: alternative.into(),
        },
    }
}

/// What importing a person's own translation did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// How many blocks it filled.
    pub blocks: usize,
    /// Blocks whose numbers do not match the original, worth a second look.
    pub numbers_differ: Vec<BlockId>,
}

/// Lays a translation a person already has, such as a translated handout, beside the original.
/// Paragraphs are separated by blank lines and fill the translatable blocks in reading order, one
/// paragraph per block. The result counts as checked and is labelled as supplied by
/// that person. When the counts differ nothing is changed, because pairing them up regardless
/// would put text beside the wrong block.
pub fn import_parallel(
    document: &mut Document,
    from: &str,
    to: &str,
    text: &str,
    by: &str,
    now: i64,
) -> Result<ImportReport, BilingualError> {
    let targets: Vec<(BlockId, String)> = translatable_blocks(document).collect();
    let supplied = paragraphs(text);
    if targets.len() != supplied.len() {
        return Err(BilingualError::CountMismatch {
            blocks: targets.len(),
            paragraphs: supplied.len(),
        });
    }
    let mut translation = Translation::new(
        from,
        to,
        Provenance {
            helper: format!("supplied by {by}"),
            model: None,
            ran_on: RanOn::Device,
            at: now,
        },
    );
    for ((id, source), paragraph) in targets.into_iter().zip(supplied) {
        translation
            .blocks
            .insert(id, TranslatedBlock::new(paragraph, source, true));
    }
    let report = ImportReport {
        blocks: translation.blocks.len(),
        numbers_differ: translation
            .blocks
            .iter()
            .filter(|(_, t)| t.numbers_differ)
            .map(|(id, _)| *id)
            .collect(),
    };
    match document
        .translations
        .iter()
        .position(|t| t.from == from && t.to == to)
    {
        Some(i) => document.translations[i] = translation,
        None => document.translations.push(translation),
    }
    Ok(report)
}

/// Splits text into paragraphs at blank lines. Lines inside a paragraph are joined with a space,
/// because a pasted handout wraps its lines wherever its page ended.
fn paragraphs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            if !current.is_empty() {
                out.push(current.join(" "));
                current.clear();
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        out.push(current.join(" "));
    }
    out
}

/// Which language the reader is looking at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Original,
    Translation,
}

/// Which way a language's text runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Ltr,
    Rtl,
}

/// Which way text in a language runs, from its BCP 47 tag. A script subtag wins over the language,
/// since Punjabi in Arabic script (`pa-Arab`) runs right to left while Punjabi itself does not.
pub fn direction(tag: &str) -> Direction {
    const RTL_LANGUAGES: [&str; 12] = [
        "ar", "he", "fa", "ur", "ps", "sd", "ug", "yi", "dv", "ckb", "iw", "ji",
    ];
    const RTL_SCRIPTS: [&str; 9] = [
        "arab", "hebr", "thaa", "syrc", "nkoo", "adlm", "rohg", "mand", "samr",
    ];
    let mut subtags = tag.split(['-', '_']);
    let language = subtags.next().unwrap_or_default().to_ascii_lowercase();
    // the script, when there is one, comes before any singleton that starts an extension
    let script = subtags
        .take_while(|s| s.len() > 1)
        .find(|s| s.len() == 4 && s.chars().all(|c| c.is_ascii_alphabetic()))
        .map(str::to_ascii_lowercase);
    let rtl = match script {
        Some(script) => RTL_SCRIPTS.contains(&script.as_str()),
        None => RTL_LANGUAGES.contains(&language.as_str()),
    };
    if rtl { Direction::Rtl } else { Direction::Ltr }
}

/// One block in both languages, for a side-by-side or swapped view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pair {
    pub block: BlockId,
    pub original: String,
    /// `None` when the block has no translation yet; the view shows the original in its place.
    pub translation: Option<String>,
    pub checked: bool,
    pub stale: bool,
    /// The translation's numbers do not match the original's.
    #[serde(default)]
    pub numbers_differ: bool,
}

/// The pairs of one translation with the languages on each side and which way each runs, so a
/// view can lay out an Arabic or Hebrew side right to left.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairsView {
    pub from: String,
    pub to: String,
    pub original_dir: Direction,
    pub translation_dir: Direction,
    pub pairs: Vec<Pair>,
}

/// The translation into `to`, made from `from` when that is known.
fn find<'a>(
    document: &'a Document,
    from: Option<&str>,
    to: &str,
) -> Result<&'a Translation, BilingualError> {
    document
        .translations
        .iter()
        .find(|t| t.to == to && from.is_none_or(|f| t.from == f))
        .ok_or_else(|| BilingualError::NoTranslation(to.to_string()))
}

/// The document as pairs of original and translation, in reading order.
pub fn pairs(document: &Document, to: &str) -> Result<Vec<Pair>, BilingualError> {
    Ok(pairs_of(document, find(document, None, to)?))
}

/// The pairs for the translation from `from` into `to`, with each side's language and direction.
pub fn view(document: &Document, from: &str, to: &str) -> Result<PairsView, BilingualError> {
    let translation = find(document, Some(from), to)?;
    Ok(PairsView {
        from: from.to_string(),
        to: to.to_string(),
        original_dir: direction(from),
        translation_dir: direction(to),
        pairs: pairs_of(document, translation),
    })
}

fn pairs_of(document: &Document, translation: &Translation) -> Vec<Pair> {
    document
        .blocks
        .iter()
        .filter(|b| !matches!(b.kind, BlockKind::PageBreak))
        .map(|b| {
            let original = b.text();
            let translated = translation.blocks.get(&b.id);
            Pair {
                block: b.id,
                stale: translated.is_some_and(|t| t.source_text != original),
                checked: translated.is_some_and(|t| t.checked),
                numbers_differ: translated.is_some_and(|t| t.numbers_differ),
                translation: translated.map(|t| t.text.clone()),
                original,
            }
        })
        .collect()
}

/// Where the reader is: a block and a character offset into its text on one side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingPosition {
    pub block: BlockId,
    pub offset: usize,
    pub side: Side,
}

/// Where the reader is in a document with several translations: a block, a character offset into
/// its text and the language they are reading, the original's or one a translation is into.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LangPosition {
    pub block: BlockId,
    pub offset: usize,
    pub lang: String,
}

/// Maps a reading position into another language. The block stays the same. The position is
/// carried to the original first and from there into the target, so moving from French to German
/// lands on the same sentence even though neither was made from the other. Within a block the
/// sentence is kept and the offset scaled inside it (see [`scale_offset_aligned`]). Where a block
/// has no translation the view shows the original in its place, so the offset is kept as it is,
/// snapped to a word start.
pub fn switch_language(
    document: &Document,
    position: &LangPosition,
    to: &str,
) -> Result<LangPosition, BilingualError> {
    if position.lang == to {
        return Ok(position.clone());
    }
    let is_original = |lang: &str| document.translations.iter().any(|t| t.from == lang);
    let original = document
        .block(position.block)
        .map(|b| b.text())
        .unwrap_or_default();
    let (original_lang, in_original) = if is_original(&position.lang) {
        (position.lang.clone(), position.offset)
    } else {
        let from = find(document, None, &position.lang)?;
        let translated = from.blocks.get(&position.block).map(|t| t.text.as_str());
        let offset = match translated {
            Some(text) => scale_offset_aligned(text, position.offset, &original),
            None => snap_to_word(&original, position.offset),
        };
        (from.from.clone(), offset)
    };
    let offset = if to == original_lang || is_original(to) {
        snap_to_word(&original, in_original)
    } else {
        let into =
            find(document, Some(&original_lang), to).or_else(|_| find(document, None, to))?;
        match into.blocks.get(&position.block) {
            Some(t) => scale_offset_aligned(&original, in_original, &t.text),
            None => snap_to_word(&original, in_original),
        }
    };
    Ok(LangPosition {
        block: position.block,
        offset,
        lang: to.to_string(),
    })
}

/// Maps a reading position between the original and its translation into `to`, the two-language
/// case of [`switch_language`].
pub fn switch_side(
    document: &Document,
    to: &str,
    position: &ReadingPosition,
) -> Result<ReadingPosition, BilingualError> {
    let translation = find(document, None, to)?;
    let (lang, target, side) = match position.side {
        Side::Original => (translation.from.as_str(), to, Side::Translation),
        Side::Translation => (to, translation.from.as_str(), Side::Original),
    };
    let moved = switch_language(
        document,
        &LangPosition {
            block: position.block,
            offset: position.offset,
            lang: lang.to_string(),
        },
        target,
    )?;
    Ok(ReadingPosition {
        block: moved.block,
        offset: moved.offset,
        side,
    })
}

/// Sentence enders that need a space or the end of the text after them, so `2.5` stays whole.
fn ends_sentence(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '؟' | '।')
}

/// Full-width enders used in Chinese and Japanese, which are not followed by a space.
fn ends_sentence_closed(c: char) -> bool {
    matches!(c, '。' | '！' | '？')
}

/// Quotes and brackets that can close a sentence after its full stop.
fn closes(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | ')' | ']' | '»' | '”' | '’' | '」' | '』' | '）'
    )
}

/// The sentences of a text as ranges of character offsets. The ranges cover the whole text in
/// order: the spaces after a sentence belong to it, so every offset falls in exactly one sentence.
/// A sentence ends at `.`, `!`, `?`, `؟` or `।` followed by a space or the end of the text. The
/// full-width `。`, `！` and `？` end one wherever they are.
pub fn sentences(text: &str) -> Vec<Range<usize>> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < len {
        let c = chars[i];
        if !ends_sentence(c) && !ends_sentence_closed(c) {
            i += 1;
            continue;
        }
        let mut closed = ends_sentence_closed(c);
        let mut j = i + 1;
        while j < len
            && (ends_sentence(chars[j]) || ends_sentence_closed(chars[j]) || closes(chars[j]))
        {
            closed |= ends_sentence_closed(chars[j]);
            j += 1;
        }
        if closed || j == len || chars[j].is_whitespace() {
            while j < len && chars[j].is_whitespace() {
                j += 1;
            }
            out.push(start..j);
            start = j;
        }
        i = j;
    }
    if start < len {
        out.push(start..len);
    }
    out
}

/// Maps an offset by sentence: when both texts have the same number of sentences, the offset stays
/// in the matching sentence and is scaled within it, so a long first sentence in one language and
/// a short one in the other cannot pull the reader into the wrong sentence. When the counts differ
/// the sentences cannot be paired, so it falls back to [`scale_offset`] over the whole text.
pub fn scale_offset_aligned(from: &str, offset: usize, to: &str) -> usize {
    let from_sentences = sentences(from);
    let to_sentences = sentences(to);
    if from_sentences.is_empty() || from_sentences.len() != to_sentences.len() {
        return scale_offset(from, offset, to);
    }
    let offset = offset.min(from.chars().count());
    let i = from_sentences
        .iter()
        .position(|r| r.contains(&offset))
        .unwrap_or(from_sentences.len() - 1);
    let slice = |text: &str, r: &Range<usize>| -> String {
        text.chars().skip(r.start).take(r.len()).collect()
    };
    let (f, t) = (&from_sentences[i], &to_sentences[i]);
    t.start + scale_offset(&slice(from, f), offset - f.start, &slice(to, t))
}

/// Scales an offset from one text to another and snaps it to a word start. An empty `from` holds
/// no place to carry over, so the result is the start of `to`.
pub fn scale_offset(from: &str, offset: usize, to: &str) -> usize {
    let from_len = from.chars().count();
    let to_len = to.chars().count();
    if from_len == 0 || to_len == 0 {
        return 0;
    }
    let scaled = (offset.min(from_len) as f64 / from_len as f64 * to_len as f64).round() as usize;
    snap_to_word(to, scaled)
}

/// Clamps an offset to the text and moves it back to the start of the word it lands in.
fn snap_to_word(text: &str, offset: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return 0;
    }
    let mut i = offset.min(chars.len() - 1);
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

/// Languages a document has translations into.
pub fn languages(document: &Document) -> BTreeSet<String> {
    document.translations.iter().map(|t| t.to.clone()).collect()
}

/// How a separator inside a number can be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Separator {
    /// Only ever groups thousands: thin, narrow and non-breaking spaces and the Arabic one.
    Group,
    /// Only ever a decimal point: the Arabic decimal separator.
    Decimal,
    /// A comma or full stop, which is a decimal point in one language and groups thousands in
    /// another.
    Either,
}

fn separator(c: char) -> Option<Separator> {
    match c {
        ',' | '.' => Some(Separator::Either),
        '\u{066B}' => Some(Separator::Decimal),
        '\u{066C}' | '\u{00A0}' | '\u{2009}' | '\u{202F}' => Some(Separator::Group),
        _ => None,
    }
}

/// A decimal digit in any of the scripts a translation is likely to use, as its ASCII digit.
fn digit(c: char) -> Option<char> {
    // ASCII, Arabic-Indic, Eastern Arabic-Indic, Devanagari and full-width digits
    const ZEROS: [u32; 5] = [0x30, 0x660, 0x6F0, 0x966, 0xFF10];
    // wrapping keeps a character below the zero out of range, so from_digit turns it down
    ZEROS
        .iter()
        .find_map(|&zero| char::from_digit((c as u32).wrapping_sub(zero), 10))
}

/// The numbers in a text, each as the values it could stand for. `2,5` and `2.5` are both two and
/// a half. `1,000` could be one thousand or one to three places, so it is both.
fn numbers(text: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if digit(chars[i]).is_none() {
            i += 1;
            continue;
        }
        let mut digits = String::new();
        let mut separators: Vec<(usize, Separator)> = Vec::new();
        while i < chars.len() {
            if let Some(d) = digit(chars[i]) {
                digits.push(d);
            } else if let Some(s) = separator(chars[i])
                && chars.get(i + 1).and_then(|&c| digit(c)).is_some()
            {
                separators.push((digits.len(), s));
            } else {
                break;
            }
            i += 1;
        }
        let decimal = separators
            .iter()
            .rev()
            .find(|(_, s)| *s == Separator::Decimal)
            .or_else(|| {
                separators
                    .iter()
                    .rev()
                    .find(|(_, s)| *s == Separator::Either)
            })
            .copied();
        let mut readings = Vec::new();
        match decimal {
            Some((at, kind)) => {
                readings.push(format!("{}.{}", &digits[..at], &digits[at..]));
                if kind == Separator::Either && digits.len() - at == 3 {
                    readings.push(digits.clone());
                }
            }
            None => readings.push(digits),
        }
        out.push(readings);
    }
    out
}

/// Whether a translation keeps every number of the original and adds none. Commas and full stops
/// are both read as decimal points and spaces inside thousands are ignored, so `2.5 ms` and
/// `2,5 ms` match while `150 Hz` and `15 Hz` do not. Units are not compared, since a translation
/// may spell them its own way.
pub fn numbers_match(original: &str, translation: &str) -> bool {
    let wanted = numbers(original);
    let found = numbers(translation);
    if wanted.len() != found.len() {
        return false;
    }
    let mut used = vec![false; found.len()];
    wanted.iter().all(|readings| {
        let hit = found
            .iter()
            .enumerate()
            .position(|(j, other)| !used[j] && other.iter().any(|r| readings.contains(r)));
        match hit {
            Some(j) => {
                used[j] = true;
                true
            }
            None => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Origin, Rect, Run, Source, SourceFormat};

    /// Stands in for a translation model: French-style word swaps that keep the shape of the text.
    struct Fake;
    impl Translator for Fake {
        fn name(&self) -> String {
            "fake".into()
        }
        fn provenance(&self, at: i64) -> Provenance {
            Provenance {
                helper: "fake translator".into(),
                model: Some("fake-1".into()),
                ran_on: RanOn::Device,
                at,
            }
        }
        fn supports(&self, from: &str, to: &str) -> bool {
            from == "en" && to == "fr"
        }
        fn translate(&self, text: &str, _from: &str, _to: &str) -> Result<String, BilingualError> {
            if text.contains("FAIL") {
                return Err(BilingualError::Failed("could not translate".into()));
            }
            Ok(text
                .replace("The period is", "La période est")
                .replace("Fourier series", "Séries de Fourier")
                .replace(
                    "A square wave has only odd harmonics.",
                    "Une onde carrée n'a que des harmoniques impaires.",
                )
                .replace(
                    "The third harmonic is at 150 Hz.",
                    "La troisième harmonique est à 150 Hz.",
                ))
        }
    }

    fn origin() -> Origin {
        Origin::new(1, Rect::new(0.0, 0.0, 10.0, 10.0))
    }

    fn doc() -> Document {
        let mut d = Document::new(
            "Week 3",
            Source {
                file_name: "w3.pdf".into(),
                format: SourceFormat::Pdf,
            },
        );
        d.push(
            BlockKind::heading(1, vec![Run::native("Fourier series", origin())]).unwrap(),
            origin(),
        );
        d.push(
            BlockKind::paragraph(vec![Run::native(
                "A square wave has only odd harmonics. The third harmonic is at 150 Hz.",
                origin(),
            )]),
            origin(),
        );
        d.push(
            BlockKind::Equation {
                runs: vec![Run::native("V = IR", origin())],
                latex: Some("V = IR".into()),
                mathml: None,
                display: true,
            },
            origin(),
        );
        d.push(
            BlockKind::paragraph(vec![Run::native("The period is 2.5 ms.", origin())]),
            origin(),
        );
        d
    }

    #[test]
    fn a_document_is_translated_block_by_block_beside_the_original() {
        let mut d = doc();
        let failures = translate_document(&mut d, &Fake, "en", "fr", 10).unwrap();
        assert!(failures.is_empty());
        assert_eq!(languages(&d), BTreeSet::from(["fr".to_string()]));
        let t = &d.translations[0];
        assert_eq!(t.translator.helper, "fake translator");
        assert_eq!(t.blocks.len(), 3, "the equation is not translated");
        assert_eq!(t.blocks[&BlockId(1)].text, "Séries de Fourier");
        assert!(!t.blocks[&BlockId(1)].checked);
        // the original is untouched
        assert_eq!(d.block(BlockId(1)).unwrap().text(), "Fourier series");
        assert!(matches!(
            translate_document(&mut d, &Fake, "en", "de", 0),
            Err(BilingualError::Unsupported { .. })
        ));
    }

    #[test]
    fn pairs_show_both_languages_and_flag_what_is_missing_or_stale() {
        let mut d = doc();
        translate_document(&mut d, &Fake, "en", "fr", 10).unwrap();
        let view = pairs(&d, "fr").unwrap();
        assert_eq!(view.len(), 4);
        assert_eq!(view[0].translation.as_deref(), Some("Séries de Fourier"));
        assert_eq!(view[2].translation, None, "equations stay as they are");

        // the original changes, so its translation is stale until it is redone
        d.block_mut(BlockId(3 + 1)).unwrap().runs_mut()[0].correct("The period is 2.6 ms.");
        let view = pairs(&d, "fr").unwrap();
        assert!(view[3].stale);
        assert_eq!(d.translations[0].stale(&d), [BlockId(4)]);
        translate_document(&mut d, &Fake, "en", "fr", 20).unwrap();
        assert!(!pairs(&d, "fr").unwrap()[3].stale);
        assert_eq!(
            d.translations.len(),
            1,
            "re-translating updates rather than duplicates"
        );
        assert!(matches!(
            pairs(&d, "de"),
            Err(BilingualError::NoTranslation(_))
        ));
    }

    #[test]
    fn a_failed_block_is_reported_and_the_rest_still_translate() {
        let mut d = doc();
        d.push(
            BlockKind::paragraph(vec![Run::native("FAIL here", origin())]),
            origin(),
        );
        let failures = translate_document(&mut d, &Fake, "en", "fr", 10).unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, BlockId(5));
        assert_eq!(d.translations[0].blocks.len(), 3);
        assert_eq!(d.translations[0].missing(&d), [BlockId(5)]);
    }

    #[test]
    fn switching_language_keeps_the_place() {
        let mut d = doc();
        translate_document(&mut d, &Fake, "en", "fr", 10).unwrap();
        // reading the second sentence of block 2 in English
        let english = "A square wave has only odd harmonics. The third harmonic is at 150 Hz.";
        let start_of_second = english.find("The third").unwrap();
        let here = ReadingPosition {
            block: BlockId(2),
            offset: start_of_second,
            side: Side::Original,
        };
        let there = switch_side(&d, "fr", &here).unwrap();
        assert_eq!(there.block, BlockId(2));
        assert_eq!(there.side, Side::Translation);
        let french = "Une onde carrée n'a que des harmoniques impaires. La troisième harmonique est à 150 Hz.";
        let landed: String = french.chars().skip(there.offset).take(12).collect();
        assert_eq!(landed, "La troisième", "the same sentence, from its start");
        // and back again lands exactly where the reader was
        let back = switch_side(&d, "fr", &there).unwrap();
        assert_eq!(back.side, Side::Original);
        assert_eq!(back.offset, start_of_second);

        // a block without a translation shows the original on both sides, so the place is kept
        let eq = ReadingPosition {
            block: BlockId(3),
            offset: 4,
            side: Side::Original,
        };
        assert_eq!(
            switch_side(&d, "fr", &eq).unwrap(),
            ReadingPosition {
                block: BlockId(3),
                offset: 4,
                side: Side::Translation
            }
        );
    }

    #[test]
    fn switching_from_a_missing_translation_keeps_the_offset() {
        let mut d = doc();
        d.push(
            BlockKind::paragraph(vec![Run::native("FAIL here please", origin())]),
            origin(),
        );
        translate_document(&mut d, &Fake, "en", "fr", 10).unwrap();
        assert_eq!(d.translations[0].missing(&d), [BlockId(5)]);
        // the translation side shows the original here, so offset 6 is inside "here"
        let on_translation = ReadingPosition {
            block: BlockId(5),
            offset: 6,
            side: Side::Translation,
        };
        assert_eq!(
            switch_side(&d, "fr", &on_translation).unwrap(),
            ReadingPosition {
                block: BlockId(5),
                offset: 5,
                side: Side::Original
            }
        );
        // an offset past the end is clamped to the last word rather than lost
        let past_the_end = ReadingPosition {
            offset: 99,
            ..on_translation
        };
        assert_eq!(switch_side(&d, "fr", &past_the_end).unwrap().offset, 10);
    }

    #[test]
    fn checking_and_correcting_a_translation() {
        let mut d = doc();
        translate_document(&mut d, &Fake, "en", "fr", 10).unwrap();
        let t = &mut d.translations[0];
        assert!(t.mark_checked(BlockId(1)));
        assert!(t.correct(BlockId(4), "La période est de 2,5 ms."));
        assert!(!t.correct(BlockId(99), "x"));
        let view = pairs(&d, "fr").unwrap();
        assert!(view[0].checked);
        assert_eq!(
            view[3].translation.as_deref(),
            Some("La période est de 2,5 ms.")
        );
        let back = Document::from_json(&d.to_json().unwrap()).unwrap();
        assert_eq!(back.translations, d.translations);
    }

    #[test]
    fn offsets_scale_and_snap_to_words() {
        assert_eq!(scale_offset("hello world", 6, "bonjour le monde"), 8);
        assert_eq!(scale_offset("abc", 0, "xyz"), 0);
        assert_eq!(scale_offset("abc", 3, ""), 0);
        // an empty text holds no place, so the reader starts at the beginning
        assert_eq!(scale_offset("", 5, "one two"), 0);
    }

    /// Stands in for a German model, so moving between two translations can be checked.
    struct FakeGerman;
    impl Translator for FakeGerman {
        fn name(&self) -> String {
            "fake german".into()
        }
        fn provenance(&self, at: i64) -> Provenance {
            Provenance {
                helper: "fake german translator".into(),
                model: None,
                ran_on: RanOn::Device,
                at,
            }
        }
        fn supports(&self, from: &str, to: &str) -> bool {
            from == "en" && to == "de"
        }
        fn translate(&self, text: &str, _from: &str, _to: &str) -> Result<String, BilingualError> {
            Ok(text
                .replace("Fourier series", "Fourier-Reihen")
                .replace(
                    "A square wave has only odd harmonics.",
                    "Eine Rechteckwelle hat nur ungerade Harmonische.",
                )
                .replace(
                    "The third harmonic is at 150 Hz.",
                    "Die dritte Harmonische liegt bei 150 Hz.",
                )
                .replace("The period is 2.5 ms.", "Die Periode beträgt 2,5 ms."))
        }
    }

    /// A cloud translator for the same pair as `Fake`.
    struct Cloud;
    impl Translator for Cloud {
        fn name(&self) -> String {
            "cloud".into()
        }
        fn provenance(&self, at: i64) -> Provenance {
            Provenance {
                helper: "cloud translator".into(),
                model: Some("large-1".into()),
                ran_on: RanOn::Cloud,
                at,
            }
        }
        fn translate(&self, text: &str, from: &str, to: &str) -> Result<String, BilingualError> {
            Fake.translate(text, from, to)
        }
    }

    /// Drops a digit, the kind of slip the number guard is there to catch.
    struct Careless;
    impl Translator for Careless {
        fn name(&self) -> String {
            "careless".into()
        }
        fn provenance(&self, at: i64) -> Provenance {
            Provenance {
                helper: "careless translator".into(),
                model: None,
                ran_on: RanOn::Device,
                at,
            }
        }
        fn translate(&self, text: &str, from: &str, to: &str) -> Result<String, BilingualError> {
            Ok(Fake.translate(text, from, to)?.replace("150", "15"))
        }
    }

    /// The character offset of `needle` in `text`.
    fn at(text: &str, needle: &str) -> usize {
        text[..text.find(needle).unwrap()].chars().count()
    }

    #[test]
    fn sentences_split_at_their_ends_and_keep_decimals_whole() {
        assert_eq!(
            sentences("One. Two! Three? Four"),
            [0..5, 5..10, 10..17, 17..21]
        );
        assert_eq!(sentences("The period is 2.5 ms. Next.").len(), 2);
        assert_eq!(sentences("He said \"stop.\" Then he left.").len(), 2);
        assert_eq!(sentences("Wait... what?!").len(), 2);
        assert_eq!(sentences("今日は晴れ。明日は雨。"), [0..6, 6..11]);
        assert_eq!(sentences("هل هذا صحيح؟ نعم.").len(), 2);
        assert!(sentences("").is_empty());
        assert_eq!(sentences("no full stop").len(), 1);
        assert_eq!(sentences("no full stop")[0], 0..12);
    }

    #[test]
    fn sentence_ranges_count_characters_not_bytes() {
        let text = "Été chaud. Hiver froid.";
        assert_ne!(text.len(), text.chars().count());
        assert_eq!(sentences(text), [0..11, 11..23]);
        let greek = "Ζεστό καλοκαίρι. Κρύος χειμώνας.";
        assert_eq!(
            scale_offset_aligned(text, at(text, "Hiver"), greek),
            at(greek, "Κρύος")
        );
        assert_eq!(
            scale_offset_aligned(text, at(text, "froid"), greek),
            at(greek, "χειμώνας")
        );
    }

    #[test]
    fn offsets_stay_in_the_matching_sentence_when_lengths_differ() {
        let from = "Short. This is a much longer second sentence.";
        let to = "Une phrase de départ bien plus longue que prévu. Court.";
        let offset = at(from, "This");
        assert_eq!(scale_offset_aligned(from, offset, to), at(to, "Court"));
        // scaling the whole text instead would land in the wrong sentence
        assert!(scale_offset(from, offset, to) < at(to, "Court"));
        // the end of the text maps to the last word of the last sentence
        assert_eq!(
            scale_offset_aligned(from, from.chars().count(), to),
            at(to, "Court")
        );
    }

    #[test]
    fn offsets_fall_back_to_the_whole_text_when_sentences_do_not_pair() {
        let from = "One. Two.";
        let to = "Un deux trois";
        assert_eq!(scale_offset_aligned(from, 5, to), scale_offset(from, 5, to));
        assert_eq!(scale_offset_aligned("", 3, to), 0);
    }

    #[test]
    fn moving_between_two_translations_goes_through_the_original() {
        let mut d = doc();
        translate_document(&mut d, &Fake, "en", "fr", 10).unwrap();
        translate_document(&mut d, &FakeGerman, "en", "de", 10).unwrap();
        let french = "Une onde carrée n'a que des harmoniques impaires. La troisième harmonique est à 150 Hz.";
        let german = "Eine Rechteckwelle hat nur ungerade Harmonische. Die dritte Harmonische liegt bei 150 Hz.";
        let english = "A square wave has only odd harmonics. The third harmonic is at 150 Hz.";
        let in_french = LangPosition {
            block: BlockId(2),
            offset: at(french, "La troisième"),
            lang: "fr".into(),
        };
        let in_german = switch_language(&d, &in_french, "de").unwrap();
        assert_eq!(
            in_german,
            LangPosition {
                block: BlockId(2),
                offset: at(german, "Die dritte"),
                lang: "de".into()
            }
        );
        let in_english = switch_language(&d, &in_german, "en").unwrap();
        assert_eq!(in_english.offset, at(english, "The third"));
        assert_eq!(switch_language(&d, &in_english, "fr").unwrap(), in_french);
        assert_eq!(switch_language(&d, &in_french, "fr").unwrap(), in_french);
        assert!(matches!(
            switch_language(&d, &in_french, "es"),
            Err(BilingualError::NoTranslation(l)) if l == "es"
        ));
        let unknown = LangPosition {
            lang: "it".into(),
            ..in_french
        };
        assert!(matches!(
            switch_language(&d, &unknown, "de"),
            Err(BilingualError::NoTranslation(l)) if l == "it"
        ));
    }

    #[test]
    fn translation_is_off_by_default() {
        let mut d = doc();
        assert_eq!(
            translate_with_policy(&mut d, Policy::default(), &[&Fake], "en", "fr", 0),
            TranslateOutcome::Off
        );
        assert!(d.translations.is_empty());
    }

    #[test]
    fn a_device_translator_is_chosen_over_a_cloud_one() {
        let mut d = doc();
        let policy = Policy {
            enabled: true,
            allow_cloud: true,
            paid_tier: true,
        };
        assert_eq!(Cloud.runs_on(), RanOn::Cloud);
        assert_eq!(
            translate_with_policy(&mut d, policy, &[&Cloud, &Fake], "en", "fr", 3),
            TranslateOutcome::Done(Vec::new())
        );
        assert_eq!(d.translations[0].translator.ran_on, RanOn::Device);
        assert_eq!(d.translations[0].translator.helper, "fake translator");
    }

    #[test]
    fn the_cloud_translates_only_when_allowed_and_paid_for() {
        for (allow_cloud, paid_tier, expect) in [
            (false, false, Some("turned off")),
            (false, true, Some("turned off")),
            (true, false, Some("paid tier")),
            (true, true, None),
        ] {
            let mut d = doc();
            let policy = Policy {
                enabled: true,
                allow_cloud,
                paid_tier,
            };
            match (
                translate_with_policy(&mut d, policy, &[&Cloud], "en", "fr", 0),
                expect,
            ) {
                (
                    TranslateOutcome::NeedsCloud {
                        reason,
                        alternative,
                    },
                    Some(why),
                ) => {
                    assert!(reason.contains(why), "{reason}");
                    assert!(reason.contains("translate en into fr"), "{reason}");
                    assert!(!alternative.is_empty());
                    assert!(d.translations.is_empty());
                }
                (TranslateOutcome::Done(failures), None) => {
                    assert!(failures.is_empty());
                    assert_eq!(d.translations[0].translator.ran_on, RanOn::Cloud);
                }
                (other, _) => panic!("{other:?} with cloud={allow_cloud} paid={paid_tier}"),
            }
        }
        // nothing handles the pair at all
        let mut d = doc();
        let policy = Policy {
            enabled: true,
            ..Policy::default()
        };
        assert!(matches!(
            translate_with_policy(&mut d, policy, &[&Fake], "en", "de", 0),
            TranslateOutcome::NeedsCloud { reason, .. } if reason == "No helper can translate en into de."
        ));
    }

    #[test]
    fn numbers_are_compared_across_decimal_styles() {
        assert!(numbers_match(
            "The period is 2.5 ms.",
            "La période est de 2,5 ms."
        ));
        assert!(!numbers_match("at 150 Hz", "à 15 Hz"));
        assert!(numbers_match("1,000 samples", "1\u{202F}000 échantillons"));
        assert!(numbers_match("1,000 samples", "1\u{00A0}000 échantillons"));
        assert!(numbers_match("1,234.5 V", "1.234,5 V"));
        assert!(numbers_match("150 Hz", "١٥٠ هرتز"));
        assert!(numbers_match("2.5 ms", "٢٫٥ ms"));
        assert!(!numbers_match("2 and 3", "2"));
        assert!(!numbers_match("2", "2 and 7"));
        assert!(numbers_match("No numbers here.", "Pas de chiffres."));
    }

    #[test]
    fn a_translation_that_changes_a_number_is_flagged_until_corrected() {
        let mut d = doc();
        translate_document(&mut d, &Careless, "en", "fr", 10).unwrap();
        let view = pairs(&d, "fr").unwrap();
        assert!(view[1].numbers_differ, "150 became 15");
        assert!(!view[0].numbers_differ);
        assert!(!view[3].numbers_differ);
        assert!(d.translations[0].correct(
            BlockId(2),
            "Une onde carrée n'a que des harmoniques impaires. La troisième harmonique est à 150 Hz."
        ));
        assert!(!pairs(&d, "fr").unwrap()[1].numbers_differ);
    }

    #[test]
    fn translations_saved_before_the_number_guard_still_load() {
        let old: TranslatedBlock =
            serde_json::from_str(r#"{"text":"Bonjour","source_text":"Hello"}"#).unwrap();
        assert!(!old.numbers_differ);
        assert!(!old.checked);
        assert!(
            !serde_json::to_string(&old)
                .unwrap()
                .contains("numbers_differ")
        );
        let mut d = doc();
        translate_document(&mut d, &Careless, "en", "fr", 10).unwrap();
        let json = d.to_json().unwrap();
        assert!(json.contains("numbers_differ"));
        assert_eq!(
            Document::from_json(&json).unwrap().translations,
            d.translations
        );
    }

    #[test]
    fn a_supplied_translation_fills_the_blocks_in_order() {
        let mut d = doc();
        translate_document(&mut d, &Fake, "en", "fr", 10).unwrap();
        let text = "Séries de Fourier\n\nUne onde carrée n'a que des harmoniques impaires.\nLa troisième harmonique est à 150 Hz.\n\n   \nLa période est de 2,5 ms.\n";
        let report = import_parallel(&mut d, "en", "fr", text, "Sam", 50).unwrap();
        assert_eq!(
            report,
            ImportReport {
                blocks: 3,
                numbers_differ: Vec::new()
            }
        );
        assert_eq!(
            d.translations.len(),
            1,
            "it replaces the machine translation"
        );
        let t = &d.translations[0];
        assert_eq!(t.translator.helper, "supplied by Sam");
        assert_eq!(t.translator.ran_on, RanOn::Device);
        assert_eq!(t.translator.model, None);
        assert_eq!(t.translator.at, 50);
        assert!(t.blocks.values().all(|b| b.checked));
        assert_eq!(
            t.blocks[&BlockId(2)].text,
            "Une onde carrée n'a que des harmoniques impaires. La troisième harmonique est à 150 Hz."
        );
        assert_eq!(t.blocks[&BlockId(4)].text, "La période est de 2,5 ms.");
        assert!(
            !t.blocks.contains_key(&BlockId(3)),
            "the equation is left alone"
        );
    }

    #[test]
    fn a_supplied_translation_that_does_not_fit_changes_nothing() {
        let mut d = doc();
        translate_document(&mut d, &Fake, "en", "fr", 10).unwrap();
        let before = d.translations.clone();
        assert_eq!(
            import_parallel(&mut d, "en", "fr", "Un.\n\nDeux.", "Sam", 50),
            Err(BilingualError::CountMismatch {
                blocks: 3,
                paragraphs: 2
            })
        );
        assert_eq!(d.translations, before);
        let report =
            import_parallel(&mut d, "en", "fr", "Un\n\nÀ 15 Hz.\n\nDeux", "Sam", 50).unwrap();
        assert_eq!(report.numbers_differ, [BlockId(2), BlockId(4)]);
    }

    #[test]
    fn right_to_left_languages_are_known_by_language_or_script() {
        for tag in [
            "ar",
            "he-IL",
            "ur",
            "pa-Arab",
            "fa",
            "AR-eg",
            "ckb",
            "ks-Arab-IN",
        ] {
            assert_eq!(direction(tag), Direction::Rtl, "{tag}");
        }
        for tag in ["fr", "en-GB", "pa", "sd-Deva", "az-Latn", "en-x-arab", ""] {
            assert_eq!(direction(tag), Direction::Ltr, "{tag}");
        }
        let mut d = doc();
        import_parallel(
            &mut d,
            "en",
            "ar",
            "سلسلة فورييه\n\nموجة مربعة. التوافقية الثالثة عند 150 هرتز.\n\nالدورة 2.5 مللي ثانية.",
            "Sam",
            0,
        )
        .unwrap();
        let view = view(&d, "en", "ar").unwrap();
        assert_eq!(view.original_dir, Direction::Ltr);
        assert_eq!(view.translation_dir, Direction::Rtl);
        assert_eq!(view.pairs.len(), 4);
        assert!(matches!(
            super::view(&d, "de", "ar"),
            Err(BilingualError::NoTranslation(_))
        ));
    }
}
