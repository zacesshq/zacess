//! The helpers that ship with the core. Both run on the device and neither uses a model, so they
//! say exactly what they are. Platform shells add on-device models behind the same trait.

use super::{AssistError, Helper};
use crate::import::ocr::{OcrEngine, Tesseract};
use crate::model::RanOn;
use std::path::Path;

/// Explains text more simply by rule: shorter sentences, plainer words, numbers untouched.
pub struct SimplifyHelper;

/// Plainer words for common academic ones. Each pair keeps the meaning.
const PLAINER: &[(&str, &str)] = &[
    ("therefore", "so"),
    ("consequently", "so"),
    ("hence", "so"),
    ("approximately", "about"),
    ("in order to", "to"),
    ("utilise", "use"),
    ("utilize", "use"),
    ("prior to", "before"),
    ("subsequently", "then"),
    ("sufficient", "enough"),
    ("insufficient", "not enough"),
    ("demonstrates", "shows"),
    ("demonstrate", "show"),
    ("additionally", "also"),
    ("furthermore", "also"),
    ("however", "but"),
    ("nevertheless", "even so"),
    ("numerous", "many"),
    ("facilitate", "help"),
    ("commence", "start"),
    ("terminate", "end"),
    ("obtain", "get"),
    ("require", "need"),
    ("requires", "needs"),
    ("component", "part"),
    ("components", "parts"),
    ("magnitude", "size"),
    ("whereas", "while"),
];

impl Helper for SimplifyHelper {
    fn name(&self) -> String {
        "rule-based simplifier".into()
    }

    fn runs_on(&self) -> RanOn {
        RanOn::Device
    }

    fn can_explain(&self) -> bool {
        true
    }

