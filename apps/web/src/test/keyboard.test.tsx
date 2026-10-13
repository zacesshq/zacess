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

  it("changes the app colours from a menu with the keyboard", async () => {
    const user = userEvent.setup();
    render(<App />);
    const button = screen.getByRole("button", { name: /^Colour mode: ?System$/ });
    expect(button).toHaveAttribute("aria-haspopup", "menu");
    expect(button).toHaveAttribute("aria-expanded", "false");
    button.focus();
    await user.keyboard("{ArrowDown}");
    await screen.findByRole("menu");
    expect(button).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("menuitemradio", { name: "System" })).toHaveAttribute("aria-checked", "true");
    // Escape closes the menu and gives the focus back to the button
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("menu")).not.toBeInTheDocument());
    expect(button).toHaveFocus();

    await user.keyboard("{Enter}");
    await screen.findByRole("menu");
    await waitFor(() => expect(screen.getByRole("menuitemradio", { name: "System" })).toHaveFocus());
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(screen.getByRole("menuitemradio", { name: "Dark" })).toHaveFocus();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(screen.queryByRole("menu")).not.toBeInTheDocument());
    expect(document.documentElement).toHaveAttribute("data-theme", "dark");
    expect(window.localStorage.getItem("zacess.theme")).toBe('"dark"');
    const after = screen.getByRole("button", { name: /^Colour mode: ?Dark$/ });
    await waitFor(() => expect(after).toHaveFocus());
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
