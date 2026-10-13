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

export function Slider({ className, labelledBy, label, valueText, onPointerDown, ...props }: SliderProps) {
  return (
    <SliderPrimitive.Root
      data-slot="slider"
      // on a touch screen the track lets the page scroll and only the thumb drags, so swiping down
      // the settings never changes a value by accident; a mouse can still press anywhere on the track
      className={cn("relative flex h-11 w-full touch-pan-y items-center select-none", className)}
      onPointerDown={(event) => {
        onPointerDown?.(event);
        const target = event.target instanceof Element ? event.target : null;
        if (event.pointerType === "touch" && !target?.closest('[role="slider"]')) event.preventDefault();
      }}
      {...props}
    >
      <SliderPrimitive.Track className="relative h-2 w-full grow overflow-hidden rounded-full border border-fg bg-surface">
        <SliderPrimitive.Range className="absolute h-full bg-fg" />
      </SliderPrimitive.Track>
      {/* the thumb is the focusable slider, so it carries the name and the spoken value */}
      {/* the thumb is a 44 pixel target for a finger, drawn as a smaller ring inside it */}
      <SliderPrimitive.Thumb
        aria-label={label}
        aria-labelledby={labelledBy}
        aria-valuetext={valueText}
        className="flex size-11 touch-none items-center justify-center rounded-full"
      >
        <span aria-hidden="true" className="pointer-events-none block size-7 rounded-full border-[3px] border-fg bg-bg" />
      </SliderPrimitive.Thumb>
    </SliderPrimitive.Root>
  );
}