    fn explain_simply(&self, text: &str) -> Result<String, AssistError> {
        let mut out = String::new();
        for sentence in split_sentences(text) {
            for short in split_long(&sentence) {
                let short = plainer(&short);
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(&capitalise(&short));
            }
        }
        if out.trim().is_empty() {
            return Err(AssistError::Failed("there was no text to simplify".into()));
        }
        Ok(out)
    }
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        current.push(c);
        let ends = matches!(c, '.' | '!' | '?')
            && chars.get(i + 1).is_none_or(|n| n.is_whitespace())
            // a decimal point or an abbreviation such as e.g. does not end a sentence
            && !chars.get(i.wrapping_sub(1)).is_some_and(|p| p.is_ascii_digit() && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit()))
            && !ends_with_abbreviation(&current);
        if ends {
            out.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

fn ends_with_abbreviation(text: &str) -> bool {
    let lower = text.trim_end().to_ascii_lowercase();
    ["e.g.", "i.e.", "etc.", "fig.", "eq.", "vs.", "approx."]
        .iter()
        .any(|a| lower.ends_with(a))
}

/// Breaks a long sentence at a clause boundary, so each piece carries one idea.
fn split_long(sentence: &str) -> Vec<String> {
    const LONG: usize = 18;
    if sentence.split_whitespace().count() <= LONG {
        return vec![sentence.to_string()];
    }
    let breaks = [
        "; ",
        ": ",
        ", which ",
        ", so ",
        ", but ",
        // built from two parts so the marker is not mistaken for prose by text linters
        concat!(", ", "and "),
        ", therefore ",
        ", because ",
        " because ",
    ];
    for marker in breaks {
        if let Some(pos) = sentence.find(marker) {
            let (head, tail) = sentence.split_at(pos);
            let tail = tail[marker.len()..].trim_start();
            let joiner = marker.trim().trim_matches([',', ';', ':']);
            let head = ensure_full_stop(head.trim_end_matches([',', ';', ':']));
            let tail = if joiner.is_empty() {
                tail.to_string()
            } else {
                format!("{joiner} {tail}")
            };
            let mut out = vec![head];
            out.extend(split_long(&tail));
            return out;
        }
    }
    vec![sentence.to_string()]
}

fn ensure_full_stop(text: &str) -> String {
    let t = text.trim();
    if t.ends_with(['.', '!', '?']) {
        t.to_string()
    } else {
        format!("{t}.")
    }
}

/// Replaces academic words with plainer ones, whole words only, keeping the first letter's case.
fn plainer(text: &str) -> String {
    let mut out = String::new();
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let trimmed = word.trim_matches(|c: char| !c.is_alphanumeric());
        let lower = trimmed.to_ascii_lowercase();
        match PLAINER
            .iter()
            .find(|(from, _)| *from == lower && !from.contains(' '))
        {
            Some((_, to)) => {
                let replacement = if trimmed.chars().next().is_some_and(char::is_uppercase) {
                    capitalise(to)
                } else {
                    (*to).to_string()
                };
                out.push_str(&word.replacen(trimmed, &replacement, 1));
            }
            None => out.push_str(word),
        }
    }
    // phrases with spaces
    for (from, to) in PLAINER.iter().filter(|(from, _)| from.contains(' ')) {
        out = out.replace(from, to);
        out = out.replace(&capitalise(from), &capitalise(to));
    }
    out
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Describes a diagram from the text in it, using the OCR engine. It says what it found rather
/// than guessing what the picture shows.
pub struct OcrDiagramHelper {
    engine: Box<dyn OcrEngine>,
}

impl OcrDiagramHelper {
    pub fn new(engine: Box<dyn OcrEngine>) -> Self {
        OcrDiagramHelper { engine }
    }

    /// With Tesseract, when it is installed.
    pub fn detect() -> Option<Self> {
        Tesseract::detect().map(|t| OcrDiagramHelper {
            engine: Box::new(t),
        })
    }
}

impl Helper for OcrDiagramHelper {
    fn name(&self) -> String {
        format!("diagram text reader ({})", self.engine.name())
    }

    fn runs_on(&self) -> RanOn {
        RanOn::Device
    }

    fn can_describe(&self) -> bool {
        true
    }

    fn describe_diagram(&self, image: &Path, context: &str) -> Result<String, AssistError> {
        let page = self
            .engine
            .recognise(image)
            .map_err(|e| AssistError::Failed(e.to_string()))?;
        let mut labels: Vec<String> = Vec::new();
        for paragraph in &page.paragraphs {
            for line in &paragraph.lines {
                let text: String = line
                    .words
                    .iter()
                    .filter(|w| w.confidence.value() >= 0.6)
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                if !text.trim().is_empty() {
                    labels.push(text);
                }
            }
        }
        let mut out = String::new();
        if !context.trim().is_empty() {
            out.push_str(&format!("Described as: {}. ", context.trim()));
        }
        if labels.is_empty() {
            out.push_str("A diagram with no readable text. What it shows still needs a description from a person.");
        } else {
            out.push_str(&format!(
                "A diagram with {} line(s) of text. The text reads: {}.",
                labels.len(),
                labels.join(" | ")
            ));
            out.push_str(" How the parts connect still needs a description from a person.");
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_sentences_are_split_and_words_made_plainer_with_numbers_kept() {
        let text = "The resistor, which is 4.7 kΩ, limits the current through the diode to approximately 2.2 mA; therefore the LED is protected from damage. Furthermore, the capacitor requires 100 nF.";
        let simple = SimplifyHelper.explain_simply(text).unwrap();
        assert!(
            simple.contains("4.7 kΩ") && simple.contains("2.2 mA") && simple.contains("100 nF"),
            "{simple}"
        );
        assert!(simple.contains("about 2.2 mA"), "{simple}");
        assert!(simple.contains("So the LED is protected"), "{simple}");
        assert!(
            simple.contains("Also, the capacitor needs 100 nF."),
            "{simple}"
        );
        assert!(
            !simple.contains("therefore") && !simple.contains("Furthermore"),
            "{simple}"
        );
        assert!(super::super::numbers_kept(text, &simple));
    }

    #[test]
    fn short_text_passes_through_and_abbreviations_do_not_end_sentences() {
        assert_eq!(
            SimplifyHelper.explain_simply("Ohm's law.").unwrap(),
            "Ohm's law."
        );
        let sentences = split_sentences("Use a resistor, e.g. 4.7 kΩ. Then measure 2.5 ms.");
        assert_eq!(
            sentences,
            ["Use a resistor, e.g. 4.7 kΩ.", "Then measure 2.5 ms."]
        );
        assert!(SimplifyHelper.explain_simply("   ").is_err());
    }

    #[test]
    fn the_diagram_reader_reports_the_text_it_can_see() {
        let Some(helper) = OcrDiagramHelper::detect() else {
            eprintln!("skipped: tesseract is not installed");
            return;
        };
        let image =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/handout-scan.png");
        let description = helper.describe_diagram(&image, "Lab handout").unwrap();
        assert!(
            description.starts_with("Described as: Lab handout. A diagram with"),
            "{description}"
        );
        assert!(
            description.contains("Laboratory safety handout"),
            "{description}"
        );
        assert!(description.ends_with("needs a description from a person."));
    }
}
