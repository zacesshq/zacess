//! Study sessions, revision paths and spaced repetition: the core logic and data model.
//!
//! Flashcards are tied to the place in a document they came from and scheduled with FSRS.
//! Reading goals measure a day or a rolling week and report progress without judgement. Reminders
//! say what is ready and never mention streaks. Everything exports to the text formats Anki and
//! Notion import.

pub mod fsrs;

use crate::library::ItemId;
use crate::model::{BlockId, BlockKind, Document};
pub use fsrs::{Memory, Rating, Scheduler};
use serde::{Deserialize, Serialize};

const DAY: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StudyError {
    #[error("no card {0}")]
    NoSuchCard(u64),
    #[error("the revision path is finished")]
    PathFinished,
    #[error("the document has no block {0}")]
    NoSuchBlock(BlockId),
}

/// Where a card or a step came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub document: ItemId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<BlockId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

impl Place {
    pub fn citation(&self) -> String {
        match self.page {
            Some(page) => format!("{}, page {page}", self.title),
            None => self.title.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewLog {
    pub at: i64,
    pub rating: Rating,
    pub interval_days: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub id: u64,
    pub front: String,
    pub back: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// `None` until the first review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<Memory>,
    /// When the card is next due, Unix seconds. A new card is due at once.
    pub due: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviews: Vec<ReviewLog>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suspended: bool,
}

/// A set of cards with its scheduler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deck {
    pub name: String,
    pub scheduler: Scheduler,
    pub cards: Vec<Card>,
}

impl Deck {
    pub fn new(name: &str) -> Self {
        Deck {
            name: name.into(),
            scheduler: Scheduler::default(),
            cards: Vec::new(),
        }
    }

    /// Adds a card, due now.
    pub fn add_card(&mut self, front: &str, back: &str, place: Option<Place>, now: i64) -> u64 {
        let id = self.cards.iter().map(|c| c.id + 1).max().unwrap_or(1);
        self.cards.push(Card {
            id,
            front: front.into(),
            back: back.into(),
            place,
            tags: Vec::new(),
            memory: None,
            due: now,
            reviews: Vec::new(),
            suspended: false,
        });
        id
    }

    /// Adds a card made from a block of a document, with its place filled from the document so the
    /// card can open its source.
    pub fn add_card_from(
        &mut self,
        document_id: ItemId,
        document: &Document,
        block: BlockId,
        front: &str,
        back: &str,
        now: i64,
    ) -> Result<u64, StudyError> {
        let found = document
            .block(block)
            .ok_or(StudyError::NoSuchBlock(block))?;
        let place = Place {
            document: document_id,
            title: document.title.clone(),
            block: Some(block),
            page: Some(found.origin.page),
        };
        Ok(self.add_card(front, back, Some(place), now))
    }

    pub fn card(&self, id: u64) -> Option<&Card> {
        self.cards.iter().find(|c| c.id == id)
    }

    /// Cards ready to review, the longest overdue first.
    pub fn due(&self, now: i64) -> Vec<&Card> {
        let mut due: Vec<&Card> = self
            .cards
            .iter()
            .filter(|c| !c.suspended && c.due <= now)
            .collect();
        due.sort_by_key(|c| (c.due, c.id));
        due
    }

    /// Records a review and schedules the card again.
    pub fn review(&mut self, id: u64, rating: Rating, now: i64) -> Result<&Card, StudyError> {
        let scheduler = self.scheduler.clone();
        let card = self
            .cards
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or(StudyError::NoSuchCard(id))?;
        let scheduled = match (&card.memory, card.reviews.last()) {
            (Some(memory), Some(last)) => {
                let elapsed = (now - last.at) as f64 / DAY as f64;
                scheduler.review(memory, rating, elapsed)
            }
            _ => scheduler.first_review(rating),
        };
        card.memory = Some(scheduled.memory);
        card.due = now + scheduled.interval_days as i64 * DAY;
        card.reviews.push(ReviewLog {
            at: now,
            rating,
            interval_days: scheduled.interval_days,
        });
        Ok(card)
    }

    /// Records a review as part of a session, so the review counts towards card goals.
    pub fn review_in(
        &mut self,
        id: u64,
        rating: Rating,
        now: i64,
        session: &mut Session,
    ) -> Result<&Card, StudyError> {
        let card = self.review(id, rating, now)?;
        session.cards_reviewed = session.cards_reviewed.saturating_add(1);
        Ok(card)
    }

    /// The chance of recalling a card now. `None` before its first review.
    pub fn recall(&self, id: u64, now: i64) -> Option<f64> {
        let card = self.card(id)?;
        let memory = card.memory.as_ref()?;
        let last = card.reviews.last()?.at;
        Some(Scheduler::retrievability(
            memory,
            (now - last) as f64 / DAY as f64,
        ))
    }

    pub fn suspend(&mut self, id: u64, suspended: bool) -> Result<(), StudyError> {
        let card = self
            .cards
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or(StudyError::NoSuchCard(id))?;
        card.suspended = suspended;
        Ok(())
    }

    /// Cards in the tab-separated text Anki imports, as HTML, with the header lines that tell it
    /// how.
    pub fn to_anki_text(&self) -> String {
        self.to_anki_text_with(&AnkiOptions::default())
    }

    /// Cards in the tab-separated text Anki imports. Each line is a GUID, the front, the back with
    /// the source under it and the tags. The GUID stays the same on every export, so importing
    /// again updates the notes instead of adding copies.
    pub fn to_anki_text_with(&self, options: &AnkiOptions) -> String {
        let mut out = format!("#separator:tab\n#html:{}\n#notetype:Basic\n", options.html);
        if let Some(deck) = options.deck.as_deref().map(one_line)
            && !deck.is_empty()
        {
            out.push_str(&format!("#deck:{deck}\n"));
        }
        out.push_str("#guid column:1\n#tags column:4\n#columns:GUID\tFront\tBack\tTags\n");
        let deck_hash = crate::library::content_hash(&self.name);
        for card in &self.cards {
            let tags = card
                .tags
                .iter()
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join("_"))
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let source = card.place.as_ref().map(Place::citation);
            let fields = if options.html {
                let mut back = anki_html(&card.back);
                if let Some(source) = source {
                    back.push_str(&format!(
                        "<br><small>Source: {}</small>",
                        anki_html(&source)
                    ));
                }
                // tags are never HTML, but a quote would still open a quoted field
                [anki_html(&card.front), back, anki_plain(&tags)]
            } else {
                let mut back = card.back.clone();
                if let Some(source) = source {
                    back.push_str(&format!("\nSource: {source}"));
                }
                [
                    anki_plain(&card.front),
                    anki_plain(&back),
                    anki_plain(&tags),
                ]
            };
            let [front, back, tags] = fields;
            out.push_str(&format!(
                "zacess-{deck_hash}-{}\t{front}\t{back}\t{tags}\n",
                card.id
            ));
        }
        out
    }

    /// Cards as the CSV Notion imports into a database, one property per column. Tags are one
    /// comma-separated cell, which Notion reads as a multi-select.
    pub fn to_notion_csv(&self, now: i64) -> String {
        let mut out = String::from("Name,Answer,Source,Due,Reviews,Recall,Tags,Deck\n");
        for card in &self.cards {
            let recall = self
                .recall(card.id, now)
                .map(|r| format!("{:.0}%", r * 100.0))
                .unwrap_or_default();
            // a comma inside a tag would split it into two options in Notion
            let tags = card
                .tags
                .iter()
                .map(|t| t.replace(',', " ").trim().to_string())
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(",");
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                csv(&card.front),
                csv(&card.back),
                csv(&card.place.as_ref().map(Place::citation).unwrap_or_default()),
                csv(&iso_date(card.due)),
                card.reviews.len(),
                recall,
                csv(&tags),
                csv(&self.name)
            ));
        }
        out
    }
}

