// Opens the built app in headless Chromium and uses it as a person would: opens a text file, a PDF
// with a text layer and a scanned page through OCR, changes the reading profile and exports HTML.
// Every screen is checked with axe-core (WCAG 2.2 A and AA, plus AAA contrast in the reading
// view), the page must never scroll sideways and every request must stay on the app's own origin.
// Screenshots at 1280 and 375 pixels wide, light and dark, are written to test-results/.
//
//   npm run build && node scripts/check-browser.mjs
//
// Set CHROMIUM_PATH to use an installed browser instead of Playwright's own.
import { mkdirSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { preview } from "vite";

const here = dirname(fileURLToPath(import.meta.url));
const web = resolve(here, "..");
const repo = resolve(web, "../..");
const out = join(web, "test-results");
mkdirSync(out, { recursive: true });

const require = createRequire(import.meta.url);
const axeSource = readFileSync(require.resolve("axe-core/axe.min.js"), "utf8");
const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

const server = await preview({ root: web, logLevel: "warn", preview: { port: 4174, strictPort: false, open: false } });
const base = server.resolvedUrls.local[0];
const origin = new URL(base).origin;
const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {});

const problems = [];
const fail = (message) => {
  problems.push(message);
  console.log(`  FAIL ${message}`);
};

async function axe(page, label) {
  // evaluated through the debugging protocol, because the page's own policy rightly refuses an
  // inline script
  await page.evaluate(axeSource);
  const results = await page.evaluate(async (tags) => {
    const main = await window.axe.run(document, { runOnly: { type: "tag", values: tags } });
    const view = document.querySelector(".zc-view .zc-doc");
    const aaa = view
      ? await window.axe.run(view, { runOnly: { type: "rule", values: ["color-contrast-enhanced"] } })
      : { violations: [] };
    const describe = (v) => `${v.id}: ${v.help} (${v.nodes.slice(0, 3).map((n) => n.target.join(" ")).join(", ")})`;
    return { aa: main.violations.map(describe), aaa: aaa.violations.map(describe), passes: main.passes.length };
  }, TAGS);
  for (const v of results.aa) fail(`${label}: ${v}`);
  for (const v of results.aaa) fail(`${label}, reading view AAA: ${v}`);
  if (!results.aa.length && !results.aaa.length) console.log(`  ${label}: axe found no violations (${results.passes} rules passed)`);
}

async function noSideways(page, label) {
  const { scroll, width } = await page.evaluate(() => ({ scroll: document.documentElement.scrollWidth, width: window.innerWidth }));
  if (scroll > width) fail(`${label}: the page scrolls sideways (${scroll} > ${width})`);
}

async function openFile(page, path, expectText, timeout = 30_000) {
  await page.locator('input[type="file"]').setInputFiles(path);
  try {
    // the status names the file, so text left from the previous document cannot satisfy the wait
    await page.getByText(`Opened ${basename(path)}.`, { exact: true }).waitFor({ timeout });
    await page.locator(".zc-view").getByText(expectText, { exact: false }).first().waitFor({ timeout: 5_000 });
  } catch (error) {
    // say what the page showed instead, so a failure on another machine can be read from the log
    const shown = await page.evaluate(() => ({
      status: document.querySelector("[aria-live='polite']")?.textContent ?? "",
      alert: document.querySelector("[role='alert']")?.textContent ?? "",
      view: document.querySelector(".zc-view")?.textContent?.slice(0, 400) ?? "",
    }));
    console.log(`  opening ${path} showed: ${JSON.stringify(shown)}`);
    throw error;
  }
}

