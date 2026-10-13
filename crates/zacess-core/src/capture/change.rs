//! What changed between two captures of a board.
//!
//! Writing on a board sits in horizontal bands: lines of text, a drawing, a row of an equation.
//! Comparing ink pixel by pixel fails twice over, because a slightly different photo of the same
//! writing moves every stroke a little and because a rewritten line sits where the old one was.
//! So the two captures are lined up, each band of writing is reduced to its column profile, how
//! much ink stands in each column. A band is the same writing when the profiles match, and
//! writing added beside a line shows as a band that grew around the old profile. Ink in bands
//! that match is kept; the rest was added or removed.

use super::frame::InkMask;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeConfig {
    /// The largest shift tried when lining two captures up, in shrunk pixels.
    pub max_shift: i64,
    /// Two bands are the same writing when their column profiles correlate at least this well.
    pub same_above: f32,
    /// When less than this share of the new ink is new and less than this share of the old ink
    /// is gone, it is the same board photographed again.
    pub repeat_below: f32,
    /// When more than this share of the earlier ink is gone, it is a wiped or different board
    /// rather than a new version.
    pub new_board_above: f32,
    /// A band of writing with at least this much ink, in shrunk pixels, that appeared or vanished
    /// is a change however full the board is.
    pub min_band_ink: usize,
}

impl Default for ChangeConfig {
    fn default() -> Self {
        ChangeConfig {
            max_shift: 15,
            same_above: 0.8,
            repeat_below: 0.03,
            new_board_above: 0.6,
            min_band_ink: 40,
        }
    }
}

/// How two captures differ.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Change {
    /// The share of the new capture's ink that was not there before.
    pub added: f32,
    /// The share of the earlier capture's ink that is gone.
    pub removed: f32,
    /// Bands of writing that appeared, each with at least the minimum ink.
    pub added_bands: u32,
    /// Bands of writing that vanished, each with at least the minimum ink.
    pub removed_bands: u32,
    /// The shift, in shrunk pixels, that lined the two captures up.
    pub shift_x: i64,
    pub shift_y: i64,
}

impl Change {
    /// How far apart the two captures are, for picking the board a photo belongs to.
    pub fn fraction(&self) -> f32 {
        self.added + self.removed
    }
}

/// What a change means for the board's history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The same writing again: nothing to keep.
    Repeat,
    /// Writing was added, rubbed out or rewritten: a new version of the same board.
    NewVersion,
    /// Most of the earlier writing is gone: a wiped or different board.
    NewBoard,
}

/// A horizontal stretch of rows that holds writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    pub top: usize,
    pub bottom: usize,
    pub ink: usize,
}

/// Rows with ink grouped into bands. Gaps of fewer than `min_gap` rows stay inside a band, so the
/// dot of an i or the gap in an equals sign does not split a line. Bands of fewer than six pixels
/// or less than one part in a thousand of the ink are dropped as specks.
pub fn bands(mask: &InkMask, min_gap: usize) -> Vec<Band> {
    let rows = row_sums(mask);
    let total: usize = rows.iter().sum();
    let mut out: Vec<Band> = Vec::new();
    let mut current: Option<(usize, usize)> = None;
    let mut gap = 0usize;
    for (y, &count) in rows.iter().enumerate() {
        if count > 0 {
            match current {
                Some((top, _)) => current = Some((top, y + 1)),
                None => current = Some((y, y + 1)),
            }
            gap = 0;
        } else if let Some((top, bottom)) = current {
            gap += 1;
            if gap >= min_gap {
                out.push(Band {
                    top,
                    bottom,
                    ink: rows[top..bottom].iter().sum(),
                });
                current = None;
            }
        }
    }
    if let Some((top, bottom)) = current {
        out.push(Band {
            top,
            bottom,
            ink: rows[top..bottom].iter().sum(),
        });
    }
    out.retain(|b| b.ink >= 6 && b.ink * 1000 >= total);
    out
}

