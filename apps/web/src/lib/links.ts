// Links in the reading view. Only web and mail addresses are ever shown as links; the address is
// never fetched, prefetched or previewed. A link only opens when the person follows it.

/** Whether an address may be shown as a link: http, https and mailto only. */
export function isWebLink(href: string): boolean {
  const value = href.trim();
  if (!/^(https?:\/\/|mailto:)/i.test(value)) return false;
  try {
    const url = new URL(value);
    return ["http:", "https:", "mailto:"].includes(url.protocol);
  } catch {
    return false;
  }
}

/** The address as a person reads it: no scheme and no trailing slash. */
export function shownAddress(href: string): string {
  const value = href.trim();
  if (/^mailto:/i.test(value)) return value.slice("mailto:".length);
  return value.replace(/^https?:\/\//i, "").replace(/\/$/, "");
}

export interface DocumentLink {
  id: string;
  text: string;
  href: string;
}

const SVG = "http://www.w3.org/2000/svg";

/** A small arrow out of a box, so a link is marked by its shape as well as its underline. */
function externalIcon(doc: Document): SVGSVGElement {
  const svg = doc.createElementNS(SVG, "svg");
  svg.setAttribute("class", "zc-link-icon");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  const path = doc.createElementNS(SVG, "path");
  path.setAttribute("d", "M6 3H3v10h10v-3M9 2h5v5M14 2 7 9");
  path.setAttribute("fill", "none");
  path.setAttribute("stroke", "currentColor");
  path.setAttribute("stroke-width", "2");
  path.setAttribute("stroke-linecap", "round");
  path.setAttribute("stroke-linejoin", "round");
  svg.append(path);
  return svg;
}

/**
 * Prepares the links in rendered document HTML: anything that is not a web or mail address is
 * turned back into plain text. Every remaining link opens in a new tab, carries an icon and
 * says so to screen readers. Returns the links in reading order.
 */
export function prepareLinks(root: Element): DocumentLink[] {
  const doc = root.ownerDocument;
  const found: DocumentLink[] = [];
  for (const a of Array.from(root.querySelectorAll("a"))) {
    const href = a.getAttribute("href") ?? "";
    if (!isWebLink(href)) {
      a.replaceWith(...Array.from(a.childNodes));
      continue;
    }
    const text = (a.textContent ?? "").replace(/\s+/g, " ").trim() || shownAddress(href);
    const id = `zc-link-${found.length + 1}`;
    a.id = id;
    a.target = "_blank";
    a.rel = "noopener noreferrer";
    a.referrerPolicy = "no-referrer";
    a.classList.add("zc-link");
    a.dataset.address = shownAddress(href);
    const hint = doc.createElement("span");
    hint.className = "zc-sr";
    hint.textContent = " (opens in a new tab)";
    a.append(externalIcon(doc), hint);
    found.push({ id, text, href: href.trim() });
  }
  return found;
}
