import * as React from "react";
import { Switch as SwitchPrimitive } from "radix-ui";
import { cn } from "@/lib/utils";

/** An on and off control. The thumb moves and fills, so the state never rests on colour alone. */
export function Switch({ className, ...props }: React.ComponentProps<typeof SwitchPrimitive.Root>) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      className={cn(
        "peer inline-flex h-8 w-14 shrink-0 items-center rounded-full border-2 border-fg bg-bg data-[state=checked]:bg-fg",
        className,
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb className="pointer-events-none block size-5 translate-x-1 rounded-full border-2 border-fg bg-bg data-[state=checked]:translate-x-7 data-[state=checked]:border-bg data-[state=checked]:bg-highlight" />
    </SwitchPrimitive.Root>
  );
}
