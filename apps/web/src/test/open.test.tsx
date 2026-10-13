import { describe, expect, it } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { openFixture, reading } from "./helpers";

describe("opening documents", () => {
  it("opens a Markdown file into headings, paragraphs and a list", async () => {
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "notes.md", "text/markdown");
    const view = reading();
    // the app's lockup is the only h1, so the document's headings start at h2
    expect(within(view).getByRole("heading", { level: 2, name: "Fourier series" })).toBeInTheDocument();
    expect(within(view).getByRole("heading", { level: 3, name: "Three things to remember" })).toBeInTheDocument();
    expect(within(view).getAllByRole("listitem")).toHaveLength(3);
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    expect(screen.getByRole("region", { name: "Document: Fourier series" })).toBeInTheDocument();
  });

  it("opens a web page, keeping the table and the image description and dropping the menu", async () => {
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "lab.html", "text/html");
    const view = reading();
    expect(within(view).getByRole("table", { name: "Measured gain" })).toBeInTheDocument();
    expect(within(view).getByRole("region", { name: "Table: Measured gain" })).toHaveAttribute("tabindex", "0");
    expect(view).toHaveTextContent("Oscilloscope trace of the output");
    expect(view).not.toHaveTextContent("Home");
    expect(view).not.toHaveTextContent("Cookie settings");
  });

  it("opens a Word file through mammoth and the core's HTML reader", async () => {
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "week3.docx");
    const view = reading();
    expect(within(view).getByRole("heading", { level: 2, name: "Week 3 signals" })).toBeInTheDocument();
    expect(within(view).getByRole("heading", { level: 3, name: "Worked example" })).toBeInTheDocument();
    expect(within(view).getByRole("table")).toHaveTextContent("Square wave");
  });

  it("reads pasted text", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.type(screen.getByLabelText("Paste text or a copied web page"), "First line of the handout.");
    await user.click(screen.getByRole("button", { name: "Read pasted text" }));
    expect(reading()).toHaveTextContent("First line of the handout.");
    expect(screen.getByText("Opened the pasted text.")).toBeInTheDocument();
  });

  it("explains a file it cannot open", async () => {
    const user = userEvent.setup({ applyAccept: false });
    render(<App />);
    await user.upload(screen.getByTestId("file-input"), new File(["x"], "slides.key"));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("slides.key could not be opened");
    expect(alert).toHaveTextContent("Choose a PDF");
  });

  it("announces progress politely while opening", async () => {
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "notes.md");
    const status = screen.getByText(/Opened notes\.md/).closest("[aria-live]");
    expect(status).toHaveAttribute("aria-live", "polite");
    await waitFor(() => expect(document.activeElement?.id).toBe("document"));
  });
});

describe("opening one file after another", () => {
  it("shows the file opened last even when an earlier one finishes later", async () => {
    const user = userEvent.setup();
    render(<App />);
    let release: (() => void) | undefined;
    const slow = new File(["# Slow file"], "slow.md", { type: "text/markdown" });
    // the first file's text arrives only after the second file has opened
    Object.defineProperty(slow, "text", {
      value: () => new Promise<string>((done) => (release = () => done("# Slow file"))),
    });
    const input = screen.getByTestId("file-input");
    await user.upload(input, slow);
    await openFixture(user, "notes.md");
    release?.();
    await new Promise((done) => setTimeout(done, 20));
    expect(screen.getByRole("region", { name: "Document: Fourier series" })).toBeInTheDocument();
    expect(screen.getByText("Opened notes.md.")).toBeInTheDocument();
  });
});
