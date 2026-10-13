import { useCallback, useEffect, useRef, useState } from "react";
import { availableVoices, indexFrom, pickVoice, sentencesOf, type Sentence } from "./speech";
import { load, save } from "./storage";

export type ReadState = "idle" | "playing" | "paused";

const HIGHLIGHT = "zc-reading";

interface HighlightRegistry {
  set(name: string, value: unknown): void;
  delete(name: string): void;
}

function highlights(): { registry: HighlightRegistry; Highlight: new (...ranges: Range[]) => unknown } | null {
  const css = (globalThis as { CSS?: { highlights?: HighlightRegistry } }).CSS;
  const Highlight = (globalThis as { Highlight?: new (...ranges: Range[]) => unknown }).Highlight;
  return css?.highlights && Highlight ? { registry: css.highlights, Highlight } : null;
}

function speech(): SpeechSynthesis | null {
  return typeof window !== "undefined" && "speechSynthesis" in window ? window.speechSynthesis : null;
}

export interface ReadAloud {
  supported: boolean;
  state: ReadState;
  /** Every voice the browser offers, before filtering. */
  allVoices: SpeechSynthesisVoice[];
  /** The voices offered: on-device only unless online voices were allowed. */
  voices: SpeechSynthesisVoice[];
  voice: SpeechSynthesisVoice | null;
  setVoice: (uri: string) => void;
  allowOnline: boolean;
  setAllowOnline: (allow: boolean) => void;
  rate: number;
  setRate: (rate: number) => void;
  /** Say each link's address after its text, which is off by default. */
  readAddresses: boolean;
  setReadAddresses: (read: boolean) => void;
  current: number;
  total: number;
  error: string | null;
  play: () => void;
  pause: () => void;
  stop: () => void;
  skip: (by: number) => void;
}

/**
 * Reads the reading view aloud a sentence at a time, highlighting each one. Pausing cancels the
 * sentence and keeps its place, because pause and resume are unreliable in several browsers;
 * playing again starts that sentence over.
 */
