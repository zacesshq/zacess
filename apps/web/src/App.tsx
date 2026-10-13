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
import { fileStem } from "@/lib/utils";
import { ExportPanel } from "@/components/ExportPanel";
import { OpenPanel } from "@/components/OpenPanel";
import { ProfilePanel, STANDARD } from "@/components/ProfilePanel";
import { ReadAloudPanel } from "@/components/ReadAloudPanel";
import { ReadingView, VIEW_SCOPE, type ReadingViewHandle } from "@/components/ReadingView";
import { ThemeSwitcher } from "@/components/ThemeSwitcher";

const LANGUAGE = "en-GB";

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
  link.href = url;
  link.download = name;
  link.rel = "noopener";
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

export default function App() {
  const [theme, setTheme, resolvedTheme] = useTheme();
  const standard = useMemo(() => defaultProfile(), []);
  const presets = useMemo<Preset[]>(() => listPresets(), []);
  const [profile, setProfile] = useState<ReadingProfile>(() => initialProfile(standard, resolvedTheme === "dark"));
  const [base, setBase] = useState<Base>(() =>
    load<Base>("preset", STANDARD, (v) => (typeof v === "string" && (v === STANDARD || presets.some((p) => p.id === v)) ? (v as Base) : STANDARD)),
  );
  const [opened, setOpened] = useState<Opened | null>(null);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [exported, setExported] = useState("");
  const view = useRef<ReadingViewHandle>(null);
  const root = useRef<HTMLElement | null>(null);
  const position = useRef<Element | null>(null);

  const baseProfile = base === STANDARD ? standard : (presets.find((p) => p.id === base)?.profile ?? standard);
  const adjusted = !sameSettings(profile, baseProfile);

  const followSentence = useCallback((sentence: Sentence | null) => {
    const rect = sentence?.range?.getBoundingClientRect?.() ?? sentence?.block.getBoundingClientRect() ?? null;
    view.current?.follow(rect);
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

  const busy = Boolean(progress && progress.fraction !== 1 && !error);

  return (
    <>
      <a className="skip-link" href={document_ ? "#document" : "#main"}>
        {document_ ? "Skip to the document" : "Skip to the main content"}
      </a>
      <header className="border-b-2 border-fg">
        <div className="mx-auto flex max-w-[96rem] flex-wrap items-center justify-between gap-4 px-4 py-4">
          <div className="grid gap-1">
            <h1 className="m-0 leading-none">
              <img src="./brand/zacess-lockup-light.svg" alt="Zacess" width={176} height={51} className="dark-hide" />
              <img src="./brand/zacess-lockup-dark.svg" alt="Zacess" width={176} height={51} className="dark-show" />
            </h1>
            <p className="m-0 font-bold" id="privacy">
              Files never leave your device.
            </p>
          </div>
          <ThemeSwitcher value={theme} onChange={setTheme} />
        </div>
      </header>

      <main id="main" tabIndex={-1} className="mx-auto max-w-[96rem] px-4 py-6">
        {/* the order follows the work: open, read, then adjust; on wide screens the document sits
            beside the controls */}
        <div className="grid gap-8 lg:grid-cols-[minmax(20rem,26rem)_minmax(0,1fr)] lg:grid-rows-[auto_1fr] lg:items-start">
          <div className="grid min-w-0 gap-6 lg:col-start-1 lg:row-start-1">
            <OpenPanel busy={busy} onFile={(f) => void open(f)} onPaste={paste} />
            <div className="grid gap-2" aria-live="polite">
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
          <div className="min-w-0 lg:col-start-2 lg:row-span-2 lg:row-start-1">
            <ReadingView ref={view} html={html} css={css} profile={profile} title={document_?.title ?? null} summary={summary} />
          </div>
          <div className="grid min-w-0 gap-8 lg:col-start-1 lg:row-start-2">
            <ProfilePanel
              profile={profile}
              presets={presets}
              standard={standard}
              base={base}
              adjusted={adjusted}
              onPreset={choosePreset}
              onChange={changeProfile}
            />
            <ReadAloudPanel reader={reader} hasDocument={Boolean(document_)} />
            <ExportPanel disabled={!document_} onExport={exportAs} message={exported} />
          </div>
        </div>
      </main>

      <footer className="border-t-2 border-fg">
        <div className="mx-auto grid max-w-[96rem] gap-1 px-4 py-4 text-sm">
          <p className="m-0">
            Zacess reads every file in this browser. Nothing is uploaded, there is no account and nothing is tracked.
            Settings are kept in this browser only.
          </p>
          <p className="m-0">Zacess is free software under the GNU Affero General Public License v3.0.</p>
        </div>
      </footer>
    </>
  );
}
