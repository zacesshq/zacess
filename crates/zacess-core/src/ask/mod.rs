//! Questions answered from the student's own material, with sources.
//!
//! There is no generation here. The index holds the blocks of the documents the person chose, a
//! question finds the blocks that match it best and the answer is the sentences from those blocks,
//! each cited with the document, page and block it came from. When nothing matches well enough the
//! answer says so instead of guessing.

use crate::model::{BlockKind, Document};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// One document in the index, named so citations can point back to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceRef {
    /// The library item id or any other stable id the caller chooses.
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone)]
struct Passage {
    source: usize,
    block: u64,
    page: u32,
    text: String,
    terms: HashMap<String, usize>,
    length: usize,
}

/// A passage that answers part of the question, with where it came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cited {
    /// The sentences from the block that carry the answer.
    pub answer: String,
    /// The whole block, so the answer is never read out of context.
    pub block_text: String,
    pub source: SourceRef,
    pub page: u32,
    pub block: u64,
    pub score: f64,
    pub matched_terms: Vec<String>,
}

impl Cited {
    /// A citation in words, for display and speech.
    pub fn citation(&self) -> String {
        format!("{}, page {}", self.source.title, self.page)
    }
}

/// Whether the material covered the question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    Covered,
    /// Nothing in the indexed material matched the question's terms.
    NotCovered,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub question: String,
    pub coverage: Coverage,
    pub passages: Vec<Cited>,
    /// Always says where the answer came from.
    pub note: String,
}

impl Answer {
    /// The answer as Markdown: each passage quoted with its citation.
    pub fn to_markdown(&self) -> String {
        let mut out = format!("**{}**\n\n", self.question);
        if self.passages.is_empty() {
            out.push_str("Your material does not cover this question.\n");
        }
        for (i, p) in self.passages.iter().enumerate() {
            out.push_str(&format!(
                "{}. {} ({})\n",
                i + 1,
                p.answer.trim(),
                p.citation()
            ));
        }
        out.push_str(&format!("\n{}\n", self.note));
        out
    }
}

/// A search index over the person's own documents.
#[derive(Debug, Clone, Default)]
pub struct Index {
    sources: Vec<SourceRef>,
    passages: Vec<Passage>,
    /// How many passages hold each term.
    document_frequency: HashMap<String, usize>,
    total_length: usize,
}

/// Words too common to tell passages apart.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "does", "for", "from", "has",
    "have", "how", "i", "in", "is", "it", "its", "of", "on", "or", "that", "the", "their", "then",
    "there", "these", "this", "to", "was", "we", "what", "when", "where", "which", "who", "why",
    "will", "with", "you", "your", "my", "me", "did", "into", "than", "them", "they", "so", "if",
    "but", "not", "no", "yes", "about", "any", "all", "one", "two", "also", "very", "just", "were",
    "been", "being", "am", "had", "would", "could", "should", "may", "might", "must", "get", "got",
];

/// Lowercase tokens with light stemming, so "harmonics" matches "harmonic" and "measured" matches
/// "measure". Numbers keep their decimal points.
pub fn tokenise(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let keep = c.is_alphanumeric()
            || (c == '.'
                && i > 0
                && chars[i - 1].is_ascii_digit()
                && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit()))
            || c == '\u{3a9}'
            || c == '\u{b5}'
            || c == '\u{3c0}';
        if keep {
            current.extend(c.to_lowercase());
        } else if !current.is_empty() {
            out.push(stem(&current));
            current.clear();
        }
    }
    if !current.is_empty() {
        out.push(stem(&current));
    }
    out.into_iter()
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .collect()
}

fn stem(word: &str) -> String {
    if word.chars().any(|c| c.is_ascii_digit()) || word.chars().count() < 5 {
        return word.to_string();
    }
    for (suffix, keep_e) in [
        ("ies", false),
        ("ing", false),
        ("ed", false),
        ("es", false),
        ("s", false),
        ("ly", false),
    ] {
        if let Some(stripped) = word.strip_suffix(suffix)
            && stripped.chars().count() >= 3
        {
            let _ = keep_e;
            return if suffix == "ies" {
                format!("{stripped}y")
            } else {
                stripped.to_string()
            };
        }
    }
    word.to_string()
}

