// Runs axe-core against exported HTML files in headless Chromium and fails on any WCAG 2.2 AA
// violation.
//
//   node check.mjs out/*.html
//
// Set CHROMIUM_PATH to use an already installed browser instead of Playwright's own.

import { chromium } from 'playwright';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error('usage: node check.mjs <file.html> [more files]');
  process.exit(2);
}

const require = createRequire(import.meta.url);
const axeSource = readFileSync(require.resolve('axe-core/axe.min.js'), 'utf8');
const tags = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'];

const launch = {};
if (process.env.CHROMIUM_PATH) {
  launch.executablePath = process.env.CHROMIUM_PATH;
}
const browser = await chromium.launch(launch);
let failed = false;
try {
  for (const file of files) {
    const page = await browser.newPage();
    await page.goto(pathToFileURL(resolve(file)).href);
    await page.addScriptTag({ content: axeSource });
    const results = await page.evaluate(
      async (runTags) => await axe.run(document, { runOnly: { type: 'tag', values: runTags } }),
      tags,
    );
    await page.close();
    const violations = results.violations;
    if (violations.length === 0) {
      console.log(`${file}: no WCAG 2.2 AA violations (${results.passes.length} rules passed)`);
      continue;
    }
    failed = true;
    console.log(`${file}: ${violations.length} violation(s)`);
    for (const v of violations) {
      console.log(`  ${v.id} [${v.impact}] ${v.help}`);
      for (const node of v.nodes.slice(0, 5)) {
        console.log(`    ${node.target.join(' ')}: ${node.failureSummary.split('\n')[1] ?? ''}`);
      }
    }
  }
} finally {
  await browser.close();
}
process.exit(failed ? 1 : 0);
