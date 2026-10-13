import "@testing-library/jest-dom/vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";
import { loadCoreSync } from "@/core";

// the tests run the real core, built by `npm run build:wasm`
loadCoreSync(readFileSync(resolve(import.meta.dirname, "../core/pkg/zacess_wasm_bg.wasm")));

// jsdom has no layout, so the parts of the DOM that Radix and the reading view measure are stubbed
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver ??= ResizeObserverStub as unknown as typeof ResizeObserver;
Element.prototype.scrollIntoView ??= function scrollIntoView() {};
Element.prototype.hasPointerCapture ??= () => false;
Element.prototype.releasePointerCapture ??= () => {};
Element.prototype.setPointerCapture ??= () => {};
globalThis.requestAnimationFrame ??= (callback: FrameRequestCallback) => setTimeout(() => callback(0), 0) as unknown as number;

afterEach(() => {
  cleanup();
  try {
    window.localStorage.clear();
  } catch {
    // storage may be unavailable, which the app handles
  }
  document.documentElement.removeAttribute("data-theme");
  vi.restoreAllMocks();
});
