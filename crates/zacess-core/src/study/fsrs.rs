//! The FSRS spaced-repetition scheduler (Free Spaced Repetition Scheduler, version 5).
//!
//! A card's memory is a stability `S`, the days until recall falls to 90%, plus a difficulty `D`
//! from 1 to 10. Each review with a rating updates both and gives the next interval for the
//! retention the person wants. The formulas and default weights are the published FSRS-5 ones.

use serde::{Deserialize, Serialize};

/// How a review went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rating {
    Again = 1,
    Hard = 2,
    Good = 3,
    Easy = 4,
}

impl Rating {
    fn grade(self) -> f64 {
        self as u8 as f64
    }
}

/// What the scheduler remembers about a card.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    /// Days until the chance of recall falls to 90%.
    pub stability: f64,
    /// From 1 (easy to remember) to 10 (hard).
    pub difficulty: f64,
}

/// The outcome of a review.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scheduled {
    pub memory: Memory,
    /// Days until the card is due again, at least 1.
    pub interval_days: u32,
}

/// The scheduler's parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scheduler {
    /// The 19 FSRS-5 weights.
    pub weights: [f64; 19],
    /// The chance of recall to aim for at review time, 0.7 to 0.97.
    pub desired_retention: f64,
    /// The longest interval the scheduler will give, in days.
    pub maximum_interval: u32,
}

const DECAY: f64 = -0.5;
const FACTOR: f64 = 19.0 / 81.0;

impl Default for Scheduler {
    fn default() -> Self {
        Scheduler {
            weights: [
                0.40255, 1.18385, 3.173, 15.69105, 7.1949, 0.5345, 1.4604, 0.0046, 1.54575, 0.1192,
                1.01925, 1.9395, 0.11, 0.29605, 2.2698, 0.2315, 2.9898, 0.51655, 0.6621,
            ],
            desired_retention: 0.9,
            maximum_interval: 365,
        }
    }
}

impl Scheduler {
    /// The chance of recalling a card `elapsed_days` after its last review.
    pub fn retrievability(memory: &Memory, elapsed_days: f64) -> f64 {
        (1.0 + FACTOR * elapsed_days.max(0.0) / memory.stability).powf(DECAY)
    }

    /// Days until recall falls to the desired retention.
    pub fn interval(&self, stability: f64) -> u32 {
        let days = stability / FACTOR * (self.desired_retention.powf(1.0 / DECAY) - 1.0);
        (days.round() as i64).clamp(1, self.maximum_interval as i64) as u32
    }

    fn initial_difficulty(&self, rating: Rating) -> f64 {
        let w = &self.weights;
        (w[4] - (w[5] * (rating.grade() - 1.0)).exp() + 1.0).clamp(1.0, 10.0)
    }

    /// The memory after a first review.
    pub fn first_review(&self, rating: Rating) -> Scheduled {
        let stability = self.weights[rating as usize - 1];
        let memory = Memory {
            stability,
            difficulty: self.initial_difficulty(rating),
        };
        Scheduled {
            memory,
            interval_days: self.interval(stability),
        }
    }

