import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  checkProfile,
  defaultProfile,
  errorMessage,
  exportEpub,
  exportHtml,
  presets as listPresets,
  readingCss,
  readingHtml,
  summary as summarise,
  type Preset,
  type PresetId,
  type ReadingProfile,
} from "@/core";
import { openFile, openPasted, type Opened, type Progress } from "@/open";
import { load, save } from "@/lib/storage";
import { useTheme } from "@/lib/theme";
import { useReadAloud } from "@/lib/useReadAloud";
import type { Sentence } from "@/lib/speech";
import { cn, fileStem } from "@/lib/utils";
import { PanelLeftClose, PanelLeftOpen } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ExportPanel } from "@/components/ExportPanel";
import { OpenPanel } from "@/components/OpenPanel";
import { ProfilePanel, STANDARD } from "@/components/ProfilePanel";
import { ReadAloudPanel } from "@/components/ReadAloudPanel";
import { ReadAloudBar } from "@/components/ReadAloudBar";
import { revealSentence } from "@/lib/scroll";
import { ReadingView, VIEW_SCOPE, type ReadingViewHandle } from "@/components/ReadingView";
import { ThemeSwitcher } from "@/components/ThemeSwitcher";

const LANGUAGE = "en-GB";

/** The project's website. A plain link, so it needs nothing in the Content Security Policy. */
const SITE = "https://zacess.com";

const footerLinks: [string, string][] = [
  ["Privacy", `${SITE}/privacy/`],
  ["Accessibility", `${SITE}/accessibility/`],
  ["Contact", `${SITE}/contact/`],
  ["Report a problem", "https://github.com/zacesshq/zacess/issues/new/choose"],
  ["Source code", "https://github.com/zacesshq/zacess"],
];

const OPEN_PANEL = "panel-open";
const SETTINGS_PANEL = "panel-settings";
const READ_ALOUD_BAR = "read-aloud-bar";

type Base = PresetId | typeof STANDARD;

function initialProfile(standard: ReadingProfile, dark: boolean): ReadingProfile {
  const saved = load<ReadingProfile | null>("profile", null, (value) => (value ? checkProfile(value) : null));
  if (saved) return saved;
  // a first visit in dark mode starts with light text, so the page does not flash white
  return dark ? { ...standard, contrast: "light_on_dark" } : standard;
}

function sameSettings(a: ReadingProfile, b: ReadingProfile): boolean {
  return JSON.stringify({ ...a, name: "" }) === JSON.stringify({ ...b, name: "" });
}

