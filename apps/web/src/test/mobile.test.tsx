import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { ACCEPT, CAMERA_ACCEPT } from "@/open";
import { limitScale, MAX_CANVAS_PIXELS } from "@/open/canvas";
import { openFixture } from "./helpers";
import { installSpeech, LOCAL, removeSpeech } from "./speech-mock";

/** Makes the page report a touch screen, as a phone or tablet does. */
function touchScreen(coarse: boolean) {
  vi.stubGlobal(
    "matchMedia",
    vi.fn((query: string) => ({
      matches: query === "(pointer: coarse)" ? coarse : false,
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
}

describe("phones and tablets", () => {
  it("offers PDFs, Word files, images, web pages, Markdown and text by type as well as by extension", () => {
    for (const type of [
      ".pdf",
      "application/pdf",
      ".docx",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      ".html",
      "text/html",
      ".md",
      "text/markdown",
      ".txt",
      "text/plain",
      "image/*",
    ]) {
      expect(ACCEPT.split(",")).toContain(type);
    }
    render(<App />);
    expect(screen.getByTestId("file-input")).toHaveAttribute("accept", ACCEPT);
  });

  it("offers Take a photo on a touch screen, opening the rear camera for an image", async () => {
    touchScreen(true);
    try {
      const user = userEvent.setup();
      render(<App />);
      const camera = screen.getByTestId("camera-input");
      expect(camera).toHaveAttribute("accept", CAMERA_ACCEPT);
      expect(CAMERA_ACCEPT).toBe("image/*");
      expect(camera).toHaveAttribute("capture", "environment");
      const click = vi.spyOn(camera, "click");
      await user.click(screen.getByRole("button", { name: "Take a photo" }));
      expect(click).toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("leaves Take a photo out with a mouse", () => {
    touchScreen(false);
    try {
      render(<App />);
      expect(screen.queryByRole("button", { name: "Take a photo" })).not.toBeInTheDocument();
      expect(screen.queryByTestId("camera-input")).not.toBeInTheDocument();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("keeps a large photo under the canvas size Safari on iPhone accepts", () => {
    // a 24 megapixel photo from a recent iPhone
    const scale = limitScale(5712, 4284, 1);
    expect(5712 * scale * 4284 * scale).toBeLessThanOrEqual(MAX_CANVAS_PIXELS + 1);
    expect(MAX_CANVAS_PIXELS).toBeLessThan(16_777_216);
    // a normal page keeps the scale it asked for
    expect(limitScale(595, 842, 2)).toBe(2);
  });

  it("opens the export instead when the browser cannot save a download", async () => {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLAnchorElement.prototype, "download")!;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    delete (HTMLAnchorElement.prototype as any).download;
    vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:zacess-test");
    vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    try {
      const user = userEvent.setup();
      render(<App />);
      await openFixture(user, "notes.md");
      await user.click(screen.getByRole("button", { name: "Download HTML" }));
      expect(open).toHaveBeenCalledWith("blob:zacess-test", "_blank", "noopener");
    } finally {
      Object.defineProperty(HTMLAnchorElement.prototype, "download", descriptor);
    }
  });

  it("fills the voice list when the voices arrive late without an event, as some WebKit builds do", async () => {
    const { synth } = installSpeech([]);
    try {
      render(<App />);
      expect(screen.getByText("No voices have loaded yet.")).toBeInTheDocument();
      synth.getVoices = () => [LOCAL];
      await waitFor(() => expect(screen.getByRole("combobox", { name: "Voice" })).toBeInTheDocument(), { timeout: 2000 });
    } finally {
      removeSpeech();
    }
  });

  it("speaks the first sentence within the tap on Play, which iPhone and iPad require", async () => {
    const { synth, spoken } = installSpeech();
    try {
      const user = userEvent.setup();
      render(<App />);
      await openFixture(user, "notes.md");
      // opening a document stops any reading, which is a cancel of its own
      synth.cancel.mockClear();
      await user.click(screen.getByRole("button", { name: "Play" }));
      // spoken synchronously in the click, with no cancel first when nothing was speaking
      expect(spoken).toHaveLength(1);
      expect(synth.cancel).not.toHaveBeenCalled();
    } finally {
      removeSpeech();
    }
  });
});