fn row_sums(mask: &InkMask) -> Vec<usize> {
    mask.bits
        .chunks(mask.width.max(1))
        .map(|row| row.iter().filter(|b| **b).count())
        .collect()
}

fn column_sums(mask: &InkMask) -> Vec<usize> {
    let mut out = vec![0usize; mask.width];
    for row in mask.bits.chunks(mask.width.max(1)) {
        for (x, &b) in row.iter().enumerate() {
            if b {
                out[x] += 1;
            }
        }
    }
    out
}

/// The shift of `b` against `a` that lines the two sums up best.
fn best_shift(a: &[usize], b: &[usize], max_shift: i64) -> i64 {
    let mut best = (0i64, u64::MIN);
    for shift in -max_shift..=max_shift {
        let mut score = 0u64;
        for (i, &va) in a.iter().enumerate() {
            let j = i as i64 + shift;
            if j >= 0 && (j as usize) < b.len() {
                score += va as u64 * b[j as usize] as u64;
            }
        }
        if score > best.1 || (score == best.1 && shift.abs() < best.0.abs()) {
            best = (shift, score);
        }
    }
    best.0
}

/// How much ink stands in each column over a stretch of rows, moved by `dy` and clamped to the
/// mask, smoothed over three columns so a one pixel wobble does not matter.
fn profile(mask: &InkMask, top: usize, bottom: usize, dy: i64) -> Vec<f32> {
    let mut raw = vec![0f32; mask.width];
    for y in top..bottom {
        let yy = y as i64 + dy;
        if yy < 0 || yy >= mask.height as i64 {
            continue;
        }
        let row = &mask.bits[yy as usize * mask.width..(yy as usize + 1) * mask.width];
        for (x, &b) in row.iter().enumerate() {
            if b {
                raw[x] += 1.0;
            }
        }
    }
    (0..raw.len())
        .map(|x| {
            let lo = x.saturating_sub(1);
            let hi = (x + 2).min(raw.len());
            raw[lo..hi].iter().sum::<f32>() / (hi - lo) as f32
        })
        .collect()
}

/// The first and one past the last column with ink.
fn extent(p: &[f32]) -> Option<(usize, usize)> {
    let first = p.iter().position(|v| *v > 0.0)?;
    let last = p.iter().rposition(|v| *v > 0.0)?;
    Some((first, last + 1))
}

/// Which columns a correlation is taken over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Window {
    /// Wherever either profile has ink.
    Either,
    /// Only where the first profile has ink.
    First,
    /// Only where the second profile has ink.
    Second,
}

/// Pearson correlation of two profiles at the horizontal shift that fits best, over the columns
/// the window names. Leaving out empty margins matters: two different lines of the same length
/// would otherwise agree on their blank ends and look alike.
fn correlation(a: &[f32], b: &[f32], max_shift: i64, window: Window) -> f32 {
    let (Some((a0, a1)), Some((b0, b1))) = (extent(a), extent(b)) else {
        return 0.0;
    };
    let mut best = -1.0f32;
    for shift in -max_shift..=max_shift {
        // the second profile is read at x + shift, so its extent moves the other way
        let (bs0, bs1) = (
            (b0 as i64 - shift).max(0) as usize,
            ((b1 as i64 - shift).max(0) as usize).min(a.len()),
        );
        let (lo, hi) = match window {
            Window::Either => (a0.min(bs0), a1.max(bs1)),
            Window::First => (a0, a1),
            Window::Second => (bs0, bs1),
        };
        if hi <= lo + 1 {
            continue;
        }
        let pairs: Vec<(f32, f32)> = (lo..hi)
            .map(|x| {
                let j = x as i64 + shift;
                let vb = if j >= 0 && (j as usize) < b.len() {
                    b[j as usize]
                } else {
                    0.0
                };
                (a[x], vb)
            })
            .collect();
        let n = pairs.len() as f32;
        let (ma, mb) = (
            pairs.iter().map(|p| p.0).sum::<f32>() / n,
            pairs.iter().map(|p| p.1).sum::<f32>() / n,
        );
        let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
        for (x, y) in &pairs {
            cov += (x - ma) * (y - mb);
            va += (x - ma) * (x - ma);
            vb += (y - mb) * (y - mb);
        }
        // two flat profiles, such as a ruled line, agree; one flat and one not do not
        let r = if va > 0.0 && vb > 0.0 {
            cov / (va * vb).sqrt()
        } else if va == 0.0 && vb == 0.0 {
            1.0
        } else {
            0.0
        };
        best = best.max(r);
    }
    best
}

