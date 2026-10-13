import type { PageInput } from "@/core";
import { recognise } from "./ocr";
import type { ProgressListener } from "./progress";

/** Small images are enlarged before OCR, which reads text best at about 30 pixels a line. */
const MIN_WIDTH = 1600;

export async function readImage(file: Blob, onProgress: ProgressListener): Promise<PageInput> {
  onProgress({ message: "Preparing the image" });
  const bitmap = await createImageBitmap(file);
  try {
    const scale = bitmap.width < MIN_WIDTH ? Math.min(3, MIN_WIDTH / bitmap.width) : 1;
    const canvas = document.createElement("canvas");
    canvas.width = Math.round(bitmap.width * scale);
    canvas.height = Math.round(bitmap.height * scale);
    const context = canvas.getContext("2d");
    if (!context) throw new Error("This browser cannot draw the image for recognition.");
    context.fillStyle = "#ffffff";
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
    const ocr = await recognise(canvas, "Image", onProgress);
    return { number: 1, width: canvas.width, height: canvas.height, ocr };
  } finally {
    bitmap.close();
  }
}
