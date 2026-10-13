// Text recognition with tesseract.js, run in a worker on the device. The worker, the engine and
// the English language data are all served from the app's own origin (see scripts/copy-assets.mjs),
// so recognising a page never contacts anyone else.
import type { OcrInput } from "@/core";
import { ownAsset, type ProgressListener } from "./progress";

type Worker = import("tesseract.js").Worker;

let worker: Promise<Worker> | null = null;
let listener: ProgressListener | null = null;
let label = "";

async function startWorker(): Promise<Worker> {
  const { createWorker, OEM } = await import("tesseract.js");
  return createWorker("eng", OEM.LSTM_ONLY, {
    workerPath: ownAsset("ocr/worker.min.js"),
    corePath: ownAsset("ocr/core/"),
    langPath: ownAsset("ocr/lang"),
    gzip: true,
    // a blob worker would need a looser security policy; the file is served from this origin
    workerBlobURL: false,
    logger: (m: { status: string; progress: number }) => {
      if (!listener) return;
      if (m.status === "recognizing text") {
        listener({ message: `${label}: ${Math.round(m.progress * 100)}% recognised`, fraction: m.progress });
      } else if (m.status.startsWith("loading")) {
        listener({ message: "Loading the text recogniser on this device" });
      }
    },
  });
}

function getWorker(): Promise<Worker> {
  if (!worker) {
    worker = startWorker();
    worker.catch(() => {
      worker = null;
    });
  }
  return worker;
}

interface Box {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

interface TesseractBlocks {
  paragraphs: {
    lines: { bbox: Box; words: { text: string; confidence: number; bbox: Box }[] }[];
  }[];
}

/** Turns tesseract.js blocks into the shape the core reads. */
export function toOcrInput(blocks: TesseractBlocks[] | null | undefined, width: number, height: number): OcrInput {
  return {
    width,
    height,
    paragraphs: (blocks ?? []).flatMap((block) =>
      block.paragraphs.map((paragraph) => ({
        lines: paragraph.lines.map((line) => ({
          left: line.bbox.x0,
          top: line.bbox.y0,
          width: line.bbox.x1 - line.bbox.x0,
          height: line.bbox.y1 - line.bbox.y0,
          words: line.words.map((word) => ({
            text: word.text,
            left: word.bbox.x0,
            top: word.bbox.y0,
            width: word.bbox.x1 - word.bbox.x0,
            height: word.bbox.y1 - word.bbox.y0,
            confidence: word.confidence,
          })),
        })),
      })),
    ),
  };
}

/** Recognises the text on a canvas holding one page or image. */
export async function recognise(
  canvas: HTMLCanvasElement,
  what: string,
  onProgress: ProgressListener,
): Promise<OcrInput> {
  const ready = await getWorker();
  listener = onProgress;
  label = what;
  try {
    const { data } = await ready.recognize(canvas, {}, { blocks: true, text: false });
    return toOcrInput(data.blocks as unknown as TesseractBlocks[], canvas.width, canvas.height);
  } finally {
    listener = null;
  }
}
