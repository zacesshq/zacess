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
    await user.click(screen.getByRole("radio", { name: "Dark" }));
    await user.upload(screen.getByTestId("file-input"), new File(["x"], "slides.key"));
    await screen.findByRole("alert");
    expect(await axeViolations()).toEqual([]);
  });

  it("has one h1, a banner, a main landmark and a footer", () => {
    render(<App />);
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    expect(screen.getByRole("banner")).toBeInTheDocument();
    expect(screen.getByRole("main")).toBeInTheDocument();
    expect(screen.getByRole("contentinfo")).toBeInTheDocument();
    expect(screen.getByText("Files never leave your device.")).toBeInTheDocument();
  });
});
