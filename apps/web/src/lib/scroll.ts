/** The edges of a box on screen, in the same units as `getBoundingClientRect`. */
export interface Span {
  top: number;
  bottom: number;
}

/**
 * How far the page has to scroll, down when positive, for a box to sit inside the visible area:
 * zero when it is already inside, otherwise the least that brings it in (the nearest edge, as
 * `block: "nearest"` does). A box taller than the area is aligned by its top edge.
 */
export function nearestScroll(box: Span, visible: Span): number {
  if (box.top >= visible.top && box.bottom <= visible.bottom) return 0;
  if (box.top < visible.top) return box.top - visible.top;
  const toBottom = box.bottom - visible.bottom;
  return Math.min(toBottom, box.top - visible.top);
}

/**
 * Scrolls the page just enough to show the sentence being read, leaving room for a sticky bar at
 * the top. The page is left alone while the sentence is on screen, so reading never moves it.
 */
export function revealSentence(box: Span, bar: Span | null, viewportHeight: number, scrollBy: (dy: number) => void) {
  const top = bar ? Math.max(0, bar.bottom) : 0;
  const delta = nearestScroll(box, { top, bottom: viewportHeight });
  if (delta !== 0) scrollBy(delta);
}
