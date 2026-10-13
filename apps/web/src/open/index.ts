import {
  documentFromHtml,
  documentFromPages,
  documentFromText,
  type Document,
  type PageInput,
} from "@/core";
import { docxToHtml } from "./docx";
import type { ProgressListener } from "./progress";

export type { Progress, ProgressListener } from "./progress";

export interface Opened {
  document: Document;
  /** Things the person should know about the result, such as pages that could not be read. */
  notes: string[];
}

export type Kind = "pdf" | "image" | "docx" | "html" | "text";

const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "gif", "bmp"];

/** The file types the file button offers. */
export const ACCEPT = [
  ".pdf",
  ".docx",
  ".html",
  ".htm",
  ".txt",
  ".md",
  ".markdown",
  ...IMAGE_EXTENSIONS.map((e) => `.${e}`),
].join(",");

/** The largest file the app opens. Larger ones would hold the page for a long time. */
export const MAX_BYTES = 200 * 1024 * 1024;

export function kindOf(file: { name: string; type: string }): Kind | null {
  const extension = file.name.toLowerCase().split(".").pop() ?? "";
  if (extension === "pdf" || file.type === "application/pdf") return "pdf";
  if (extension === "docx") return "docx";
  if (extension === "html" || extension === "htm" || file.type === "text/html") return "html";
  if (["txt", "md", "markdown"].includes(extension) || file.type === "text/plain" || file.type === "text/markdown") return "text";
  if (IMAGE_EXTENSIONS.includes(extension) || file.type.startsWith("image/")) return "image";
  return null;
}

function readAsText(file: Blob): Promise<string> {
  return file.text();
}

function readAsBuffer(file: Blob): Promise<ArrayBuffer> {
  return file.arrayBuffer();
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** Opens a file into a document, entirely on this device. */
export async function openFile(file: File, onProgress: ProgressListener): Promise<Opened> {
  const kind = kindOf(file);
  if (!kind) {
    throw new Error(
      `${file.name} is not a file Zacess can open yet. Choose a PDF, a Word file (.docx), an image, a web page (.html) or a text file.`,
    );
  }
  if (file.size > MAX_BYTES) {
    throw new Error(`${file.name} is larger than 200 MB, which is more than the browser version opens.`);
  }
  onProgress({ message: `Opening ${file.name}` });
  switch (kind) {
    case "text":
      return { document: documentFromText(await readAsText(file), file.name), notes: [] };
    case "html":
      return { document: documentFromHtml(await readAsText(file), file.name, "html", false), notes: [] };
    case "docx": {
      onProgress({ message: `Reading the Word file ${file.name}` });
      const { html, warnings } = await docxToHtml(await readAsBuffer(file));
      const notes = warnings.length ? [`${plural(warnings.length, "part", "parts")} of the Word file could not be kept exactly.`] : [];
      return { document: documentFromHtml(html, file.name, "docx", true), notes };
    }
    case "image": {
      const { readImage } = await import("./image");
      const page = await readImage(file, onProgress);
      return fromPages(file.name, "", "image", [page]);
    }
    case "pdf": {
      const { readPdf } = await import("./pdf");
      const { title, pages } = await readPdf(await readAsBuffer(file), onProgress);
      return fromPages(file.name, title, "pdf", pages);
    }
  }
}

function fromPages(fileName: string, title: string, format: "pdf" | "image", pages: PageInput[]): Opened {
  const result = documentFromPages({ file_name: fileName, title, format, pages });
  const notes: string[] = [];
  if (result.pages_recognised.length > 0 && format === "pdf") {
    notes.push(`Text on ${plural(result.pages_recognised.length, "page", "pages")} was recognised from the page image.`);
  }
  if (result.pages_needing_ocr.length > 0) {
    notes.push(`No text was found on ${plural(result.pages_needing_ocr.length, "page", "pages")}: ${result.pages_needing_ocr.join(", ")}.`);
  }
  return { document: result.document, notes };
}

/** Opens pasted text, using the HTML of a copied web page when the clipboard held it. */
export function openPasted(text: string, html: string | null): Opened {
  if (html && html.trim()) {
    try {
      return { document: documentFromHtml(html, "Pasted page", "html", true), notes: [] };
    } catch {
      // the copied formatting held nothing readable, so the plain text is used instead
    }
  }
  if (!text.trim()) throw new Error("Paste some text or a copied web page first.");
  return { document: documentFromText(text, "Pasted text"), notes: [] };
}
