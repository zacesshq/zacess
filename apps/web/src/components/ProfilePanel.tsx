import { useId } from "react";
import { Minus, Plus } from "lucide-react";
import type { ContrastTheme, Preset, PresetId, ReadingProfile } from "@/core";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupCard } from "@/components/ui/radio-group";
import { Switch } from "@/components/ui/switch";
import { Section } from "./Section";
import { SliderField } from "./SliderField";

/** The core's contrast themes, with the colours from docs/reading-profiles.md for the sample. */
export const THEMES: { id: ContrastTheme; name: string; text: string; background: string }[] = [
  { id: "dark_on_light", name: "Dark text on white", text: "#111111", background: "#ffffff" },
  { id: "light_on_dark", name: "Light text on near black", text: "#f5f5f5", background: "#121212" },
  { id: "yellow_on_black", name: "Yellow text on black", text: "#ffeb3b", background: "#000000" },
  { id: "black_on_cream", name: "Black text on cream", text: "#000000", background: "#fdf6e3" },
];

export const STANDARD = "standard";

/** The text size limits, in points. The core accepts 6 to 96. */
export const MIN_SIZE = 10;
export const MAX_SIZE = 96;

interface Props {
  profile: ReadingProfile;
  presets: Preset[];
  standard: ReadingProfile;
  /** The preset the settings started from; "standard" when none was chosen. */
  base: PresetId | typeof STANDARD;
  /** True once a setting differs from the preset it started from. */
  adjusted: boolean;
  onPreset: (id: PresetId | typeof STANDARD) => void;
  onChange: (profile: ReadingProfile) => void;
}

const round = (value: number, places: number) => Number(value.toFixed(places));

export function ProfilePanel({ profile, presets, base, adjusted, onPreset, onChange }: Props) {
  const presetLabel = useId();
  const themeLabel = useId();
  const rulerId = useId();
  const set = <K extends keyof ReadingProfile>(key: K, value: ReadingProfile[K]) => onChange({ ...profile, [key]: value });
  const size = profile.font_size_pt;
  const zoom = (by: number) => set("font_size_pt", Math.min(MAX_SIZE, Math.max(MIN_SIZE, Math.round(size + by))));

  return (
    <Section title="Reading settings">
      <div className="grid gap-2">
        <span id={presetLabel} className="font-bold">
          Reading profile
        </span>
        <RadioGroup aria-labelledby={presetLabel} value={base} onValueChange={(v) => onPreset(v as PresetId)}>
          <RadioGroupCard value={STANDARD}>Standard</RadioGroupCard>
          {presets.map((p) => (
            <RadioGroupCard key={p.id} value={p.id}>
              {p.name}
            </RadioGroupCard>
          ))}
        </RadioGroup>
        <p className="text-sm text-muted" aria-live="polite">
          {adjusted ? "Adjusted from this profile. Choose it again to undo your changes." : "Choosing a profile changes the settings below."}
        </p>
      </div>

      <div className="grid gap-2">
        <SliderField
          label="Text size"
          value={size}
          min={MIN_SIZE}
          max={MAX_SIZE}
          step={1}
          shown={`${size} pt`}
          spoken={`${size} points`}
          onChange={(v) => set("font_size_pt", v)}
        />
        <div className="flex gap-2">
          <Button size="sm" onClick={() => zoom(-2)} disabled={size <= MIN_SIZE}>
            <Minus aria-hidden="true" />
            Smaller
          </Button>
          <Button size="sm" onClick={() => zoom(2)} disabled={size >= MAX_SIZE}>
            <Plus aria-hidden="true" />
            Larger
          </Button>
        </div>
      </div>
      <SliderField
        label="Line spacing"
        value={profile.line_spacing}
        min={1}
        max={3}
        step={0.05}
        shown={`${profile.line_spacing.toFixed(2)} times`}
        onChange={(v) => set("line_spacing", round(v, 2))}
      />
      <SliderField
        label="Letter spacing"
        value={profile.character_spacing}
        min={0}
        max={0.5}
        step={0.01}
        shown={`${profile.character_spacing.toFixed(2)} em`}
        spoken={`${profile.character_spacing.toFixed(2)} of the text size`}
        onChange={(v) => set("character_spacing", round(v, 2))}
      />
      <SliderField
        label="Column width"
        value={profile.column_width}
        min={20}
        max={150}
        step={1}
        shown={`${profile.column_width} characters`}
        hint="The longest line. The column also narrows to fit the screen, so the page never scrolls sideways."
        onChange={(v) => set("column_width", Math.round(v))}
      />

      <div className="grid gap-2">
        <span id={themeLabel} className="font-bold">
          Colour theme
        </span>
        <RadioGroup
          aria-labelledby={themeLabel}
          value={profile.contrast}
          onValueChange={(v) => set("contrast", v as ContrastTheme)}
        >
          {THEMES.map((t) => (
            <RadioGroupCard key={t.id} value={t.id}>
              <span
                aria-hidden="true"
                className="inline-flex h-8 w-11 shrink-0 items-center justify-center rounded border-2 border-border font-bold"
                style={{ color: t.text, background: t.background }}
              >
                Aa
              </span>
              {t.name}
            </RadioGroupCard>
          ))}
        </RadioGroup>
      </div>

      <div className="grid gap-3">
        <div className="flex items-center justify-between gap-3">
          <Label htmlFor={rulerId}>Reading ruler</Label>
          <Switch
            id={rulerId}
            checked={profile.reading_ruler.enabled}
            onCheckedChange={(enabled) => set("reading_ruler", { ...profile.reading_ruler, enabled })}
          />
        </div>
        <p className="text-sm text-muted">A band that follows the pointer, the keyboard focus and the sentence being read.</p>
        {profile.reading_ruler.enabled ? (
          <SliderField
            label="Ruler height"
            value={profile.reading_ruler.lines}
            min={1}
            max={5}
            step={1}
            shown={`${profile.reading_ruler.lines} ${profile.reading_ruler.lines === 1 ? "line" : "lines"}`}
            onChange={(v) => set("reading_ruler", { ...profile.reading_ruler, lines: Math.round(v) })}
          />
        ) : null}
      </div>
    </Section>
  );
}
