// Read aloud with the browser's own speech synthesis. Only voices that run on the device are
// offered unless the person chooses otherwise, because an online voice sends the text it reads to
// whoever provides the voice.

export interface VoiceLike {
  name: string;
  lang: string;
  localService: boolean;
  voiceURI: string;
  default?: boolean;
}

/** The voices to offer: on-device ones only, unless online voices were explicitly allowed. */
export function availableVoices<T extends VoiceLike>(voices: readonly T[], allowOnline: boolean): T[] {
  return voices
    .filter((v) => allowOnline || v.localService)
    .slice()
    .sort((a, b) => Number(b.localService) - Number(a.localService) || a.lang.localeCompare(b.lang) || a.name.localeCompare(b.name));
}

/** The voice to start with: the saved one when it is still offered, else the best English match. */
export function pickVoice<T extends VoiceLike>(voices: readonly T[], saved: string | null, language: string): T | null {
  if (voices.length === 0) return null;
  const kept = saved ? voices.find((v) => v.voiceURI === saved) : undefined;
  if (kept) return kept;
  const lang = language.toLowerCase();
  return (
    voices.find((v) => v.lang.toLowerCase() === lang) ??
    voices.find((v) => v.lang.toLowerCase().startsWith(lang.split("-")[0])) ??
    voices.find((v) => v.default) ??
    voices[0]
  );
}

import { shownAddress } from "./links";

/** One sentence of the reading view, with where it sits so it can be highlighted. */
export interface Sentence {
  /** The sentence as it reads on the page. */
  text: string;
  /** What the voice says: the text with a short cue after each link. */
  spoken: string;
  /** The block element the sentence belongs to. */
  block: Element;
  /** The text range, when the sentence is made of text in the page. */
  range: Range | null;
}

/** Elements whose own text is read as a unit, in document order. */
const READABLE = "h1, h2, h3, h4, h5, h6, p, li, caption, th, td, figcaption, pre, dt, dd, blockquote";

