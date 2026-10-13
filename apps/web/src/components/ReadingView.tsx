import { forwardRef, useCallback, useEffect, useId, useImperativeHandle, useRef, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import type { ReadingProfile, Summary } from "@/core";
import { Button } from "@/components/ui/button";
import { prepareLinks, shownAddress, type DocumentLink } from "@/lib/links";

export const VIEW_SCOPE = ".zc-view";

/** Elements a click or a selection can set the reading position to. */
const POSITIONS = "h2, h3, h4, h5, h6, p, li, caption, th, td, figure, pre, blockquote";

export interface ReadingViewHandle {
  /** The element the person last clicked, focused or selected in, where reading aloud starts. */
  position: () => Element | null;
  /** Moves the ruler to a rectangle on screen, such as the sentence being read. */
  follow: (rect: DOMRect | null) => void;
  root: () => HTMLElement | null;
}

interface Props {
  html: string | null;
  css: string;
  profile: ReadingProfile;
  title: string | null;
  summary: Summary | null;
}

/** Points to pixels: the profile measures text in points, the page in CSS pixels. */
const PX_PER_PT = 96 / 72;

export const ReadingView = forwardRef<ReadingViewHandle, Props>(function ReadingView(
  { html, css, profile, title, summary },
  ref,
) {
  const view = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const position = useRef<Element | null>(null);
  const [band, setBand] = useState<number | null>(null);
  const [links, setLinks] = useState<DocumentLink[]>([]);
  const [linksOpen, setLinksOpen] = useState(false);
  // the full address of the link under the pointer or the keyboard focus
  const [tip, setTip] = useState<{ text: string; top: number; left?: number; right?: number } | null>(null);
  const listId = useId();
  const ruler = profile.reading_ruler;
  const bandHeight = ruler.lines * profile.font_size_pt * PX_PER_PT * profile.line_spacing;

  const place = useCallback(
    (clientY: number, height = 0) => {
      const box = view.current?.getBoundingClientRect();
      if (!box) return;
      // the band is centred on the line or on the middle of a taller element
      setBand(clientY - box.top + height / 2 - bandHeight / 2);
    },
    [bandHeight],
  );

  const setPosition = useCallback((target: EventTarget | Node | null) => {
    const element = target instanceof Element ? target : target instanceof Node ? target.parentElement : null;
    const found = element?.closest(POSITIONS);
    if (!found || !content.current?.contains(found)) return;
    position.current?.classList.remove("zc-position");
    position.current = found;
    found.classList.add("zc-position");
  }, []);

  useImperativeHandle(
    ref,
    () => ({
      position: () => {
        const selection = window.getSelection?.();
        if (selection && selection.rangeCount > 0 && !selection.isCollapsed) setPosition(selection.anchorNode);
        return position.current?.isConnected ? position.current : null;
      },
      follow: (rect) => {
        if (rect && ruler.enabled) place(rect.top, rect.height);
      },
      root: () => content.current,
    }),
    [place, ruler.enabled, setPosition],
  );

  // a new document starts with no position. Its links are made safe and listed and wide tables
  // scroll inside their own region
  useEffect(() => {
    position.current = null;
    setTip(null);
    const root = content.current;
    if (!root || !html) {
      setLinks([]);
      return;
    }
    setLinks(prepareLinks(root));
    root.querySelectorAll("table").forEach((table, i) => {
      if (table.parentElement?.classList.contains("zc-table-scroll")) return;
      const wrap = document.createElement("div");
      wrap.className = "zc-table-scroll";
      wrap.tabIndex = 0;
      wrap.setAttribute("role", "region");
      const caption = table.querySelector("caption")?.textContent?.trim();
      wrap.setAttribute("aria-label", caption ? `Table: ${caption}` : `Table ${i + 1}`);
      table.replaceWith(wrap);
      wrap.appendChild(table);
    });
  }, [html]);

  const showTip = (target: EventTarget | null) => {
    const link = target instanceof Element ? target.closest("a.zc-link") : null;
    const box = view.current?.getBoundingClientRect();
    if (!(link instanceof HTMLElement) || !box) return;
    const r = link.getBoundingClientRect();
    // a link in the right half gets a tip that grows leftwards, so it never leaves the view
    const fromLeft = r.left - box.left;
    const placement =
      fromLeft > box.width / 2 ? { right: Math.max(0, box.right - r.right) } : { left: Math.max(0, fromLeft) };
    setTip({ text: link.dataset.address ?? "", top: r.bottom - box.top + 4, ...placement });
  };
  const hideTip = (event?: { relatedTarget: EventTarget | null }) => {
    const next = event?.relatedTarget;
    // moving onto the tip itself keeps it open, so it can be read with the pointer over it
    if (next instanceof Element && next.closest(".zc-link-tip")) return;
    setTip(null);
  };

  const goToLink = (id: string) => {
    const link = content.current?.querySelector<HTMLElement>(`#${CSS.escape(id)}`);
    link?.focus();
    link?.scrollIntoView?.({ block: "center", behavior: "auto" });
  };

  const style = {
    "--zc-column": `${profile.column_width}ch`,
  } as React.CSSProperties;

  return (
    <section
      id="document"
      tabIndex={-1}
      aria-label={title ? `Document: ${title}` : "Document"}
      className="grid min-w-0 gap-3"
    >
      {summary ? (
        <div className="grid gap-1 text-sm">
          <p>
            {summary.blocks} {summary.blocks === 1 ? "block" : "blocks"}
            {summary.pages > 0 ? `, ${summary.pages} ${summary.pages === 1 ? "page" : "pages"}` : ""}
            {summary.images_without_alt > 0
              ? `, ${summary.images_without_alt} ${summary.images_without_alt === 1 ? "image" : "images"} still needing a description`
              : ""}
            .
          </p>
          {summary.uncertain > 0 ? (
            <p>
              <strong>
                {summary.uncertain} {summary.uncertain === 1 ? "piece" : "pieces"} of text to check.
              </strong>{" "}
              The recogniser was unsure of them, so they are highlighted with a dotted underline and a screen reader says
              &ldquo;unclear&rdquo; first.
            </p>
          ) : null}
        </div>
      ) : null}
      {links.length > 0 ? (
        <div className="grid gap-2">
          <div>
            <Button
              size="sm"
              aria-expanded={linksOpen}
              aria-controls={listId}
              onClick={() => setLinksOpen((open) => !open)}
            >
              {linksOpen ? <ChevronDown aria-hidden="true" /> : <ChevronRight aria-hidden="true" />}
              Links in this document ({links.length})
            </Button>
          </div>
          <ol id={listId} hidden={!linksOpen} className="m-0 grid list-none gap-2 p-0" aria-label="Links in this document">
            {links.map((link) => (
              <li key={link.id}>
                <button
                  type="button"
                  onClick={() => goToLink(link.id)}
                  className="grid w-full min-h-11 gap-0.5 rounded-md border-2 border-border bg-bg px-3 py-2 text-left text-fg hover:border-fg"
                >
                  <span className="font-bold underline">{link.text}</span>
                  <span className="text-sm break-all text-muted">{shownAddress(link.href)}</span>
                </button>
              </li>
            ))}
          </ol>
        </div>
      ) : null}
      <div
        ref={view}
        className="zc-view min-w-0 rounded-md border-2 border-fg"
        style={style}
        onPointerMove={(e) => {
          if (ruler.enabled && e.pointerType !== "touch") place(e.clientY);
        }}
        onFocus={(e) => {
          setPosition(e.target);
          showTip(e.target);
          if (ruler.enabled) {
            const r = (e.target as Element).getBoundingClientRect();
            place(r.top, r.height);
          }
        }}
        onClick={(e) => setPosition(e.target)}
        onMouseOver={(e) => showTip(e.target)}
        onMouseOut={(e) => {
          if ((e.target as Element).closest?.("a.zc-link")) hideTip(e);
        }}
        onBlur={(e) => {
          if ((e.target as Element).closest?.("a.zc-link")) hideTip(e);
        }}
        onKeyDown={(e) => {
          // the tip can always be dismissed without moving the pointer or the focus
          if (e.key === "Escape") setTip(null);
        }}
      >
        <style>{css}</style>
        {html ? (
          <div ref={content} dangerouslySetInnerHTML={{ __html: html }} />
        ) : (
          <div ref={content} className="zc-doc">
            <p>Open a file or paste some text and it appears here, in your reading settings.</p>
          </div>
        )}
        {tip ? (
          <div
            aria-hidden="true"
            className="zc-link-tip"
            style={{ top: tip.top, left: tip.left, right: tip.right }}
            onMouseLeave={() => setTip(null)}
          >
            {tip.text}
          </div>
        ) : null}
        {ruler.enabled && band !== null ? (
          <div aria-hidden="true" className="zc-ruler zc-band" style={{ top: band, height: bandHeight, position: "absolute" }} />
        ) : null}
      </div>
    </section>
  );
});
