# zacess-core

The Rust engine behind Zacess. It holds the document model, reading profiles, import from PDF,
images, Word files, web pages and text with OCR where a page has no text, maths, the accessible
HTML, EPUB and braille exports, read aloud and speech, bilingual reading, recordings and timelines, the local library,
study decks and plans, universal capture and the local-first sync engine. The apps and the browser
version are thin interfaces over it.

> [!NOTE]
> This is a read-only copy published from [zacesshq/zacess](https://github.com/zacesshq/zacess).
> Issues, discussions and pull requests go there. Changes made here would be overwritten by the
> next publish.

## Building and testing

The crate builds as part of the Cargo workspace in the monorepo. From a clone of
[zacesshq/zacess](https://github.com/zacesshq/zacess):

```sh
cargo build -p zacess-core
cargo test -p zacess-core
cargo run -p zacess-core --example export -- tests/fixtures/slides-signals.pdf out/
```

Some tests call Tesseract, Poppler and liblouis when they are installed. The default `library`
feature brings in the SQLite library store; the browser build turns it off. Each part is described
in the [docs](https://github.com/zacesshq/zacess/tree/main/docs).

## Licence

[GNU Affero General Public License v3.0 only](LICENSE).
