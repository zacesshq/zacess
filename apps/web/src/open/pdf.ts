// PDFs are read with pdf.js: its text layer gives positioned text, which the core lays out the same
// way the desktop build lays out its own PDF reader's output. A page with no text is drawn to a
// canvas and recognised on the device.
import type { LinkInput, PageInput, TextItem } from "@/core";
import { recognise } from "./ocr";
import { ownAsset, type ProgressListener } from "./progress";

/** Pages are drawn at twice their size for OCR, about 144 dots per inch. */
const OCR_SCALE = 2;
const FONT_WAIT_MS = 3000;

const wait = (ms: number) => new Promise<void>((done) => setTimeout(done, ms));

async function pdfjs() {
  const lib = await import("pdfjs-dist");
  const worker = await import("pdfjs-dist/build/pdf.worker.min.mjs?url");
  lib.GlobalWorkerOptions.workerSrc = worker.default;
  return lib;
}

export interface ReadPdf {
  title: string;
  pages: PageInput[];
}

const BOLD = /bold|black|heavy|semibold|demi/i;

interface Annotation {
  subtype?: string;
  url?: unknown;
  rect?: number[];
}

/**
 * The page's web and mail link areas. Only the annotation data is read: nothing a link points to
 * is ever fetched. Links to places inside the PDF have no address and are left as plain text.
 */
async function linksOf(
  page: { getAnnotations: (options?: { intent?: string }) => Promise<unknown[]> },
  viewport: { convertToViewportPoint: (x: number, y: number) => number[] },
): Promise<LinkInput[]> {
  const annotations = (await page.getAnnotations({ intent: "display" }).catch(() => [])) as Annotation[];
  const links: LinkInput[] = [];
  for (const a of annotations) {
    if (a.subtype !== "Link" || typeof a.url !== "string" || !a.rect || a.rect.length !== 4) continue;
    const [x1, y1] = viewport.convertToViewportPoint(a.rect[0], a.rect[1]);
    const [x2, y2] = viewport.convertToViewportPoint(a.rect[2], a.rect[3]);
    links.push({ url: a.url, x: Math.min(x1, x2), y: Math.min(y1, y2), width: Math.abs(x2 - x1), height: Math.abs(y2 - y1) });
  }
  return links;
}

export async function readPdf(data: ArrayBuffer, onProgress: ProgressListener): Promise<ReadPdf> {
  const lib = await pdfjs();
  const task = lib.getDocument({
    data: new Uint8Array(data),
    cMapUrl: ownAsset("pdfjs/cmaps/"),
    cMapPacked: true,
    standardFontDataUrl: ownAsset("pdfjs/standard_fonts/"),
    iccUrl: ownAsset("pdfjs/iccs/"),
    wasmUrl: ownAsset("pdfjs/wasm/"),
    enableXfa: false,
  });
  const pdf = await task.promise;
  try {
    const metadata = await pdf.getMetadata().catch(() => null);
    const info = (metadata?.info ?? {}) as { Title?: unknown };
    const title = typeof info.Title === "string" ? info.Title.trim() : "";
    const pages: PageInput[] = [];
    for (let number = 1; number <= pdf.numPages; number++) {
      onProgress({ message: `Reading page ${number} of ${pdf.numPages}`, fraction: (number - 1) / pdf.numPages });
      const page = await pdf.getPage(number);
      const viewport = page.getViewport({ scale: 1 });
      const content = await page.getTextContent();
      // the operator list loads each font, which is where a bold face shows in its name. Bold only
      // helps find headings, so a page whose fonts are slow to load is read without waiting
      await Promise.race([page.getOperatorList().catch(() => undefined), wait(FONT_WAIT_MS)]);
      const items: TextItem[] = [];
      for (const raw of content.items) {
        if (!("str" in raw) || !raw.str.trim()) continue;
        const t = lib.Util.transform(viewport.transform, raw.transform);
        let bold = false;
        try {
          if (page.commonObjs.has(raw.fontName)) {
            const font = page.commonObjs.get(raw.fontName) as { name?: string; bold?: boolean };
            bold = Boolean(font.bold) || BOLD.test(font.name ?? "");
          }
        } catch {
          bold = false;
        }
        items.push({
          x: t[4],
          baseline: t[5],
          width: raw.width * viewport.scale,
          size: Math.hypot(t[2], t[3]),
          bold,
          text: raw.str,
        });
      }
      const entry: PageInput = { number, width: viewport.width, height: viewport.height, items, links: await linksOf(page, viewport) };
      if (items.length === 0) {
        const canvas = document.createElement("canvas");
        const scaled = page.getViewport({ scale: OCR_SCALE });
        canvas.width = Math.ceil(scaled.width);
        canvas.height = Math.ceil(scaled.height);
        await page.render({ canvas, viewport: scaled }).promise;
        entry.ocr = await recognise(canvas, `Page ${number} of ${pdf.numPages}`, onProgress);
        canvas.width = 0;
        canvas.height = 0;
      }
      pages.push(entry);
      page.cleanup();
    }
    return { title, pages };
  } finally {
    await task.destroy();
  }
}
