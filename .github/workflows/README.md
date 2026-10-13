# Workflows

| Workflow | Runs on | What it does |
| --- | --- | --- |
| [`ci.yml`](ci.yml) | Pull requests | Checks formatting, runs Clippy with warnings as errors and runs the tests. Scores the document quality suite and fails if any score is lower than the pull request's baseline or the base branch's. Exports the fixtures and checks the HTML with axe-core against WCAG 2.2 AA and the EPUBs with EPUBCheck |
