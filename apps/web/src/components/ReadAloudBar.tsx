import { forwardRef } from "react";
import { Pause, Play, SkipBack, SkipForward, Square } from "lucide-react";
import type { ReadAloud } from "@/lib/useReadAloud";
import { Button } from "@/components/ui/button";
import { SliderField } from "./SliderField";

/**
 * Play, Pause, Stop, the sentence buttons and Speed, kept at the top of the viewport while the
 * document is on screen. Below 640 pixels the button names are for screen readers only, so all
 * five buttons fit one row at 320 pixels wide.
 */
export const ReadAloudBar = forwardRef<HTMLDivElement, { reader: ReadAloud; hidden?: boolean; id?: string }>(
  function ReadAloudBar({ reader, hidden, id }, ref) {
    if (!reader.supported) return null;
    const status =
      reader.state === "playing"
        ? `Reading sentence ${reader.current} of ${reader.total}`
        : reader.state === "paused"
          ? `Paused at sentence ${reader.current} of ${reader.total}`
          : "Stopped";
    return (
      <div
        ref={ref}
        id={id}
        hidden={hidden}
        role="group"
        aria-label="Read aloud controls"
        className="sticky top-0 z-20 mb-4 flex flex-wrap items-center gap-x-4 gap-y-2 border-2 border-fg bg-bg p-2"
      >
        <div className="flex gap-2">
          {reader.state === "playing" ? (
            <Button variant="primary" onClick={reader.pause} aria-label="Pause">
              <Pause aria-hidden="true" />
              <span className="max-sm:sr-only">Pause</span>
            </Button>
          ) : (
            <Button variant="primary" onClick={reader.play} aria-label={reader.state === "paused" ? "Resume" : "Play"}>
              <Play aria-hidden="true" />
              <span className="max-sm:sr-only">{reader.state === "paused" ? "Resume" : "Play"}</span>
            </Button>
          )}
          <Button onClick={reader.stop} disabled={reader.state === "idle"} aria-label="Stop">
            <Square aria-hidden="true" />
            <span className="max-sm:sr-only">Stop</span>
          </Button>
          <Button size="icon" onClick={() => reader.skip(-1)} disabled={reader.state === "idle"} aria-label="Previous sentence">
            <SkipBack aria-hidden="true" />
          </Button>
          <Button size="icon" onClick={() => reader.skip(1)} disabled={reader.state === "idle"} aria-label="Next sentence">
            <SkipForward aria-hidden="true" />
          </Button>
        </div>
        <div className="min-w-40 flex-1">
        <SliderField
          label="Speed"
          value={reader.rate}
          min={0.5}
          max={2}
          step={0.1}
          shown={`${reader.rate.toFixed(1)} times`}
          onChange={(v) => reader.setRate(Number(v.toFixed(1)))}
        />
        </div>
        <p className="m-0 w-full text-sm" role="status">
          {status}
        </p>
        {reader.error ? (
          <p role="alert" className="m-0 w-full font-bold text-danger">
            {reader.error}
          </p>
        ) : null}
      </div>
    );
  },
);
