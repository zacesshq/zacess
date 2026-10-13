// Copies the files the app loads at run time from its own origin into public/, so nothing is
// fetched from a third-party CDN: the tesseract.js worker, its WebAssembly cores, the English OCR
// data and the pdf.js character maps, standard fonts, colour profiles and image decoders. The copies are generated and never
// committed.
import { cpSync, existsSync, mkdirSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const pub = resolve(here, "../public");
const require = createRequire(import.meta.url);

function packageDir(name, from) {
  const paths = from ? [from] : undefined;
  return dirname(require.resolve(`${name}/package.json`, { paths }));
}

const tesseract = packageDir("tesseract.js");
// tesseract.js loads the core version it was built against, which npm may nest under it
const core = packageDir("tesseract.js-core", tesseract);
const english = packageDir("@tesseract.js-data/eng");
const pdfjs = packageDir("pdfjs-dist");

const ocr = join(pub, "ocr");
rmSync(ocr, { recursive: true, force: true });
mkdirSync(join(ocr, "core"), { recursive: true });
mkdirSync(join(ocr, "lang"), { recursive: true });
cpSync(join(tesseract, "dist", "worker.min.js"), join(ocr, "worker.min.js"));
// only the LSTM builds, since the app always runs the LSTM engine; each build is about 4 MB
for (const file of [
  "tesseract-core-lstm.wasm.js",
  "tesseract-core-simd-lstm.wasm.js",
  "tesseract-core-relaxedsimd-lstm.wasm.js",
]) {
  const from = join(core, file);
  if (existsSync(from)) cpSync(from, join(ocr, "core", file));
}
cpSync(join(english, "4.0.0_best_int", "eng.traineddata.gz"), join(ocr, "lang", "eng.traineddata.gz"));

const pdf = join(pub, "pdfjs");
rmSync(pdf, { recursive: true, force: true });
mkdirSync(pdf, { recursive: true });
cpSync(join(pdfjs, "cmaps"), join(pdf, "cmaps"), { recursive: true });
cpSync(join(pdfjs, "standard_fonts"), join(pdf, "standard_fonts"), { recursive: true });
cpSync(join(pdfjs, "iccs"), join(pdf, "iccs"), { recursive: true });
// image decoders for scanned pages; the script engine for PDF forms is left out, as the app never
// runs a document's own scripts
cpSync(join(pdfjs, "wasm"), join(pdf, "wasm"), {
  recursive: true,
  filter: (source) => !/quickjs/.test(source),
});
console.log("copied the OCR and PDF assets into public/");
