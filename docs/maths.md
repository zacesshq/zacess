# Maths and engineering mode

How equations, subscripts, superscripts, circuit symbols and technical tables survive import and
reach the reader as structure, speech and keyboard navigation. The code is in
`crates/zacess-core/src/maths` and the import rules in [import.md](import.md#layout-analysis).

## What is preserved

| Material | Import | In the document model | Export |
| --- | --- | --- | --- |
| Subscripts and superscripts | Smaller text shifted off the baseline | `script` on the run | `<sub>` and `<sup>` in text and MathML scripts inside an equation |
| Equations | A short block with a relation such as `=` and mostly symbols | An `equation` block with its source rebuilt as LaTeX, for example `V_{out} = V_{in} × R_{2} / (R_{1} + R_{2})` | Presentation MathML with the source in an annotation and the natural reading in `alttext` |
| Technical tables | Consecutive rows of short, aligned cells | A `table` block with header cells | `<table>` with `<th scope>` |
| Circuit and maths symbols | Kept as text with their confidence | Runs | Spoken by name |

## The equation parser

`maths::parse` turns plain text or simple LaTeX into a tree of numbers, identifiers, operators,
subscripts, superscripts, fractions, roots and text. It understands Greek letters, common
functions, `\frac`, `\sqrt`, `\angle`, `\parallel` and the usual relations. Anything else stays as
text, so nothing is dropped. `maths::to_mathml` renders the tree as presentation MathML.

## Natural speech

`maths::speech::speak` reads an equation the way a lecturer says it.

| Written | Spoken |
| --- | --- |
| `V_{out} = I R` | V out equals I R |
| `P = I^2 R` | P equals I squared R |
| `\frac{R_2}{R_1 + R_2}` | the fraction R 2 over R 1 plus R 2 end fraction |
| `Z = R \angle 45°` | Z equals R angle 45 degrees |
| `R_1 \parallel R_2` | R 1 in parallel with R 2 |
| `4.7 kΩ` | 4.7 kilo-ohms |
| `12 ± 5%` | 12 plus or minus 5 percent |

A subscript is read as part of the name, a square and a cube are read as such and any other power
as "to the power". A simple fraction is "a over b"; one with more inside is framed as "the fraction
... end fraction" so the listener knows where it ends. `speak_unit` expands prefixes and units
(`mA`, `µF`, `kHz`) and `speak_quantity` joins a value to its unit.

## Keyboard navigation

`maths::Navigator` steps through an equation one part at a time, for the interface to bind to the
arrow keys:

| Move | What happens |
| --- | --- |
| Next, previous | The next or previous part at the same level, staying put at either end |
| Enter | Into a part with inner structure, such as a fraction's numerator |
| Leave | Back out to the part that holds the current one |

Every move returns the part's role (number, identifier, fraction, subscript and so on), its speech,
its MathML and its place among its siblings, so the interface can both announce and highlight it.
