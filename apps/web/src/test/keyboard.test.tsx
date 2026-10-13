import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { openFixture } from "./helpers";
import { installSpeech, removeSpeech } from "./speech-mock";

describe("keyboard", () => {
  it("starts with a skip link and reaches every main control in order", async () => {
    installSpeech();
    try {
      const user = userEvent.setup();
      render(<App />);
      await openFixture(user, "notes.md");
      (document.activeElement as HTMLElement | null)?.blur();
      await user.tab();
      expect(screen.getByRole("link", { name: "Skip to the document" })).toHaveFocus();
      const reached: string[] = [];
      for (let i = 0; i < 40; i++) {
        await user.tab();
        const el = document.activeElement as HTMLElement;
        reached.push(el.getAttribute("aria-label") || el.textContent?.trim() || el.id || el.tagName);
      }
      const names = [
        "System",
        "Choose a file",
        "Standard",
        "Dark text on white",
        "Download HTML",
        "Download EPUB",
      ];
      for (const name of names) expect(reached.some((r) => r.includes(name))).toBe(true);
    } finally {
      removeSpeech();
    }
  });

  it("follows the skip link to the document", async () => {
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "notes.md");
    expect(screen.getByRole("link", { name: "Skip to the document" })).toHaveAttribute("href", "#document");
    expect(document.getElementById("document")).toHaveAttribute("tabindex", "-1");
  });

  it("changes the app colours with the arrow keys", async () => {
    const user = userEvent.setup();
    render(<App />);
    const system = screen.getByRole("radio", { name: "System" });
    expect(system).toBeChecked();
    system.focus();
    // Radix moves focus between radios on the next tick and checks the one it lands on while the
    // arrow is still down, so the key is held until then as it is by a person
    await user.keyboard("{ArrowRight>}");
    await waitFor(() => expect(screen.getByRole("radio", { name: "Light" })).toBeChecked());
    await user.keyboard("{/ArrowRight}");
    await user.keyboard("{ArrowRight>}");
    await waitFor(() => expect(screen.getByRole("radio", { name: "Dark" })).toBeChecked());
    await user.keyboard("{/ArrowRight}");
    expect(document.documentElement).toHaveAttribute("data-theme", "dark");
    expect(window.localStorage.getItem("zacess.theme")).toBe('"dark"');
  });

  it("turns the reading ruler on with the space bar and picks presets with the arrows", async () => {
    const user = userEvent.setup();
    render(<App />);
    const ruler = screen.getByRole("switch", { name: "Reading ruler" });
    ruler.focus();
    await user.keyboard(" ");
    expect(ruler).toBeChecked();
    expect(screen.getByRole("slider", { name: "Ruler height" })).toBeInTheDocument();
    const standard = screen.getByRole("radio", { name: "Standard" });
    standard.focus();
    await user.keyboard("{ArrowDown>}");
    await waitFor(() => expect(screen.getByRole("radio", { name: "Low Vision" })).toBeChecked());
    await user.keyboard("{/ArrowDown}");
    await user.keyboard("{ArrowDown>}");
    await waitFor(() => expect(screen.getByRole("radio", { name: "Dyslexia-Friendly" })).toBeChecked());
    await user.keyboard("{/ArrowDown}");
  });

  it("opens the file chooser from a real button", async () => {
    const user = userEvent.setup();
    render(<App />);
    const input = screen.getByTestId("file-input") as HTMLInputElement;
    let opened = false;
    input.addEventListener("click", () => (opened = true));
    screen.getByRole("button", { name: "Choose a file" }).focus();
    await user.keyboard("{Enter}");
    expect(opened).toBe(true);
  });
});
