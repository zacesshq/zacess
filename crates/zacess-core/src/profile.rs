//! Reading profiles: how a person wants every document laid out.
//!
//! A profile is applied to a document as a view. The document itself is borrowed immutably, so
//! applying a profile can never change it. Profiles never change silently either: any adaptation is
//! offered as a list of [`SettingChange`]s for the person to accept. The JSON form is documented in
//! `docs/reading-profiles.md`.

use crate::model::Document;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// The version of the profile JSON written by this build. The rules match the document model's.
pub const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("profile uses schema version {found}, this build reads up to {PROFILE_SCHEMA_VERSION}")]
    UnsupportedVersion { found: u32 },
    #[error("invalid profile JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid profile: {0}")]
    Invalid(String),
    #[error("a profile called \"{0}\" already exists")]
    NameTaken(String),
    #[error("no profile called \"{0}\"")]
    NotFound(String),
    #[error("profile storage: {0}")]
    Io(#[from] std::io::Error),
}

/// The generic family to fall back on when a named font is not installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontFallback {
    SansSerif,
    Serif,
    Monospace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Font {
    /// A font family name, such as "Atkinson Hyperlegible". Empty means the system font.
    pub family: String,
    pub fallback: FontFallback,
}

/// Text and background colours. Every theme meets WCAG AAA contrast (7:1) for body text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContrastTheme {
    DarkOnLight,
    LightOnDark,
    YellowOnBlack,
    BlackOnCream,
}

/// The colours a theme resolves to, as `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub text: &'static str,
    pub background: &'static str,
    pub link: &'static str,
    /// The background behind low-confidence text that needs checking.
    pub uncertain: &'static str,
    pub focus: &'static str,
    pub ruler: &'static str,
}

impl ContrastTheme {
    pub fn palette(self) -> Palette {
        match self {
            ContrastTheme::DarkOnLight => Palette {
                text: "#111111",
                background: "#ffffff",
                link: "#0b3d91",
                uncertain: "#ffe58a",
                focus: "#0b3d91",
                ruler: "#dbe7ff",
            },
            ContrastTheme::LightOnDark => Palette {
                text: "#f5f5f5",
                background: "#121212",
                link: "#9cc4ff",
                uncertain: "#5c4300",
                focus: "#9cc4ff",
                ruler: "#24324a",
            },
            ContrastTheme::YellowOnBlack => Palette {
                text: "#ffeb3b",
                background: "#000000",
                link: "#8fd3ff",
                uncertain: "#4a2a00",
                focus: "#ffffff",
                ruler: "#2a2a2a",
            },
            ContrastTheme::BlackOnCream => Palette {
                text: "#000000",
                background: "#fdf6e3",
                link: "#003a8c",
                uncertain: "#ffd27a",
                focus: "#003a8c",
                ruler: "#efe3c0",
            },
        }
    }
}

/// How large the text cursor and pointer are drawn, as a multiple of the system size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorSize {
    Standard,
    Large,
    ExtraLarge,
}

impl CursorSize {
    pub fn scale(self) -> f32 {
        match self {
            CursorSize::Standard => 1.0,
            CursorSize::Large => 2.0,
            CursorSize::ExtraLarge => 3.0,
        }
    }
}

/// A band that follows the line being read.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ReadingRuler {
    pub enabled: bool,
    /// Where the band sits, from 0 (top of the view) to 1 (bottom).
    pub position: f32,
    /// How many lines the band covers.
    pub lines: u8,
}

/// Every setting a reading profile holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingProfile {
    pub schema_version: u32,
    pub name: String,
    pub font: Font,
    /// Body text size in points.
    pub font_size_pt: f32,
    /// Line height as a multiple of the font size.
    pub line_spacing: f32,
    /// Space after each paragraph, in multiples of the font size.
    pub paragraph_spacing: f32,
    /// Extra space between letters, in multiples of the font size.
    pub character_spacing: f32,
    /// The longest line, in average character widths.
    pub column_width: u16,
    pub contrast: ContrastTheme,
    pub cursor_size: CursorSize,
    pub reading_ruler: ReadingRuler,
    /// One column with nothing side by side.
    #[serde(default = "yes")]
    pub single_column: bool,
    /// No animation or moving content.
    #[serde(default = "yes")]
    pub reduce_motion: bool,
    /// Hide decorative images and borders that carry no meaning.
    #[serde(default)]
    pub hide_decoration: bool,
}

