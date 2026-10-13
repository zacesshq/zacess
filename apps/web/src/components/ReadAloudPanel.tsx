import { useId } from "react";
import type { ReadAloud } from "@/lib/useReadAloud";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { Switch } from "@/components/ui/switch";
import { Section } from "./Section";

export function ReadAloudPanel({ reader }: { reader: ReadAloud }) {
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

      <p className="text-sm text-muted">
        Play, Pause, Stop, the sentence buttons and Speed sit in a bar above the document and stay in view while you
        read. Reading starts from the paragraph you last clicked or selected. With none, it starts at the top.
      </p>
      {reader.error ? (
        <p role="alert" className="font-bold text-danger">
          {reader.error}
        </p>
      ) : null}
    </Section>
  );
}
