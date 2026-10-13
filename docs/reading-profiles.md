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
| `contrast` | string | `dark_on_light`, `light_on_dark`, `yellow_on_black` or `black_on_cream` |
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