fn yes() -> bool {
    true
}

/// The ready-made starting points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    LowVision,
    DyslexiaFriendly,
    ReducedVisualClutter,
    ScreenReader,
}

impl Preset {
    pub const ALL: [Preset; 4] = [
        Preset::LowVision,
        Preset::DyslexiaFriendly,
        Preset::ReducedVisualClutter,
        Preset::ScreenReader,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Preset::LowVision => "Low Vision",
            Preset::DyslexiaFriendly => "Dyslexia-Friendly",
            Preset::ReducedVisualClutter => "Reduced Visual Clutter",
            Preset::ScreenReader => "Screen Reader",
        }
    }
}

impl Default for ReadingProfile {
    fn default() -> Self {
        Self {
            schema_version: PROFILE_SCHEMA_VERSION,
            name: "Standard".into(),
            font: Font {
                family: String::new(),
                fallback: FontFallback::SansSerif,
            },
            font_size_pt: 12.0,
            line_spacing: 1.5,
            paragraph_spacing: 1.0,
            character_spacing: 0.0,
            column_width: 70,
            contrast: ContrastTheme::DarkOnLight,
            cursor_size: CursorSize::Standard,
            reading_ruler: ReadingRuler {
                enabled: false,
                position: 0.4,
                lines: 1,
            },
            single_column: true,
            reduce_motion: true,
            hide_decoration: false,
        }
    }
}

impl ReadingProfile {
    pub fn preset(preset: Preset) -> Self {
        let base = Self {
            name: preset.name().into(),
            ..Self::default()
        };
        match preset {
            Preset::LowVision => Self {
                font: Font {
                    family: "Atkinson Hyperlegible".into(),
                    fallback: FontFallback::SansSerif,
                },
                font_size_pt: 24.0,
                line_spacing: 1.6,
                paragraph_spacing: 1.5,
                character_spacing: 0.02,
                // short lines so the eye never travels far at a large size
                column_width: 45,
                contrast: ContrastTheme::YellowOnBlack,
                cursor_size: CursorSize::ExtraLarge,
                reading_ruler: ReadingRuler {
                    enabled: true,
                    position: 0.4,
                    lines: 1,
                },
                ..base
            },
            Preset::DyslexiaFriendly => Self {
                font: Font {
                    family: "Lexend".into(),
                    fallback: FontFallback::SansSerif,
                },
                font_size_pt: 16.0,
                // British Dyslexia Association style guide asks for at least 1.5 line spacing and wider
                // letter spacing
                line_spacing: 1.75,
                paragraph_spacing: 2.0,
                character_spacing: 0.12,
                column_width: 60,
                contrast: ContrastTheme::BlackOnCream,
                cursor_size: CursorSize::Large,
                reading_ruler: ReadingRuler {
                    enabled: true,
                    position: 0.4,
                    lines: 2,
                },
                ..base
            },
            Preset::ReducedVisualClutter => Self {
                font_size_pt: 14.0,
                paragraph_spacing: 1.5,
                column_width: 65,
                hide_decoration: true,
                ..base
            },
            // a screen reader user hears structure rather than layout, so the visual settings stay
            // plain and the ruler is off to keep focus handling simple
            Preset::ScreenReader => Self {
                font_size_pt: 14.0,
                hide_decoration: true,
                reading_ruler: ReadingRuler {
                    enabled: false,
                    position: 0.4,
                    lines: 1,
                },
                ..base
            },
        }
    }

