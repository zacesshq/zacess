//! Confidence thresholds: which recognised text needs a person to check it.
//!
//! A misread word is usually obvious from context. A misread digit, sign or unit is not. It
//! can change the meaning of a result, so numbers, operators and units use a stricter threshold
//! than ordinary words. The rules are documented in `docs/ocr.md`.

use crate::model::{BlockId, Document, Run};

/// An ordinary word below this confidence is flagged for checking.
pub const WORD_THRESHOLD: f32 = 0.80;
/// A number, operator, unit or symbol below this confidence is flagged for checking.
pub const NUMERIC_THRESHOLD: f32 = 0.95;

const SYMBOLS: &[char] = &[
    '+', '\u{2212}', '\u{00d7}', '\u{00f7}', '=', '<', '>', '\u{00b1}', '%', '\u{00b0}',
    '\u{03a9}', '\u{00b5}', '\u{03bc}', '\u{03c0}', '\u{221a}', '\u{2211}', '\u{222b}', '\u{2248}',
    '\u{2264}', '\u{2265}', '\u{2260}', '/', '^', '*',
];

/// Units written as letters, which carry no digit or symbol of their own.
const UNITS: &[&str] = &[
    "V", "mV", "kV", "A", "mA", "uA", "W", "mW", "kW", "MW", "Hz", "kHz", "MHz", "GHz", "s", "ms",
    "us", "ns", "m", "mm", "cm", "km", "g", "kg", "mg", "N", "J", "kJ", "Pa", "kPa", "F", "nF",
    "pF", "uF", "H", "mH", "dB", "rad", "K", "mol", "C", "T", "Wb", "S", "lm", "lx",
];

/// Whether a token is a number, an operator, a unit or another symbol a misread would change the
/// meaning of.
pub fn is_numeric_or_symbol(token: &str) -> bool {
    let token =
        token.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '(' | ')' | '[' | ']' | '.'));
    if token.is_empty() {
        return false;
    }
    token
        .chars()
        .any(|c| c.is_ascii_digit() || SYMBOLS.contains(&c))
        || (token.starts_with('-') && token[1..].starts_with(|c: char| c.is_ascii_digit()))
        || UNITS.contains(&token)
}

/// The confidence a piece of text must reach before it is shown without a flag.
pub fn threshold_for(text: &str) -> f32 {
    if text.split_whitespace().any(is_numeric_or_symbol) {
        NUMERIC_THRESHOLD
    } else {
        WORD_THRESHOLD
    }
}

/// The threshold for text that follows other text. A short token straight after a number is
/// read as its unit, so a misread unit such as `kQ` for `k\u{3a9}` is held to the numeric threshold.
pub fn threshold_after(previous: Option<&str>, text: &str) -> f32 {
    let follows_number = previous
        .and_then(|p| p.split_whitespace().last())
        .is_some_and(|last| last.chars().any(|c| c.is_ascii_digit()));
    let unit_like = text.trim_end_matches(['.', ',', ';', ':']).chars().count() <= 3
        && text.chars().next().is_some_and(char::is_alphabetic);
    if follows_number && unit_like {
        NUMERIC_THRESHOLD
    } else {
        threshold_for(text)
    }
}

/// Whether a run should be highlighted for checking: it is below its threshold and nobody has
/// confirmed or corrected it yet.
pub fn needs_checking(run: &Run) -> bool {
    !run.is_checked() && run.confidence.value() < threshold_for(&run.text)
}

/// [`needs_checking`] with the run before it as context, for a unit after a number.
pub fn needs_checking_after(previous: Option<&Run>, run: &Run) -> bool {
    !run.is_checked()
        && run.confidence.value() < threshold_after(previous.map(|p| p.text.as_str()), &run.text)
}

/// Which runs of a block need checking, in order, judged with their context.
pub fn flags_for(runs: &[&Run]) -> Vec<bool> {
    runs.iter()
        .enumerate()
        .map(|(i, run)| needs_checking_after(i.checked_sub(1).map(|j| runs[j]), run))
        .collect()
}

/// Every run in the document that needs checking, with the block it belongs to.
pub fn uncertain_runs(document: &Document) -> impl Iterator<Item = (BlockId, &Run)> {
    document.blocks.iter().flat_map(|block| {
        let runs = block.runs();
        let flags = flags_for(&runs);
        runs.into_iter()
            .zip(flags)
            .filter(|(_, flagged)| *flagged)
            .map(move |(run, _)| (block.id, run))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Confidence, Origin, Rect};

    fn run(text: &str, confidence: f32) -> Run {
        Run::recognised(
            text,
            Confidence::new(confidence),
            Origin::new(1, Rect::default()),
        )
    }

    #[test]
    fn numbers_units_and_symbols_are_recognised() {
        for token in [
            "4.7", "kΩ", "−0.35", "-3", "45°", "π/4", "±", "mA", "Hz", "12V", "(1.5)",
        ] {
            assert!(is_numeric_or_symbol(token), "{token}");
        }
        for token in ["the", "Potential", "and", "notes.", "Vout", ""] {
            assert!(!is_numeric_or_symbol(token), "{token}");
        }
    }

    #[test]
    fn numbers_need_a_stricter_threshold_than_words() {
        assert_eq!(threshold_for("tolerance"), WORD_THRESHOLD);
        assert_eq!(threshold_for("R1 = 4.7 kΩ"), NUMERIC_THRESHOLD);
        assert!(!needs_checking(&run("tolerance", 0.86)));
        assert!(
            !needs_checking(&run("kQ", 0.86)),
            "alone, kQ is just a word"
        );
        assert!(
            needs_checking_after(Some(&run("4.7", 0.99)), &run("kQ", 0.86)),
            "after a number it is a unit, so 86% is flagged"
        );
        assert!(!needs_checking_after(
            Some(&run("4.7", 0.99)),
            &run("through", 0.86)
        ));
        let runs = [run("160", 0.93), run("Q", 0.89), run("shifts", 0.9)];
        let refs: Vec<&Run> = runs.iter().collect();
        assert_eq!(flags_for(&refs), [true, true, false]);
        assert!(needs_checking(&run("Potentail", 0.5)));
        assert!(!needs_checking(&run("4.7", 0.96)));
    }

    #[test]
    fn checked_runs_are_never_flagged() {
        let mut r = run("4.7", 0.1);
        assert!(needs_checking(&r));
        r.confirm();
        assert!(!needs_checking(&r));
        let mut r = run("4.1", 0.1);
        r.correct("4.7");
        assert!(!needs_checking(&r));
    }
}
