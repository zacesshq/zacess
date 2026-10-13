// The core, compiled from crates/zacess-wasm. Every document and profile crosses the boundary in
// its documented JSON form (docs/document-model.md and docs/reading-profiles.md).
import init, * as wasm from "./pkg/zacess_wasm.js";
import type { Document, PagesInput, Preset, ReadingProfile, Summary } from "./types";

export type * from "./types";

let ready: Promise<void> | null = null;

/** Loads the WebAssembly core once. Later calls wait for the same load. */
export function loadCore(source?: Parameters<typeof init>[0]): Promise<void> {
  if (!ready) {
    ready = init(source ? { module_or_path: source } : undefined).then(() => undefined);
    // a failed load can be retried, for example after the connection comes back
    ready.catch(() => {
      ready = null;
    });
  }
  return ready;
}

/** Loads the core from bytes already in memory, which is how the tests run it. */
export function loadCoreSync(bytes: BufferSource): void {
  wasm.initSync({ module: bytes });
  ready = Promise.resolve();
}

/** The message a core error carries, so the app can show it as written. */
export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

export function defaultProfile(): ReadingProfile {
  return JSON.parse(wasm.defaultProfile()) as ReadingProfile;
}

export function presets(): Preset[] {
  return JSON.parse(wasm.presets()) as Preset[];
}

/** Checks a profile with the core and returns it in its current form. Throws with the reasons. */
export function checkProfile(profile: unknown): ReadingProfile {
  return JSON.parse(wasm.checkProfile(JSON.stringify(profile))) as ReadingProfile;
}

export function readingCss(profile: ReadingProfile, scope: string): string {
  return wasm.readingCss(JSON.stringify(profile), scope);
}

export function documentFromText(text: string, fileName: string): Document {
  return JSON.parse(wasm.documentFromText(text, fileName)) as Document;
}

export function documentFromHtml(
  html: string,
  fileName: string,
  format: "html" | "docx",
  fragment: boolean,
): Document {
  return JSON.parse(wasm.documentFromHtml(html, fileName, format, fragment)) as Document;
}

export interface PagesResult {
  document: Document;
  pages_needing_ocr: number[];
  pages_recognised: number[];
}

export function documentFromPages(input: PagesInput): PagesResult {
  return JSON.parse(wasm.documentFromPages(JSON.stringify(input))) as PagesResult;
}

export function summary(document: Document): Summary {
  return JSON.parse(wasm.summary(JSON.stringify(document))) as Summary;
}

export function readingHtml(document: Document, profile: ReadingProfile): string {
  return wasm.readingHtml(JSON.stringify(document), JSON.stringify(profile));
}

export function exportHtml(document: Document, profile: ReadingProfile, language: string): string {
  return wasm.exportHtml(JSON.stringify(document), JSON.stringify(profile), language);
}

/** An EPUB 3 book. The time comes from the page, since WebAssembly in a browser has no clock. */
export function exportEpub(
  document: Document,
  profile: ReadingProfile,
  language: string,
  modified = new Date(),
): Uint8Array {
  return wasm.exportEpub(JSON.stringify(document), JSON.stringify(profile), language, Math.floor(modified.getTime() / 1000));
}
