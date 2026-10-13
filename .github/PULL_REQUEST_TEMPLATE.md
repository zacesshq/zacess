## What changed

<!-- What changed and why. -->

## Closes

<!-- Closes #N -->

## How it was checked

<!-- The commands run and anything checked by hand. -->

- [ ] Rust changes pass `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`
- [ ] TypeScript changes pass the type check, lint and tests
- [ ] A change to import or OCR ran the document quality suite and no score went down

## Accessibility impact

- [ ] Every interface change meets WCAG 2.2 AA (tick this too when there is none)
- [ ] Works with a keyboard alone and focus is always visible
- [ ] Checked with a screen reader
- [ ] Colour is never the only signal and motion follows the reduced motion setting

## Sign-off

- [ ] Every commit is signed off (`git commit -s`) under the [Developer Certificate of Origin](https://developercertificate.org/)
