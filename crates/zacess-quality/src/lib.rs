//! Scores importers against the document quality test suite in `tests/fixtures`.
//!
//! Four measures are reported for every fixture, every category and overall:
//!
//! - **character accuracy**: one minus the edit distance between the expected and actual text,
//!   divided by the expected length
//! - **number and symbol accuracy**: the share of expected tokens holding a digit or a maths symbol
//!   that appear exactly in the output
//! - **reading order**: the share of neighbouring expected blocks that come out in the same order
//! - **heading detection**: the F1 score of detected headings against expected headings
//!
//! A score can only be compared with the same fixture's earlier score. [`compare`] lists every score
//! that went down, which is what fails a pull request.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use zacess_core::model::{BlockKind, Document, ListItem, join_runs};

/// Blocks must be at least this similar to count as the same block.
const BLOCK_MATCH: f64 = 0.6;
/// Headings must be at least this similar to count as detected.
const HEADING_MATCH: f64 = 0.8;

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestEntry {
    pub name: String,
    pub category: String,
    pub file: String,
}

/// The hand-checked answer for one fixture.
#[derive(Debug, Clone, Deserialize)]
pub struct Expected {
    pub name: String,
    pub category: String,
    pub file: String,
    pub has_text_layer: bool,
    pub pages: u32,
    pub blocks: Vec<ExpectedBlock>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExpectedBlock {
    pub page: u32,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub level: Option<u8>,
    pub text: String,
}

/// A block reduced to what the scorer compares. List items count as blocks of their own.
#[derive(Debug, Clone, PartialEq)]
pub struct FlatBlock {
    pub kind: &'static str,
    pub level: Option<u8>,
    pub text: String,
}

/// The four measures, each from 0 to 1. `None` means the fixture has nothing to measure.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Scores {
    pub character_accuracy: Option<f64>,
    pub number_symbol_accuracy: Option<f64>,
    pub reading_order: Option<f64>,
    pub heading_detection: Option<f64>,
}

impl Scores {
    pub const NAMES: [&'static str; 4] = [
        "character_accuracy",
        "number_symbol_accuracy",
        "reading_order",
        "heading_detection",
    ];

    pub fn values(&self) -> [Option<f64>; 4] {
        [
            self.character_accuracy,
            self.number_symbol_accuracy,
            self.reading_order,
            self.heading_detection,
        ]
    }

    fn from_values(v: [Option<f64>; 4]) -> Self {
        Scores {
            character_accuracy: v[0],
            number_symbol_accuracy: v[1],
            reading_order: v[2],
            heading_detection: v[3],
        }
    }

    /// Rounded to four places so reports and baselines stay stable and readable.
    fn rounded(self) -> Self {
        Scores::from_values(
            self.values()
                .map(|v| v.map(|x| (x * 10_000.0).round() / 10_000.0)),
        )
    }

