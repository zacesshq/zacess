import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { copyrightYears } from "@/lib/site";
import { axeViolations, openFixture } from "./helpers";
import { installSpeech, removeSpeech } from "./speech-mock";

describe("axe-core", () => {
  it("finds no WCAG 2.2 A or AA violations on the first screen", async () => {
    render(<App />);
    expect(await axeViolations()).toEqual([]);
  });

  it("finds none with a document open, read aloud available and the ruler on", async () => {
    installSpeech();
    try {
      const user = userEvent.setup();
      render(<App />);
      await openFixture(user, "lab.html");
      await user.click(screen.getByRole("radio", { name: "Low Vision" }));
      expect(await axeViolations()).toEqual([]);
    } finally {
      removeSpeech();
    }
  });

  it("finds none in either warm theme or the Gentle Warm preset with a document open", async () => {
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "lab.html");
    await user.click(screen.getByRole("radio", { name: "Gentle Warm" }));
    expect(await axeViolations()).toEqual([]);
    const themes = screen.getByRole("radiogroup", { name: "Colour theme" });
    for (const name of [/Warm Dark/, /Warm Light/]) {
      await user.click(within(themes).getByRole("radio", { name }));
      expect(await axeViolations()).toEqual([]);
    }
  });

  it("finds none in the dark theme after an error", async () => {
    const user = userEvent.setup({ applyAccept: false });
    render(<App />);
    await user.click(screen.getByRole("button", { name: /Colour mode/ }));
    await user.click(await screen.findByRole("menuitemradio", { name: "Dark" }));
    await user.upload(screen.getByTestId("file-input"), new File(["x"], "slides.key"));
    await screen.findByRole("alert");
    expect(await axeViolations()).toEqual([]);
  });

  it("has one h1, a banner, a main landmark and a footer, with links to the website", () => {
    render(<App />);
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    expect(screen.getByRole("banner")).toBeInTheDocument();
    expect(screen.getByRole("main")).toBeInTheDocument();
    expect(screen.getByRole("contentinfo")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Zacess home" })).toHaveAttribute("href", "https://zacess.com");
    expect(screen.getByRole("link", { name: "About Zacess" })).toHaveAttribute("href", "https://zacess.com");
  });

  it("has a footer with the copyright line, the licence and links to the website and GitHub, in order", () => {
    render(<App />);
    const footer = screen.getByRole("contentinfo");
    expect(footer).toHaveTextContent(/Copyright 2026( to \d{4})? Zacess\. Zacess is free software under the GNU Affero General Public License v3\.0\./);
    expect(within(footer).getByRole("link", { name: "GNU Affero General Public License v3.0" })).toHaveAttribute(
      "href",
      "https://github.com/zacesshq/zacess/blob/main/LICENSE",
    );
    const nav = within(footer).getByRole("navigation", { name: "Footer" });
    expect(within(nav).getAllByRole("link").map((a) => [a.textContent, a.getAttribute("href")])).toEqual([
      ["Docs", "https://zacess.com/docs/"],
      ["Privacy", "https://zacess.com/privacy/"],
      ["Accessibility", "https://zacess.com/accessibility/"],
      ["Security", "https://zacess.com/security/"],
      ["Contact", "https://zacess.com/contact/"],
      ["Source code", "https://github.com/zacesshq/zacess"],
      ["Report a problem", "https://github.com/zacesshq/zacess/issues/new/choose"],
    ]);
    for (const link of within(footer).getAllByRole("link")) {
      // every footer link leaves the app, so none passes the app's address on
      expect(link).toHaveAttribute("rel", "noreferrer");
      expect(link.getAttribute("href")).toMatch(/^https:\/\/(zacess\.com|github\.com\/zacesshq\/zacess)(\/|$)/);
    }
    // jsdom has no layout, so the 44 pixel box is asserted by its classes here and measured by scripts/check-browser.mjs
    for (const link of within(nav).getAllByRole("link")) expect(link).toHaveClass("min-h-11", "min-w-11");
  });

  it("reaches every footer link in order with the keyboard", async () => {
    const user = userEvent.setup();
    render(<App />);
    const footer = screen.getByRole("contentinfo");
    within(footer).getByRole("link", { name: "GNU Affero General Public License v3.0" }).focus();
    for (const link of within(within(footer).getByRole("navigation", { name: "Footer" })).getAllByRole("link")) {
      await user.tab();
      expect(link).toHaveFocus();
    }
  });

  it("writes the copyright year as one year or as a range", () => {
    expect(copyrightYears(2026, 2026)).toBe("2026");
    expect(copyrightYears(2026, 2027)).toBe("2026 to 2027");
    expect(copyrightYears(2026)).toMatch(/^2026( to \d{4})?$/);
  });

  it("states the privacy promise once, in the footer", () => {
    render(<App />);
    const text = document.body.textContent ?? "";
    expect(screen.getByRole("contentinfo")).toHaveTextContent("Your files and settings stay in this browser: nothing is uploaded");
    for (const repeat of [/never leave/i, /nothing is uploaded/i, /on this device/i, /keep everything here/i]) {
      expect(text.match(new RegExp(repeat.source, "gi"))?.length ?? 0).toBeLessThanOrEqual(1);
    }
    expect(text.match(/never leave/gi)).toBeNull();
    expect(text.match(/on this device/gi)).toBeNull();
  });

  it("finds none with the theme menu open and with the panel hidden", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getByRole("button", { name: /Colour mode/ }));
    await screen.findByRole("menu");
    expect(await axeViolations()).toEqual([]);
    await user.keyboard("{Escape}");
    await openFixture(user, "notes.md");
    await user.click(screen.getByRole("button", { name: "Hide panel" }));
    expect(await axeViolations()).toEqual([]);
  });
});
