/** What an import is doing, for the progress bar and the spoken status. */
export interface Progress {
  message: string;
  /** From 0 to 1 when the share done is known. */
  fraction?: number;
}

export type ProgressListener = (progress: Progress) => void;

/** The address of a file the app serves itself, wherever the app is hosted. */
export function ownAsset(path: string): string {
  return new URL(path, document.baseURI).href;
}
