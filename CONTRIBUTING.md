# Contributing to Zacess

Thank you for helping make reading accessible. This guide covers how work moves through the project.

## How a change happens

1. Every change starts from an issue that says what "done" means.
2. Work on a branch named after its type and topic, such as `feat/pdf-import` or `fix/ocr-confidence`.
3. Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`,
   `docs:`, `test:`, `refactor:` and `chore:`.
4. Open a pull request that closes its issue. It is squash-merged once checks pass and it is reviewed.

## Project conventions

- **Accessibility is a requirement, not a feature.** Every interface change meets WCAG 2.2 AA, works
  with a keyboard alone and is checked with a screen reader. Colour is never the only signal and
  motion respects reduced-motion settings.
- **Accuracy over appearance.** Any change to import or OCR runs the document quality suite in
  `tests/fixtures`. A drop in accuracy blocks the merge.
- **Local first.** A feature works on the device unless it genuinely cannot. Anything that sends data
  to a server says so before it runs.
- **The original is the source of truth.** Generated or converted content never replaces the original
  without it staying one step away.
- **Rust** code passes `cargo fmt`, `cargo clippy -- -D warnings` and `cargo test`.
- **TypeScript** code passes the type check, lint and tests.
- **Python** workers pass `ruff` and `pytest`.
- Documentation uses UK English.

## Reporting a problem

Open an issue with the document type, what you expected and what happened. For an OCR or reading-order
problem, attach the page if you are allowed to share it.