async function session(scheme, width) {
  const label = `${scheme} ${width}`;
  console.log(`${label}:`);
  const context = await browser.newContext({
    viewport: { width, height: width > 600 ? 900 : 800 },
    colorScheme: scheme,
    acceptDownloads: true,
    reducedMotion: "reduce",
  });
  const page = await context.newPage();
  const foreign = [];
  page.on("request", (r) => {
    const url = new URL(r.url());
    if (!["blob:", "data:"].includes(url.protocol) && url.origin !== origin) foreign.push(r.url());
  });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
    if (m.type() === "warning") console.log(`  console warning: ${m.text()}`);
  });

  await page.goto(base);
  await page.getByRole("button", { name: "Choose a file" }).waitFor();
  await axe(page, `${label} first screen`);
  await noSideways(page, `${label} first screen`);

  await openFile(page, join(web, "src/test/fixtures/notes.md"), "Any periodic signal");
  await page.getByRole("radio", { name: "Low Vision" }).click();
  const css = await page.locator(".zc-view style").textContent();
  if (!css.includes("font-size: 24pt")) fail(`${label}: the Low Vision preset did not apply`);
  await page.locator(".zc-view").getByText("Harmonics are whole multiples").hover();

  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page.getByRole("button", { name: "Download HTML" }).click(),
  ]);
  const saved = join(out, `export-${scheme}-${width}.html`);
  await download.saveAs(saved);
  const html = readFileSync(saved, "utf8");
  if (!html.startsWith("<!DOCTYPE html>") || !html.includes("Fourier series") || !html.includes("#ffeb3b")) {
    fail(`${label}: the HTML export is not the document in the Low Vision profile`);
  } else {
    console.log(`  exported ${download.suggestedFilename()} (${html.length} bytes)`);
  }

  await axe(page, `${label} document open`);
  await noSideways(page, `${label} document open`);
  await page.screenshot({ path: join(out, `app-${scheme}-${width}.png`), fullPage: true });
  await page.locator("#document").screenshot({ path: join(out, `document-${scheme}-${width}.png`) });

  // the largest text size must still reflow into the column
  const size = page.getByRole("slider", { name: "Text size" });
  await size.focus();
  await page.keyboard.press("End");
  await noSideways(page, `${label} at 96 pt`);
  if (width < 600) await page.locator("#document").screenshot({ path: join(out, `document-96pt-${scheme}-${width}.png`) });

  if (scheme === "light" && width > 600) {
    await page.getByRole("radio", { name: "Standard" }).click();
    await openFile(page, join(repo, "tests/fixtures/slides-signals.pdf"), "Fourier");
    console.log("  opened a PDF through its text layer");
    await page.locator("#document").screenshot({ path: join(out, "document-pdf.png") });
    await openFile(page, join(web, "src/test/fixtures/links.pdf"), "course page");
    const link = page.locator(".zc-view a.zc-link");
    const hrefs = await link.evaluateAll((all) => all.map((a) => [a.textContent, a.getAttribute("href"), a.target, a.rel]));
    if (hrefs.length !== 1 || hrefs[0][1] !== "https://example.org/course" || !hrefs[0][0].startsWith("course page")) {
      fail(`${label}: the PDF link annotation did not become one link on "course page": ${JSON.stringify(hrefs)}`);
    } else {
      console.log("  a PDF link annotation became a link on its words");
    }
    if ((await page.locator(".zc-view").innerHTML()).includes("javascript:")) fail(`${label}: a javascript: address reached the page`);
    await page.getByRole("button", { name: /Links in this document/ }).click();
    await page.getByRole("list", { name: "Links in this document" }).getByRole("button").first().click();
    if (!(await link.first().evaluate((a) => a === document.activeElement))) fail(`${label}: the links list did not move focus to the link`);
    await page.locator(".zc-link-tip").waitFor();
    await axe(page, `${label} links`);
    await page.locator("#document").screenshot({ path: join(out, "document-links.png") });
    await openFile(page, join(repo, "tests/fixtures/handout-scan.png"), "Laboratory safety", 120_000);
    console.log("  recognised a scanned image on the device");
    await page.locator("#document").screenshot({ path: join(out, "document-ocr.png") });
    await axe(page, `${label} OCR result`);
  }

  if (foreign.length) fail(`${label}: requests left the app's origin: ${[...new Set(foreign)].join(", ")}`);
  else console.log("  every request stayed on the app's own origin");
  for (const e of errors) fail(`${label}: console error: ${e}`);
  await context.close();
}

try {
  for (const scheme of ["light", "dark"]) {
    for (const width of [1280, 375]) await session(scheme, width);
  }
} finally {
  await browser.close();
  await new Promise((done) => server.httpServer.close(done));
}

if (problems.length) {
  console.log(`\n${problems.length} problem(s)`);
  process.exit(1);
}
console.log(`\nall checks passed; screenshots in ${out}`);