/// How cards are written for Anki.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnkiOptions {
    /// The deck Anki puts the cards in. `None` leaves the choice to the import screen.
    pub deck: Option<String>,
    /// Fields as HTML, so line breaks and the source survive. Off, fields are plain text quoted
    /// where they need it and Anki escapes them itself.
    pub html: bool,
}

impl Default for AnkiOptions {
    fn default() -> Self {
        AnkiOptions {
            deck: None,
            html: true,
        }
    }
}

/// Text for one HTML field: escaped, with line breaks as `<br>` and never a tab.
fn anki_html(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        // a field opening with a quote would be read as a quoted field
        .replace('"', "&quot;")
        .replace(['\r', '\n'], "<br>")
        .replace('\t', " ")
}

/// Text for one plain field: quoted when it holds a tab, a line break or a quote, as Anki's
/// reader expects.
fn anki_plain(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    if text.contains(['\t', '\n', '"']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Words joined as "a, b and c".
fn list_words(words: &[String]) -> String {
    let words: Vec<String> = words.iter().map(|w| one_line(w)).collect();
    match words.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

fn csv(text: &str) -> String {
    let text = text.replace('\t', " ");
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// A date as `YYYY-MM-DD` from Unix seconds.
pub fn iso_date(secs: i64) -> String {
    let days = secs.div_euclid(DAY);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

// --- reading goals and sessions ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Measure {
    Minutes,
    Blocks,
    Pages,
    Cards,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    /// Since the start of today, local time as the plan's offset gives it.
    Day,
    /// The last seven days, rolling, so a quiet day never resets anything.
    Week,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goal {
    pub measure: Measure,
    pub target: u32,
    pub period: Period,
}

/// One sitting with a document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub document: ItemId,
    pub started: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks_read: Vec<BlockId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages_read: Vec<u32>,
    #[serde(default)]
    pub cards_reviewed: u32,
}

impl Session {
    pub fn start(document: ItemId, now: i64) -> Self {
        Session {
            document,
            started: now,
            ended: None,
            blocks_read: Vec::new(),
            pages_read: Vec::new(),
            cards_reviewed: 0,
        }
    }

    pub fn read(&mut self, block: BlockId, page: u32) {
        if !self.blocks_read.contains(&block) {
            self.blocks_read.push(block);
        }
        if !self.pages_read.contains(&page) {
            self.pages_read.push(page);
        }
    }

    pub fn end(&mut self, now: i64) {
        self.ended = Some(now.max(self.started));
    }

    pub fn minutes(&self, now: i64) -> u32 {
        let end = self.ended.unwrap_or(now).max(self.started);
        ((end - self.started) / 60) as u32
    }
}

/// How far along a goal is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub goal: Goal,
    pub done: u32,
    /// From 0 to 1, capped at 1.
    pub fraction: f32,
    pub reached: bool,
}

/// The person's goals, the sessions that count towards them and the revision paths under way.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// What the plan is called in the library. Empty for a plan saved before plans had names.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub goals: Vec<Goal>,
    pub sessions: Vec<Session>,
    /// When the last reminder was given, so there is never more than one a day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reminder: Option<i64>,
    /// Revision paths, each keeping its own position.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<RevisionPath>,
    /// The person's offset from UTC in minutes, so a day starts at their midnight. Zero is UTC,
    /// which a plan saved before the offset existed reads as.
    #[serde(default)]
    pub utc_offset_minutes: i32,
}

/// The local day number of a time: days since 1 January 1970 in the zone `utc_offset_minutes`
/// ahead of UTC.
pub fn local_day(now: i64, utc_offset_minutes: i32) -> i64 {
    (now + utc_offset_minutes as i64 * 60).div_euclid(DAY)
}

impl Plan {
    /// The local day number of a time in this plan's zone.
    pub fn local_day(&self, now: i64) -> i64 {
        local_day(now, self.utc_offset_minutes)
    }

    fn window_start(&self, period: Period, now: i64) -> i64 {
        match period {
            // local midnight, back in UTC seconds
            Period::Day => self.local_day(now) * DAY - self.utc_offset_minutes as i64 * 60,
            Period::Week => now - 7 * DAY,
        }
    }

    pub fn progress(&self, now: i64) -> Vec<Progress> {
        self.goals
            .iter()
            .map(|goal| {
                let start = self.window_start(goal.period, now);
                let counted = self
                    .sessions
                    .iter()
                    .filter(|s| s.ended.unwrap_or(now) >= start);
                let done: u32 = counted
                    .map(|s| match goal.measure {
                        Measure::Minutes => s.minutes(now),
                        Measure::Blocks => s.blocks_read.len() as u32,
                        Measure::Pages => s.pages_read.len() as u32,
                        Measure::Cards => s.cards_reviewed,
                    })
                    .sum();
                let fraction = if goal.target == 0 {
                    1.0
                } else {
                    (done as f32 / goal.target as f32).min(1.0)
                };
                Progress {
                    goal: goal.clone(),
                    done,
                    fraction,
                    reached: done >= goal.target,
                }
            })
            .collect()
    }
}

// --- revision paths ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    /// Read from one heading to the next.
    Read {
        heading: String,
        from: BlockId,
        to: BlockId,
        page: u32,
    },
    /// Review the cards made from that section.
    Review { heading: String, cards: Vec<u64> },
    /// Look back over the headings covered.
    Recap { headings: Vec<String> },
}

/// A path through a document: read a section, review its cards and recap at the end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevisionPath {
    pub document: ItemId,
    /// The document's title when the path was made. Empty for a path saved before paths had one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    pub steps: Vec<Step>,
    pub position: usize,
}