/// How a band of one capture relates to the rows under it in the other.
#[derive(Debug, Clone, Copy, PartialEq)]
enum BandMatch {
    /// The same writing.
    Same,
    /// The band holds the other capture's writing and this much more ink besides.
    MoreHere(usize),
    /// The other capture holds this band's writing and this much more ink besides.
    MoreThere(usize),
    Different,
}

/// Compares a band of `from` with the rows under it in `other`, moved by `dy`. Writing is the
/// same when about as much ink stands there and the profiles correlate; when one side has much
/// more ink, the smaller side's writing may still sit inside the larger, which is writing added
/// or removed beside it rather than a different line.
fn match_band(
    from: &InkMask,
    band: &Band,
    other: &InkMask,
    dy: i64,
    config: &ChangeConfig,
) -> BandMatch {
    let a = profile(from, band.top, band.bottom, 0);
    let b = profile(other, band.top, band.bottom, dy);
    let (ink_a, ink_b) = (a.iter().sum::<f32>(), b.iter().sum::<f32>());
    if ink_a <= 0.0 || ink_b <= 0.0 {
        return BandMatch::Different;
    }
    let ratio = ink_b / ink_a;
    if (0.75..=1.0 / 0.75).contains(&ratio) {
        if correlation(&a, &b, config.max_shift, Window::Either) >= config.same_above {
            BandMatch::Same
        } else {
            BandMatch::Different
        }
    } else if ratio < 0.75 {
        if correlation(&a, &b, config.max_shift, Window::Second) >= config.same_above {
            BandMatch::MoreHere((ink_a - ink_b).round() as usize)
        } else {
            BandMatch::Different
        }
    } else if correlation(&a, &b, config.max_shift, Window::First) >= config.same_above {
        BandMatch::MoreThere((ink_b - ink_a).round() as usize)
    } else {
        BandMatch::Different
    }
}

/// Compares two masks of the same size band by band. Pixels hidden in either photo are left out
/// of both, so a person standing in front of part of the board does not count as a change.
pub fn compare(old: &InkMask, new: &InkMask, config: &ChangeConfig) -> Change {
    let hidden: Vec<bool> = old
        .hidden
        .iter()
        .zip(&new.hidden)
        .map(|(a, b)| *a || *b)
        .collect();
    let old = old.without(&hidden);
    let new = new.without(&hidden);
    let (old_count, new_count) = (old.count(), new.count());
    if old_count == 0 && new_count == 0 {
        return Change {
            added: 0.0,
            removed: 0.0,
            added_bands: 0,
            removed_bands: 0,
            shift_x: 0,
            shift_y: 0,
        };
    }
    let dy = best_shift(&row_sums(&old), &row_sums(&new), config.max_shift);
    let dx = best_shift(&column_sums(&old), &column_sums(&new), config.max_shift);
    let (mut added, mut added_bands) = (0usize, 0u32);
    for band in bands(&new, 2) {
        let extra = match match_band(&new, &band, &old, -dy, config) {
            BandMatch::Same | BandMatch::MoreThere(_) => 0,
            BandMatch::MoreHere(extra) => extra,
            BandMatch::Different => band.ink,
        };
        added += extra;
        added_bands += u32::from(extra >= config.min_band_ink);
    }
    let (mut removed, mut removed_bands) = (0usize, 0u32);
    for band in bands(&old, 2) {
        let extra = match match_band(&old, &band, &new, dy, config) {
            BandMatch::Same | BandMatch::MoreThere(_) => 0,
            BandMatch::MoreHere(extra) => extra,
            BandMatch::Different => band.ink,
        };
        removed += extra;
        removed_bands += u32::from(extra >= config.min_band_ink);
    }
    Change {
        added: if new_count == 0 {
            0.0
        } else {
            (added as f32 / new_count as f32).min(1.0)
        },
        removed: if old_count == 0 {
            0.0
        } else {
            (removed as f32 / old_count as f32).min(1.0)
        },
        added_bands,
        removed_bands,
        shift_x: dx,
        shift_y: dy,
    }
}

