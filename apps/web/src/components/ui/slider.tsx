import * as React from "react";
import { Slider as SliderPrimitive } from "radix-ui";
import { cn } from "@/lib/utils";

interface SliderProps extends Omit<React.ComponentProps<typeof SliderPrimitive.Root>, "aria-label" | "aria-labelledby"> {
  /** The id of the element that names the slider. */
  labelledBy?: string;
  label?: string;
  /** The value as words, such as "24 points", for screen readers. */
  valueText?: string;
}

export function Slider({ className, labelledBy, label, valueText, ...props }: SliderProps) {
  return (
    <SliderPrimitive.Root
      data-slot="slider"
      className={cn("relative flex h-11 w-full touch-none items-center select-none", className)}
      {...props}
    >
      <SliderPrimitive.Track className="relative h-2 w-full grow overflow-hidden rounded-full border border-fg bg-surface">
        <SliderPrimitive.Range className="absolute h-full bg-fg" />
      </SliderPrimitive.Track>
      {/* the thumb is the focusable slider, so it carries the name and the spoken value */}
      <SliderPrimitive.Thumb
        aria-label={label}
        aria-labelledby={labelledBy}
        aria-valuetext={valueText}
        className="block size-7 rounded-full border-[3px] border-fg bg-bg"
      />
    </SliderPrimitive.Root>
  );
}