    fn mean(all: &[Scores]) -> Scores {
        let mut out = [None; 4];
        for (i, slot) in out.iter_mut().enumerate() {
            let present: Vec<f64> = all.iter().filter_map(|s| s.values()[i]).collect();
            if !present.is_empty() {
                *slot = Some(present.iter().sum::<f64>() / present.len() as f64);
            }
        }
        Scores::from_values(out).rounded()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureReport {
    pub category: String,
    /// Why the importer produced nothing, when it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub scores: Scores,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub overall: Scores,
    pub categories: BTreeMap<String, Scores>,
    pub fixtures: BTreeMap<String, FixtureReport>,
}

/// Reads the manifest and every expected answer from a fixtures folder.
pub fn load_suite(dir: &Path) -> Result<Vec<Expected>, String> {
    let manifest_path = dir.join("manifest.json");
    let manifest: Vec<ManifestEntry> = read_json(&manifest_path)?;
    let mut suite = Vec::new();
    for entry in manifest {
        let expected: Expected = read_json(&dir.join(format!("{}.expected.json", entry.name)))?;
        if expected.file != entry.file || expected.category != entry.category {
            return Err(format!("{} does not match its manifest entry", entry.name));
        }
        if !dir.join(&entry.file).is_file() {
            return Err(format!("fixture file {} is missing", entry.file));
        }
        suite.push(expected);
    }
    Ok(suite)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Runs the importer on every fixture and scores the result.
pub fn run_suite(dir: &Path, suite: &[Expected]) -> Report {
    let mut fixtures = BTreeMap::new();
    for expected in suite {
        let (actual, error) = match zacess_core::import::import_file(&dir.join(&expected.file)) {
            Ok(imported) => (flatten(&imported.document), None),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };
        let scores = score(expected, &actual).rounded();
        fixtures.insert(
            expected.name.clone(),
            FixtureReport {
                category: expected.category.clone(),
                error,
                scores,
            },
        );
    }
    summarise(fixtures)
}

fn summarise(fixtures: BTreeMap<String, FixtureReport>) -> Report {
    let mut by_category: BTreeMap<String, Vec<Scores>> = BTreeMap::new();
    for report in fixtures.values() {
        by_category
            .entry(report.category.clone())
            .or_default()
            .push(report.scores);
    }
    let all: Vec<Scores> = fixtures.values().map(|r| r.scores).collect();
    Report {
        overall: Scores::mean(&all),
        categories: by_category
            .into_iter()
            .map(|(k, v)| (k, Scores::mean(&v)))
            .collect(),
        fixtures,
    }
}

/// Reduces a document to the blocks the scorer compares, in reading order.
pub fn flatten(doc: &Document) -> Vec<FlatBlock> {
    fn items(list: &[ListItem], out: &mut Vec<FlatBlock>) {
        for item in list {
            out.push(FlatBlock {
                kind: "list_item",
                level: None,
                text: join_runs(&item.runs),
            });
            items(&item.children, out);
        }
    }
    let mut out = Vec::new();
    for block in &doc.blocks {
        match &block.kind {
            BlockKind::Heading { level, runs } => out.push(FlatBlock {
                kind: "heading",
                level: Some(*level),
                text: join_runs(runs),
            }),
            BlockKind::List { items: list, .. } => items(list, &mut out),
            BlockKind::PageBreak => {}
            BlockKind::Paragraph { .. }
            | BlockKind::Table { .. }
            | BlockKind::Image { .. }
            | BlockKind::Equation { .. } => {
                let text = block.text();
                if !text.is_empty() {
                    out.push(FlatBlock {
                        kind: "paragraph",
                        level: None,
                        text,
                    });
                }
            }
        }
    }
    out
}

/// Scores one fixture's output against its expected answer.
pub fn score(expected: &Expected, actual: &[FlatBlock]) -> Scores {
    let expected_text = normalise(
        &expected
            .blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let actual_text = normalise(
        &actual
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    );

    Scores {
        character_accuracy: character_accuracy(&expected_text, &actual_text),
        number_symbol_accuracy: number_symbol_accuracy(&expected_text, &actual_text),
        reading_order: reading_order(expected, actual),
        heading_detection: heading_detection(expected, actual),
    }
}

/// Collapses runs of whitespace so line wrapping does not count as an error.
pub fn normalise(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn character_accuracy(expected: &str, actual: &str) -> Option<f64> {
    let len = expected.chars().count();
    if len == 0 {
        return None;
    }
    Some((1.0 - edit_distance(expected, actual) as f64 / len as f64).max(0.0))
}

/// Levenshtein distance counted in characters, using two rows of memory.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitution = prev[j] + usize::from(ca != cb);
            curr[j + 1] = substitution.min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// How alike two strings are, from 0 to 1.
pub fn similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (normalise(a), normalise(b));
    let longest = a.chars().count().max(b.chars().count());
    if longest == 0 {
        return 1.0;
    }
    1.0 - edit_distance(&a, &b) as f64 / longest as f64
}

pub use zacess_core::confidence::is_numeric_or_symbol;

fn numeric_tokens(text: &str) -> Vec<&str> {
    text.split_whitespace()
        .map(|t| t.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '(' | ')' | '[' | ']')))
        // a full stop ends a sentence, but stays when it is a decimal point inside the token
        .map(|t| t.strip_suffix('.').unwrap_or(t))
        .filter(|t| !t.is_empty() && is_numeric_or_symbol(t))
        .collect()
}

pub fn number_symbol_accuracy(expected: &str, actual: &str) -> Option<f64> {
    let wanted = numeric_tokens(expected);
    if wanted.is_empty() {
        return None;
    }
    let mut available: HashMap<&str, usize> = HashMap::new();
    for token in numeric_tokens(actual) {
        *available.entry(token).or_default() += 1;
    }
    let mut found = 0;
    for token in &wanted {
        if let Some(count) = available.get_mut(token)
            && *count > 0
        {
            *count -= 1;
            found += 1;
        }
    }
    Some(found as f64 / wanted.len() as f64)
}

/// Finds the output block each expected block became, using each output block once.
fn match_blocks(expected: &[ExpectedBlock], actual: &[FlatBlock]) -> Vec<Option<usize>> {
    let mut used = vec![false; actual.len()];
    expected
        .iter()
        .map(|e| {
            let best = actual
                .iter()
                .enumerate()
                .filter(|(i, _)| !used[*i])
                .map(|(i, a)| (i, similarity(&e.text, &a.text)))
                .filter(|(_, s)| *s >= BLOCK_MATCH)
                .max_by(|x, y| x.1.total_cmp(&y.1));
            best.map(|(i, _)| {
                used[i] = true;
                i
            })
        })
        .collect()
}

pub fn reading_order(expected: &Expected, actual: &[FlatBlock]) -> Option<f64> {
    if expected.blocks.is_empty() {
        return None;
    }
    let matched = match_blocks(&expected.blocks, actual);
    if matched.len() == 1 {
        return Some(if matched[0].is_some() { 1.0 } else { 0.0 });
    }
    let pairs = matched.windows(2);
    let total = pairs.len();
    let correct = pairs
        .filter(|w| matches!((w[0], w[1]), (Some(a), Some(b)) if a < b))
        .count();
    Some(correct as f64 / total as f64)
}

pub fn heading_detection(expected: &Expected, actual: &[FlatBlock]) -> Option<f64> {
    let wanted: Vec<&str> = expected
        .blocks
        .iter()
        .filter(|b| b.kind == "heading")
        .map(|b| b.text.as_str())
        .collect();
    let found: Vec<&str> = actual
        .iter()
        .filter(|b| b.kind == "heading")
        .map(|b| b.text.as_str())
        .collect();
    if wanted.is_empty() && found.is_empty() {
        return None;
    }
    let mut used = vec![false; found.len()];
    let mut hits = 0;
    for w in &wanted {
        if let Some(i) =
            (0..found.len()).find(|&i| !used[i] && similarity(w, found[i]) >= HEADING_MATCH)
        {
            used[i] = true;
            hits += 1;
        }
    }
    if hits == 0 {
        return Some(0.0);
    }
    let precision = hits as f64 / found.len() as f64;
    let recall = hits as f64 / wanted.len() as f64;
    Some(2.0 * precision * recall / (precision + recall))
}

/// One score that went down.
#[derive(Debug, Clone, PartialEq)]
pub struct Regression {
    pub scope: String,
    pub measure: &'static str,
    pub before: f64,
    pub after: Option<f64>,
}

impl std::fmt::Display for Regression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.after {
            Some(after) => {
                write!(
                    f,
                    "{} {}: {:.4} down to {:.4}",
                    self.scope, self.measure, self.before, after
                )
            }
            None => write!(
                f,
                "{} {}: {:.4} is no longer measured",
                self.scope, self.measure, self.before
            ),
        }
    }
}

