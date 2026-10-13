import { Download } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Section } from "./Section";

export function ExportPanel({
  disabled,
  onExport,
  message,
}: {
  disabled: boolean;
  onExport: (format: "html" | "epub") => void;
  message: string;
}) {
  return (
    <Section title="Export">
      <p className="text-sm text-muted">
        Saves the document in your reading settings, with its headings, lists, tables and image descriptions kept.
      </p>
      <div className="flex flex-wrap gap-2">
        <Button onClick={() => onExport("html")} disabled={disabled}>
          <Download aria-hidden="true" />
          Download HTML
        </Button>
        <Button onClick={() => onExport("epub")} disabled={disabled}>
          <Download aria-hidden="true" />
          Download EPUB
        </Button>
      </div>
      <p role="status" className="text-sm">
        {message}
      </p>
    </Section>
  );
}
