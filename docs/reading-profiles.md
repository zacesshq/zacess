# Reading profiles

A reading profile is how a person wants every document laid out. It lives in
`crates/zacess-core/src/profile.rs`.

## Rules

- **Applying a profile never alters the document.** A profile is applied as a read-only view that
  borrows the document, so the original stays exactly as imported.
- **Profiles never change silently.** Any adaptation is offered as a list of setting changes, each with
  its old and new value, for the person to accept or refuse.
- **Every contrast theme meets WCAG AAA** (7:1) for body text, links and highlighted low-confidence
  text. A unit test checks each one.

## Presets

| Preset | Font | Size | Line spacing | Letter spacing | Line length | Theme | Extras |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Low Vision | Atkinson Hyperlegible | 24 pt | 1.6 | 0.02 em | 45 characters | Yellow on black | Extra large cursor, reading ruler |
| Dyslexia-Friendly | Lexend | 16 pt | 1.75 | 0.12 em | 60 characters | Black on cream | Large cursor, two-line reading ruler |
| Reduced Visual Clutter | System | 14 pt | 1.5 | None | 65 characters | Dark on light | Decoration hidden |
| Screen Reader | System | 14 pt | 1.5 | None | 70 characters | Dark on light | Decoration hidden, no ruler |

Every preset uses one column and no motion. A preset is a starting point: it can be saved under a new
name and changed.

## Saving and exporting

Profiles are saved one JSON file per profile. Names are unique, ignoring case, spacing and punctuation.
A profile can be created, updated, renamed, deleted, exported to a standalone file and imported again.
An exported file is the JSON form below, so it can be shared or kept as a backup.

## JSON form

An example is in [`examples/profile.json`](examples/profile.json). A unit test parses that file.

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | The version of this JSON form, currently `1`. Versioning follows the [document model rules](document-model.md#versioning) |
| `name` | string | The profile name shown to the person |
| `font.family` | string | A font family name. Empty means the system font |
| `font.fallback` | string | `sans_serif`, `serif` or `monospace`, used when the font is not installed |
| `font_size_pt` | number | Body text size in points, 6 to 96 |
| `line_spacing` | number | Line height as a multiple of the font size, 1 to 3 |
| `paragraph_spacing` | number | Space after each paragraph in multiples of the font size, 0 to 5 |
| `character_spacing` | number | Extra space between letters in multiples of the font size, 0 to 0.5 |
| `column_width` | integer | The longest line in average character widths, 20 to 150 |
| `contrast` | string | `dark_on_light`, `light_on_dark`, `yellow_on_black`, `black_on_cream`, `high_contrast_dark`, `high_contrast_light`, `deep_contrast_dark` or `deep_contrast_light` |
| `cursor_size` | string | `standard`, `large` (twice the system size) or `extra_large` (three times) |
| `reading_ruler.enabled` | boolean | Whether a band follows the line being read |
| `reading_ruler.position` | number | Where the band sits, from 0 (top of the view) to 1 (bottom) |
| `reading_ruler.lines` | integer | How many lines the band covers, 1 to 5 |
| `single_column` | boolean | One column with nothing side by side. `true` when missing |
| `reduce_motion` | boolean | No animation or moving content. `true` when missing |
| `hide_decoration` | boolean | Hide decorative images and borders that carry no meaning. `false` when missing |

## Contrast themes

| Theme | Text | Background | Contrast |
| --- | --- | --- | --- |
| `dark_on_light` | `#111111` | `#ffffff` | 18.9:1 |
| `light_on_dark` | `#f5f5f5` | `#121212` | 17.2:1 |
| `yellow_on_black` | `#ffeb3b` | `#000000` | 17.2:1 |
| `black_on_cream` | `#000000` | `#fdf6e3` | 19.5:1 |
| `high_contrast_dark` | `#ffffff` | `#000000` | 21:1 |
| `high_contrast_light` | `#000000` | `#ffffff` | 21:1 |
| `deep_contrast_dark` | `#ffffff` | `#000000` | 21:1 |
| `deep_contrast_light` | `#000000` | `#ffffff` | 21:1 |

The two high contrast themes match the High Contrast Dark and High Contrast Light
[code themes](https://zacess.com/themes/). They take the code themes' link and focus colours, use
the selection colour for the reading ruler and use the other theme's warning colour behind
low-confidence text. Adding them changed none of the original four ids, so a profile saved before
them loads unchanged. A unit test reads every id back.

The two deep contrast themes match the Deep Contrast Dark and Deep Contrast Light code themes,
with a yellow focus ring on the dark one and a red focus ring on the light one. Their links are a
shade lighter on the dark theme and a shade darker on the light one so they keep 7:1 on the
reading ruler and the low-confidence background as well as the page. They come after the high
contrast themes, so every earlier id keeps its meaning.

### Every colour in each theme

Contrast is measured with the WCAG 2.2 relative luminance formula (`contrast_ratio` in
`profile.rs`), against the theme's background unless the column says otherwise.

| Theme | Link | Low-confidence background (text on it) | Focus ring | Reading ruler (text on it) |
| --- | --- | --- | --- | --- |
| `dark_on_light` | `#0b3d91`, 10.0:1 | `#ffe58a`, 15.1:1 | `#0b3d91`, 10.0:1 | `#dbe7ff`, 15.2:1 |
| `light_on_dark` | `#9cc4ff`, 10.5:1 | `#5c4300`, 8.5:1 | `#9cc4ff`, 10.5:1 | `#24324a`, 11.8:1 |
| `yellow_on_black` | `#8fd3ff`, 12.9:1 | `#4a2a00`, 10.6:1 | `#ffffff`, 21:1 | `#2a2a2a`, 11.8:1 |
| `black_on_cream` | `#003a8c`, 9.8:1 | `#ffd27a`, 14.8:1 | `#003a8c`, 9.8:1 | `#efe3c0`, 16.4:1 |
| `high_contrast_dark` | `#8fd0ff`, 12.6:1 | `#5a3a00`, 10.3:1 | `#f5c400`, 12.8:1 | `#0a2552`, 15.0:1 |
| `high_contrast_light` | `#003d99`, 9.9:1 | `#ffd28a`, 14.8:1 | `#003d99`, 9.9:1 | `#fff1a0`, 18.3:1 |
| `deep_contrast_dark` | `#6bceff`, 11.9:1 | `#403300`, 12.4:1 | `#ffff00`, 19.6:1 | `#1c1c38`, 16.5:1 |
| `deep_contrast_light` | `#0037df`, 8.0:1 | `#fff1a0`, 18.3:1 | `#b5200d`, 6.6:1 | `#e8f0ff`, 18.3:1 |

Low-confidence text also carries a dotted underline and the focus ring is an outline, so neither
rests on colour alone. In the high and deep contrast themes the low-confidence background, the
focus ring and the ruler are three different colours. Links keep at least 7:1 on the ruler and,
in the deep contrast themes, on the low-confidence background too. Unit tests check all of this.