function ownTextNodes(container: Element): Text[] {
  const nodes: Text[] = [];
  const walker = container.ownerDocument.createTreeWalker(container, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const parent = node.parentElement;
      if (!parent) return NodeFilter.FILTER_REJECT;
      // text that belongs to a nested readable element is read with that element instead
      if (parent.closest(READABLE) !== container) return NodeFilter.FILTER_REJECT;
      // the hidden "unclear:" and "opens in a new tab" notes are for screen readers; the page
      // already shows both
      if (parent.closest(".zc-sr")) return NodeFilter.FILTER_REJECT;
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  for (let node = walker.nextNode(); node; node = walker.nextNode()) nodes.push(node as Text);
  return nodes;
}

function segment(text: string, language: string): { start: number; end: number }[] {
  const out: { start: number; end: number }[] = [];
  const Segmenter = (Intl as unknown as { Segmenter?: typeof Intl.Segmenter }).Segmenter;
  if (Segmenter) {
    for (const s of new Segmenter(language, { granularity: "sentence" }).segment(text)) {
      out.push({ start: s.index, end: s.index + s.segment.length });
    }
    return out;
  }
  const pattern = /[^.!?]+[.!?]*\s*/g;
  for (let m = pattern.exec(text); m; m = pattern.exec(text)) out.push({ start: m.index, end: m.index + m[0].length });
  return out;
}

function locate(nodes: Text[], offsets: number[], position: number, atEnd: boolean): [Text, number] {
  for (let i = 0; i < nodes.length; i++) {
    const start = offsets[i];
    const end = start + nodes[i].data.length;
    if (position < end || (atEnd && position === end)) return [nodes[i], position - start];
  }
  const last = nodes[nodes.length - 1];
  return [last, last.data.length];
}

export interface SpeechOptions {
  /** Say each link's address after its text. Off by default: addresses are long and rarely help. */
  readAddresses?: boolean;
}

/** Whether a link's text is its own address, as with a bare web address in pasted text. */
function isAddressText(text: string, href: string): boolean {
  const t = text.trim().replace(/\/$/, "").toLowerCase();
  return t === href.trim().replace(/\/$/, "").toLowerCase() || t === shownAddress(href).toLowerCase() || /^https?:\/\//.test(t);
}

/**
 * How a link is said: its text, then a short "link" cue. The address itself is said only when the
 * person turned that on, so a link written as a bare address becomes "a web link".
 */
export function speakLink(text: string, href: string, options: SpeechOptions = {}): string {
  const mail = /^mailto:/i.test(href);
  if (isAddressText(text, href)) {
    if (!options.readAddresses) return mail ? "an email address (link)" : "a web link";
    return `${shownAddress(href)} (link)`;
  }
  if (!options.readAddresses) return `${text} (link)`;
  return `${text} (${mail ? "email link to" : "link to"} ${shownAddress(href)})`;
}

/** Splits the reading view into sentences, in reading order. Images are read by their description. */
export function sentencesOf(root: Element, language = "en", options: SpeechOptions = {}): Sentence[] {
  const sentences: Sentence[] = [];
  const elements = root.querySelectorAll(`${READABLE}, img[alt]`);
  for (const element of elements) {
    if (element.tagName === "IMG") {
      const alt = element.getAttribute("alt")?.trim();
      if (alt) sentences.push({ text: `Image: ${alt}`, spoken: `Image: ${alt}`, block: element.closest("figure") ?? element, range: null });
      continue;
    }
    const nodes = ownTextNodes(element);
    if (nodes.length === 0) continue;
    const offsets: number[] = [];
    // where each link's text starts and ends, so it can be spoken with its cue
    const links: { start: number; end: number; href: string }[] = [];
    let text = "";
    nodes.forEach((node, i) => {
      const link = node.parentElement?.closest("a[href]") ?? null;
      if (link && link !== nodes[i - 1]?.parentElement?.closest("a[href]")) {
        links.push({ start: text.length, end: text.length, href: link.getAttribute("href") ?? "" });
      }
      offsets.push(text.length);
      text += node.data;
      if (link) links[links.length - 1].end = text.trimEnd().length;
    });
    for (const { start, end } of segment(text, language)) {
      const piece = text.slice(start, end);
      const trimmedStart = start + (piece.length - piece.trimStart().length);
      const trimmedEnd = end - (piece.length - piece.trimEnd().length);
      if (trimmedEnd <= trimmedStart) continue;
      let range: Range | null = null;
      try {
        range = root.ownerDocument.createRange();
        const [startNode, startOffset] = locate(nodes, offsets, trimmedStart, false);
        const [endNode, endOffset] = locate(nodes, offsets, trimmedEnd, true);
        range.setStart(startNode, startOffset);
        range.setEnd(endNode, endOffset);
      } catch {
        range = null;
      }
      let spoken = "";
      let at = trimmedStart;
      for (const link of links.filter((l) => l.start >= trimmedStart && l.end <= trimmedEnd && l.end > l.start)) {
        spoken += text.slice(at, link.start) + speakLink(text.slice(link.start, link.end), link.href, options);
        at = link.end;
      }
      spoken += text.slice(at, trimmedEnd);
      sentences.push({
        text: text.slice(trimmedStart, trimmedEnd).replace(/\s+/g, " "),
        spoken: spoken.replace(/\s+/g, " "),
        block: element,
        range,
      });
    }
  }
  return sentences;
}

/** The first sentence at or after an element, for starting from where the person is reading. */
export function indexFrom(sentences: Sentence[], element: Element | null): number {
  if (!element) return 0;
  const found = sentences.findIndex(
    (s) => s.block === element || element.contains(s.block) || s.block.contains(element) ||
      Boolean(element.compareDocumentPosition(s.block) & Node.DOCUMENT_POSITION_FOLLOWING),
  );
  return found < 0 ? 0 : found;
}
