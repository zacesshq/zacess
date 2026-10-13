import { useId, useRef, useState, type ClipboardEvent, type DragEvent } from "react";
import { ClipboardPaste, FileUp } from "lucide-react";
import { ACCEPT } from "@/open";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";
import { Section } from "./Section";

interface Props {
  busy: boolean;
  onFile: (file: File) => void;
  onPaste: (text: string, html: string | null) => void;
}

export function OpenPanel({ busy, onFile, onPaste }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);
  const [text, setText] = useState("");
  // the HTML of a copied web page, kept while the box still holds exactly what was pasted
  const [html, setHtml] = useState<{ html: string; text: string } | null>(null);
  const pasteId = useId();
  const pasteHint = useId();
  const dropHint = useId();

  const drop = (event: DragEvent) => {
    event.preventDefault();
    setDragging(false);
    if (busy) return;
    const file = event.dataTransfer.files?.[0];
    if (file) onFile(file);
  };

  const pasted = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const copied = event.clipboardData.getData("text/html");
    const plain = event.clipboardData.getData("text/plain");
    const box = event.currentTarget;
    const empty = box.value.trim() === "" || (box.selectionStart === 0 && box.selectionEnd === box.value.length);
    setHtml(copied && empty ? { html: copied, text: plain } : null);
  };

  const keptHtml = html && html.text.trim() === text.trim() ? html.html : null;

  return (
    <Section title="Open a document">
      <div
        onDragEnter={(e) => {
          e.preventDefault();
          setDragging(true);
        }}
        onDragOver={(e) => e.preventDefault()}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
        }}
        onDrop={drop}
        className={cn(
          "grid justify-items-start gap-3 rounded-md border-[3px] border-dashed border-fg p-4",
          dragging && "border-solid bg-surface",
        )}
      >
        <p id={dropHint}>
          {dragging ? (
            <strong>Drop the file to open it.</strong>
          ) : (
            <>
              Drop a file here or choose one. PDF, Word (.docx), images, web pages (.html), text and Markdown.
            </>
          )}
        </p>
        <Button variant="primary" onClick={() => input.current?.click()} disabled={busy} aria-describedby={dropHint}>
          <FileUp aria-hidden="true" />
          Choose a file
        </Button>
        <input
          ref={input}
          type="file"
          accept={ACCEPT}
          hidden
          data-testid="file-input"
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (file) onFile(file);
            // the same file can be chosen again after a change of settings
            e.target.value = "";
          }}
        />
      </div>

      <div className="grid gap-2">
        <Label htmlFor={pasteId}>Paste text or a copied web page</Label>
        <p id={pasteHint} className="text-sm text-muted">
          Copied web pages keep their headings, lists and tables.
        </p>
        <textarea
          id={pasteId}
          aria-describedby={pasteHint}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onPaste={pasted}
          rows={4}
          className="w-full rounded-md border-2 border-border bg-bg p-3 text-base text-fg"
        />
        <div>
          <Button onClick={() => onPaste(text, keptHtml)} disabled={busy || !text.trim()}>
            <ClipboardPaste aria-hidden="true" />
            Read pasted text
          </Button>
        </div>
      </div>
    </Section>
  );
}