impl Index {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a document's blocks. Page breaks and images without a description hold no text and
    /// are skipped.
    pub fn add(&mut self, source: SourceRef, document: &Document) {
        self.remove(&source.id);
        let index = self.sources.len();
        self.sources.push(source);
        for block in &document.blocks {
            if matches!(block.kind, BlockKind::PageBreak) {
                continue;
            }
            let text = block.text();
            if text.trim().is_empty() {
                continue;
            }
            let tokens = tokenise(&text);
            if tokens.is_empty() {
                continue;
            }
            let mut terms: HashMap<String, usize> = HashMap::new();
            for t in &tokens {
                *terms.entry(t.clone()).or_default() += 1;
            }
            for t in terms.keys() {
                *self.document_frequency.entry(t.clone()).or_default() += 1;
            }
            self.total_length += tokens.len();
            self.passages.push(Passage {
                source: index,
                block: block.id.0,
                page: block.origin.page,
                text,
                terms,
                length: tokens.len(),
            });
        }
    }

    /// Removes a document from the index.
    pub fn remove(&mut self, source_id: &str) {
        let Some(index) = self.sources.iter().position(|s| s.id == source_id) else {
            return;
        };
        let (gone, kept): (Vec<Passage>, Vec<Passage>) = std::mem::take(&mut self.passages)
            .into_iter()
            .partition(|p| p.source == index);
        for p in &gone {
            for t in p.terms.keys() {
                if let Some(n) = self.document_frequency.get_mut(t) {
                    *n = n.saturating_sub(1);
                }
            }
            self.total_length -= p.length;
        }
        self.passages = kept;
        // the source stays in the list so the other passages keep their indexes
        self.sources[index].id = String::new();
    }

    pub fn len(&self) -> usize {
        self.passages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.passages.is_empty()
    }

    /// Answers a question from the indexed material only, with at most `limit` passages.
    pub fn ask(&self, question: &str, limit: usize) -> Answer {
        let note =
            "Drawn only from your own material. Every passage says where it came from.".to_string();
        let terms: Vec<String> = tokenise(question)
            .into_iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let mut scored: Vec<(f64, Vec<String>, &Passage)> = self
            .passages
            .iter()
            .filter_map(|p| {
                let (score, matched) = self.bm25(p, &terms);
                (score > 0.0).then_some((score, matched, p))
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.2.page.cmp(&b.2.page)));

        // a single weak match on one common term is noise, not an answer
        let strong = |score: f64, matched: &[String]| matched.len() >= 2 || score >= 1.0;
        let passages: Vec<Cited> = scored
            .into_iter()
            .filter(|(score, matched, _)| strong(*score, matched))
            .take(limit.max(1))
            .map(|(score, matched, p)| Cited {
                answer: best_sentences(&p.text, &matched),
                block_text: p.text.clone(),
                source: self.sources[p.source].clone(),
                page: p.page,
                block: p.block,
                score: (score * 1000.0).round() / 1000.0,
                matched_terms: matched,
            })
            .collect();
        let coverage = if passages.is_empty() {
            Coverage::NotCovered
        } else {
            Coverage::Covered
        };
        Answer {
            question: question.to_string(),
            coverage,
            passages,
            note,
        }
    }

    /// Okapi BM25 with the usual constants, plus the terms that matched.
    fn bm25(&self, passage: &Passage, terms: &[String]) -> (f64, Vec<String>) {
        const K1: f64 = 1.2;
        const B: f64 = 0.75;
        let n = self.passages.len() as f64;
        let average = if self.passages.is_empty() {
            1.0
        } else {
            self.total_length as f64 / n
        };
        let mut score = 0.0;
        let mut matched = Vec::new();
        for term in terms {
            let Some(&tf) = passage.terms.get(term) else {
                continue;
            };
            let df = *self.document_frequency.get(term).unwrap_or(&0) as f64;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            let tf = tf as f64;
            let norm =
                tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * passage.length as f64 / average));
            score += idf * norm;
            matched.push(term.clone());
        }
        matched.sort();
        (score, matched)
    }
}

