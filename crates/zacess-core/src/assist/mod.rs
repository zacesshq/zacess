//! Optional helpers that describe diagrams and explain things more simply.
//!
//! The rules from the plan are built in rather than hoped for:
//!
//! 1. The original is the source of truth. A helper's result is an [`Annotation`] beside the
//!    block, labelled with who made it and where it ran. It carries the block's own text.
//! 2. Helpers are off by default. Nothing runs until the person turns them on.
//! 3. On-device helpers come first. The cloud is used only when the person allows it and has the
//!    paid tier; otherwise the assistant explains what it needs and offers the alternative.
//! 4. Where a mistake could mislead, nothing is rewritten: an equation is explained by its exact
//!    natural reading. Numbers in a simplified text are kept word for word.

pub mod helpers;

use crate::maths::speech;
use crate::model::{Annotation, AnnotationKind, BlockId, BlockKind, Document, Provenance, RanOn};
use std::path::Path;

pub use helpers::{OcrDiagramHelper, SimplifyHelper};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssistError {
    #[error("the helper cannot do that: {0}")]
    Unsupported(String),
    #[error("the helper failed: {0}")]
    Failed(String),
}

/// Something that can describe a diagram or explain text more simply.
pub trait Helper {
    fn name(&self) -> String;
    fn runs_on(&self) -> RanOn;
    /// The model behind the helper, when there is one.
    fn model(&self) -> Option<String> {
        None
    }
    fn can_explain(&self) -> bool {
        false
    }
    fn can_describe(&self) -> bool {
        false
    }
    fn explain_simply(&self, _text: &str) -> Result<String, AssistError> {
        Err(AssistError::Unsupported(format!(
            "{} does not explain text",
            self.name()
        )))
    }
    fn describe_diagram(&self, image: &Path, context: &str) -> Result<String, AssistError> {
        let _ = (image, context);
        Err(AssistError::Unsupported(format!(
            "{} does not describe diagrams",
            self.name()
        )))
    }
}

/// What the person has allowed. Everything is off until they turn it on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Policy {
    pub enabled: bool,
    pub allow_cloud: bool,
    pub paid_tier: bool,
}

/// What asking for help led to.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Help was added beside the block.
    Added(Annotation),
    /// Helpers are turned off.
    Off,
    /// No device helper can do this and the cloud is not available, with what the person can do.
    NeedsCloud {
        reason: String,
        alternative: String,
    },
    /// The block has nothing a helper could work on, such as an image with no file.
    NothingToWorkOn(String),
    Failed(String),
}

/// Chooses a helper under the policy and records what it produced beside the block.
pub struct Assistant {
    pub policy: Policy,
    device: Vec<Box<dyn Helper>>,
    cloud: Vec<Box<dyn Helper>>,
}

impl Assistant {
    pub fn new(policy: Policy) -> Self {
        Assistant {
            policy,
            device: Vec::new(),
            cloud: Vec::new(),
        }
    }

    /// Adds a helper. Device helpers are tried before cloud ones whatever the order of adding.
    pub fn with_helper(mut self, helper: Box<dyn Helper>) -> Self {
        match helper.runs_on() {
            RanOn::Device => self.device.push(helper),
            RanOn::Cloud => self.cloud.push(helper),
        }
        self
    }

    /// The helpers the policy allows right now, device first.
    fn available(&self) -> impl Iterator<Item = &dyn Helper> {
        let cloud_ok = self.policy.allow_cloud && self.policy.paid_tier;
        self.device
            .iter()
            .chain(self.cloud.iter().filter(move |_| cloud_ok))
            .map(|h| h.as_ref())
    }

    fn cloud_could(&self, want: impl Fn(&dyn Helper) -> bool) -> bool {
        self.cloud.iter().any(|h| want(h.as_ref()))
    }

    fn needs_cloud(&self, task: &str, alternative: &str) -> Outcome {
        Outcome::NeedsCloud {
            reason: needs_cloud(&self.policy, task),
            alternative: alternative.to_string(),
        }
    }

