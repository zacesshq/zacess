import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { defaultProfile } from "@/core";
import { openFixture } from "./helpers";

function viewCss(): string {
  return document.querySelector(".zc-view style")?.textContent ?? "";
}

describe("reading profiles", () => {
  it("applies a preset from the core to the reading view", async () => {
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "notes.md");
    const presets = screen.getByRole("radiogroup", { name: "Reading profile" });
    await user.click(within(presets).getByRole("radio", { name: "Low Vision" }));
    const css = viewCss();
    expect(css).toContain("font-size: 24pt");
    expect(css).toContain("#ffeb3b");
    expect(css).toContain(".zc-view .zc-doc");
    expect(within(screen.getByRole("radiogroup", { name: "Colour theme" })).getByRole("radio", { name: /Yellow text on black/ })).toBeChecked();
    expect(screen.getByRole("switch", { name: "Reading ruler" })).toBeChecked();
  });

  it("changes text size from the keyboard and saves it in this browser", async () => {
    const user = userEvent.setup();
    const { unmount } = render(<App />);
    const size = screen.getByRole("slider", { name: "Text size" });
    expect(size).toHaveAttribute("aria-valuetext", "12 points");
    size.focus();
    await user.keyboard("{ArrowRight}{ArrowRight}{PageUp}");
    expect(size).toHaveAttribute("aria-valuenow", "24");
    expect(viewCss()).toContain("font-size: 24pt");
    expect(screen.getByText(/Adjusted from this profile/)).toBeInTheDocument();
    await user.keyboard("{End}");
    expect(size).toHaveAttribute("aria-valuenow", "96");
    expect(JSON.parse(window.localStorage.getItem("zacess.profile") ?? "{}").font_size_pt).toBe(96);
    unmount();
    render(<App />);
    expect(screen.getByRole("slider", { name: "Text size" })).toHaveAttribute("aria-valuenow", "96");
  });

  it("changes the colour theme, line spacing, letter spacing and column width", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getByRole("radio", { name: /Black text on cream/ }));
    expect(viewCss()).toContain("#fdf6e3");
    screen.getByRole("slider", { name: "Line spacing" }).focus();
    await user.keyboard("{End}");
    expect(viewCss()).toContain("line-height: 3");
    screen.getByRole("slider", { name: "Letter spacing" }).focus();
    await user.keyboard("{ArrowRight}");
    expect(viewCss()).toContain("letter-spacing: 0.01em");
    screen.getByRole("slider", { name: "Column width" }).focus();
    await user.keyboard("{Home}");
    expect(viewCss()).toContain("max-width: 20ch");
  });

  it("offers both high contrast themes and keeps the choice in this browser", async () => {
    const user = userEvent.setup();
    const { unmount } = render(<App />);
    const themes = screen.getByRole("radiogroup", { name: "Colour theme" });
    expect(within(themes).getAllByRole("radio")).toHaveLength(6);
    await user.click(within(themes).getByRole("radio", { name: /High Contrast Dark/ }));
    let css = viewCss();
    expect(css).toContain("background: #000000; color: #ffffff");
    expect(css).toContain("color-scheme: dark");
    expect(css).toContain("#8fd0ff");
    expect(css).toContain("#f5c400");
    await user.click(within(themes).getByRole("radio", { name: /High Contrast Light/ }));
    css = viewCss();
    expect(css).toContain("background: #ffffff; color: #000000");
    expect(css).toContain("color-scheme: light");
    expect(css).toContain("#003d99");
    expect(JSON.parse(window.localStorage.getItem("zacess.profile") ?? "{}").contrast).toBe("high_contrast_light");
    unmount();
    render(<App />);
    expect(within(screen.getByRole("radiogroup", { name: "Colour theme" })).getByRole("radio", { name: /High Contrast Light/ })).toBeChecked();
  });

  it("still loads a profile saved with one of the original four themes", () => {
    window.localStorage.setItem("zacess.profile", JSON.stringify({ ...defaultProfile(), font_size_pt: 20, contrast: "black_on_cream" }));
    render(<App />);
    expect(screen.getByRole("radio", { name: /Black text on cream/ })).toBeChecked();
    expect(screen.getByRole("slider", { name: "Text size" })).toHaveAttribute("aria-valuenow", "20");
    expect(viewCss()).toContain("#fdf6e3");
  });

  it("ignores a damaged saved profile and starts from the standard one", () => {
    window.localStorage.setItem("zacess.profile", JSON.stringify({ font_size_pt: 999 }));
    render(<App />);
    expect(screen.getByRole("slider", { name: "Text size" })).toHaveAttribute("aria-valuenow", "12");
  });

  it("works when storage refuses every read and write", async () => {
    const user = userEvent.setup();
    const get = window.localStorage.getItem;
    const set = window.localStorage.setItem;
    Object.defineProperty(window.localStorage, "getItem", { configurable: true, value: () => { throw new Error("blocked"); } });
    Object.defineProperty(window.localStorage, "setItem", { configurable: true, value: () => { throw new Error("blocked"); } });
    try {
      render(<App />);
      await user.click(screen.getByRole("radio", { name: "Dyslexia-Friendly" }));
      expect(viewCss()).toContain("letter-spacing: 0.12em");
    } finally {
      Object.defineProperty(window.localStorage, "getItem", { configurable: true, value: get });
      Object.defineProperty(window.localStorage, "setItem", { configurable: true, value: set });
    }
  });
});
