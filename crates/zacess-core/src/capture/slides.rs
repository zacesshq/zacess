//! Lecture mode: slides photographed from a projector.
//!
//! A phone watching a projector sends a frame every second or so. Most are the slide already
//! seen, shaken, relit or out of focus. This module decides, for each frame, whether it is a
//! slide already kept, a step of one (a bullet appearing, an animation moving on) or a new slide.
//! It keeps the clearest frame of every run of repeats and records when each slide was on screen,
//! so a kept slide can be placed on the lecture timeline. The camera is the app's; this module
//! takes frames as files.
//!
//! Frames are compared in two steps. A perceptual hash, a fingerprint of the pattern of light and
//! dark that survives a change of exposure and a few pixels of shake, settles the easy cases: a
//! near identical frame is a repeat and a frame with a quite different layout is a new slide. The
//! frames in between, which is where a slide being built up step by step lives, go to the same
//! comparison of bands of content that tells boards apart (see the change module). It lines the
//! two frames up to take the shake out and then asks what was added or taken away.

use super::change::{self, ChangeConfig, Verdict};
use super::frame::{Frame, InkMask};
use super::{CaptureError, CaptureKind, title_for};
use crate::import::image::import_image;
use crate::import::ocr::OcrEngine;
use crate::model::Document;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Bumped when a stored slide session's shape changes.
pub const SLIDE_SCHEMA_VERSION: u32 = 1;

/// Slide and frame ids start here on the lecture timeline, so they never collide with the ids of
/// whiteboard captures that share it.
pub const SLIDE_ID_BASE: u32 = 1_000_000;

/// A hash with fewer structural bits than this has no layout to speak of. A slide has dozens and
/// a lens cap or a blank wall has almost none.
const MIN_STRUCTURE: u32 = 12;

/// A content mask with fewer set cells than this has nothing to compare.
const MIN_MASK_CELLS: usize = 30;

/// How frames are judged. The defaults are starting points to tune on a real projector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlideConfig {
    /// The longer side frames are shrunk to before anything is measured, in pixels.
    pub max_side: usize,
    /// Frames whose hashes are this far apart or less, as a share of the hash bits that carry
    /// structure, are the same slide in the same state. Kept small, because a bullet added to a
    /// slide changes the hash only a little.
    pub duplicate_below: f32,
    /// Frames this far apart or more are different slides.
    pub new_above: f32,
    /// Frames closer than `new_above` but beyond `duplicate_below` are compared band by band. If
    /// that cannot be done, this decides: closer than it is a repeat and anything else is new.
    pub near_duplicate_below: f32,
    /// A pixel is part of the content when its edge is stronger than this, in grey levels.
    pub edge_floor: f32,
    /// Content is looked at this many times coarser than the frame, so the strokes of a line of
    /// text run together and noise at the edge of a stroke cannot reshape it.
    pub mask_pool: usize,
    /// How the bands of content are compared, on a mask that is `mask_pool` times smaller than
    /// the frame. A change of up to 15% of the content, with no whole line of it added or taken
    /// away, is a repeat. More than 60% of the earlier content gone is a different slide.
    /// Anything between is a step of the same slide.
    pub change: ChangeConfig,
    /// A frame whose brightness varies less than this is a lens cap or a blank wall.
    pub min_contrast: f32,
    /// The most steps a slide can have. A video or an animation that never settles would
    /// otherwise add a step for every frame. Beyond it, builds count as repeats of the last step.
    pub max_steps: usize,
    /// A new slide or step is kept once this many frames in a row agree on it. Frames caught in
    /// the middle of a transition never agree with the next one and are dropped.
    pub confirm_frames: u32,
    /// A later frame replaces the kept one when it is this many times as sharp.
    pub clearer_by: f32,
}

impl Default for SlideConfig {
    fn default() -> Self {
        SlideConfig {
            max_side: 640,
            duplicate_below: 0.01,
            new_above: 0.30,
            near_duplicate_below: 0.05,
            edge_floor: 10.0,
            mask_pool: 4,
            change: ChangeConfig {
                max_shift: 8,
                same_above: 0.8,
                repeat_below: 0.15,
                new_board_above: 0.6,
                min_band_ink: 200,
            },
            min_contrast: 4.0,
            max_steps: 30,
            confirm_frames: 2,
            clearer_by: 1.05,
        }
    }
}