    /// The memory after a later review, `elapsed_days` after the previous one.
    pub fn review(&self, memory: &Memory, rating: Rating, elapsed_days: f64) -> Scheduled {
        let w = &self.weights;
        let r = Self::retrievability(memory, elapsed_days);
        let (s, d) = (memory.stability, memory.difficulty);

        // difficulty moves with the rating and drifts back towards the difficulty of an easy card
        let next_d = d - w[6] * (rating.grade() - 3.0);
        let difficulty =
            (w[7] * self.initial_difficulty(Rating::Easy) + (1.0 - w[7]) * next_d).clamp(1.0, 10.0);

        let stability = if elapsed_days < 1.0 {
            // a same-day review: the short-term formula
            s * (w[17] * (rating.grade() - 3.0 + w[18])).exp()
        } else if rating == Rating::Again {
            let forgotten =
                w[11] * d.powf(-w[12]) * ((s + 1.0).powf(w[13]) - 1.0) * (w[14] * (1.0 - r)).exp();
            forgotten.min(s)
        } else {
            let hard_penalty = if rating == Rating::Hard { w[15] } else { 1.0 };
            let easy_bonus = if rating == Rating::Easy { w[16] } else { 1.0 };
            s * ((w[8]).exp()
                * (11.0 - d)
                * s.powf(-w[9])
                * ((w[10] * (1.0 - r)).exp() - 1.0)
                * hard_penalty
                * easy_bonus
                + 1.0)
        };
        let stability = stability.clamp(0.01, 36500.0);
        let memory = Memory {
            stability,
            difficulty,
        };
        Scheduled {
            memory,
            interval_days: self.interval(stability),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_intervals_grow_with_the_rating() {
        let s = Scheduler::default();
        let intervals: Vec<u32> = [Rating::Again, Rating::Hard, Rating::Good, Rating::Easy]
            .iter()
            .map(|r| s.first_review(*r).interval_days)
            .collect();
        assert_eq!(intervals, [1, 1, 3, 16], "{intervals:?}");
        assert!(
            s.first_review(Rating::Again).memory.difficulty
                > s.first_review(Rating::Easy).memory.difficulty
        );
    }

    #[test]
    fn retrievability_starts_at_one_and_decays_to_the_target_at_the_interval() {
        let s = Scheduler::default();
        let memory = s.first_review(Rating::Good).memory;
        assert!((Scheduler::retrievability(&memory, 0.0) - 1.0).abs() < 1e-9);
        let at_interval = Scheduler::retrievability(&memory, s.interval(memory.stability) as f64);
        assert!((at_interval - 0.9).abs() < 0.03, "{at_interval}");
        assert!(Scheduler::retrievability(&memory, 30.0) < 0.6);
    }

    #[test]
    fn good_reviews_lengthen_intervals_and_forgetting_shortens_them() {
        let s = Scheduler::default();
        let mut memory = s.first_review(Rating::Good).memory;
        let mut last = s.interval(memory.stability);
        for _ in 0..4 {
            let next = s.review(&memory, Rating::Good, last as f64);
            assert!(
                next.interval_days > last,
                "{} then {}",
                last,
                next.interval_days
            );
            memory = next.memory;
            last = next.interval_days;
        }
        assert!(last > 30);
        let forgotten = s.review(&memory, Rating::Again, last as f64);
        assert!(forgotten.memory.stability < memory.stability);
        assert!(forgotten.interval_days < last);
        assert!(forgotten.memory.difficulty > memory.difficulty);
    }

    #[test]
    fn easy_beats_good_beats_hard_on_the_same_card() {
        let s = Scheduler::default();
        let memory = s
            .review(&s.first_review(Rating::Good).memory, Rating::Good, 3.0)
            .memory;
        let hard = s.review(&memory, Rating::Hard, 10.0).interval_days;
        let good = s.review(&memory, Rating::Good, 10.0).interval_days;
        let easy = s.review(&memory, Rating::Easy, 10.0).interval_days;
        assert!(hard < good && good < easy, "{hard} {good} {easy}");
        let same_day = s.review(&memory, Rating::Good, 0.0);
        assert!(same_day.memory.stability > memory.stability);
    }

    #[test]
    fn the_maximum_interval_and_retention_are_respected() {
        let s = Scheduler {
            maximum_interval: 10,
            ..Scheduler::default()
        };
        assert_eq!(s.first_review(Rating::Easy).interval_days, 10);
        let relaxed = Scheduler {
            desired_retention: 0.8,
            ..Scheduler::default()
        };
        assert!(
            relaxed.first_review(Rating::Good).interval_days
                > Scheduler::default()
                    .first_review(Rating::Good)
                    .interval_days
        );
    }
}
