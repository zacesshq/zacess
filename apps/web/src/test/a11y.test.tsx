import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
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
