import { useId } from "react";
import { Pause, Play, SkipBack, SkipForward, Square } from "lucide-react";
import type { ReadAloud } from "@/lib/useReadAloud";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { Switch } from "@/components/ui/switch";
import { Section } from "./Section";
import { SliderField } from "./SliderField";

export function ReadAloudPanel({ reader, hasDocument }: { reader: ReadAloud; hasDocument: boolean }) {
  const voiceId = useId();
  const onlineId = useId();
  const onlineHint = useId();
  const addressesId = useId();
  const addressesHint = useId();

  if (!reader.supported) {
    return (
      <Section title="Read aloud">
        <p>This browser has no speech voices, so read aloud is not available here.</p>
      </Section>
    );
  }

  const status =
    reader.state === "playing"
      ? `Reading sentence ${reader.current} of ${reader.total}`
      : reader.state === "paused"
        ? `Paused at sentence ${reader.current} of ${reader.total}`
        : "Stopped";

  return (
    <Section title="Read aloud">
      <div className="grid gap-2">
        {reader.voices.length > 0 ? <Label htmlFor={voiceId}>Voice</Label> : <span className="font-bold">Voice</span>}
        {reader.voices.length > 0 ? (
          <NativeSelect
            id={voiceId}
            value={reader.voice?.voiceURI ?? ""}
            onChange={(e) => reader.setVoice(e.target.value)}
          >
            {reader.voices.map((v) => (
              <option key={v.voiceURI} value={v.voiceURI}>
                {v.name} ({v.lang}){v.localService ? "" : ", online"}
              </option>
            ))}
          </NativeSelect>
        ) : (
          <p>
            {reader.allVoices.length > 0
              ? "No built-in voices were found. Online voices are available below."
              : "No voices have loaded yet."}
          </p>
        )}
      </div>

      <div className="grid gap-1">
        <div className="flex items-center justify-between gap-3">
          <Label htmlFor={onlineId}>Show online voices</Label>
          <Switch
            id={onlineId}
            aria-describedby={onlineHint}
            checked={reader.allowOnline}
            onCheckedChange={reader.setAllowOnline}
          />
        </div>
        <p id={onlineHint} className="text-sm text-muted">
          Off by default. An online voice sends the text it reads to the company that provides the voice.
        </p>
      </div>

      <div className="grid gap-1">
        <div className="flex items-center justify-between gap-3">
          <Label htmlFor={addressesId}>Read web addresses aloud</Label>
          <Switch
            id={addressesId}
            aria-describedby={addressesHint}
            checked={reader.readAddresses}
            onCheckedChange={reader.setReadAddresses}
          />
        </div>
        <p id={addressesHint} className="text-sm text-muted">
          Off by default: a link is read as its text followed by &ldquo;link&rdquo;.
        </p>
      </div>

      <SliderField
        label="Speed"
        value={reader.rate}
        min={0.5}
        max={2}
        step={0.1}
        shown={`${reader.rate.toFixed(1)} times`}
        onChange={(v) => reader.setRate(Number(v.toFixed(1)))}
      />

      <div className="flex flex-wrap gap-2" role="group" aria-label="Read aloud controls">
        {reader.state === "playing" ? (
          <Button variant="primary" onClick={reader.pause}>
            <Pause aria-hidden="true" />
            Pause
          </Button>
        ) : (
          <Button variant="primary" onClick={reader.play} disabled={!hasDocument}>
            <Play aria-hidden="true" />
            {reader.state === "paused" ? "Resume" : "Play"}
          </Button>
        )}
        <Button onClick={reader.stop} disabled={reader.state === "idle"}>
          <Square aria-hidden="true" />
          Stop
        </Button>
        <Button size="icon" onClick={() => reader.skip(-1)} disabled={reader.state === "idle"} aria-label="Previous sentence">
          <SkipBack aria-hidden="true" />
        </Button>
        <Button size="icon" onClick={() => reader.skip(1)} disabled={reader.state === "idle"} aria-label="Next sentence">
          <SkipForward aria-hidden="true" />
        </Button>
      </div>
      <p className="text-sm" role="status">
        {status}
      </p>
      <p className="text-sm text-muted">
        Reading starts from the paragraph you last clicked or selected. With none, it starts at the top.
      </p>
      {reader.error ? (
        <p role="alert" className="font-bold text-danger">
          {reader.error}
        </p>
      ) : null}
    </Section>
  );
}
