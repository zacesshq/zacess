/**
 * The largest canvas a page or photo is drawn to for text recognition. Safari on iPhone and iPad
 * refuses a canvas over about 16.7 million pixels and draws nothing. A recent phone photo is
 * larger than that; 12 million still keeps print well above the 30 pixels a line OCR reads best at.
 */
export const MAX_CANVAS_PIXELS = 12_000_000;

/** The scale to draw at: the one wanted, lowered when the result would pass the canvas limit. */
export function limitScale(width: number, height: number, wanted: number): number {
  const area = width * height * wanted * wanted;
  return area > MAX_CANVAS_PIXELS ? Math.sqrt(MAX_CANVAS_PIXELS / (width * height)) : wanted;
}