    /// Every problem with the settings, empty when the profile is valid.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.name.trim().is_empty() {
            out.push("name is empty".to_string());
        }
        let ranges: [(&str, f32, f32, f32); 4] = [
            ("font_size_pt", self.font_size_pt, 6.0, 96.0),
            ("line_spacing", self.line_spacing, 1.0, 3.0),
            ("paragraph_spacing", self.paragraph_spacing, 0.0, 5.0),
            ("character_spacing", self.character_spacing, 0.0, 0.5),
        ];
        for (field, value, min, max) in ranges {
            if !(min..=max).contains(&value) {
                out.push(format!("{field} {value} is outside {min} to {max}"));
            }
        }
        if !(20..=150).contains(&self.column_width) {
            out.push(format!(
                "column_width {} is outside 20 to 150",
                self.column_width
            ));
        }
        if !(0.0..=1.0).contains(&self.reading_ruler.position) {
            out.push(format!(
                "reading_ruler.position {} is outside 0 to 1",
                self.reading_ruler.position
            ));
        }
        if !(1..=5).contains(&self.reading_ruler.lines) {
            out.push(format!(
                "reading_ruler.lines {} is outside 1 to 5",
                self.reading_ruler.lines
            ));
        }
        out
    }

    pub fn validate(&self) -> Result<(), ProfileError> {
        match self.problems().as_slice() {
            [] => Ok(()),
            problems => Err(ProfileError::Invalid(problems.join("; "))),
        }
    }

    pub fn from_json(json: &str) -> Result<Self, ProfileError> {
        #[derive(Deserialize)]
        struct VersionOnly {
            schema_version: u32,
        }
        let VersionOnly { schema_version } = serde_json::from_str(json)?;
        if schema_version > PROFILE_SCHEMA_VERSION {
            return Err(ProfileError::UnsupportedVersion {
                found: schema_version,
            });
        }
        let mut profile: ReadingProfile = serde_json::from_str(json)?;
        profile.schema_version = PROFILE_SCHEMA_VERSION;
        profile.validate()?;
        Ok(profile)
    }

    /// The documented JSON form, used for saving and exporting.
    pub fn to_json(&self) -> Result<String, ProfileError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Applies the profile to a document as a read-only view.
    pub fn apply<'a>(&'a self, document: &'a Document) -> ProfiledDocument<'a> {
        ProfiledDocument {
            document,
            profile: self,
        }
    }

    /// The settings that differ between two profiles, so a change is always shown before it is
    /// made rather than applied silently.
    pub fn changes_to(&self, other: &ReadingProfile) -> Vec<SettingChange> {
        let a = serde_json::to_value(self).expect("profiles always serialise");
        let b = serde_json::to_value(other).expect("profiles always serialise");
        let mut out = Vec::new();
        diff_values("", &a, &b, &mut out);
        out.retain(|c| c.setting != "name" && c.setting != "schema_version");
        out
    }
}

/// One setting that would change, with its old and new values as JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingChange {
    pub setting: String,
    pub from: serde_json::Value,
    pub to: serde_json::Value,
}

impl fmt::Display for SettingChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} to {}", self.setting, self.from, self.to)
    }
}

fn diff_values(
    path: &str,
    a: &serde_json::Value,
    b: &serde_json::Value,
    out: &mut Vec<SettingChange>,
) {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for (key, av) in a {
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                let bv = b.get(key).unwrap_or(&serde_json::Value::Null);
                diff_values(&child, av, bv, out);
            }
        }
        _ if a != b => out.push(SettingChange {
            setting: path.to_string(),
            from: a.clone(),
            to: b.clone(),
        }),
        _ => {}
    }
}

/// A document seen through a profile. It only borrows the document, so the original is untouched.
#[derive(Debug, Clone, Copy)]
pub struct ProfiledDocument<'a> {
    pub document: &'a Document,
    pub profile: &'a ReadingProfile,
}

impl ProfiledDocument<'_> {
    /// A style sheet for the reading view and for HTML export.
    pub fn css(&self) -> String {
        profile_css(self.profile)
    }
}

