import { useId, type ReactNode } from "react";
import { cn } from "@/lib/utils";

/** A titled part of the controls, as a labelled region with its own heading. */
export function Section({ title, children, className }: { title: string; children: ReactNode; className?: string }) {
  const id = useId();
  return (
    <section aria-labelledby={id} className={cn("grid min-w-0 grid-cols-[minmax(0,1fr)] gap-4 border-t-2 border-fg pt-4", className)}>
      <h2 id={id} className="text-xl font-bold">
        {title}
      </h2>
      {children}
    </section>
  );
}
