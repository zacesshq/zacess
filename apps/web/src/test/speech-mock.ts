import { vi } from "vitest";
import type { VoiceLike } from "@/lib/speech";

export const LOCAL: VoiceLike = { name: "Daniel", lang: "en-GB", localService: true, voiceURI: "local-daniel", default: true };
export const LOCAL_2: VoiceLike = { name: "Karen", lang: "en-AU", localService: true, voiceURI: "local-karen" };
export const ONLINE: VoiceLike = { name: "Cloud English", lang: "en-GB", localService: false, voiceURI: "online-english" };

export interface SpokenUtterance {
  text: string;
  voice: VoiceLike | null;
  rate: number;
  onend: (() => void) | null;
  onerror: ((e: { error: string }) => void) | null;
}

/** A stand-in for the browser's speech synthesis that records what it was asked to say. */
export function installSpeech(voices: VoiceLike[] = [LOCAL, LOCAL_2, ONLINE]) {
  const spoken: SpokenUtterance[] = [];
  class Utterance {
    text: string;
    voice: VoiceLike | null = null;
    rate = 1;
    lang = "";
    onend: (() => void) | null = null;
    onerror: ((e: { error: string }) => void) | null = null;
    constructor(text: string) {
      this.text = text;
    }
  }
  const synth = {
    getVoices: () => voices,
    speak: vi.fn((u: SpokenUtterance) => spoken.push(u)),
    cancel: vi.fn(),
    pause: vi.fn(),
    resume: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  };
  vi.stubGlobal("SpeechSynthesisUtterance", Utterance);
  Object.defineProperty(window, "speechSynthesis", { value: synth, configurable: true, writable: true });
  return { synth, spoken };
}

export function removeSpeech() {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  delete (window as any).speechSynthesis;
  vi.unstubAllGlobals();
}