    /// Explains a block more simply and records the explanation beside it.
    pub fn explain_simply(&self, document: &mut Document, block: BlockId, now: i64) -> Outcome {
        if !self.policy.enabled {
            return Outcome::Off;
        }
        let Some(target) = document.block(block) else {
            return Outcome::NothingToWorkOn(format!("no block {block}"));
        };
        let original = target.text();
        if original.trim().is_empty() {
            return Outcome::NothingToWorkOn("the block has no text".into());
        }

        // an equation is never rewritten: its exact reading is the simple explanation
        if let BlockKind::Equation { latex, .. } = &target.kind {
            let source = latex.clone().unwrap_or_else(|| original.clone());
            let text = format!("Read aloud, this says: {}.", speech::speak(&source));
            return self.record(
                document,
                block,
                AnnotationKind::SimpleExplanation,
                text,
                Provenance {
                    helper: "equation reader".into(),
                    model: None,
                    ran_on: RanOn::Device,
                    at: now,
                },
            );
        }

        let Some(helper) = self.available().find(|h| h.can_explain()) else {
            let alternative = "Read the original with the reading ruler and the glossary. Cloud helpers can be turned on in Settings.";
            return if self.cloud_could(|h| h.can_explain()) {
                self.needs_cloud("explain this more simply", alternative)
            } else {
                Outcome::NeedsCloud {
                    reason: "No helper can explain text more simply.".into(),
                    alternative: alternative.into(),
                }
            };
        };
        match helper.explain_simply(&original) {
            Ok(text) if numbers_kept(&original, &text) => self.record(
                document,
                block,
                AnnotationKind::SimpleExplanation,
                text,
                Provenance {
                    helper: helper.name(),
                    model: helper.model(),
                    ran_on: helper.runs_on(),
                    at: now,
                },
            ),
            Ok(_) => Outcome::Failed(format!(
                "{} changed a number, date or symbol in the text, so its explanation was not kept",
                helper.name()
            )),
            Err(e) => Outcome::Failed(e.to_string()),
        }
    }

    /// Describes an image block's diagram and records the description beside it.
    pub fn describe_diagram(
        &self,
        document: &mut Document,
        block: BlockId,
        asset_dir: &Path,
        now: i64,
    ) -> Outcome {
        if !self.policy.enabled {
            return Outcome::Off;
        }
        let Some(target) = document.block(block) else {
            return Outcome::NothingToWorkOn(format!("no block {block}"));
        };
        let BlockKind::Image { asset, alt_text } = &target.kind else {
            return Outcome::NothingToWorkOn("the block is not an image".into());
        };
        let Some(asset) = asset else {
            return Outcome::NothingToWorkOn("the image has no file to look at".into());
        };
        let image = asset_dir.join(asset);
        if !image.is_file() {
            return Outcome::NothingToWorkOn(format!(
                "the image file {} is missing",
                image.display()
            ));
        }
        let context = alt_text.clone().unwrap_or_default();

        let Some(helper) = self.available().find(|h| h.can_describe()) else {
            let alternative =
                "Ask someone to describe the diagram. Cloud helpers can be turned on in Settings.";
            return if self.cloud_could(|h| h.can_describe()) {
                self.needs_cloud("describe this diagram", alternative)
            } else {
                Outcome::NeedsCloud {
                    reason: "No helper can describe diagrams.".into(),
                    alternative: alternative.into(),
                }
            };
        };
        match helper.describe_diagram(&image, &context) {
            Ok(text) => self.record(
                document,
                block,
                AnnotationKind::DiagramDescription,
                text,
                Provenance {
                    helper: helper.name(),
                    model: helper.model(),
                    ran_on: helper.runs_on(),
                    at: now,
                },
            ),
            Err(e) => Outcome::Failed(e.to_string()),
        }
    }

    fn record(
        &self,
        document: &mut Document,
        block: BlockId,
        kind: AnnotationKind,
        text: String,
        provenance: Provenance,
    ) -> Outcome {
        let annotation = Annotation {
            id: 0,
            block,
            kind,
            text,
            original_text: String::new(),
            provenance,
        };
        match document.annotate(annotation) {
            Ok(id) => Outcome::Added(
                document
                    .annotations
                    .iter()
                    .find(|a| a.id == id)
                    .cloned()
                    .expect("just added"),
            ),
            Err(e) => Outcome::Failed(e.to_string()),
        }
    }
}

