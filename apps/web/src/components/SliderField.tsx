import { useId } from "react";
import { Slider } from "@/components/ui/slider";

interface Props {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  /** The value as shown, such as "24 pt". */
  shown: string;
  /** The value as read out, such as "24 points". */
  spoken?: string;
  onChange: (value: number) => void;
  hint?: string;
}

export function SliderField({ label, value, min, max, step, shown, spoken, onChange, hint }: Props) {
  const id = useId();
  return (
    <div className="grid gap-1">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3">
        <span id={`${id}-label`} className="font-bold">
          {label}
        </span>
        <span aria-hidden="true" className="tabular-nums">
          {shown}
        </span>
      </div>
      {hint ? (
        <p id={`${id}-hint`} className="text-sm text-muted">
          {hint}
        </p>
      ) : null}
      <Slider
        labelledBy={`${id}-label`}
        valueText={spoken ?? shown}
        value={[value]}
        min={min}
        max={max}
        step={step}
        onValueChange={([next]) => {
          if (typeof next === "number" && Number.isFinite(next)) onChange(next);
        }}
      />
    </div>
  );
}
