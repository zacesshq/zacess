import * as React from "react";
import { Slot } from "radix-ui";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex min-h-11 items-center justify-center gap-2 rounded-md border-2 px-4 py-2 text-base font-bold whitespace-nowrap select-none disabled:cursor-not-allowed disabled:opacity-60 [&_svg]:size-5 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        // the highlight is always paired with ink, never shown alone on white
        primary:
          "border-fg bg-fg text-bg hover:border-fg hover:bg-highlight hover:text-ink",
        outline: "border-fg bg-bg text-fg hover:bg-surface",
        ghost: "border-transparent bg-transparent text-fg hover:border-fg",
      },
      size: {
        default: "",
        sm: "min-h-10 px-3 text-sm",
        icon: "size-11 px-0",
      },
    },
    defaultVariants: { variant: "outline", size: "default" },
  },
);

export interface ButtonProps
  extends React.ComponentProps<"button">,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

export function Button({ className, variant, size, asChild = false, type, ...props }: ButtonProps) {
  const Component = asChild ? Slot.Root : "button";
  return (
    <Component
      data-slot="button"
      type={asChild ? undefined : (type ?? "button")}
      className={cn(buttonVariants({ variant, size }), className)}
      {...props}
    />
  );
}

export { buttonVariants };