/// Why a task that only a cloud helper could do did not run, worded the same wherever it is said.
pub(crate) fn needs_cloud(policy: &Policy, task: &str) -> String {
    if !policy.allow_cloud {
        format!(
            "No helper on this device can {task}. A cloud helper could, but sending data to the cloud is turned off."
        )
    } else if !policy.paid_tier {
        format!("No helper on this device can {task}. Cloud helpers are part of the paid tier.")
    } else {
        format!("No helper can {task}.")
    }
}

/// Every number, unit and symbol in the original must appear unchanged in the explanation.
pub fn numbers_kept(original: &str, explanation: &str) -> bool {
    let is_numeric = |t: &str| t.chars().any(|c| c.is_ascii_digit());
    let clean = |t: &str| {
        t.trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':' | '(' | ')'))
            .to_string()
    };
    let explained: Vec<String> = explanation.split_whitespace().map(clean).collect();
    original
        .split_whitespace()
        .map(clean)
        .filter(|t| is_numeric(t))
        .all(|t| explained.contains(&t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Origin, Rect, Run, Source, SourceFormat};

    struct CloudExplainer;
    impl Helper for CloudExplainer {
        fn name(&self) -> String {
            "cloud model".into()
        }
        fn runs_on(&self) -> RanOn {
            RanOn::Cloud
        }
        fn model(&self) -> Option<String> {
            Some("large-model-1".into())
        }
        fn can_explain(&self) -> bool {
            true
        }
        fn explain_simply(&self, text: &str) -> Result<String, AssistError> {
            Ok(format!("In short: {text}"))
        }
    }

    struct Careless;
    impl Helper for Careless {
        fn name(&self) -> String {
            "careless".into()
        }
        fn runs_on(&self) -> RanOn {
            RanOn::Device
        }
        fn can_explain(&self) -> bool {
            true
        }
        fn explain_simply(&self, _text: &str) -> Result<String, AssistError> {
            Ok("The resistor is about 5 kilo-ohms.".into())
        }
    }

    fn origin() -> Origin {
        Origin::new(1, Rect::new(0.0, 0.0, 10.0, 10.0))
    }

    fn doc() -> Document {
        let mut d = Document::new(
            "Notes",
            Source {
                file_name: "n.pdf".into(),
                format: SourceFormat::Pdf,
            },
        );
        d.push(
            BlockKind::paragraph(vec![Run::native(
                "The resistor, which is 4.7 kΩ, limits the current; therefore the LED is protected.",
                origin(),
            )]),
            origin(),
        );
        d.push(
            BlockKind::Equation {
                runs: vec![Run::native("P = I^2 R", origin())],
                latex: Some("P = I^2 R".into()),
                mathml: None,
                display: true,
            },
            origin(),
        );
        d.push(
            BlockKind::Image {
                alt_text: None,
                asset: Some("missing.png".into()),
            },
            origin(),
        );
        d
    }

    #[test]
    fn helpers_are_off_by_default() {
        let mut d = doc();
        let assistant = Assistant::new(Policy::default()).with_helper(Box::new(SimplifyHelper));
        assert_eq!(
            assistant.explain_simply(&mut d, BlockId(1), 0),
            Outcome::Off
        );
        assert!(d.annotations.is_empty());
    }

    #[test]
    fn a_device_helper_runs_first_and_its_result_is_labelled_beside_the_original() {
        let mut d = doc();
        let policy = Policy {
            enabled: true,
            allow_cloud: true,
            paid_tier: true,
        };
        let assistant = Assistant::new(policy)
            .with_helper(Box::new(CloudExplainer))
            .with_helper(Box::new(SimplifyHelper));
        let Outcome::Added(annotation) = assistant.explain_simply(&mut d, BlockId(1), 42) else {
            panic!()
        };
        assert_eq!(annotation.provenance.ran_on, RanOn::Device);
        assert_eq!(annotation.provenance.helper, "rule-based simplifier");
        assert!(annotation.text.contains("4.7 kΩ"), "{}", annotation.text);
        assert!(
            annotation.text.contains("so the LED"),
            "{}",
            annotation.text
        );
        assert_eq!(
            annotation.original_text,
            d.block(BlockId(1)).unwrap().text()
        );
        assert!(
            annotation
                .label()
                .contains("generated by rule-based simplifier on this device")
        );
        assert_eq!(d.annotations_for(BlockId(1)).count(), 1);
        // the content itself is untouched
        assert!(d.block(BlockId(1)).unwrap().text().contains("therefore"));
    }

    #[test]
    fn the_cloud_is_used_only_when_allowed_and_paid_for() {
        let mut d = doc();
        for (allow_cloud, paid_tier, expect_cloud) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, true, true),
        ] {
            let policy = Policy {
                enabled: true,
                allow_cloud,
                paid_tier,
            };
            let assistant = Assistant::new(policy).with_helper(Box::new(CloudExplainer));
            match assistant.explain_simply(&mut d, BlockId(1), 0) {
                Outcome::Added(a) => {
                    assert!(
                        expect_cloud,
                        "cloud ran with allow_cloud={allow_cloud} paid={paid_tier}"
                    );
                    assert_eq!(a.provenance.ran_on, RanOn::Cloud);
                    assert_eq!(a.provenance.model.as_deref(), Some("large-model-1"));
                }
                Outcome::NeedsCloud {
                    reason,
                    alternative,
                } => {
                    assert!(!expect_cloud);
                    assert!(reason.to_lowercase().contains("cloud"), "{reason}");
                    assert!(!alternative.is_empty());
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn an_explanation_that_changes_a_number_is_refused() {
        let mut d = doc();
        let assistant = Assistant::new(Policy {
            enabled: true,
            ..Policy::default()
        })
        .with_helper(Box::new(Careless));
        assert!(
            matches!(assistant.explain_simply(&mut d, BlockId(1), 0), Outcome::Failed(m) if m.contains("changed a number"))
        );
        assert!(d.annotations.is_empty());
        assert!(numbers_kept(
            "R1 = 4.7 kΩ at 25 °C",
            "R1 is 4.7 kΩ at 25 °C."
        ));
        assert!(!numbers_kept("R1 = 4.7 kΩ", "R1 is 4.8 kΩ"));
    }

    #[test]
    fn an_equation_is_explained_by_its_exact_reading() {
        let mut d = doc();
        let assistant = Assistant::new(Policy {
            enabled: true,
            ..Policy::default()
        });
        let Outcome::Added(a) = assistant.explain_simply(&mut d, BlockId(2), 0) else {
            panic!()
        };
        assert_eq!(a.text, "Read aloud, this says: P equals I squared R.");
        assert_eq!(a.provenance.helper, "equation reader");
    }

    #[test]
    fn a_diagram_needs_its_file_and_a_helper_that_can_see() {
        let mut d = doc();
        let dir = tempfile::tempdir().unwrap();
        let assistant = Assistant::new(Policy {
            enabled: true,
            ..Policy::default()
        })
        .with_helper(Box::new(SimplifyHelper));
        assert!(
            matches!(assistant.describe_diagram(&mut d, BlockId(3), dir.path(), 0), Outcome::NothingToWorkOn(m) if m.contains("missing"))
        );
        std::fs::write(dir.path().join("missing.png"), b"not really an image").unwrap();
        assert!(
            matches!(assistant.describe_diagram(&mut d, BlockId(3), dir.path(), 0), Outcome::NeedsCloud { reason, .. } if reason.contains("No helper can describe"))
        );
        assert!(matches!(
            assistant.describe_diagram(&mut d, BlockId(1), dir.path(), 0),
            Outcome::NothingToWorkOn(_)
        ));
    }

    #[test]
    fn annotations_round_trip_through_json_and_can_be_removed() {
        let mut d = doc();
        let assistant = Assistant::new(Policy {
            enabled: true,
            ..Policy::default()
        })
        .with_helper(Box::new(SimplifyHelper));
        let Outcome::Added(a) = assistant.explain_simply(&mut d, BlockId(1), 7) else {
            panic!()
        };
        let back = Document::from_json(&d.to_json().unwrap()).unwrap();
        assert_eq!(back.annotations, d.annotations);
        assert!(d.remove_annotation(a.id));
        assert!(!d.remove_annotation(a.id));
        assert!(d.annotations.is_empty());
    }
}
