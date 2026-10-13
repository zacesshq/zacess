# Export

How a document plus a reading profile becomes accessible output. The code lives in
`crates/zacess-core/src/export`.

```sh
cargo run -p zacess-core --example export -- tests/fixtures/slides-signals.pdf out/ low-vision
```

## HTML

One standalone page: the profile's style sheet inline, the document inside `<main>`, the language
on `<html>` and real structure throughout.

| Block | Markup |
| --- | --- |
| Heading | `<h1>` to `<h6>`. Levels never skip, so a document whose first heading is level 2 starts at `<h1>` and nothing jumps from `<h2>` to `<h4>` |
| Paragraph | `<p>`. A quotation is a `<p>` inside `<blockquote>` and code is `<pre><code>` with its spacing kept |
| List | `<ul>` or `<ol>` with nested lists inside their `<li>` |
| Table | `<table>` with `<caption>`, `<thead>` for leading header rows, `<th scope="col">` and `<th scope="row">`, `colspan` and `rowspan` |
| Image | `<figure><img alt="…"/></figure>`. An image with no description yet says so in its alt text rather than being silent. One whose file cannot be found is described in a `<figcaption>` instead |
| Equation | `<math>` with presentation MathML built from the recognised equation and the original kept in an `<annotation>`; `display="block"` when set on its own line |
| Page break | `<hr role="doc-pagebreak" aria-label="Page N"/>`, invisible but present, so a screen reader can announce and navigate pages |

Every block carries `id="b<id>"` from the document model, so a link can point at any block. A run
with a link becomes `<a href>` when the address is a web, mail or in-page one. Any other address is
left out, so a captured `javascript:` link can never run from an export.

### Low-confidence text

A run that [needs checking](ocr.md#confidence-thresholds) is wrapped as

```html
<span class="zc-uncertain" data-confidence="0.86"><span class="zc-sr">unclear: </span>kQ</span>
```

The outer span is highlighted by the profile's style sheet with a background that keeps AAA
contrast and a dotted underline, so colour is never the only signal. The inner span is visually
hidden text, so a screen reader reads "unclear: kQ" and the person knows to check it. A run that has
been confirmed or corrected is plain.

### Checked against WCAG 2.2 AA

`tools/accessibility/check.mjs` loads exported pages in headless Chromium and runs axe-core with the
WCAG 2.0, 2.1 and 2.2 A and AA rule sets, failing on any violation. CI runs it on the exported
fixtures and the example document, which covers every block type.

```sh
cd tools/accessibility && npm ci && npx playwright install chromium
node check.mjs ../../out/*.html
```

## EPUB 3

A stored ZIP container with `mimetype` first, holding:

| File | Contents |
| --- | --- |
| `META-INF/container.xml` | Points at the package file |
| `EPUB/package.opf` | Title, language, a stable identifier derived from the document, `dcterms:modified` and schema.org accessibility metadata: access modes, features (structural navigation, reading order, table of contents, page break markers, MathML and alternative text when every image has one), hazards and a summary |
| `EPUB/nav.xhtml` | A table of contents nested by heading level and a hidden page list linking every page marker |
| `EPUB/content.xhtml` | The same body as the HTML export, serialised as XHTML with `epub:type="pagebreak"` on the page markers |
| `EPUB/style.css` | The profile's style sheet |
| Images | Bundled when their files are found next to the source, with the manifest entry each needs |

The content document declares the `mathml` property when it holds an equation, as EPUBCheck requires.

### Checked with EPUBCheck

A unit test runs EPUBCheck on the example document whenever `EPUBCHECK_JAR` points at
`epubcheck.jar`. CI runs it on every exported fixture with warnings treated as failures.

```sh
EPUBCHECK_JAR=/path/to/epubcheck.jar cargo test -p zacess-core export
```

## Braille

Braille-ready files are described in [braille.md](braille.md).