impl SlideConfig {
    /// Refuses settings that would make every frame a duplicate or every frame new.
    pub fn validate(&self) -> Result<(), CaptureError> {
        let unit = |name: &str, v: f32| {
            if v > 0.0 && v <= 1.0 {
                Ok(())
            } else {
                Err(CaptureError::Config(format!(
                    "{name} must be above 0 and at most 1, not {v}"
                )))
            }
        };
        unit("duplicate_below", self.duplicate_below)?;
        unit("new_above", self.new_above)?;
        unit("near_duplicate_below", self.near_duplicate_below)?;
        unit("same_above", self.change.same_above)?;
        unit("repeat_below", self.change.repeat_below)?;
        unit("new_board_above", self.change.new_board_above)?;
        if self.duplicate_below >= self.new_above {
            return Err(CaptureError::Config(
                "duplicate_below must be below new_above".into(),
            ));
        }
        if !(1..=16).contains(&self.mask_pool) {
            return Err(CaptureError::Config("mask_pool must be 1 to 16".into()));
        }
        if self.max_side < 64 {
            return Err(CaptureError::Config(format!(
                "max_side must be at least 64, not {}",
                self.max_side
            )));
        }
        if self.change.max_shift < 0
            || !self.edge_floor.is_finite()
            || !self.min_contrast.is_finite()
        {
            return Err(CaptureError::Config(
                "max_shift cannot be negative and the floors must be numbers".into(),
            ));
        }
        if self.change.repeat_below >= self.change.new_board_above
            || self.near_duplicate_below >= self.new_above
        {
            return Err(CaptureError::Config(
                "repeat_below must be below new_board_above and near_duplicate_below below new_above".into(),
            ));
        }
        if self.max_steps == 0 || self.min_contrast < 0.0 || self.min_contrast > 128.0 {
            return Err(CaptureError::Config(
                "max_steps must be at least 1 and min_contrast 0 to 128".into(),
            ));
        }
        if self.confirm_frames == 0 || self.confirm_frames > 10 {
            return Err(CaptureError::Config(
                "confirm_frames must be 1 to 10".into(),
            ));
        }
        if self.clearer_by.is_nan() || self.clearer_by < 1.0 {
            return Err(CaptureError::Config("clearer_by must be at least 1".into()));
        }
        Ok(())
    }
}

/// A perceptual hash of a frame: 256 bits saying which way brightness falls between neighbouring
/// areas of a 16 by 16 grid. A second set of bits says where that fall is large enough to trust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlideHash {
    bits: [u64; 4],
    strong: [u64; 4],
}

/// Packs flags into words, 64 to a word.
fn pack(flags: &[bool]) -> [u64; 4] {
    let mut out = [0u64; 4];
    for (i, &f) in flags.iter().take(256).enumerate() {
        out[i / 64] |= u64::from(f) << (i % 64);
    }
    out
}

impl SlideHash {
    /// A bit is set where an area is brighter than its right-hand neighbour. It is strong where
    /// the difference is large against the spread of the whole grid. A change of exposure scales
    /// every difference, so it leaves the same bits strong.
    pub fn of(frame: &Frame) -> SlideHash {
        let integral = frame.integral();
        let grid = frame.grid_means(&integral, (0, 0, frame.width, frame.height), 17, 16);
        let n = grid.len() as f32;
        let mean = grid.iter().sum::<f32>() / n;
        let std = (grid.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n).sqrt();
        let limit = 1.5f32.max(0.15 * std);
        let (mut bits, mut strong) = (Vec::with_capacity(256), Vec::with_capacity(256));
        for r in 0..16 {
            for c in 0..16 {
                let (a, b) = (grid[r * 17 + c], grid[r * 17 + c + 1]);
                bits.push(a > b);
                strong.push((a - b).abs() > limit);
            }
        }
        SlideHash {
            bits: pack(&bits),
            strong: pack(&strong),
        }
    }

    /// Of the bits either hash trusts, the share on which they disagree, 0 to 1.
    pub fn distance(&self, other: &SlideHash) -> f32 {
        let (mut differ, mut counted) = (0u32, 0u32);
        for i in 0..4 {
            let mask = self.strong[i] | other.strong[i];
            counted += mask.count_ones();
            differ += ((self.bits[i] ^ other.bits[i]) & mask).count_ones();
        }
        if counted == 0 {
            0.0
        } else {
            differ as f32 / counted as f32
        }
    }

    /// How many bits carry structure. A frame with almost none has no layout to hash.
    pub fn structure(&self) -> u32 {
        self.strong.iter().map(|b| b.count_ones()).sum()
    }
}

/// What one frame is to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Match {
    /// The same slide in the same state.
    Duplicate,
    /// The same slide with part of it added or taken away: a bullet, an animation step.
    Build,
    /// A different slide.
    New,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    /// The share of structural hash bits that differ, 0 to 1.
    pub distance: f32,
    /// How the content differs, when the band comparison was needed and possible.
    pub change: Option<change::Change>,
    pub verdict: Match,
}

/// Compares the kept frame (`old`) with a new one. Each is its hash and, when it has one, its
/// content mask.
pub fn compare(
    old: (&SlideHash, Option<&InkMask>),
    new: (&SlideHash, Option<&InkMask>),
    config: &SlideConfig,
) -> Comparison {
    let distance = old.0.distance(new.0);
    let done = |verdict, change| Comparison {
        distance,
        change,
        verdict,
    };
    if distance >= config.new_above {
        return done(Match::New, None);
    }
    // a frame with almost no structure has nothing for a hash to say, so it is not trusted to
    // call two frames the same
    if distance <= config.duplicate_below && old.0.structure() >= MIN_STRUCTURE {
        return done(Match::Duplicate, None);
    }
    match (old.1, new.1) {
        (Some(a), Some(b))
            if a.width == b.width
                && a.height == b.height
                && (a.count() >= MIN_MASK_CELLS || b.count() >= MIN_MASK_CELLS) =>
        {
            let found = change::compare(a, b, &config.change);
            let verdict = match change::classify(&found, &config.change) {
                Verdict::Repeat => Match::Duplicate,
                Verdict::NewVersion => Match::Build,
                Verdict::NewBoard => Match::New,
            };
            done(verdict, Some(found))
        }
        _ => done(
            if distance <= config.near_duplicate_below {
                Match::Duplicate
            } else {
                Match::New
            },
            None,
        ),
    }
}

