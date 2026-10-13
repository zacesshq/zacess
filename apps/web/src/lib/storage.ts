// Settings are kept in this browser only. Storage can be missing or refuse writes (a private
// window, blocked site data), so every read and write is guarded and the app works without it.

const PREFIX = "zacess.";

export function load<T>(key: string, fallback: T, check?: (value: unknown) => T): T {
  try {
    const raw = window.localStorage.getItem(PREFIX + key);
    if (raw === null) return fallback;
    const value: unknown = JSON.parse(raw);
    return check ? check(value) : (value as T);
  } catch {
    return fallback;
  }
}

export function save(key: string, value: unknown): void {
  try {
    window.localStorage.setItem(PREFIX + key, JSON.stringify(value));
  } catch {
    // nothing to do: the setting still applies until the page is closed
  }
}
