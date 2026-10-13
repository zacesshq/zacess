# zacess-quality

The scorer for the Zacess document quality test suite. It imports every fixture in
`tests/fixtures` with the core and reports character accuracy, number and symbol accuracy, reading
order and heading detection for every fixture, every category and overall. CI fails a pull request
when any score goes down.

> [!NOTE]
> This is a read-only copy published from [zacesshq/zacess](https://github.com/zacesshq/zacess).
> Issues, discussions and pull requests go there. Changes made here would be overwritten by the
> next publish.

## Building and testing

The crate builds as part of the Cargo workspace in the monorepo and reads the fixtures from it.
From a clone of [zacesshq/zacess](https://github.com/zacesshq/zacess):

```sh
cargo test -p zacess-quality
cargo run -p zacess-quality -- score --fixtures tests/fixtures
cargo run -p zacess-quality -- check --fixtures tests/fixtures --baseline tests/fixtures/baseline.json
```

`score` prints the scores and can write them to a file with `--out`, which is how the baseline is
updated after an improvement. `check` exits with an error when any score is lower than in any
baseline. The scanned fixtures need Tesseract and Poppler installed.

## Licence

[GNU Affero General Public License v3.0 only](LICENSE).