impl RevisionPath {
    /// Builds a path from a document's headings and the deck's cards for it. A document without
    /// headings is one section.
    pub fn from_document(document_id: ItemId, document: &Document, deck: &Deck) -> Self {
        let mut sections: Vec<(String, BlockId, BlockId, u32)> = Vec::new();
        let content: Vec<&crate::model::Block> = document
            .blocks
            .iter()
            .filter(|b| !matches!(b.kind, BlockKind::PageBreak))
            .collect();
        for block in &content {
            if let BlockKind::Heading { .. } = block.kind {
                sections.push((block.text(), block.id, block.id, block.origin.page));
            } else if let Some(last) = sections.last_mut() {
                last.2 = block.id;
            } else if let Some(first) = content.first() {
                sections.push((
                    document.title.clone(),
                    first.id,
                    block.id,
                    first.origin.page,
                ));
            }
        }
        let mut steps = Vec::new();
        for (heading, from, to, page) in &sections {
            steps.push(Step::Read {
                heading: heading.clone(),
                from: *from,
                to: *to,
                page: *page,
            });
            let cards: Vec<u64> = deck
                .cards
                .iter()
                .filter(|c| {
                    c.place.as_ref().is_some_and(|p| {
                        p.document == document_id
                            && p.block.is_some_and(|b| b.0 >= from.0 && b.0 <= to.0)
                    })
                })
                .map(|c| c.id)
                .collect();
            if !cards.is_empty() {
                steps.push(Step::Review {
                    heading: heading.clone(),
                    cards,
                });
            }
        }
        if sections.len() > 1 {
            steps.push(Step::Recap {
                headings: sections.iter().map(|s| s.0.clone()).collect(),
            });
        }
        RevisionPath {
            document: document_id,
            title: document.title.clone(),
            steps,
            position: 0,
        }
    }