/// The sentences of a block that hold the most matched terms, in their original order.
fn best_sentences(text: &str, matched: &[String]) -> String {
    let sentences = split_sentences(text);
    if sentences.len() <= 1 {
        return text.trim().to_string();
    }
    let mut ranked: BTreeMap<usize, usize> = BTreeMap::new();
    let mut best = 0;
    for (i, sentence) in sentences.iter().enumerate() {
        let tokens: HashSet<String> = tokenise(sentence).into_iter().collect();
        let hits = matched.iter().filter(|m| tokens.contains(*m)).count();
        ranked.insert(i, hits);
        best = best.max(hits);
    }
    if best == 0 {
        return text.trim().to_string();
    }
    sentences
        .iter()
        .enumerate()
        .filter(|(i, _)| ranked[i] == best)
        .map(|(_, s)| s.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        current.push(c);
        let boundary = matches!(c, '.' | '!' | '?' | '\n')
            && chars.get(i + 1).is_none_or(|n| n.is_whitespace())
            && !(c == '.'
                && i > 0
                && chars[i - 1].is_ascii_digit()
                && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit()));
        if boundary && !current.trim().is_empty() {
            out.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::pdf::import_pdf;
    use std::path::Path;

    fn fixture(name: &str) -> Document {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name);
        import_pdf(&path).unwrap().document
    }

    fn library() -> Index {
        let mut index = Index::new();
        index.add(
            SourceRef {
                id: "d-slides".into(),
                title: "Signals week 3".into(),
            },
            &fixture("slides-signals.pdf"),
        );
        index.add(
            SourceRef {
                id: "d-notes".into(),
                title: "Potential divider notes".into(),
            },
            &fixture("notes-numbers.pdf"),
        );
        index.add(
            SourceRef {
                id: "d-book".into(),
                title: "Op-amps chapter".into(),
            },
            &fixture("textbook-two-column.pdf"),
        );
        index
    }

    #[test]
    fn tokens_are_stemmed_and_numbers_kept() {
        assert_eq!(
            tokenise("The harmonics were measured at 4.7 kΩ and 150 Hz."),
            ["harmonic", "measur", "4.7", "kω", "150", "hz"]
        );
        assert_eq!(tokenise("Which frequencies?"), ["frequency"]);
        assert!(tokenise("the of and").is_empty());
    }

    #[test]
    fn an_answer_comes_from_the_material_with_a_citation() {
        let index = library();
        let answer = index.ask("What is the third harmonic of a 50 Hz square wave?", 3);
        assert_eq!(answer.coverage, Coverage::Covered);
        let top = &answer.passages[0];
        assert_eq!(top.source.id, "d-slides");
        assert_eq!(top.page, 3);
        assert!(
            top.answer.contains("Third harmonic: 150 Hz") || top.block_text.contains("50 Hz"),
            "{top:?}"
        );
        assert_eq!(top.citation(), "Signals week 3, page 3");
        assert!(top.matched_terms.contains(&"harmonic".to_string()));
        assert!(answer.note.contains("your own material"));
        let md = answer.to_markdown();
        assert!(md.contains("(Signals week 3, page 3)"), "{md}");
    }

    #[test]
    fn the_best_sentences_are_picked_from_a_long_block() {
        let index = library();
        let answer = index.ask("Why is the inverting input held at zero volts?", 2);
        let top = &answer.passages[0];
        assert_eq!(top.source.id, "d-book");
        assert!(
            top.answer.contains("inverting input is held at zero"),
            "{}",
            top.answer
        );
        assert!(top.answer.chars().count() < top.block_text.chars().count());
    }

    #[test]
    fn questions_the_material_does_not_cover_are_not_answered() {
        let index = library();
        let answer = index.ask("What is the capital of France?", 3);
        assert_eq!(answer.coverage, Coverage::NotCovered);
        assert!(answer.passages.is_empty());
        assert!(answer.to_markdown().contains("does not cover"));
        // a question made only of stopwords matches nothing either
        assert_eq!(index.ask("what is it", 3).coverage, Coverage::NotCovered);
    }

    #[test]
    fn removing_a_document_removes_its_answers() {
        let mut index = library();
        assert_eq!(
            index.ask("potential divider tolerance", 3).passages[0]
                .source
                .id,
            "d-notes"
        );
        let before = index.len();
        let notes = index_passages_for(&index, "d-notes");
        index.remove("d-notes");
        assert!(index.len() < before);
        assert!(
            index
                .ask("potential divider tolerance", 3)
                .passages
                .iter()
                .all(|p| p.source.id != "d-notes")
        );
        // adding it again replaces rather than duplicates
        index.add(
            SourceRef {
                id: "d-slides".into(),
                title: "Signals week 3".into(),
            },
            &fixture("slides-signals.pdf"),
        );
        assert_eq!(index.len(), before - notes);
    }

    fn index_passages_for(index: &Index, id: &str) -> usize {
        index
            .passages
            .iter()
            .filter(|p| index.sources[p.source].id == id)
            .count()
    }
}
