import * as React from "react";
import { cn } from "@/lib/utils";

/** The platform's own select, which every screen reader and switch device already knows. */
export function NativeSelect({ className, ...props }: React.ComponentProps<"select">) {
  return (
    <select
      data-slot="native-select"
      className={cn(
        "min-h-11 w-full rounded-md border-2 border-border bg-bg px-3 py-2 text-base text-fg",
        className,
      )}
      {...props}
    />
  );
}
