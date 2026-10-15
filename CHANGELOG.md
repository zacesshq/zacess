# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- The README says the browser version is live at app.zacess.com and links to it.
- A PDF set at 1.5 times or double line spacing now keeps its paragraphs together instead of giving every printed line its own paragraph, so read aloud no longer speaks fragments.
- The read aloud controls in the browser version now sit in a bar above the document that stays in view. The page now only scrolls when the sentence being read leaves the viewport instead of jumping back to the document.
- A PDF link area no longer pulls in neighbouring words it only touches. Bare web and email addresses in PDF text are linked like pasted text.
- A braille translator that rejects its tables and exits before reading the text now reports its own error. Before this the write failed with a broken pipe and the run could fail at random.
- The repository link in `Cargo.toml` now points at this repository.

### Added

- A hide and show control for the settings panel, a colour mode menu with System as the default and links to the website in the browser version.
- Phone and tablet support in the browser version: touch sized controls, a Take a photo button, safe area spacing, an installable manifest and checks on WebKit and Chromium.
- Read-only copies of `crates/zacess-core`, `crates/zacess-quality` and `apps/app` in the `core`, `quality` and `app` repositories, each with its own README and licence.
- Issue forms for bugs, feature requests and accessibility barriers, a pull request template with an accessibility checklist, CODEOWNERS and Dependabot.
- The changelog, code of conduct, security policy, support guide, licensing notice, EditorConfig, Git attributes and markdown linting.
- A Cloudflare Workers config for serving the browser version at `app.zacess.com`, with deploy steps in [docs/browser-version.md](docs/browser-version.md).
- The first browser version in `apps/web`: open a PDF, an image, a Word file, a web page or pasted text on the device, read it in a reading profile, listen to it and download accessible HTML or EPUB. The core reaches the browser as WebAssembly through `crates/zacess-wasm`.
- Universal capture: web clips, shared text, images and files go straight into the library's inbox.
- Bilingual reading that keeps the reading place across languages, with translation behind the labelled helper rules.
- Study decks, revision paths and plans with FSRS scheduling, stored in the library, plus the complete set of exports.
- Lecture capture: crash-safe audio recording, on-device transcription with live captions, slide and build step detection from projector frames, whiteboard and notes capture with versions and search, all joined on one timeline.
- Answers to questions from the student's own material with sources.
- Labelled helpers for diagram descriptions and simpler explanations.
- Local-first sync built to the agreed conflict rules.
- Braille-ready export through liblouis with safe pages and checked cells.
- A maths and engineering mode.
- The local library with autosave and version history.
- Accessible HTML and EPUB 3 export.
- Import from PDFs with a text layer and from images with OCR and a confidence for every word.
- Reading profiles with presets.
- The document model.
- The document quality test suite and its scorer, run on every pull request.
- The Cargo and TypeScript workspace with CI.

### Changed

- Documentation links point to [zacess.com](https://zacess.com) for the roadmap, privacy commitments and how Zacess works.