    pub fn current(&self) -> Option<&Step> {
        self.steps.get(self.position)
    }

    pub fn complete_step(&mut self) -> Result<Option<&Step>, StudyError> {
        if self.position >= self.steps.len() {
            return Err(StudyError::PathFinished);
        }
        self.position += 1;
        Ok(self.steps.get(self.position))
    }

    pub fn is_finished(&self) -> bool {
        self.position >= self.steps.len()
    }

    pub fn fraction_done(&self) -> f32 {
        if self.steps.is_empty() {
            1.0
        } else {
            self.position as f32 / self.steps.len() as f32
        }
    }

    /// The path as a Markdown checklist Notion imports as a page: steps already done are ticked
    /// and each review lists the fronts of its cards.
    pub fn to_notion_markdown(&self, deck: &Deck) -> String {
        let title = if self.title.trim().is_empty() {
            "Revision path".to_string()
        } else {
            one_line(&self.title)
        };
        let mut out = format!("# {title}\n\n");
        for (index, step) in self.steps.iter().enumerate() {
            let tick = if index < self.position { "x" } else { " " };
            let text = match step {
                Step::Read { heading, page, .. } => {
                    format!("Read {} (page {page})", one_line(heading))
                }
                Step::Review { heading, cards } => match cards.len() {
                    1 => format!("Review 1 card from {}", one_line(heading)),
                    n => format!("Review {n} cards from {}", one_line(heading)),
                },
                Step::Recap { headings } => format!("Recap {}", list_words(headings)),
            };
            out.push_str(&format!("- [{tick}] {text}\n"));
            if let Step::Review { cards, .. } = step {
                for card in cards.iter().filter_map(|id| deck.card(*id)) {
                    out.push_str(&format!("    - {}\n", one_line(&card.front)));
                }
            }
        }
        out
    }
}

// --- reminders ----------------------------------------------------------------------------------

/// What a reminder says. It names what is ready and never counts days missed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reminder {
    pub message: String,
    pub due_cards: usize,
    pub goals_reached: usize,
    pub goals_total: usize,
}

/// Words a reminder never uses: they turn a habit into a debt.
pub const PRESSURE_WORDS: &[&str] = &[
    "streak", "missed", "broke", "broken", "lost", "behind", "failed", "must", "late",
];

