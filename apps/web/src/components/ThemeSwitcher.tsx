import { useId } from "react";
import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import type { ThemeChoice } from "@/lib/theme";

const CHOICES: { value: ThemeChoice; label: string }[] = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

/** The app's own colours. The reading view keeps the colour theme from the reading settings. */
export function ThemeSwitcher({ value, onChange }: { value: ThemeChoice; onChange: (choice: ThemeChoice) => void }) {
  const id = useId();
  return (
    <div className="flex flex-wrap items-center gap-2">
      <span id={id} className="text-sm font-bold">
        App colours
      </span>
      <RadioGroupPrimitive.Root
        aria-labelledby={id}
        value={value}
        onValueChange={(v) => onChange(v as ThemeChoice)}
        orientation="horizontal"
        className="inline-flex rounded-md border-2 border-fg"
      >
        {CHOICES.map((c) => (
          <RadioGroupPrimitive.Item
            key={c.value}
            value={c.value}
            className="min-h-10 border-fg px-3 text-sm font-bold text-fg not-first:border-l-2 data-[state=checked]:bg-fg data-[state=checked]:text-bg data-[state=checked]:underline data-[state=checked]:decoration-2 data-[state=checked]:underline-offset-4"
          >
            {c.label}
          </RadioGroupPrimitive.Item>
        ))}
      </RadioGroupPrimitive.Root>
    </div>
  );
}
