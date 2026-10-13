import { useEffect, useState } from "react";

const COARSE = "(pointer: coarse)";

function matches(): boolean {
  try {
    return window.matchMedia(COARSE).matches;
  } catch {
    return false;
  }
}

/** True on a touch screen such as a phone or tablet, where a camera control makes sense. */
export function useCoarsePointer(): boolean {
  const [coarse, setCoarse] = useState(matches);
  useEffect(() => {
    let media: MediaQueryList | null = null;
    const update = () => setCoarse(matches());
    try {
      media = window.matchMedia(COARSE);
      media.addEventListener("change", update);
    } catch {
      media = null;
    }
    return () => media?.removeEventListener("change", update);
  }, []);
  return coarse;
}
