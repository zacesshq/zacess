# Workflows

| Workflow | Runs on | What it does |
| --- | --- | --- |
| [`ci.yml`](ci.yml) | Pull requests | Checks formatting, runs Clippy with warnings as errors and runs the tests. Scores the document quality suite and fails if any score is lower than the pull request's baseline or the base branch's. Exports the fixtures and checks the HTML with axe-core against WCAG 2.2 AA and the EPUBs with EPUBCheck. Builds the browser version and checks it in jsdom and Chromium |
| [`split.yml`](split.yml) | Push to `main` in `zacesshq/zacess`, also run by hand | Publishes `crates/zacess-core`, `crates/zacess-quality` and `apps/app` to the read-only repositories `core`, `quality` and `app`. Does nothing in any other repository |
| [`markdownlint.yml`](markdownlint.yml) | Push to `main` and pull requests, also run by hand | Lints every markdown file with the rules in `.markdownlint.json` |