/// A content mask rebuilt from a file when it is needed. It is never stored so two sessions
/// count as equal whether or not their masks have been rebuilt.
#[derive(Debug, Clone, Default)]
struct MaskCache {
    mask: Option<InkMask>,
    /// The file could not be read to rebuild the mask, so it is not tried again.
    failed: bool,
}

impl PartialEq for MaskCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// One kept frame of a slide: the clearest seen of a run of repeats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlideStep {
    /// Unique across the session.
    pub id: u32,
    /// 1 for the slide as first shown, counting up for each build.
    pub number: u32,
    pub file: PathBuf,
    /// Unix seconds.
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    /// How many frames showed this state of the slide.
    pub frames: u32,
    pub sharpness: f32,
    /// The hash of the frame that first showed this state. It stays put when a clearer frame
    /// replaces the file, so a long run of near repeats cannot drift away from the slide.
    pub hash: SlideHash,
    /// What OCR read from the kept frame; empty until [`SlideSession::read`] has run.
    #[serde(default)]
    pub lines: Vec<String>,
    #[serde(default)]
    pub document: Option<Document>,
    /// The content of the frame that first showed this state, rebuilt from the file when a
    /// stored session is loaded.
    #[serde(skip)]
    mask: MaskCache,
}

/// A slide of the lecture: its states in order and when the lecturer came back to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slide {
    pub id: u32,
    pub title: String,
    pub steps: Vec<SlideStep>,
    /// When the lecturer came back to this slide, in Unix seconds.
    pub revisits: Vec<i64>,
}

impl Slide {
    /// The state the slide ends in, which is the one to read and study from.
    pub fn final_step(&self) -> Option<&SlideStep> {
        self.steps.last()
    }

    /// When the slide was last on screen, counting returns to it.
    pub fn last_seen_at(&self) -> i64 {
        self.steps
            .iter()
            .map(|s| s.last_seen_at)
            .chain(self.revisits.iter().copied())
            .max()
            .unwrap_or(0)
    }
}