/// The CSS for a profile. Class names are shared with the HTML export.
pub fn profile_css(p: &ReadingProfile) -> String {
    let palette = p.contrast.palette();
    let light = matches!(
        p.contrast,
        ContrastTheme::DarkOnLight | ContrastTheme::BlackOnCream
    );
    let scheme = if light { "light" } else { "dark" };
    // multiply darkens only the background on a light theme and screen lightens only the
    // background on a dark one, so the ruler never lowers the text contrast
    let blend = if light { "multiply" } else { "screen" };
    let fallback = match p.font.fallback {
        FontFallback::SansSerif => "sans-serif",
        FontFallback::Serif => "serif",
        FontFallback::Monospace => "monospace",
    };
    let family = if p.font.family.is_empty() {
        format!("system-ui, {fallback}")
    } else {
        format!("\"{}\", {fallback}", p.font.family.replace(['"', '\\'], ""))
    };
    let mut css = format!(
        ":root {{ color-scheme: {scheme}; }}\n\
         body {{ margin: 0; background: {bg}; color: {fg}; }}\n\
         .zc-doc {{ font-family: {family}; font-size: {size}pt; line-height: {line}; \
         letter-spacing: {chars}em; max-width: {width}ch; margin: 0 auto; padding: 1em; }}\n\
         .zc-doc p, .zc-doc ul, .zc-doc ol, .zc-doc table, .zc-doc figure {{ margin: 0 0 {para}em; }}\n\
         .zc-doc a {{ color: {link}; }}\n\
         .zc-doc :focus-visible {{ outline: 3px solid {focus}; outline-offset: 2px; }}\n\
         .zc-uncertain {{ background: {uncertain}; color: inherit; \
         text-decoration: underline dotted; text-underline-offset: 0.2em; }}\n\
         .zc-sr {{ position: absolute; width: 1px; height: 1px; overflow: hidden; \
         clip: rect(0 0 0 0); white-space: nowrap; }}\n\
         .zc-doc table {{ border-collapse: collapse; }}\n\
         .zc-doc th, .zc-doc td {{ border: 1px solid currentColor; padding: 0.25em 0.5em; }}\n",
        scheme = scheme,
        bg = palette.background,
        fg = palette.text,
        size = p.font_size_pt,
        line = p.line_spacing,
        chars = p.character_spacing,
        width = p.column_width,
        para = p.paragraph_spacing,
        link = palette.link,
        focus = palette.focus,
        uncertain = palette.uncertain,
    );
    if p.reading_ruler.enabled {
        css.push_str(&format!(
            ".zc-ruler {{ position: fixed; left: 0; right: 0; top: {top}%; \
             height: calc({lines} * {line}em); background: {ruler}; pointer-events: none; \
             mix-blend-mode: {blend}; }}\n",
            top = p.reading_ruler.position * 100.0,
            lines = p.reading_ruler.lines,
            line = p.line_spacing,
            ruler = palette.ruler,
        ));
    }
    if p.reduce_motion {
        css.push_str("* { animation: none !important; transition: none !important; scroll-behavior: auto !important; }\n");
    }
    if p.hide_decoration {
        css.push_str(".zc-decorative { display: none; }\n");
    }
    css
}

/// Saved, named profiles kept as one JSON file each in a folder.
#[derive(Debug, Clone)]
pub struct ProfileStore {
    dir: PathBuf,
}

impl ProfileStore {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, ProfileError> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path_for(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{}.json", file_stem(name)))
    }

    /// Every saved profile, sorted by name.
    pub fn list(&self) -> Result<Vec<ReadingProfile>, ProfileError> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "json") {
                out.push(ReadingProfile::from_json(&fs::read_to_string(&path)?)?);
            }
        }
        out.sort_by_key(|p| p.name.to_lowercase());
        Ok(out)
    }

    pub fn load(&self, name: &str) -> Result<ReadingProfile, ProfileError> {
        let path = self.path_for(name);
        if !path.exists() {
            return Err(ProfileError::NotFound(name.to_string()));
        }
        ReadingProfile::from_json(&fs::read_to_string(path)?)
    }

    /// Saves a new profile. Names are unique, ignoring case and punctuation.
    pub fn create(&self, profile: &ReadingProfile) -> Result<(), ProfileError> {
        profile.validate()?;
        if self.path_for(&profile.name).exists() {
            return Err(ProfileError::NameTaken(profile.name.clone()));
        }
        self.write(profile)
    }

    /// Saves changes to an existing profile.
    pub fn update(&self, profile: &ReadingProfile) -> Result<(), ProfileError> {
        profile.validate()?;
        if !self.path_for(&profile.name).exists() {
            return Err(ProfileError::NotFound(profile.name.clone()));
        }
        self.write(profile)
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), ProfileError> {
        let mut profile = self.load(from)?;
        let same_file = self.path_for(from) == self.path_for(to);
        if !same_file && self.path_for(to).exists() {
            return Err(ProfileError::NameTaken(to.to_string()));
        }
        profile.name = to.to_string();
        profile.validate()?;
        self.write(&profile)?;
        if !same_file {
            fs::remove_file(self.path_for(from))?;
        }
        Ok(())
    }

    pub fn delete(&self, name: &str) -> Result<(), ProfileError> {
        let path = self.path_for(name);
        if !path.exists() {
            return Err(ProfileError::NotFound(name.to_string()));
        }
        Ok(fs::remove_file(path)?)
    }

    /// Writes a profile as a standalone JSON file for sharing or backup.
    pub fn export(&self, name: &str, to: &Path) -> Result<(), ProfileError> {
        let profile = self.load(name)?;
        Ok(fs::write(to, profile.to_json()?)?)
    }

    /// Reads a profile exported elsewhere and saves it here.
    pub fn import(&self, from: &Path) -> Result<ReadingProfile, ProfileError> {
        let profile = ReadingProfile::from_json(&fs::read_to_string(from)?)?;
        self.create(&profile)?;
        Ok(profile)
    }

    fn write(&self, profile: &ReadingProfile) -> Result<(), ProfileError> {
        let path = self.path_for(&profile.name);
        // write then rename so a crash part way through never leaves a half-written profile
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, profile.to_json()?)?;
        fs::rename(tmp, path)?;
        Ok(())
    }
}