/** Hands the browser a file to save. Nothing is uploaded: the file is made here and saved here. */
function download(name: string, data: BlobPart, type: string) {
  const url = URL.createObjectURL(new Blob([data], { type }));
  const link = document.createElement("a");
  if ("download" in link) {
    link.href = url;
    link.download = name;
    link.rel = "noopener";
    document.body.appendChild(link);
    link.click();
    link.remove();
  } else {
    // a browser without the download attribute opens the file instead, from where it can be shared or saved
    window.open(url, "_blank", "noopener");
  }
  // kept for a minute, because Safari on iPhone and iPad can read the file after the click returns
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

export default function App() {
  const [theme, setTheme, resolvedTheme] = useTheme();
  const standard = useMemo(() => defaultProfile(), []);
  const presets = useMemo<Preset[]>(() => listPresets(), []);
  const [profile, setProfile] = useState<ReadingProfile>(() => initialProfile(standard, resolvedTheme === "dark"));
  const [base, setBase] = useState<Base>(() =>
    load<Base>("preset", STANDARD, (v) => (typeof v === "string" && (v === STANDARD || presets.some((p) => p.id === v)) ? (v as Base) : STANDARD)),
  );
  const [panelOpen, setPanelOpen] = useState(() => load<boolean>("panel", true, (v) => v !== false));
  const [opened, setOpened] = useState<Opened | null>(null);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [exported, setExported] = useState("");
  const view = useRef<ReadingViewHandle>(null);
  const root = useRef<HTMLElement | null>(null);
  const position = useRef<Element | null>(null);
  const barRef = useRef<HTMLDivElement>(null);

  const baseProfile = base === STANDARD ? standard : (presets.find((p) => p.id === base)?.profile ?? standard);
  const adjusted = !sameSettings(profile, baseProfile);

  const followSentence = useCallback((sentence: Sentence | null) => {
    const rect = sentence?.range?.getBoundingClientRect?.() ?? sentence?.block.getBoundingClientRect() ?? null;
    view.current?.follow(rect);
    // the page only moves when the sentence has left the viewport. It then moves by the least
    // that brings it back, so Speed and Pause stay where they were pressed
    if (rect) {
      const bar = barRef.current?.getBoundingClientRect();
      revealSentence(rect, bar ? { top: bar.top, bottom: bar.bottom } : null, window.innerHeight, (dy) =>
        window.scrollBy({ top: dy, behavior: "auto" }),
      );
    }
  }, []);

  // the read-aloud hook reads these at the moment Play is pressed
  const rootProxy = useMemo(
    () => ({
      get current() {
        root.current = view.current?.root() ?? null;
        return root.current;
      },
    }),
    [],
  ) as React.RefObject<HTMLElement | null>;
  const positionProxy = useMemo(
    () => ({
      get current() {
        position.current = view.current?.position() ?? null;
        return position.current;
      },
    }),
    [],
  ) as React.RefObject<Element | null>;
  const reader = useReadAloud(rootProxy, positionProxy, LANGUAGE, followSentence);

  const changeProfile = (next: ReadingProfile) => {
    setProfile(next);
    save("profile", next);
  };

  const choosePreset = (id: Base) => {
    const chosen = id === STANDARD ? standard : presets.find((p) => p.id === id)?.profile;
    if (!chosen) return;
    setBase(id);
    save("preset", id);
    changeProfile(chosen);
  };

  const stopReading = reader.stop;
  const show = useCallback(
    (result: Opened) => {
      stopReading();
      setOpened(result);
      setExported("");
      // move to the document, so a keyboard or screen reader user lands on what was opened, unless
      // the person has already moved the focus somewhere else
      const before = document.activeElement;
      requestAnimationFrame(() => {
        const now = document.activeElement;
        if (now === before || now === document.body || now === null) document.getElementById("document")?.focus();
      });
    },
    [stopReading],
  );

  // only the file opened last is shown, however long an earlier one takes to finish
  const latest = useRef(0);
  const open = useCallback(
    async (file: File) => {
      const token = ++latest.current;
      const current = () => token === latest.current;
      setError(null);
      setProgress({ message: `Opening ${file.name}` });
      try {
        const result = await openFile(file, (p) => {
          if (current()) setProgress(p);
        });
        if (!current()) return;
        show(result);
        setProgress({ message: `Opened ${file.name}.`, fraction: 1 });
      } catch (e) {
        if (!current()) return;
        setProgress(null);
        setError(`${file.name} could not be opened. ${errorMessage(e)}`);
      }
    },
    [show],
  );

  const paste = (text: string, html: string | null) => {
    latest.current++;
    setError(null);
    try {
      show(openPasted(text, html));
      setProgress({ message: "Opened the pasted text.", fraction: 1 });
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  // a file dropped anywhere on the page opens, rather than the browser leaving the app to show it
  useEffect(() => {
    const over = (e: DragEvent) => e.preventDefault();
    const drop = (e: DragEvent) => {
      if (e.defaultPrevented) return;
      e.preventDefault();
      const file = e.dataTransfer?.files?.[0];
      if (file) void open(file);
    };
    window.addEventListener("dragover", over);
    window.addEventListener("drop", drop);
    return () => {
      window.removeEventListener("dragover", over);
      window.removeEventListener("drop", drop);
    };
  }, [open]);

  const document_ = opened?.document ?? null;
  // the body markup does not depend on the settings, only the style sheet does, so changing a
  // setting never rebuilds the text being read aloud
  const html = useMemo(() => (document_ ? readingHtml(document_, standard) : null), [document_, standard]);
  const summary = useMemo(() => (document_ ? summarise(document_) : null), [document_]);
  const css = useMemo(() => readingCss(profile, VIEW_SCOPE), [profile]);

  const exportAs = (format: "html" | "epub") => {
    if (!document_) return;
    try {
      const name = `${fileStem(document_.title)}.${format}`;
      if (format === "html") download(name, exportHtml(document_, profile, LANGUAGE), "text/html");
      else download(name, exportEpub(document_, profile, LANGUAGE) as Uint8Array<ArrayBuffer>, "application/epub+zip");
      setExported(`Saved ${name} to your downloads.`);
    } catch (e) {
      setExported(`The export failed. ${errorMessage(e)}`);
    }
  };

  // the controls can be hidden only while a document is open, so the way to open one is never hidden
  const collapsed = Boolean(document_) && !panelOpen;
  const togglePanel = () => {
    const next = !panelOpen;
    setPanelOpen(next);
    save("panel", next);
  };

  const busy = Boolean(progress && progress.fraction !== 1 && !error);

  return (
    <>
      <a className="skip-link" href={document_ ? "#document" : "#main"}>
        {document_ ? "Skip to the document" : "Skip to the main content"}
      </a>
      <header className="zc-safe-top border-b-2 border-fg">
        {/* one row from 320 pixels up: on a phone the lockup is smaller, the link reads "About" and,
            below 375 pixels, the colour mode button keeps only its icon. Only very large text or
            zoom wraps the buttons under the lockup */}
        <div className="zc-gutter mx-auto flex max-w-[96rem] flex-wrap items-center justify-between gap-x-3 gap-y-2 py-2 sm:gap-x-6 sm:py-4">
          <h1 className="m-0 leading-none">
            <a href={SITE} aria-label="Zacess home" className="inline-flex min-h-11 items-center rounded-sm">
              <img src="./brand/zacess-lockup-light.svg" alt="" width={176} height={51} className="dark-hide h-9 w-auto sm:h-[51px]" />
              <img src="./brand/zacess-lockup-dark.svg" alt="" width={176} height={51} className="dark-show h-9 w-auto sm:h-[51px]" />
            </a>
          </h1>
          <div className="flex flex-wrap items-center gap-x-1 sm:gap-x-4">
            <a
              href={SITE}
              className="inline-flex min-h-11 items-center px-2 font-bold underline decoration-2 underline-offset-4 hover:decoration-4"
            >
              {/* one inline run, so the space before "Zacess" survives the flex layout */}
              <span>
                About <span className="max-sm:sr-only">Zacess</span>
              </span>
            </a>
            <ThemeSwitcher value={theme} onChange={setTheme} />
          </div>
        </div>
      </header>

      <main id="main" tabIndex={-1} className="zc-gutter mx-auto max-w-[96rem] py-6">
        {/* the order follows the work: open, read, then adjust; from tablet width the document sits
            beside the controls. With a document open the controls can be hidden to give it the
            full width */}
        {document_ ? (
          <div className="mb-6 flex flex-wrap items-center gap-x-3 gap-y-1">
            <Button
              size="sm"
              aria-expanded={!collapsed}
              aria-controls={reader.supported ? `${OPEN_PANEL} ${SETTINGS_PANEL} ${READ_ALOUD_BAR}` : `${OPEN_PANEL} ${SETTINGS_PANEL}`}
              onClick={togglePanel}
            >
              {collapsed ? <PanelLeftOpen aria-hidden="true" /> : <PanelLeftClose aria-hidden="true" />}
              {collapsed ? "Show panel" : "Hide panel"}
            </Button>
            {collapsed ? (
              <span className="text-sm text-muted">Opening, reading settings, read aloud and export are hidden.</span>
            ) : null}
          </div>
        ) : null}
        <div
          className={cn(
            collapsed ? "grid gap-4" : "grid gap-8",
            !collapsed &&
              "md:grid-cols-[minmax(16rem,19rem)_minmax(0,1fr)] md:grid-rows-[auto_1fr] md:items-start md:gap-6 lg:grid-cols-[minmax(20rem,26rem)_minmax(0,1fr)] lg:gap-8",
          )}
        >
          <div className={cn("grid min-w-0 gap-6", !collapsed && "md:col-start-1 md:row-start-1")}>
            <div id={OPEN_PANEL} hidden={collapsed}>
              <OpenPanel busy={busy} onFile={(f) => void open(f)} onPaste={paste} />
            </div>
            {/* progress and errors stay outside the panel, so they are seen and announced while it is hidden */}
            <div className="grid gap-2 empty:-mt-6" aria-live="polite">
              {progress ? (
                <>
                  <p className="m-0">{progress.message}</p>
                  {progress.fraction !== 1 && !error ? (
                    <progress
                      aria-label="Progress"
                      className="h-3 w-full"
                      max={1}
                      value={progress.fraction ?? undefined}
                    />
                  ) : null}
                </>
              ) : null}
              {opened?.notes.map((note) => (
                <p key={note} className="m-0">
                  {note}
                </p>
              ))}
            </div>
            {error ? (
              <p role="alert" className="m-0 rounded-md border-2 border-danger p-3 font-bold text-danger">
                {error}
              </p>
            ) : null}
          </div>
          <div className={cn("min-w-0", !collapsed && "md:col-start-2 md:row-span-2 md:row-start-1")}>
            {document_ ? (
              <ReadAloudBar
                ref={barRef}
                id={READ_ALOUD_BAR}
                reader={reader}
                hidden={collapsed}
              />
            ) : null}
            <ReadingView ref={view} html={html} css={css} profile={profile} title={document_?.title ?? null} summary={summary} />
          </div>
          <div id={SETTINGS_PANEL} hidden={collapsed} className="grid min-w-0 gap-8 md:col-start-1 md:row-start-2">
            <ProfilePanel
              profile={profile}
              presets={presets}
              standard={standard}
              base={base}
              adjusted={adjusted}
              onPreset={choosePreset}
              onChange={changeProfile}
            />
            <ReadAloudPanel reader={reader} />
            <ExportPanel disabled={!document_} onExport={exportAs} message={exported} />
          </div>
        </div>
      </main>

      <footer className="border-t-2 border-fg">
        <div className="zc-gutter zc-safe-bottom mx-auto flex max-w-[96rem] flex-wrap items-start justify-between gap-x-10 gap-y-4 pt-5 text-sm">
          <div className="grid max-w-[46rem] gap-1">
            <p className="m-0">
              Zacess reads every file in this browser. Nothing is uploaded, there is no account and nothing is tracked.
              Settings are kept in this browser only.
            </p>
            <p className="m-0">
              Zacess is free software under the{" "}
              <a className="underline underline-offset-2" href="https://github.com/zacesshq/zacess/blob/main/LICENSE">
                GNU Affero General Public License v3.0
              </a>
              .
            </p>
          </div>
          <nav aria-label="Footer">
            <ul className="m-0 flex list-none flex-wrap gap-x-5 p-0">
              {footerLinks.map(([label, href]) => (
                <li key={href}>
                  <a className="inline-flex min-h-11 items-center font-bold underline underline-offset-2" href={href}>
                    {label}
                  </a>
                </li>
              ))}
            </ul>
          </nav>
        </div>
      </footer>
    </>
  );
}
