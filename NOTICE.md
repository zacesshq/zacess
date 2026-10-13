# Licensing notice

Zacess is licensed under the [GNU Affero General Public License v3.0 only](LICENSE) (AGPL-3.0-only). Each published component repository (`core`, `quality` and `app`) carries the same licence file.

Anyone can use, study, modify and self-host Zacess. Anyone who runs a modified version as a network service must make their modified source available to that service's users.

## Third-party components

Zacess depends on open-source packages that are pulled in at build time and are not vendored into this repository, apart from the font listed below. Each is distributed under its own licence by its own authors. The main ones are:

### The core (Rust)

| Package | Used for | Licence |
| --- | --- | --- |
| lopdf | Reading PDFs | MIT |
| image | Decoding JPEG and PNG | MIT or Apache-2.0 |
| hound | Reading and writing WAV audio | Apache-2.0 |
| rusqlite (with bundled SQLite) | The local library store | MIT (SQLite is public domain) |
| serde, serde_json | The JSON forms of documents and profiles | MIT or Apache-2.0 |
| thiserror, tempfile | Errors and temporary files | MIT or Apache-2.0 |
| wasm-bindgen, getrandom | The browser build of the core | MIT or Apache-2.0 |

### The browser version (`apps/web`)

| Package | Used for | Licence |
| --- | --- | --- |
| pdf.js (`pdfjs-dist`) | Reading PDF text layers and links, drawing pages for OCR | Apache-2.0 |
| tesseract.js and tesseract.js-core | OCR in the browser | Apache-2.0 |
| `@tesseract.js-data/eng` | The English OCR model from Tesseract's `tessdata_best` | MIT (package), Apache-2.0 (model data) |
| mammoth | Turning Word files into HTML | BSD-2-Clause |
| React, React DOM | The interface | MIT |
| Radix UI (`radix-ui`), shadcn/ui patterns | Accessible interface parts | MIT |
| Tailwind CSS, tailwind-merge, clsx | Styling | MIT |
| class-variance-authority | Styling variants | Apache-2.0 |
| Lucide (`lucide-react`) | Icons | ISC |

The full lists are in `Cargo.toml`, `Cargo.lock`, `apps/web/package.json` and `package-lock.json`.

### Bundled font

The Atkinson Hyperlegible font in `apps/web/public/fonts` is copyright 2020 Braille Institute of America, Inc. and is licensed under the [SIL Open Font License 1.1](apps/web/public/fonts/OFL-LICENSE).

### Tools called but not bundled

Tesseract, Poppler, liblouis and whisper.cpp are run as separate programs when they are installed. They are not part of Zacess and keep their own licences.

## Test fixtures

Every document, image, page and audio clip in `tests/fixtures` was written or generated for this repository and is covered by its licence. Nothing was photographed or recorded from a person. The sample speech was made with eSpeak NG. The only outside material is the DejaVu font used to draw some fixtures, whose licence allows embedding. See [tests/fixtures/README.md](tests/fixtures/README.md).