pub fn classify(change: &Change, config: &ChangeConfig) -> Verdict {
    if change.added < config.repeat_below
        && change.removed < config.repeat_below
        && change.added_bands == 0
        && change.removed_bands == 0
    {
        Verdict::Repeat
    } else if change.removed > config.new_board_above {
        Verdict::NewBoard
    } else {
        Verdict::NewVersion
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(width: usize, height: usize, ink: &dyn Fn(usize, usize) -> bool) -> InkMask {
        let mut bits = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                bits.push(ink(x, y));
            }
        }
        InkMask::new(width, height, bits)
    }

    /// A line of "text": columns with ink chosen by a hash of the column and the seed, so two
    /// seeds give two unrelated lines, over the rows given.
    fn text(
        x: usize,
        y: usize,
        left: usize,
        right: usize,
        top: usize,
        bottom: usize,
        seed: usize,
    ) -> bool {
        let h = (x as u64).wrapping_mul(0x9E37_79B1)
            ^ (seed as u64).wrapping_mul(0x85EB_CA77)
            ^ ((y / 3) as u64).wrapping_mul(0xC2B2_AE3D);
        (left..right).contains(&x) && (top..bottom).contains(&y) && (h >> 7) % 5 < 2
    }

    #[test]
    fn the_same_writing_shifted_is_a_repeat() {
        let config = ChangeConfig::default();
        let a = mask(120, 60, &|x, y| {
            text(x, y, 10, 60, 10, 20, 1) || text(x, y, 10, 50, 30, 40, 2)
        });
        // the same board photographed 9 pixels right and 4 down, a re-aimed phone
        let b = mask(120, 60, &|x, y| {
            x >= 9
                && y >= 4
                && (text(x - 9, y - 4, 10, 60, 10, 20, 1) || text(x - 9, y - 4, 10, 50, 30, 40, 2))
        });
        let change = compare(&a, &b, &config);
        assert_eq!((change.shift_x, change.shift_y), (9, 4));
        assert_eq!(change.fraction(), 0.0, "{change:?}");
        assert_eq!(classify(&change, &config), Verdict::Repeat);
        assert_eq!(bands(&a, 2).len(), 2);
    }

    #[test]
    fn added_rewritten_and_wiped_lines_are_told_apart() {
        let config = ChangeConfig::default();
        let first = mask(80, 60, &|x, y| text(x, y, 10, 60, 10, 20, 1));
        let with_line = mask(80, 60, &|x, y| {
            text(x, y, 10, 60, 10, 20, 1) || text(x, y, 10, 60, 30, 40, 2)
        });
        let change = compare(&first, &with_line, &config);
        assert!(
            (change.added - 0.5).abs() < 0.05 && change.removed == 0.0,
            "half the new ink is new: {change:?}"
        );
        assert_eq!(classify(&change, &config), Verdict::NewVersion);

        // a different line in the same place is a change, even though its ink sits where the old did
        let rewritten = mask(80, 60, &|x, y| text(x, y, 10, 60, 10, 20, 4));
        let change = compare(&first, &rewritten, &config);
        assert!(change.added > 0.9 && change.removed > 0.9, "{change:?}");
        assert_eq!(
            classify(&change, &config),
            Verdict::NewBoard,
            "the only line was replaced"
        );

        let two = mask(80, 60, &|x, y| {
            text(x, y, 10, 60, 10, 20, 1) || text(x, y, 10, 60, 30, 40, 2)
        });
        let second_rewritten = mask(80, 60, &|x, y| {
            text(x, y, 10, 60, 10, 20, 1) || text(x, y, 10, 60, 30, 40, 3)
        });
        let change = compare(&two, &second_rewritten, &config);
        assert!(
            (0.3..0.7).contains(&change.added) && (0.3..0.7).contains(&change.removed),
            "{change:?}"
        );
        assert_eq!(classify(&change, &config), Verdict::NewVersion);

        let empty = mask(80, 60, &|_, _| false);
        assert_eq!(compare(&empty, &empty, &config).fraction(), 0.0);
        let from_nothing = compare(&empty, &first, &config);
        assert_eq!((from_nothing.added, from_nothing.removed), (1.0, 0.0));
        let wiped = compare(&first, &empty, &config);
        assert_eq!((wiped.added, wiped.removed), (0.0, 1.0));
        assert_eq!(classify(&wiped, &config), Verdict::NewBoard);
    }

    #[test]
    fn writing_added_beside_a_line_counts_only_the_addition() {
        let config = ChangeConfig::default();
        let line = mask(120, 60, &|x, y| text(x, y, 10, 50, 10, 20, 1));
        let extended = mask(120, 60, &|x, y| text(x, y, 10, 90, 10, 20, 1));
        let change = compare(&line, &extended, &config);
        assert!(
            (change.added - 0.5).abs() < 0.08 && change.removed == 0.0,
            "{change:?}"
        );
        assert_eq!(classify(&change, &config), Verdict::NewVersion);
        let back = compare(&extended, &line, &config);
        assert!(
            back.added == 0.0 && (back.removed - 0.5).abs() < 0.08,
            "{back:?}"
        );
        assert_eq!(
            classify(&back, &config),
            Verdict::NewVersion,
            "half the line rubbed out is not a wiped board"
        );
    }

    #[test]
    fn a_small_addition_to_a_full_board_is_still_a_new_version() {
        let config = ChangeConfig::default();
        // twenty lines, then one short line more
        let full = mask(120, 240, &|x, y| {
            (0..20).any(|i| text(x, y, 10, 110, 5 + i * 11, 13 + i * 11, i))
        });
        let more = mask(120, 240, &|x, y| {
            full_ink(x, y) || text(x, y, 10, 40, 226, 234, 99)
        });
        fn full_ink(x: usize, y: usize) -> bool {
            (0..20).any(|i| text(x, y, 10, 110, 5 + i * 11, 13 + i * 11, i))
        }
        let change = compare(&full, &more, &config);
        assert!(
            change.added > 0.01 && change.added < 0.1 && change.removed == 0.0,
            "{change:?}"
        );
        assert_eq!(classify(&change, &config), Verdict::NewVersion);
    }

    #[test]
    fn hidden_pixels_are_left_out_of_both_sides() {
        let config = ChangeConfig::default();
        let board = mask(120, 60, &|x, y| {
            text(x, y, 10, 110, 10, 20, 1) || text(x, y, 10, 110, 30, 40, 2)
        });
        let mut blocked = board.clone();
        // a person covers the right third of the second photo
        for y in 0..60 {
            for x in 80..120 {
                blocked.hidden[y * 120 + x] = true;
                blocked.bits[y * 120 + x] = false;
            }
        }
        let change = compare(&board, &blocked, &config);
        assert_eq!(classify(&change, &config), Verdict::Repeat, "{change:?}");
    }
}