/// Every score in `current` that is lower than in `baseline`, overall, by category and by fixture.
pub fn compare(baseline: &Report, current: &Report) -> Vec<Regression> {
    let mut out = Vec::new();
    let mut check = |scope: String, before: &Scores, after: Option<&Scores>| {
        for (i, measure) in Scores::NAMES.iter().enumerate() {
            let Some(b) = before.values()[i] else {
                continue;
            };
            let a = after.and_then(|s| s.values()[i]);
            // both sides are rounded to four places, so any drop is real rather than float noise
            if a.is_none_or(|a| a < b) {
                out.push(Regression {
                    scope: scope.clone(),
                    measure,
                    before: b,
                    after: a,
                });
            }
        }
    };
    check("overall".into(), &baseline.overall, Some(&current.overall));
    for (name, before) in &baseline.categories {
        check(
            format!("category {name}"),
            before,
            current.categories.get(name),
        );
    }
    for (name, before) in &baseline.fixtures {
        check(
            format!("fixture {name}"),
            &before.scores,
            current.fixtures.get(name).map(|f| &f.scores),
        );
    }
    out
}

/// A Markdown table of a report, for the terminal and the CI job summary.
pub fn markdown(report: &Report) -> String {
    let cell = |v: Option<f64>| v.map_or("n/a".to_string(), |x| format!("{:.1}%", x * 100.0));
    let mut out = String::from(
        "| Scope | Characters | Numbers and symbols | Reading order | Headings |\n| --- | --- | --- | --- | --- |\n",
    );
    let mut row = |scope: &str, s: &Scores, note: &str| {
        let [a, b, c, d] = s.values().map(cell);
        out.push_str(&format!("| {scope}{note} | {a} | {b} | {c} | {d} |\n"));
    };
    row("**Overall**", &report.overall, "");
    for (name, s) in &report.categories {
        row(&format!("Category: {name}"), s, "");
    }
    for (name, f) in &report.fixtures {
        let note = if f.error.is_some() {
            " (not imported)"
        } else {
            ""
        };
        row(name, &f.scores, note);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected(blocks: &[(&str, &str)]) -> Expected {
        Expected {
            name: "t".into(),
            category: "c".into(),
            file: "t.pdf".into(),
            has_text_layer: true,
            pages: 1,
            blocks: blocks
                .iter()
                .map(|(kind, text)| ExpectedBlock {
                    page: 1,
                    kind: kind.to_string(),
                    level: (*kind == "heading").then_some(1),
                    text: text.to_string(),
                })
                .collect(),
        }
    }

    fn flat(blocks: &[(&'static str, &str)]) -> Vec<FlatBlock> {
        blocks
            .iter()
            .map(|(kind, text)| FlatBlock {
                kind,
                level: None,
                text: text.to_string(),
            })
            .collect()
    }

    #[test]
    fn edit_distance_counts_characters() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("4.7 kΩ", "4.7 kQ"), 1);
    }

    #[test]
    fn perfect_output_scores_one_everywhere() {
        let e = expected(&[
            ("heading", "Title"),
            ("paragraph", "R1 = 4.7 kΩ"),
            ("list_item", "Next"),
        ]);
        let a = flat(&[
            ("heading", "Title"),
            ("paragraph", "R1 = 4.7 kΩ"),
            ("list_item", "Next"),
        ]);
        let s = score(&e, &a);
        assert_eq!(s.values(), [Some(1.0); 4]);
    }

    #[test]
    fn empty_output_scores_zero() {
        let e = expected(&[("heading", "Title"), ("paragraph", "V = 12 V")]);
        let s = score(&e, &[]);
        assert_eq!(s.values(), [Some(0.0); 4]);
    }

    #[test]
    fn a_misread_minus_sign_is_caught() {
        // the unit V matched, the minus sign read as a hyphen did not
        let acc = number_symbol_accuracy("drop of \u{2212}0.35 V.", "drop of -0.35 V.").unwrap();
        assert_eq!(acc, 0.5);
        let acc =
            number_symbol_accuracy("drop of \u{2212}0.35 V.", "drop of \u{2212}0.35 V").unwrap();
        assert_eq!(acc, 1.0);
        assert_eq!(number_symbol_accuracy("no numbers here", "anything"), None);
    }

    #[test]
    fn reading_order_counts_neighbouring_pairs() {
        let e = expected(&[
            ("paragraph", "first block"),
            ("paragraph", "second block"),
            ("paragraph", "third block"),
        ]);
        let swapped = flat(&[
            ("paragraph", "second block"),
            ("paragraph", "first block"),
            ("paragraph", "third block"),
        ]);
        assert_eq!(reading_order(&e, &swapped), Some(0.5));
        let right = flat(&[
            ("paragraph", "first block"),
            ("paragraph", "second block"),
            ("paragraph", "third block"),
        ]);
        assert_eq!(reading_order(&e, &right), Some(1.0));
    }

    #[test]
    fn heading_detection_is_an_f1_score() {
        let e = expected(&[
            ("heading", "Alpha"),
            ("heading", "Beta"),
            ("paragraph", "text"),
        ]);
        let a = flat(&[("heading", "Alpha"), ("heading", "text")]);
        // one hit out of two found and two wanted
        assert_eq!(heading_detection(&e, &a), Some(0.5));
    }

    #[test]
    fn compare_lists_every_drop() {
        let mut fixtures = BTreeMap::new();
        fixtures.insert(
            "a".to_string(),
            FixtureReport {
                category: "c".into(),
                error: None,
                scores: Scores::from_values([Some(0.9), Some(1.0), None, Some(0.5)]),
            },
        );
        let baseline = summarise(fixtures.clone());
        assert!(compare(&baseline, &baseline).is_empty());

        fixtures.get_mut("a").unwrap().scores.character_accuracy = Some(0.8);
        let lower = summarise(fixtures.clone());
        let drops = compare(&baseline, &lower);
        // the fixture, its category and the overall score all went down
        assert_eq!(drops.len(), 3, "{drops:?}");
        assert!(drops.iter().all(|d| d.measure == "character_accuracy"));

        fixtures.get_mut("a").unwrap().scores.character_accuracy = Some(0.95);
        assert!(compare(&baseline, &summarise(fixtures.clone())).is_empty());

        fixtures.clear();
        let missing = compare(&baseline, &summarise(fixtures));
        assert!(
            missing
                .iter()
                .any(|d| d.scope == "fixture a" && d.after.is_none())
        );
    }

    #[test]
    fn the_suite_loads_and_every_fixture_is_present() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let suite = load_suite(&dir).unwrap();
        let categories: std::collections::BTreeSet<&str> =
            suite.iter().map(|e| e.category.as_str()).collect();
        for needed in ["numbers", "scanned", "slides", "textbook"] {
            assert!(categories.contains(needed), "no {needed} fixture");
        }
        for e in &suite {
            assert!(!e.blocks.is_empty(), "{} has no expected blocks", e.name);
            assert!(e.blocks.iter().all(|b| b.page >= 1 && b.page <= e.pages));
        }
    }

    #[test]
    fn the_committed_baseline_matches_a_fresh_run() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let suite = load_suite(&dir).unwrap();
        let current = run_suite(&dir, &suite);
        let baseline: Report = read_json(&dir.join("baseline.json")).unwrap();
        let drops = compare(&baseline, &current);
        assert!(
            drops.is_empty(),
            "scores went down:\n{}",
            drops
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
