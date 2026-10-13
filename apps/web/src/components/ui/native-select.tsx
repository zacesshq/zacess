import * as React from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * The platform's own select, which every screen reader and switch device already knows. Its own
 * drawing is turned off because Safari ignores a minimum height on a drawn select, which left it 27
 * pixels tall on an iPhone; the list itself is still the platform's picker.
 */
export function NativeSelect({ className, ...props }: React.ComponentProps<"select">) {
  return (
    <div className="relative min-w-0">
      <select
        data-slot="native-select"
        className={cn(
          "min-h-11 w-full appearance-none truncate rounded-md border-2 border-border bg-bg py-2 pr-10 pl-3 text-base text-fg",
          className,
        )}
        {...props}
      />
      <ChevronDown aria-hidden="true" className="pointer-events-none absolute top-1/2 right-3 size-5 -translate-y-1/2 text-fg" />
    </div>
  );
}
