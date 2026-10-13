import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { openFixture } from "./helpers";

describe("the side panel", () => {
  it("has no toggle until a document is open", () => {
    render(<App />);
    expect(screen.queryByRole("button", { name: "Hide panel" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Choose a file" })).toBeVisible();
  });

  it("hides and shows the controls, keeping the focus on the toggle", async () => {
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "notes.md");
    const toggle = screen.getByRole("button", { name: "Hide panel" });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    for (const id of toggle.getAttribute("aria-controls")!.split(" ")) expect(document.getElementById(id)).not.toBeNull();

    await user.click(toggle);
    const show = screen.getByRole("button", { name: "Show panel" });
    expect(show).toHaveAttribute("aria-expanded", "false");
    expect(show).toHaveFocus();
    expect(screen.queryByRole("button", { name: "Choose a file" })).not.toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Reading settings" })).not.toBeInTheDocument();
    expect(screen.getByRole("region", { name: /Document/ })).toBeInTheDocument();
    expect(window.localStorage.getItem("zacess.panel")).toBe("false");

    await user.click(show);
    expect(screen.getByRole("button", { name: "Hide panel" })).toHaveFocus();
    expect(screen.getByRole("button", { name: "Choose a file" })).toBeInTheDocument();
    expect(window.localStorage.getItem("zacess.panel")).toBe("true");
  });

  it("remembers the choice for the next document", async () => {
    window.localStorage.setItem("zacess.panel", "false");
    const user = userEvent.setup();
    render(<App />);
    // with nothing open, the way to open a document is always shown
    expect(screen.getByRole("button", { name: "Choose a file" })).toBeInTheDocument();
    await openFixture(user, "notes.md");
    expect(screen.getByRole("button", { name: "Show panel" })).toHaveAttribute("aria-expanded", "false");
    // the status of what was opened stays visible with the panel hidden
    expect(screen.getByText("Opened notes.md.")).toBeVisible();
  });
});