export function useReadAloud(
  root: React.RefObject<HTMLElement | null>,
  position: React.RefObject<Element | null>,
  language: string,
  onSentence?: (sentence: Sentence | null) => void,
): ReadAloud {
  const synth = speech();
  const [allVoices, setAllVoices] = useState<SpeechSynthesisVoice[]>([]);
  const [allowOnline, setAllowOnlineState] = useState(() => load<boolean>("voices.online", false, (v) => v === true));
  const [voiceUri, setVoiceUri] = useState<string | null>(() => load<string | null>("voices.voice", null, (v) => (typeof v === "string" ? v : null)));
  const [readAddresses, setReadAddressesState] = useState(() => load<boolean>("voices.addresses", false, (v) => v === true));
  const [rate, setRateState] = useState(() => load<number>("voices.rate", 1, (v) => (typeof v === "number" && v >= 0.5 && v <= 2 ? v : 1)));
  const [state, setState] = useState<ReadState>("idle");
  const [current, setCurrent] = useState(0);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const sentences = useRef<Sentence[]>([]);
  const index = useRef(0);
  const run = useRef(0);
  const marked = useRef<Element | null>(null);

  useEffect(() => {
    if (!synth) return;
    const update = () => setAllVoices(synth.getVoices());
    update();
    synth.addEventListener?.("voiceschanged", update);
    return () => synth.removeEventListener?.("voiceschanged", update);
  }, [synth]);

  const voices = availableVoices(allVoices, allowOnline);
  const voice = pickVoice(voices, voiceUri, language);
  const voiceRef = useRef(voice);
  voiceRef.current = voice;
  const rateRef = useRef(rate);
  rateRef.current = rate;

  const clearMark = useCallback(() => {
    highlights()?.registry.delete(HIGHLIGHT);
    marked.current?.classList.remove("zc-reading-block");
    marked.current = null;
  }, []);

  const mark = useCallback(
    (sentence: Sentence) => {
      clearMark();
      const h = highlights();
      if (h && sentence.range) h.registry.set(HIGHLIGHT, new h.Highlight(sentence.range));
      sentence.block.classList.add("zc-reading-block");
      marked.current = sentence.block;
      sentence.block.scrollIntoView?.({ block: "nearest", behavior: "auto" });
      onSentence?.(sentence);
    },
    [clearMark, onSentence],
  );

  const speakFrom = useCallback(
    (i: number) => {
      if (!synth) return;
      const token = ++run.current;
      const list = sentences.current;
      if (i >= list.length) {
        index.current = 0;
        setCurrent(0);
        setState("idle");
        clearMark();
        onSentence?.(null);
        return;
      }
      index.current = i;
      setCurrent(i + 1);
      const sentence = list[i];
      mark(sentence);
      const utterance = new SpeechSynthesisUtterance(sentence.spoken);
      if (voiceRef.current) {
        utterance.voice = voiceRef.current;
        utterance.lang = voiceRef.current.lang;
      } else {
        utterance.lang = language;
      }
      utterance.rate = rateRef.current;
      utterance.onend = () => {
        if (token === run.current) speakFrom(i + 1);
      };
      utterance.onerror = (event) => {
        if (token !== run.current || event.error === "interrupted" || event.error === "canceled") return;
        run.current++;
        setState("idle");
        clearMark();
        setError("The voice stopped with an error. Try another voice.");
      };
      synth.speak(utterance);
    },
    [synth, mark, clearMark, language, onSentence],
  );

  const play = useCallback(() => {
    if (!synth || !root.current) return;
    setError(null);
    if (state === "paused" && sentences.current.length) {
      setState("playing");
      speakFrom(index.current);
      return;
    }
    sentences.current = sentencesOf(root.current, language, { readAddresses });
    setTotal(sentences.current.length);
    if (sentences.current.length === 0) {
      setError("There is nothing to read yet. Open a document first.");
      return;
    }
    synth.cancel();
    setState("playing");
    speakFrom(indexFrom(sentences.current, position.current));
  }, [synth, root, position, language, state, speakFrom, readAddresses]);

  const pause = useCallback(() => {
    if (!synth || state !== "playing") return;
    run.current++;
    synth.cancel();
    setState("paused");
  }, [synth, state]);

  const stop = useCallback(() => {
    if (!synth) return;
    run.current++;
    synth.cancel();
    index.current = 0;
    setCurrent(0);
    setState("idle");
    clearMark();
    onSentence?.(null);
  }, [synth, clearMark, onSentence]);

  const skip = useCallback(
    (by: number) => {
      if (!synth || state === "idle" || sentences.current.length === 0) return;
      const next = Math.min(Math.max(index.current + by, 0), sentences.current.length - 1);
      if (state === "playing") {
        run.current++;
        synth.cancel();
        speakFrom(next);
      } else {
        index.current = next;
        setCurrent(next + 1);
        mark(sentences.current[next]);
      }
    },
    [synth, state, speakFrom, mark],
  );

  // a new document or a closed page ends reading
  useEffect(() => () => {
    run.current++;
    synth?.cancel();
  }, [synth]);

  return {
    supported: Boolean(synth),
    state,
    allVoices,
    voices,
    voice,
    setVoice: (uri) => {
      setVoiceUri(uri);
      save("voices.voice", uri);
    },
    allowOnline,
    setAllowOnline: (allow) => {
      setAllowOnlineState(allow);
      save("voices.online", allow);
    },
    readAddresses,
    setReadAddresses: (read) => {
      setReadAddressesState(read);
      save("voices.addresses", read);
    },
    rate,
    setRate: (next) => {
      setRateState(next);
      save("voices.rate", next);
    },
    current,
    total,
    error,
    play,
    pause,
    stop,
    skip,
  };
}
