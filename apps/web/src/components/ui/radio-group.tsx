import * as React from "react";
import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import { cn } from "@/lib/utils";

export function RadioGroup({ className, ...props }: React.ComponentProps<typeof RadioGroupPrimitive.Root>) {
  return <RadioGroupPrimitive.Root data-slot="radio-group" className={cn("grid gap-2", className)} {...props} />;
}

/** A radio drawn as a card. The chosen one gets a thick border and a filled dot, not only a colour. */
export function RadioGroupCard({
  className,
  children,
  ...props
}: React.ComponentProps<typeof RadioGroupPrimitive.Item>) {
  return (
    <RadioGroupPrimitive.Item
      data-slot="radio-group-item"
      className={cn(
        "group flex min-h-11 w-full items-center gap-3 rounded-md border-2 border-border bg-bg px-3 py-2 text-left text-base text-fg data-[state=checked]:border-fg data-[state=checked]:shadow-[inset_0_0_0_2px_var(--fg)]",
        className,
      )}
      {...props}
    >
      <span
        aria-hidden="true"
        className="flex size-5 shrink-0 items-center justify-center rounded-full border-2 border-fg"
      >
        <RadioGroupPrimitive.Indicator className="block size-2.5 rounded-full bg-fg" />
      </span>
      {children}
    </RadioGroupPrimitive.Item>
  );
}