/// What happened to a frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SlideEvent {
    /// A lens cap, a blank wall or a frame with nothing to measure. Nothing was kept.
    Unusable,
    /// The same slide in the same state. `clearer` says the frame replaced the kept one.
    Duplicate {
        slide: u32,
        step: u32,
        clearer: bool,
    },
    /// The lecturer went back to a slide shown earlier.
    Revisit { slide: u32, step: u32 },
    /// A change not yet confirmed by enough frames.
    Pending { frames: u32 },
    /// A slide not seen before.
    NewSlide { slide: u32, step: u32 },
    /// The same slide with part of it changed.
    Build { slide: u32, step: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum PendingKind {
    New,
    Build { slide: usize },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Pending {
    kind: PendingKind,
    file: PathBuf,
    first_at: i64,
    last_at: i64,
    frames: u32,
    sharpness: f32,
    hash: SlideHash,
    #[serde(skip)]
    mask: MaskCache,
}

/// A frame loaded and measured.
struct Measured {
    hash: SlideHash,
    mask: InkMask,
    sharpness: f32,
    contrast: f32,
}

fn measure(path: &Path, config: &SlideConfig) -> Result<Measured, CaptureError> {
    let (image, _) = Frame::load_turned(path)?;
    let frame = Frame::from_image(&image).shrunk_to(config.max_side);
    Ok(Measured {
        hash: SlideHash::of(&frame),
        mask: frame.content_mask(config.edge_floor, config.mask_pool),
        sharpness: frame.sharpness(),
        contrast: frame.contrast(),
    })
}

/// The slides of one lecture, found from its frames, with any change waiting for confirmation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlideSession {
    pub schema_version: u32,
    pub slides: Vec<Slide>,
    next_slide: u32,
    next_step: u32,
    current: Option<usize>,
    pending: Option<Pending>,
}

impl Default for SlideSession {
    fn default() -> Self {
        SlideSession {
            schema_version: SLIDE_SCHEMA_VERSION,
            slides: Vec::new(),
            next_slide: 0,
            next_step: 0,
            current: None,
            pending: None,
        }
    }
}

impl SlideSession {
    /// An empty session.
    pub fn new() -> Self {
        SlideSession::default()
    }

    /// Rebuilds the content masks a session read back from JSON does not carry, from the files.
    /// That includes the change waiting for confirmation.
    fn rebuild_masks(&mut self, config: &SlideConfig) {
        let rebuild = |file: &Path, cache: &mut MaskCache| {
            if cache.mask.is_none() && !cache.failed {
                match measure(file, config) {
                    Ok(found) => cache.mask = Some(found.mask),
                    Err(_) => cache.failed = true,
                }
            }
        };
        for slide in &mut self.slides {
            for step in &mut slide.steps {
                rebuild(&step.file, &mut step.mask);
            }
        }
        if let Some(pending) = &mut self.pending {
            rebuild(&pending.file, &mut pending.mask);
        }
    }

    /// The kept state a frame is closest to, among every step of every slide, so a return to the
    /// first state of a slide that was later built up is recognised. A repeat beats a step and a
    /// step beats a new slide. Then the smaller hash distance wins, then the slide on screen,
    /// then the later step.
    fn closest(
        &self,
        frame: &Measured,
        config: &SlideConfig,
    ) -> Option<(usize, usize, Comparison)> {
        let rank = |m: Match| match m {
            Match::Duplicate => 0,
            Match::Build => 1,
            Match::New => 2,
        };
        let mut best: Option<(usize, usize, Comparison)> = None;
        for (s, slide) in self.slides.iter().enumerate() {
            for (t, step) in slide.steps.iter().enumerate() {
                let c = compare(
                    (&step.hash, step.mask.mask.as_ref()),
                    (&frame.hash, Some(&frame.mask)),
                    config,
                );
                let better = best.as_ref().is_none_or(|(bs, bt, b)| {
                    if rank(c.verdict) != rank(b.verdict) {
                        return rank(c.verdict) < rank(b.verdict);
                    }
                    if c.distance != b.distance {
                        return c.distance < b.distance;
                    }
                    let (here, there) = (self.current == Some(s), self.current == Some(*bs));
                    if here != there {
                        return here;
                    }
                    s == *bs && t > *bt
                });
                if better {
                    best = Some((s, t, c));
                }
            }
        }
        best
    }

    /// Takes in one frame. `taken_at` is Unix seconds.
    pub fn push(
        &mut self,
        path: &Path,
        taken_at: i64,
        config: &SlideConfig,
    ) -> Result<SlideEvent, CaptureError> {
        config.validate()?;
        let frame = measure(path, config)?;
        if frame.contrast < config.min_contrast {
            return Ok(SlideEvent::Unusable);
        }
        self.rebuild_masks(config);
        let Some((s, t, comparison)) = self.closest(&frame, config) else {
            return Ok(self.consider(PendingKind::New, path, taken_at, frame, config));
        };
        match comparison.verdict {
            Match::Duplicate => {
                // the change that was waiting for confirmation was a flicker, not a new state
                self.pending = None;
                Ok(self.merge(s, t, path, taken_at, frame.sharpness, config))
            }
            Match::Build if self.slides[s].steps.len() >= config.max_steps => {
                self.pending = None;
                let last = self.slides[s].steps.len() - 1;
                Ok(self.merge(s, last, path, taken_at, frame.sharpness, config))
            }
            Match::Build => Ok(self.consider(
                PendingKind::Build { slide: s },
                path,
                taken_at,
                frame,
                config,
            )),
            Match::New => Ok(self.consider(PendingKind::New, path, taken_at, frame, config)),
        }
    }

    /// Folds a repeat into the kept step, keeping the clearer of the two frames.
    fn merge(
        &mut self,
        s: usize,
        t: usize,
        path: &Path,
        taken_at: i64,
        sharpness: f32,
        config: &SlideConfig,
    ) -> SlideEvent {
        let revisit = self.current != Some(s);
        let slide = &mut self.slides[s];
        let step = &mut slide.steps[t];
        step.frames += 1;
        step.first_seen_at = step.first_seen_at.min(taken_at);
        step.last_seen_at = step.last_seen_at.max(taken_at);
        let clearer = sharpness > step.sharpness * config.clearer_by;
        if clearer {
            step.file = path.to_path_buf();
            step.sharpness = sharpness;
            // the text was read from another frame
            step.document = None;
            step.lines.clear();
        }
        let (slide_id, step_number) = (slide.id, step.number);
        if revisit {
            slide.revisits.push(taken_at);
            self.current = Some(s);
            return SlideEvent::Revisit {
                slide: slide_id,
                step: step_number,
            };
        }
        SlideEvent::Duplicate {
            slide: slide_id,
            step: step_number,
            clearer,
        }
    }

    /// Holds a change until enough frames in a row agree on it.
    fn consider(
        &mut self,
        kind: PendingKind,
        path: &Path,
        taken_at: i64,
        frame: Measured,
        config: &SlideConfig,
    ) -> SlideEvent {
        let agrees = self.pending.as_ref().is_some_and(|p| {
            compare(
                (&p.hash, p.mask.mask.as_ref()),
                (&frame.hash, Some(&frame.mask)),
                config,
            )
            .verdict
                == Match::Duplicate
        });
        if agrees && let Some(pending) = &mut self.pending {
            pending.frames += 1;
            pending.first_at = pending.first_at.min(taken_at);
            pending.last_at = pending.last_at.max(taken_at);
            if frame.sharpness > pending.sharpness * config.clearer_by {
                pending.file = path.to_path_buf();
                pending.sharpness = frame.sharpness;
            }
        } else {
            self.pending = Some(Pending {
                kind,
                file: path.to_path_buf(),
                first_at: taken_at,
                last_at: taken_at,
                frames: 1,
                sharpness: frame.sharpness,
                hash: frame.hash,
                mask: MaskCache {
                    mask: Some(frame.mask),
                    failed: false,
                },
            });
        }
        let frames = self.pending.as_ref().map(|p| p.frames).unwrap_or(0);
        if frames >= config.confirm_frames {
            return self.commit();
        }
        SlideEvent::Pending { frames }
    }

    fn commit(&mut self) -> SlideEvent {
        let Some(pending) = self.pending.take() else {
            return SlideEvent::Unusable;
        };
        self.next_step += 1;
        let mut step = SlideStep {
            id: self.next_step,
            number: 1,
            file: pending.file,
            first_seen_at: pending.first_at,
            last_seen_at: pending.last_at,
            frames: pending.frames,
            sharpness: pending.sharpness,
            hash: pending.hash,
            lines: Vec::new(),
            document: None,
            mask: pending.mask,
        };
        match pending.kind {
            PendingKind::Build { slide } if slide < self.slides.len() => {
                let target = &mut self.slides[slide];
                step.number = target.steps.len() as u32 + 1;
                let event = SlideEvent::Build {
                    slide: target.id,
                    step: step.number,
                };
                target.steps.push(step);
                self.current = Some(slide);
                event
            }
            _ => {
                self.next_slide += 1;
                let id = self.next_slide;
                self.slides.push(Slide {
                    id,
                    title: format!("Slide {id}"),
                    steps: vec![step],
                    revisits: Vec::new(),
                });
                self.current = Some(self.slides.len() - 1);
                SlideEvent::NewSlide { slide: id, step: 1 }
            }
        }
    }

    /// Ends the lecture. A change still waiting for confirmation is kept, because the last
    /// frames of a lecture may be the only ones of the last slide. The price is that a frame
    /// caught in a fade at the very end is kept too, since no later frame shows it was a fade.
    pub fn finish(&mut self) -> Vec<SlideEvent> {
        if self.pending.is_some() {
            vec![self.commit()]
        } else {
            Vec::new()
        }
    }

    /// Reads the text of every kept frame that has not been read. Titles each slide from its
    /// final state. Returns how many frames were read.
    pub fn read(&mut self, ocr: &dyn OcrEngine) -> Result<usize, CaptureError> {
        let mut read = 0;
        for slide in &mut self.slides {
            for step in &mut slide.steps {
                if step.document.is_some() {
                    continue;
                }
                let document = import_upright(&step.file, ocr)?;
                step.lines = super::document_lines(&document);
                step.document = Some(document);
                read += 1;
            }
            if let Some(document) = slide.steps.last().and_then(|s| s.document.as_ref()) {
                slide.title = title_for(document, CaptureKind::Slide, slide.id);
            }
        }
        Ok(read)
    }

    /// The slide with a given id.
    pub fn slide(&self, id: u32) -> Option<&Slide> {
        self.slides.iter().find(|s| s.id == id)
    }
}

/// OCR a photo the right way up. The engine reads the file itself and ignores the orientation
/// note a phone stores, so a turned photo is written out turned first.
fn import_upright(path: &Path, ocr: &dyn OcrEngine) -> Result<Document, CaptureError> {
    let (image, turned) = Frame::load_turned(path)?;
    let mut document = if turned {
        let dir = tempfile::tempdir().map_err(|e| CaptureError::Image(e.to_string()))?;
        let upright = dir.path().join("upright.png");
        image
            .save(&upright)
            .map_err(|e| CaptureError::Image(e.to_string()))?;
        import_image(&upright, ocr)?.document
    } else {
        import_image(path, ocr)?.document
    };
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        document.source.file_name = name.to_string();
    }
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::ImportError;
    use crate::import::ocr::{OcrPage, Tesseract};

    fn frame(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/slides")
            .join(name)
    }

    #[derive(Debug, serde::Deserialize)]
    struct Sidecar {
        frames: Vec<Described>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct Described {
        file: String,
    }

    fn all_frames() -> Vec<PathBuf> {
        let sidecar: Sidecar =
            serde_json::from_str(&std::fs::read_to_string(frame("frames.json")).unwrap()).unwrap();
        sidecar.frames.iter().map(|f| frame(&f.file)).collect()
    }

    fn measured(name: &str, config: &SlideConfig) -> Measured {
        measure(&frame(name), config).unwrap()
    }

    struct NoOcr;
    impl OcrEngine for NoOcr {
        fn name(&self) -> String {
            "none".into()
        }
        fn recognise(&self, _image: &Path) -> Result<OcrPage, ImportError> {
            Ok(OcrPage {
                width: 1.0,
                height: 1.0,
                paragraphs: Vec::new(),
            })
        }
    }

    fn verdict(a: &str, b: &str) -> Match {
        let config = SlideConfig::default();
        let (a, b) = (measured(a, &config), measured(b, &config));
        compare((&a.hash, Some(&a.mask)), (&b.hash, Some(&b.mask)), &config).verdict
    }

    #[test]
    fn the_hash_settles_exact_repeats_shake_light_and_focus_but_not_builds() {
        let config = SlideConfig::default();
        let one = measured("a1-slide-one.jpg", &config);
        assert!(
            one.hash.structure() >= 3 * MIN_STRUCTURE,
            "{}",
            one.hash.structure()
        );
        assert_eq!(one.hash.distance(&one.hash), 0.0);
        let again = measured("a2-slide-one-again.jpg", &config);
        assert_eq!(one.hash, again.hash, "the same photograph hashes the same");
        for name in [
            "a3-slide-one-shake.jpg",
            "a4-slide-one-dim.jpg",
            "a5-slide-one-soft.jpg",
            "d2-back-to-one.jpg",
        ] {
            let d = one.hash.distance(&measured(name, &config).hash);
            assert!(d <= config.duplicate_below, "{name}: {d}");
        }
        // a bullet appearing moves the hash a little, but more than a repeat does
        let step1 = measured("b1-slide-two-step1.jpg", &config);
        let step2 = measured("b3-slide-two-step2.jpg", &config);
        let built = step1.hash.distance(&step2.hash);
        assert!(
            built > config.duplicate_below && built < config.new_above,
            "{built}"
        );
        // a different kind of slide is far off
        let diagram = measured("c2-diagram.jpg", &config);
        assert!(one.hash.distance(&diagram.hash) >= config.new_above);
        // a lens cap has nothing to hash
        let blank = measured("d1-nothing.jpg", &config);
        assert!(blank.contrast < config.min_contrast && blank.hash.structure() < MIN_STRUCTURE);
    }

    #[test]
    fn frames_are_judged_the_same_a_step_or_a_new_slide() {
        for (a, b) in [
            ("a1-slide-one.jpg", "a2-slide-one-again.jpg"),
            ("a1-slide-one.jpg", "a3-slide-one-shake.jpg"),
            ("a1-slide-one.jpg", "a4-slide-one-dim.jpg"),
            ("a1-slide-one.jpg", "a5-slide-one-soft.jpg"),
            ("b1-slide-two-step1.jpg", "b2-slide-two-step1-again.jpg"),
            ("b3-slide-two-step2.jpg", "b4-slide-two-step2-again.jpg"),
            ("c2-diagram.jpg", "c3-diagram-again.jpg"),
        ] {
            assert_eq!(verdict(a, b), Match::Duplicate, "{a} against {b}");
        }
        for (a, b) in [
            ("b1-slide-two-step1.jpg", "b3-slide-two-step2.jpg"),
            ("b3-slide-two-step2.jpg", "b5-slide-two-step3.jpg"),
            ("b1-slide-two-step1.jpg", "b5-slide-two-step3.jpg"),
            (
                "b2-slide-two-step1-again.jpg",
                "b4-slide-two-step2-again.jpg",
            ),
        ] {
            assert_eq!(verdict(a, b), Match::Build, "{a} against {b}");
        }
        for (a, b) in [
            ("a1-slide-one.jpg", "b1-slide-two-step1.jpg"),
            ("a1-slide-one.jpg", "b5-slide-two-step3.jpg"),
            ("a3-slide-one-shake.jpg", "b3-slide-two-step2.jpg"),
            ("b5-slide-two-step3.jpg", "c2-diagram.jpg"),
        ] {
            assert_eq!(verdict(a, b), Match::New, "{a} against {b}");
        }
    }

    fn run(config: &SlideConfig) -> (SlideSession, Vec<SlideEvent>) {
        let mut session = SlideSession::new();
        let mut events = Vec::new();
        for (i, path) in all_frames().iter().enumerate() {
            events.push(session.push(path, 1000 + i as i64, config).unwrap());
        }
        events.extend(session.finish());
        (session, events)
    }

    #[test]
    fn a_lecture_of_frames_becomes_slides_with_steps_and_one_return() {
        let (session, events) = run(&SlideConfig::default());
        use SlideEvent::*;
        assert_eq!(
            events,
            [
                Pending { frames: 1 },          // a1
                NewSlide { slide: 1, step: 1 }, // a2 confirms it
                Duplicate {
                    slide: 1,
                    step: 1,
                    clearer: false
                }, // a3 shake
                Duplicate {
                    slide: 1,
                    step: 1,
                    clearer: false
                }, // a4 light
                Duplicate {
                    slide: 1,
                    step: 1,
                    clearer: false
                }, // a5 soft
                Pending { frames: 1 },          // b1
                NewSlide { slide: 2, step: 1 }, // b2 confirms it
                Pending { frames: 1 },          // b3
                Build { slide: 2, step: 2 },    // b4 confirms it
                Pending { frames: 1 },          // b5
                Build { slide: 2, step: 3 },    // b6 confirms it
                Pending { frames: 1 },          // c1 half way through a fade
                Pending { frames: 1 },          // c2 replaces it
                NewSlide { slide: 3, step: 1 }, // c3 confirms it
                Unusable,                       // d1 lens covered
                Revisit { slide: 1, step: 1 },  // d2
                Revisit { slide: 2, step: 1 },  // d3 back to slide two before it was built
            ],
            "{events:#?}"
        );
        assert_eq!(session.slides.len(), 3);
        assert_eq!(
            session.slides[1]
                .steps
                .iter()
                .map(|s| s.number)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(
            session.slides[0].revisits,
            [1015],
            "the return is recorded at its time"
        );
        assert_eq!(session.slides[1].revisits, [1016]);
        assert_eq!(
            session.slides[1].steps.len(),
            3,
            "going back to step 1 added no step"
        );
        // the transition frame was dropped: no step holds it
        assert!(
            session
                .slides
                .iter()
                .flat_map(|s| &s.steps)
                .all(|s| !s.file.ends_with("c1-transition.jpg"))
        );
        // times: slide one was first seen on the first frame
        assert_eq!(session.slides[0].steps[0].first_seen_at, 1000);
        assert_eq!(session.slides[0].last_seen_at(), 1015);
    }

    #[test]
    fn without_confirmation_a_frame_caught_in_a_fade_becomes_a_step() {
        let config = SlideConfig {
            confirm_frames: 1,
            ..SlideConfig::default()
        };
        let (session, events) = run(&config);
        assert!(
            session
                .slides
                .iter()
                .flat_map(|s| &s.steps)
                .any(|s| s.file.ends_with("c1-transition.jpg")),
            "{events:#?}"
        );
    }

    #[test]
    fn the_clearest_frame_of_a_run_is_the_one_kept() {
        let config = SlideConfig::default();
        let mut session = SlideSession::new();
        // the soft frame comes first, twice, then the sharp one arrives later
        for (name, at) in [("a5-slide-one-soft.jpg", 1), ("a5-slide-one-soft.jpg", 2)] {
            session.push(&frame(name), at, &config).unwrap();
        }
        assert!(
            session.slides[0].steps[0]
                .file
                .ends_with("a5-slide-one-soft.jpg")
        );
        let event = session
            .push(&frame("a1-slide-one.jpg"), 3, &config)
            .unwrap();
        assert_eq!(
            event,
            SlideEvent::Duplicate {
                slide: 1,
                step: 1,
                clearer: true
            }
        );
        let step = session.slides[0].steps[0].clone();
        assert!(step.file.ends_with("a1-slide-one.jpg"), "{:?}", step.file);
        assert_eq!(
            (step.frames, step.first_seen_at, step.last_seen_at),
            (3, 1, 3)
        );
        // the hash stayed that of the first frame, so a run cannot drift away from the slide
        assert_eq!(step.hash, measured("a5-slide-one-soft.jpg", &config).hash);
        // a dimmer and shakier frame later does not take its place
        let event = session
            .push(&frame("a4-slide-one-dim.jpg"), 4, &config)
            .unwrap();
        assert_eq!(
            event,
            SlideEvent::Duplicate {
                slide: 1,
                step: 1,
                clearer: false
            }
        );
        assert!(
            session.slides[0].steps[0]
                .file
                .ends_with("a1-slide-one.jpg")
        );
    }

    #[test]
    fn the_last_slide_of_a_lecture_is_kept_even_with_one_frame_and_settings_are_checked() {
        let config = SlideConfig::default();
        let mut session = SlideSession::new();
        session.push(&frame("c2-diagram.jpg"), 5, &config).unwrap();
        assert!(session.slides.is_empty());
        assert_eq!(
            session.finish(),
            [SlideEvent::NewSlide { slide: 1, step: 1 }]
        );
        assert!(session.finish().is_empty());
        for bad in [
            SlideConfig {
                duplicate_below: 0.0,
                ..SlideConfig::default()
            },
            SlideConfig {
                duplicate_below: 0.5,
                ..SlideConfig::default()
            },
            SlideConfig {
                confirm_frames: 0,
                ..SlideConfig::default()
            },
            SlideConfig {
                mask_pool: 0,
                ..SlideConfig::default()
            },
            SlideConfig {
                clearer_by: 0.5,
                ..SlideConfig::default()
            },
            SlideConfig {
                max_side: 10,
                ..SlideConfig::default()
            },
        ] {
            assert!(
                matches!(
                    session.push(&frame("a1-slide-one.jpg"), 0, &bad),
                    Err(CaptureError::Config(_))
                ),
                "{bad:?}"
            );
        }
        assert!(SlideConfig::default().validate().is_ok());
        assert!(
            session
                .push(Path::new("/no/such/frame.jpg"), 0, &config)
                .is_err()
        );
    }

    #[test]
    fn a_saved_session_is_reloaded_and_still_knows_its_slides() {
        let config = SlideConfig::default();
        let (session, _) = run(&config);
        let json = serde_json::to_string(&session).unwrap();
        let mut back: SlideSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back, session, "a reloaded session equals the original");
        assert_eq!(back.slides.len(), 3);
        assert!(
            back.slides[0].steps[0].mask.mask.is_none(),
            "masks are not stored"
        );
        // a frame of slide two's last step is recognised from the files, not taken as new
        let event = back
            .push(&frame("b6-slide-two-step3-again.jpg"), 2000, &config)
            .unwrap();
        assert!(
            matches!(
                event,
                SlideEvent::Revisit { slide: 2, step: 3 }
                    | SlideEvent::Duplicate {
                        slide: 2,
                        step: 3,
                        ..
                    }
            ),
            "{event:?}"
        );
        assert!(
            back.slides
                .iter()
                .flat_map(|s| &s.steps)
                .all(|s| s.mask.mask.is_some())
        );
    }

    #[test]
    fn kept_slides_are_read_and_titled_from_their_final_state() {
        let Some(ocr) = Tesseract::detect() else {
            eprintln!("skipped: tesseract is not installed");
            return;
        };
        let (mut session, _) = run(&SlideConfig::default());
        let read = session.read(&ocr).unwrap();
        assert_eq!(
            read, 5,
            "slide one, three steps of slide two and the diagram"
        );
        assert_eq!(session.read(&ocr).unwrap(), 0, "nothing is read twice");
        // light text on a coloured title bar is not read by Tesseract, which reads dark on light, so
        // the title falls back to the first line it did read
        assert!(
            session.slides[..2]
                .iter()
                .all(|s| !s.title.starts_with("Slide ")),
            "{:?}",
            session.slides.iter().map(|s| &s.title).collect::<Vec<_>>()
        );
        assert_eq!(
            session.slides[2].title, "Slide 3",
            "a slide with nothing readable keeps its number"
        );
        assert!(
            session.slides[0]
                .final_step()
                .unwrap()
                .lines
                .iter()
                .any(|l| l.to_lowercase().contains("harmonics"))
        );
        let last = session.slides[1].final_step().unwrap();
        assert!(
            last.lines
                .iter()
                .any(|l| l.to_lowercase().contains("zeros")),
            "{:?}",
            last.lines
        );
        let first = &session.slides[1].steps[0];
        assert!(
            !first
                .lines
                .iter()
                .any(|l| l.to_lowercase().contains("zeros")),
            "the first step lacks the later bullets"
        );
    }

    #[test]
    fn slides_are_placed_on_the_lecture_timeline_with_their_steps_and_returns() {
        use crate::timeline::{Recording, Timeline};
        let (mut session, _) = run(&SlideConfig::default());
        session.read(&NoOcr).unwrap();
        let mut timeline = Timeline::new("Signals", Recording::new("lecture.wav", 990, 60.0));
        timeline.set_slides(&session);
        let marks: Vec<(u32, u32, f32)> = timeline
            .captures
            .iter()
            .map(|c| (c.board - SLIDE_ID_BASE, c.version, c.at))
            .collect();
        assert_eq!(
            marks,
            [
                (1, 1, 10.0),
                (2, 1, 15.0),
                (2, 2, 17.0),
                (2, 3, 19.0),
                (3, 1, 22.0)
            ],
            "{marks:?}"
        );
        assert!(
            timeline
                .captures
                .iter()
                .all(|c| c.kind == CaptureKind::Slide && c.capture > SLIDE_ID_BASE)
        );
        let first = &timeline.captures[0];
        assert_eq!(
            first.last_seen_at, 25.0,
            "slide one was last on screen when the lecturer came back"
        );
        assert_eq!(
            timeline
                .position(18.0)
                .capture
                .map(|c| (c.board - SLIDE_ID_BASE, c.version)),
            Some((2, 2))
        );
        // setting slides again replaces them and setting boards keeps them
        timeline.set_slides(&session);
        assert_eq!(timeline.captures.len(), 5);
        timeline.set_captures(&crate::capture::CaptureSession::new());
        assert_eq!(timeline.captures.len(), 5);
        // each new slide is a chapter signal, so a long lecture gets chapters from its slides
        let mut long = Timeline::new("Long", Recording::new("l.wav", 990, 1200.0));
        long.set_slides(&session);
        assert_eq!(long.captures.iter().filter(|c| c.version == 1).count(), 3);
    }

    #[test]
    fn a_return_to_any_state_of_a_built_slide_is_a_return() {
        let config = SlideConfig::default();
        let (mut session, _) = run(&config);
        let before = session.slides[1].steps.len();
        for (name, step) in [("b3-slide-two-step2.jpg", 2), ("b1-slide-two-step1.jpg", 1)] {
            // go to another slide first so slide two is not the one on screen
            session
                .push(&frame("c3-diagram-again.jpg"), 3000, &config)
                .unwrap();
            let event = session.push(&frame(name), 3001, &config).unwrap();
            assert_eq!(event, SlideEvent::Revisit { slide: 2, step }, "{name}");
        }
        assert_eq!(session.slides[1].steps.len(), before);
    }

    #[test]
    fn a_slide_that_never_settles_stops_growing_at_the_step_cap() {
        let config = SlideConfig {
            max_steps: 2,
            ..SlideConfig::default()
        };
        let mut session = SlideSession::new();
        let mut events = Vec::new();
        for (i, name) in [
            "b1-slide-two-step1.jpg",
            "b2-slide-two-step1-again.jpg",
            "b3-slide-two-step2.jpg",
            "b4-slide-two-step2-again.jpg",
            "b5-slide-two-step3.jpg",
            "b6-slide-two-step3-again.jpg",
        ]
        .iter()
        .enumerate()
        {
            events.push(session.push(&frame(name), i as i64, &config).unwrap());
        }
        assert_eq!(session.slides.len(), 1);
        assert_eq!(session.slides[0].steps.len(), 2, "{events:#?}");
        assert!(
            matches!(events[4], SlideEvent::Duplicate { .. }),
            "{events:#?}"
        );
    }

    #[test]
    fn frames_with_no_content_to_compare_fall_back_to_the_hash() {
        let config = SlideConfig::default();
        let hash = measured("a1-slide-one.jpg", &config).hash;
        let other = measured("b1-slide-two-step1.jpg", &config).hash;
        assert!(hash.distance(&other) > config.near_duplicate_below);
        let empty = InkMask::new(160, 90, vec![false; 160 * 90]);
        // two empty masks say nothing, so the hash decides
        assert_eq!(
            compare((&hash, Some(&empty)), (&other, Some(&empty)), &config).verdict,
            Match::New
        );
        assert_eq!(
            compare((&hash, Some(&empty)), (&hash, Some(&empty)), &config).verdict,
            Match::Duplicate
        );
        // the settings must be in order
        let bad = SlideConfig {
            near_duplicate_below: 0.4,
            ..SlideConfig::default()
        };
        assert!(matches!(bad.validate(), Err(CaptureError::Config(_))));
        let mut bad = SlideConfig::default();
        bad.change.repeat_below = 0.9;
        assert!(bad.validate().is_err());
        assert!(
            SlideConfig {
                max_steps: 0,
                ..SlideConfig::default()
            }
            .validate()
            .is_err()
        );
    }
}
