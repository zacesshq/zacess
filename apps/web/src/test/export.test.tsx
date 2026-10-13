import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { openFixture } from "./helpers";

function captureDownloads() {
  const blobs: Blob[] = [];
  const names: string[] = [];
  vi.spyOn(URL, "createObjectURL").mockImplementation((blob) => {
    blobs.push(blob as Blob);
    return "blob:zacess-test";
  });
  vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) {
    names.push(this.download);
  });
  return { blobs, names };
}

describe("export", () => {
  it("is unavailable until a document is open", () => {
    render(<App />);
    expect(screen.getByRole("button", { name: "Download HTML" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Download EPUB" })).toBeDisabled();
  });

  it("saves accessible HTML in the current profile", async () => {
    const user = userEvent.setup();
    const { blobs, names } = captureDownloads();
    render(<App />);
    await openFixture(user, "notes.md");
    await user.click(screen.getByRole("radio", { name: "Low Vision" }));
    await user.click(screen.getByRole("button", { name: "Download HTML" }));
    expect(names).toEqual(["fourier-series.html"]);
    expect(blobs[0].type).toBe("text/html");
    const html = await blobs[0].text();
    expect(html).toMatch(/^<!DOCTYPE html>/);
    expect(html).toContain('<html lang="en-GB">');
    expect(html).toContain("<h1");
    expect(html).toContain("#ffeb3b");
    expect(screen.getByText("Saved fourier-series.html to your downloads.")).toBeInTheDocument();
  });

  it("saves an EPUB 3 book", async () => {
    const user = userEvent.setup();
    const { blobs, names } = captureDownloads();
    render(<App />);
    await openFixture(user, "notes.md");
    await user.click(screen.getByRole("button", { name: "Download EPUB" }));
    expect(names).toEqual(["fourier-series.epub"]);
    expect(blobs[0].type).toBe("application/epub+zip");
    const bytes = new Uint8Array(await blobs[0].arrayBuffer());
    expect(String.fromCharCode(...bytes.slice(0, 2))).toBe("PK");
    expect(String.fromCharCode(...bytes.slice(30, 38))).toBe("mimetype");
  });
});