/// A file name for a profile name: lowercase letters and digits with single hyphens.
fn file_stem(name: &str) -> String {
    let mut stem = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            stem.push(c);
        } else if !stem.ends_with('-') && !stem.is_empty() {
            stem.push('-');
        }
    }
    let stem = stem.trim_end_matches('-').to_string();
    if stem.is_empty() {
        "profile".into()
    } else {
        stem
    }
}

/// WCAG relative luminance of a `#rrggbb` colour.
pub fn relative_luminance(hex: &str) -> f64 {
    let hex = hex.trim_start_matches('#');
    let channel = |i: usize| {
        let v = u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
}

/// WCAG contrast ratio between two `#rrggbb` colours, from 1 to 21.
pub fn contrast_ratio(a: &str, b: &str) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlockKind, Origin, Rect, Run, Source, SourceFormat};

    fn document() -> Document {
        let mut doc = Document::new(
            "Notes",
            Source {
                file_name: "n.pdf".into(),
                format: SourceFormat::Pdf,
            },
        );
        let origin = Origin::new(1, Rect::new(0.0, 0.0, 10.0, 10.0));
        doc.push(
            BlockKind::paragraph(vec![Run::native("Ohm's law", origin)]),
            origin,
        );
        doc
    }

    #[test]
    fn every_preset_is_valid_and_named() {
        for preset in Preset::ALL {
            let profile = ReadingProfile::preset(preset);
            assert_eq!(profile.name, preset.name());
            assert_eq!(profile.problems(), Vec::<String>::new(), "{preset:?}");
        }
        assert!(ReadingProfile::preset(Preset::LowVision).font_size_pt >= 18.0);
        assert!(ReadingProfile::preset(Preset::DyslexiaFriendly).character_spacing > 0.0);
        assert!(ReadingProfile::preset(Preset::ReducedVisualClutter).hide_decoration);
    }

    #[test]
    fn every_theme_meets_aaa_contrast() {
        for theme in [
            ContrastTheme::DarkOnLight,
            ContrastTheme::LightOnDark,
            ContrastTheme::YellowOnBlack,
            ContrastTheme::BlackOnCream,
        ] {
            let p = theme.palette();
            assert!(
                contrast_ratio(p.text, p.background) >= 7.0,
                "{theme:?} text"
            );
            assert!(
                contrast_ratio(p.text, p.uncertain) >= 7.0,
                "{theme:?} uncertain text"
            );
            assert!(
                contrast_ratio(p.link, p.background) >= 7.0,
                "{theme:?} link"
            );
            assert!(
                contrast_ratio(p.focus, p.background) >= 3.0,
                "{theme:?} focus ring"
            );
        }
        assert!((contrast_ratio("#000000", "#ffffff") - 21.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_out_of_range_settings() {
        let mut p = ReadingProfile {
            font_size_pt: 200.0,
            column_width: 5,
            ..Default::default()
        };
        p.reading_ruler.position = 1.5;
        let problems = p.problems();
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(matches!(p.validate(), Err(ProfileError::Invalid(_))));
    }

    #[test]
    fn json_round_trips_and_refuses_newer_versions() {
        let profile = ReadingProfile::preset(Preset::DyslexiaFriendly);
        let back = ReadingProfile::from_json(&profile.to_json().unwrap()).unwrap();
        assert_eq!(back, profile);

        let mut value = serde_json::to_value(&profile).unwrap();
        value["schema_version"] = (PROFILE_SCHEMA_VERSION + 1).into();
        assert!(matches!(
            ReadingProfile::from_json(&value.to_string()),
            Err(ProfileError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn documented_example_parses() {
        let json = include_str!("../../../docs/examples/profile.json");
        let profile = ReadingProfile::from_json(json).unwrap();
        assert_eq!(profile.name, "Lectures");
        assert_eq!(profile.contrast, ContrastTheme::YellowOnBlack);
    }

    #[test]
    fn applying_a_profile_never_alters_the_document() {
        let doc = document();
        let before = doc.clone();
        for preset in Preset::ALL {
            let profile = ReadingProfile::preset(preset);
            let view = profile.apply(&doc);
            let css = view.css();
            assert!(css.contains(".zc-doc"));
            assert!(std::ptr::eq(view.document, &doc));
        }
        assert_eq!(doc, before);
    }

    #[test]
    fn css_reflects_the_settings() {
        let css = profile_css(&ReadingProfile::preset(Preset::LowVision));
        assert!(css.contains("font-size: 24pt"));
        assert!(css.contains("max-width: 45ch"));
        assert!(css.contains("\"Atkinson Hyperlegible\", sans-serif"));
        assert!(css.contains(".zc-ruler"));
        assert!(
            css.contains("mix-blend-mode: screen"),
            "a dark theme lightens under the ruler"
        );
        assert!(
            profile_css(&ReadingProfile::preset(Preset::DyslexiaFriendly))
                .contains("mix-blend-mode: multiply")
        );
        assert!(css.contains("animation: none"));
        assert!(!profile_css(&ReadingProfile::preset(Preset::ScreenReader)).contains(".zc-ruler"));
    }

    #[test]
    fn changes_are_listed_rather_than_applied_silently() {
        let a = ReadingProfile::preset(Preset::ReducedVisualClutter);
        let mut b = a.clone();
        b.font_size_pt = 18.0;
        b.reading_ruler.enabled = true;
        b.name = "Renamed".into();
        let changes = a.changes_to(&b);
        let settings: Vec<&str> = changes.iter().map(|c| c.setting.as_str()).collect();
        assert_eq!(settings, ["font_size_pt", "reading_ruler.enabled"]);
        assert_eq!(changes[0].to_string(), "font_size_pt: 14.0 to 18.0");
        assert!(a.changes_to(&a).is_empty());
    }

    #[test]
    fn store_saves_names_renames_exports_and_imports() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::open(dir.path().join("profiles")).unwrap();

        let mut lectures = ReadingProfile::preset(Preset::LowVision);
        lectures.name = "Lectures".into();
        store.create(&lectures).unwrap();
        store
            .create(&ReadingProfile::preset(Preset::DyslexiaFriendly))
            .unwrap();
        assert!(matches!(
            store.create(&lectures),
            Err(ProfileError::NameTaken(_))
        ));
        // the same name with different case or spacing is the same profile
        let clash = ReadingProfile {
            name: "  LECTURES ".into(),
            ..lectures.clone()
        };
        assert!(matches!(
            store.create(&clash),
            Err(ProfileError::NameTaken(_))
        ));

        let names: Vec<String> = store.list().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Dyslexia-Friendly", "Lectures"]);

        lectures.font_size_pt = 28.0;
        store.update(&lectures).unwrap();
        assert_eq!(store.load("Lectures").unwrap().font_size_pt, 28.0);

        store.rename("Lectures", "Revision").unwrap();
        assert!(matches!(
            store.load("Lectures"),
            Err(ProfileError::NotFound(_))
        ));
        assert_eq!(store.load("Revision").unwrap().font_size_pt, 28.0);

        let exported = dir.path().join("revision.json");
        store.export("Revision", &exported).unwrap();
        store.delete("Revision").unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        let imported = store.import(&exported).unwrap();
        assert_eq!(imported.name, "Revision");
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn file_stems_are_safe() {
        assert_eq!(file_stem("Low Vision"), "low-vision");
        assert_eq!(file_stem("../../etc/passwd"), "etc-passwd");
        assert_eq!(file_stem("***"), "profile");
    }
}
