import { useEffect, useState } from "react";
import { load, save } from "./storage";

export type ThemeChoice = "system" | "light" | "dark";

const KEY = "theme";

function systemDark(): boolean {
  try {
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  } catch {
    return false;
  }
}

export function resolveTheme(choice: ThemeChoice): "light" | "dark" {
  if (choice === "system") return systemDark() ? "dark" : "light";
  return choice;
}

/** The app's own colours: the system's choice unless the person picked light or dark. */
export function useTheme(): [ThemeChoice, (choice: ThemeChoice) => void, "light" | "dark"] {
  const [choice, setChoice] = useState<ThemeChoice>(() =>
    load<ThemeChoice>(KEY, "system", (v) => (v === "light" || v === "dark" || v === "system" ? v : "system")),
  );
  const [resolved, setResolved] = useState(() => resolveTheme(choice));

  useEffect(() => {
    const apply = () => {
      const next = resolveTheme(choice);
      setResolved(next);
      if (choice === "system") document.documentElement.removeAttribute("data-theme");
      else document.documentElement.setAttribute("data-theme", next);
    };
    apply();
    let media: MediaQueryList | null = null;
    try {
      media = window.matchMedia("(prefers-color-scheme: dark)");
      media.addEventListener("change", apply);
    } catch {
      media = null;
    }
    return () => media?.removeEventListener("change", apply);
  }, [choice]);

  const update = (next: ThemeChoice) => {
    setChoice(next);
    save(KEY, next);
  };
  return [choice, update, resolved];
}