impl Plan {
    /// A reminder for now. `None` when one was already given today or there is nothing to say.
    pub fn reminder(&mut self, deck: &Deck, now: i64) -> Option<Reminder> {
        let today = self.local_day(now);
        if self
            .last_reminder
            .is_some_and(|t| self.local_day(t) == today)
        {
            return None;
        }
        let due = deck.due(now).len();
        let progress = self.progress(now);
        let reached = progress.iter().filter(|p| p.reached).count();
        let open: Vec<&Progress> = progress.iter().filter(|p| !p.reached).collect();
        if due == 0 && open.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        match due {
            0 => {}
            1 => parts.push("1 card is ready when you want it.".to_string()),
            n => parts.push(format!("{n} cards are ready when you want them.")),
        }
        if let Some(p) = open.first() {
            let left = p.goal.target.saturating_sub(p.done);
            let unit = match p.goal.measure {
                Measure::Minutes => "minutes",
                Measure::Blocks => "blocks",
                Measure::Pages => "pages",
                Measure::Cards => "cards",
            };
            let period = match p.goal.period {
                Period::Day => "today",
                Period::Week => "this week",
            };
            parts.push(format!(
                "{left} {unit} would complete your reading goal for {period}. Any amount counts."
            ));
        }
        if reached > 0 && reached == progress.len() {
            parts.push("Every goal is met. Nothing more is needed.".to_string());
        }
        self.last_reminder = Some(now);
        Some(Reminder {
            message: parts.join(" "),
            due_cards: due,
            goals_reached: reached,
            goals_total: progress.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Origin, Rect, Run, Source, SourceFormat};

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
                "Any periodic signal is a sum of sinusoids.",
                origin(),
            )]),
            origin(),
        );
        d.push(
            BlockKind::heading(2, vec![Run::native("Worked example", origin())]).unwrap(),
            origin(),
        );
        d.push(
            BlockKind::paragraph(vec![Run::native("The third harmonic is 150 Hz.", origin())]),
            origin(),
        );
        d
    }

    fn place(block: u64) -> Place {
        Place {
            document: ItemId("d-1".into()),
            title: "Week 3".into(),
            block: Some(BlockId(block)),
            page: Some(1),
        }
    }

    #[test]
    fn cards_are_scheduled_with_fsrs_and_come_due_in_order() {
        let mut deck = Deck::new("Signals");
        let a = deck.add_card("Third harmonic of 50 Hz?", "150 Hz", Some(place(4)), 1_000);
        let b = deck.add_card(
            "Periodic signal is a sum of?",
            "Sinusoids",
            Some(place(2)),
            1_000,
        );
        assert_eq!(
            deck.due(1_000).iter().map(|c| c.id).collect::<Vec<_>>(),
            [a, b]
        );
        assert_eq!(deck.recall(a, 1_000), None);

        let card = deck.review(a, Rating::Good, 1_000).unwrap();
        assert_eq!(card.due, 1_000 + 3 * DAY);
        assert_eq!(
            deck.due(1_000).iter().map(|c| c.id).collect::<Vec<_>>(),
            [b]
        );
        assert!(deck.recall(a, 1_000).unwrap() > 0.99);
        assert!(deck.recall(a, 1_000 + 3 * DAY).unwrap() < 0.95);

        deck.review(b, Rating::Again, 1_000).unwrap();
        assert_eq!(deck.card(b).unwrap().due, 1_000 + DAY);
        deck.suspend(b, true).unwrap();
        assert!(deck.due(1_000 + 10 * DAY).iter().all(|c| c.id != b));
        assert!(matches!(
            deck.review(99, Rating::Good, 0),
            Err(StudyError::NoSuchCard(99))
        ));
        let json = serde_json::to_string(&deck).unwrap();
        assert_eq!(serde_json::from_str::<Deck>(&json).unwrap(), deck);
    }

    #[test]
    fn goals_measure_today_or_a_rolling_week_without_resetting() {
        let now = 20 * DAY + 3_600;
        let mut plan = Plan {
            goals: vec![
                Goal {
                    measure: Measure::Minutes,
                    target: 30,
                    period: Period::Day,
                },
                Goal {
                    measure: Measure::Blocks,
                    target: 10,
                    period: Period::Week,
                },
            ],
            ..Default::default()
        };
        let mut yesterday = Session::start(ItemId("d-1".into()), now - DAY);
        for i in 0..6 {
            yesterday.read(BlockId(i), 1);
        }
        yesterday.end(now - DAY + 25 * 60);
        plan.sessions.push(yesterday);
        let mut today = Session::start(ItemId("d-1".into()), now - 20 * 60);
        today.read(BlockId(1), 1);
        today.read(BlockId(1), 1);
        plan.sessions.push(today);

        let progress = plan.progress(now);
        assert_eq!(
            (progress[0].done, progress[0].reached),
            (20, false),
            "only today counts for a daily goal"
        );
        assert_eq!(
            (progress[1].done, progress[1].reached),
            (7, false),
            "the week counts both days"
        );
        assert!((progress[0].fraction - 20.0 / 30.0).abs() < 1e-6);
        // a quiet day later, nothing is lost from the weekly count
        assert_eq!(plan.progress(now + DAY)[1].done, 7);
    }

    #[test]
    fn a_revision_path_reads_each_section_then_reviews_its_cards() {
        let mut deck = Deck::new("Signals");
        deck.add_card("Q", "A", Some(place(4)), 0);
        let mut path = RevisionPath::from_document(ItemId("d-1".into()), &doc(), &deck);
        let kinds: Vec<&str> = path
            .steps
            .iter()
            .map(|s| match s {
                Step::Read { .. } => "read",
                Step::Review { .. } => "review",
                Step::Recap { .. } => "recap",
            })
            .collect();
        assert_eq!(kinds, ["read", "read", "review", "recap"]);
        assert!(
            matches!(path.current(), Some(Step::Read { heading, from, to, .. }) if heading == "Fourier series" && *from == BlockId(1) && *to == BlockId(2))
        );
        path.complete_step().unwrap();
        path.complete_step().unwrap();
        assert!(matches!(path.current(), Some(Step::Review { cards, .. }) if cards == &[1]));
        assert!((path.fraction_done() - 0.5).abs() < 1e-6);
        path.complete_step().unwrap();
        assert_eq!(path.complete_step().unwrap(), None);
        assert!(path.is_finished());
        assert_eq!(path.complete_step(), Err(StudyError::PathFinished));

        let no_headings = Document::new(
            "Plain",
            Source {
                file_name: "p.pdf".into(),
                format: SourceFormat::Pdf,
            },
        );
        assert!(
            RevisionPath::from_document(ItemId("d-2".into()), &no_headings, &deck)
                .steps
                .is_empty()
        );
    }

    #[test]
    fn reminders_name_what_is_ready_and_never_apply_pressure() {
        let now = 50 * DAY;
        let mut deck = Deck::new("Signals");
        deck.add_card("Q1", "A1", None, now);
        deck.add_card("Q2", "A2", None, now);
        let mut plan = Plan {
            goals: vec![Goal {
                measure: Measure::Minutes,
                target: 30,
                period: Period::Day,
            }],
            ..Default::default()
        };
        let reminder = plan.reminder(&deck, now).unwrap();
        assert_eq!(reminder.due_cards, 2);
        assert!(
            reminder
                .message
                .starts_with("2 cards are ready when you want them."),
            "{}",
            reminder.message
        );
        assert!(
            reminder.message.contains(
                "30 minutes would complete your reading goal for today. Any amount counts."
            )
        );
        let lower = reminder.message.to_lowercase();
        for word in PRESSURE_WORDS {
            assert!(!lower.contains(word), "{word} in {lower}");
        }
        assert_eq!(
            plan.reminder(&deck, now + 3_600),
            None,
            "one reminder a day"
        );
        // days later with nothing due and goals met there is nothing to say
        let mut done = Session::start(ItemId("d-1".into()), now + 5 * DAY);
        done.end(now + 5 * DAY + 40 * 60);
        plan.sessions.push(done);
        let empty = Deck::new("Empty");
        assert_eq!(plan.reminder(&empty, now + 5 * DAY + 3_600), None);
    }

    #[test]
    fn exports_are_in_the_formats_anki_and_notion_import() {
        let mut deck = Deck::new("Signals");
        let id = deck.add_card(
            "Third harmonic of 50 Hz?",
            "150 Hz, \"odd\" only",
            Some(place(4)),
            86_400 * 20_000,
        );
        deck.cards[0].tags = vec!["signals".into(), "week 3".into()];
        deck.review(id, Rating::Good, 86_400 * 20_000).unwrap();
        let anki = deck.to_anki_text();
        assert!(anki.starts_with("#separator:tab\n#html:true\n#notetype:Basic\n"));
        assert!(
            anki.ends_with(
                "\tThird harmonic of 50 Hz?\t150 Hz, &quot;odd&quot; only<br><small>Source: Week 3, page 1</small>\tsignals week_3\n"
            ),
            "{anki}"
        );
        let csv = deck.to_notion_csv(86_400 * 20_000);
        assert!(csv.starts_with("Name,Answer,Source,Due,Reviews,Recall,Tags,Deck\n"));
        assert!(csv.contains("Third harmonic of 50 Hz?,\"150 Hz, \"\"odd\"\" only\",\"Week 3, page 1\",2024-10-07,1,100%,\"signals,week 3\",Signals\n"), "{csv}");
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(1_700_000_000), "2023-11-14");
    }

    fn tricky_deck() -> Deck {
        let mut deck = Deck::new("Signals & Systems");
        deck.add_card(
            "Is x < y\tor y > x?",
            "Both hold.\nThe second line\r\nand a third",
            Some(place(2)),
            0,
        );
        deck.add_card("#hash at the start", "\"quoted\" answer", None, 0);
        deck.cards[1].tags = vec!["week 3".into(), "maths, signals".into()];
        deck
    }

    #[test]
    fn the_anki_text_has_an_exact_header_and_four_fields_on_every_line() {
        let deck = tricky_deck();
        let text = deck.to_anki_text_with(&AnkiOptions {
            deck: Some("Year 2::Signals\nand more".into()),
            html: true,
        });
        let header = "#separator:tab\n#html:true\n#notetype:Basic\n#deck:Year 2::Signals and more\n#guid column:1\n#tags column:4\n#columns:GUID\tFront\tBack\tTags\n";
        assert!(text.starts_with(header), "{text}");
        let lines: Vec<&str> = text[header.len()..].lines().collect();
        assert_eq!(
            lines.len(),
            2,
            "one line per card, a multi-line answer included"
        );
        for line in &lines {
            assert_eq!(line.split('\t').count(), 4, "{line}");
            assert!(
                line.starts_with("zacess-"),
                "a line never opens as a comment"
            );
        }
        let fields: Vec<&str> = lines[0].split('\t').collect();
        assert_eq!(fields[1], "Is x &lt; y or y &gt; x?");
        assert_eq!(
            fields[2],
            "Both hold.<br>The second line<br>and a third<br><small>Source: Week 3, page 1</small>"
        );
        let second: Vec<&str> = lines[1].split('\t').collect();
        assert_eq!(second[2], "&quot;quoted&quot; answer");
        assert_eq!(second[3], "week_3 maths,_signals");

        // without a deck name the line is left out
        let plain_header = deck.to_anki_text();
        assert!(!plain_header.contains("#deck:"));
    }

    #[test]
    fn anki_guids_stay_the_same_on_every_export() {
        let mut deck = tricky_deck();
        let guids = |deck: &Deck| -> Vec<String> {
            deck.to_anki_text()
                .lines()
                .filter(|l| !l.starts_with('#'))
                .map(|l| l.split('\t').next().unwrap().to_string())
                .collect()
        };
        let first = guids(&deck);
        assert_eq!(first.len(), 2);
        assert_ne!(first[0], first[1]);
        assert!(first[0].starts_with("zacess-") && first[0].ends_with("-1"));
        // reviews and edits change the content, never the GUID
        deck.review(1, Rating::Good, 10).unwrap();
        deck.cards[0].back = "Changed".into();
        assert_eq!(guids(&deck), first);
        assert_eq!(guids(&deck.clone()), first);
        // another deck's card 1 is a different note
        let mut other = Deck::new("Other");
        other.add_card("Q", "A", None, 0);
        assert_ne!(guids(&other)[0], first[0]);
    }

    #[test]
    fn plain_anki_text_quotes_fields_so_lines_survive() {
        let deck = tricky_deck();
        let text = deck.to_anki_text_with(&AnkiOptions {
            deck: None,
            html: false,
        });
        assert!(text.starts_with("#separator:tab\n#html:false\n#notetype:Basic\n#guid column:1\n"));
        assert!(
            text.contains("\t\"Is x < y\tor y > x?\"\t\"Both hold.\nThe second line\nand a third\nSource: Week 3, page 1\"\t\n"),
            "{text}"
        );
        assert!(text.contains("\t#hash at the start\t\"\"\"quoted\"\" answer\"\t"));
    }

    #[test]
    fn notion_csv_quotes_what_needs_it_and_holds_no_tabs() {
        let deck = tricky_deck();
        let csv = deck.to_notion_csv(0);
        let mut lines = csv.split('\n');
        assert_eq!(
            lines.next(),
            Some("Name,Answer,Source,Due,Reviews,Recall,Tags,Deck")
        );
        assert!(!csv.contains('\t'));
        assert!(
            csv.contains("Is x < y or y > x?,\"Both hold.\nThe second line\r\nand a third\",\"Week 3, page 1\",1970-01-01,0,,,Signals & Systems\n"),
            "{csv}"
        );
        assert!(
            csv.contains(",\"week 3,maths  signals\",Signals & Systems\n"),
            "tags are one quoted cell and a comma inside a tag never splits it: {csv}"
        );
    }

    #[test]
    fn a_revision_path_exports_as_a_notion_checklist() {
        let mut deck = Deck::new("Signals");
        deck.add_card("What is the\tthird harmonic?", "150 Hz", Some(place(4)), 0);
        let mut path = RevisionPath::from_document(ItemId("d-1".into()), &doc(), &deck);
        path.complete_step().unwrap();
        let md = path.to_notion_markdown(&deck);
        assert_eq!(
            md,
            "# Week 3\n\n\
             - [x] Read Fourier series (page 1)\n\
             - [ ] Read Worked example (page 1)\n\
             - [ ] Review 1 card from Worked example\n\
             \x20   - What is the third harmonic?\n\
             - [ ] Recap Fourier series and Worked example\n"
        );
        assert!(!md.contains('\t'));
        path.complete_step().unwrap();
        path.complete_step().unwrap();
        path.complete_step().unwrap();
        assert_eq!(path.to_notion_markdown(&deck).matches("- [x]").count(), 4);
        assert_eq!(
            list_words(&["a".into(), "b".into(), "c".into()]),
            "a, b and c"
        );
        // a path saved before paths had a title still reads and still exports
        let old = r#"{"document":"d-1","steps":[],"position":0}"#;
        let old: RevisionPath = serde_json::from_str(old).unwrap();
        assert_eq!(old.to_notion_markdown(&deck), "# Revision path\n\n");
    }

    #[test]
    fn reviews_in_a_session_count_towards_a_cards_goal() {
        let now = 30 * DAY + 3_600;
        let mut deck = Deck::new("Signals");
        for i in 0..3 {
            deck.add_card(&format!("Q{i}"), "A", None, now);
        }
        let mut plan = Plan {
            goals: vec![Goal {
                measure: Measure::Cards,
                target: 3,
                period: Period::Day,
            }],
            ..Default::default()
        };
        let mut session = Session::start(ItemId("d-1".into()), now);
        for id in 1..=3 {
            deck.review_in(id, Rating::Good, now + id as i64, &mut session)
                .unwrap();
        }
        assert!(deck.review_in(99, Rating::Good, now, &mut session).is_err());
        assert_eq!(
            session.cards_reviewed, 3,
            "a failed review counts for nothing"
        );
        session.end(now + 600);
        plan.sessions.push(session);
        let progress = plan.progress(now + 700);
        assert_eq!((progress[0].done, progress[0].reached), (3, true));
    }

    #[test]
    fn the_day_starts_at_local_midnight() {
        // 22:30 UTC is 23:30 at an offset of +60 minutes, half an hour before local midnight
        let before = 40 * DAY + 22 * 3_600 + 30 * 60;
        let after = before + 3_600;
        assert_eq!(local_day(before, 60), 40);
        assert_eq!(local_day(after, 60), 41);
        assert_eq!(local_day(after, 0), 40);
        assert_eq!(local_day(DAY / 2, -13 * 60), -1);

        let mut plan = Plan {
            goals: vec![Goal {
                measure: Measure::Minutes,
                target: 30,
                period: Period::Day,
            }],
            utc_offset_minutes: 60,
            ..Default::default()
        };
        let mut late = Session::start(ItemId("d-1".into()), before - 20 * 60);
        late.end(before);
        plan.sessions.push(late);
        assert_eq!(
            plan.progress(before + 60)[0].done,
            20,
            "still the same local day"
        );
        assert_eq!(
            plan.progress(after)[0].done,
            0,
            "after local midnight the daily goal starts again"
        );
        let utc = Plan {
            utc_offset_minutes: 0,
            ..plan.clone()
        };
        assert_eq!(
            utc.progress(after)[0].done,
            20,
            "in UTC it is still the same day"
        );

        // one reminder a local day: the next is allowed once local midnight has passed
        let mut deck = Deck::new("Signals");
        deck.add_card("Q", "A", None, 0);
        assert!(plan.reminder(&deck, before).is_some());
        assert!(plan.reminder(&deck, before + 20 * 60).is_none());
        assert!(plan.reminder(&deck, after).is_some());
        let mut utc = utc;
        utc.last_reminder = None;
        assert!(utc.reminder(&deck, before).is_some());
        assert!(
            utc.reminder(&deck, after).is_none(),
            "the same UTC day gets no second reminder"
        );
    }

    #[test]
    fn plans_saved_before_paths_and_offsets_still_read() {
        let old = r#"{"goals":[{"measure":"cards","target":3,"period":"day"}],"sessions":[],"last_reminder":5}"#;
        let plan: Plan = serde_json::from_str(old).unwrap();
        assert_eq!(plan.utc_offset_minutes, 0);
        assert!(plan.paths.is_empty() && plan.name.is_empty());
        assert_eq!(plan.last_reminder, Some(5));
        let json = serde_json::to_string(&plan).unwrap();
        assert_eq!(serde_json::from_str::<Plan>(&json).unwrap(), plan);
    }

    #[test]
    fn a_card_from_a_document_points_at_its_block_and_page() {
        let document = doc();
        let mut deck = Deck::new("Signals");
        let id = deck
            .add_card_from(
                ItemId("d-1".into()),
                &document,
                BlockId(4),
                "Third harmonic?",
                "150 Hz",
                7,
            )
            .unwrap();
        assert_eq!(deck.card(id).unwrap().place, Some(place(4)));
        assert_eq!(deck.card(id).unwrap().due, 7);
        assert_eq!(
            deck.add_card_from(ItemId("d-1".into()), &document, BlockId(99), "Q", "A", 7),
            Err(StudyError::NoSuchBlock(BlockId(99)))
        );
    }
}
