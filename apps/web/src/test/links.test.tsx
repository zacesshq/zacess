import { afterEach, describe, expect, it } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { isWebLink, shownAddress } from "@/lib/links";
import { sentencesOf, speakLink } from "@/lib/speech";
import { axeViolations, openFixture, reading } from "./helpers";
import { installSpeech, removeSpeech } from "./speech-mock";

afterEach(() => removeSpeech());

async function paste(user: ReturnType<typeof userEvent.setup>, text: string) {
  await user.type(screen.getByLabelText("Paste text or a copied web page"), text);
  await user.click(screen.getByRole("button", { name: "Read pasted text" }));
}

describe("links", () => {
  it("accepts only web and mail addresses", () => {
    expect(isWebLink("https://example.org")).toBe(true);
    expect(isWebLink("mailto:a@b.c")).toBe(true);
    for (const bad of ["javascript:alert(1)", "data:text/html,x", "ftp://example.org", "#top", "  ", "https://"]) {
      expect(isWebLink(bad)).toBe(false);
    }
    expect(shownAddress("https://example.org/week-3/")).toBe("example.org/week-3");
    expect(shownAddress("mailto:tutor@example.org")).toBe("tutor@example.org");
  });

  it("links only the address in pasted text, not the full stop after it", async () => {
    const user = userEvent.setup();
    render(<App />);
    await paste(user, "Notes are at https://example.org/week-3. Bring them.");
    const link = within(reading()).getByRole("link");
    expect(link).toHaveAttribute("href", "https://example.org/week-3");
    // jsdom joins inline text without the space a browser keeps
    expect(link).toHaveAccessibleName(/^https:\/\/example\.org\/week-3 ?\(opens in a new tab\)$/);
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
    expect(link.querySelector("svg")).toHaveAttribute("aria-hidden", "true");
    expect(link.nextSibling?.textContent?.startsWith(".")).toBe(true);
  });

  it("keeps web and mail links and turns every other address into plain text", async () => {
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "links.html", "text/html");
    const view = reading();
    const links = within(view).getAllByRole("link");
    expect(links.map((l) => l.getAttribute("href"))).toEqual(["https://example.org/fourier", "mailto:tutor@example.org"]);
    expect(view).toHaveTextContent("Never run this script, open this data, use this file server or jump to the top.");
    expect(view.innerHTML).not.toMatch(/javascript:|data:text|ftp:/);
  });

  it("lists every link in a disclosure and moves focus to the one chosen", async () => {
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "links.html", "text/html");
    const toggle = screen.getByRole("button", { name: "Links in this document (2)" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("list", { name: "Links in this document" })).toBeNull();
    toggle.focus();
    await user.keyboard("{Enter}");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    const list = screen.getByRole("list", { name: "Links in this document" });
    const items = within(list).getAllByRole("button");
    expect(items.map((b) => b.textContent)).toEqual(["the Fourier notesexample.org/fourier", "your tutortutor@example.org"]);
    await user.click(items[1]);
    expect(within(reading()).getByRole("link", { name: /your tutor/ })).toHaveFocus();
  });

  it("shows the full address while a link has the focus and hides it on Escape", async () => {
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "links.html", "text/html");
    within(reading()).getByRole("link", { name: /Fourier notes/ }).focus();
    await waitFor(() => expect(document.querySelector(".zc-link-tip")).toHaveTextContent("example.org/fourier"));
    await user.keyboard("{Escape}");
    expect(document.querySelector(".zc-link-tip")).toBeNull();
  });

  it("reads a link as its text and a cue, never the address unless asked", () => {
    const root = document.createElement("article");
    root.innerHTML =
      '<p>Start with <a href="https://example.org/fourier">the Fourier notes</a>. See <a href="https://example.org/x">https://example.org/x</a>.</p>';
    document.body.append(root);
    expect(sentencesOf(root, "en").map((s) => s.spoken)).toEqual([
      "Start with the Fourier notes (link).",
      "See a web link.",
    ]);
    expect(sentencesOf(root, "en", { readAddresses: true }).map((s) => s.spoken)).toEqual([
      "Start with the Fourier notes (link to example.org/fourier).",
      "See example.org/x (link).",
    ]);
    root.remove();
    expect(speakLink("your tutor", "mailto:t@e.org", { readAddresses: true })).toBe("your tutor (email link to t@e.org)");
  });

  it("speaks pasted links with the cue and offers the address setting off by default", async () => {
    const { spoken } = installSpeech();
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "links.html", "text/html");
    const setting = screen.getByRole("switch", { name: "Read web addresses aloud" });
    expect(setting).not.toBeChecked();
    await user.click(screen.getByRole("button", { name: "Play" }));
    await waitFor(() => expect(spoken.length).toBeGreaterThan(0));
    spoken[0].onend?.();
    await waitFor(() => expect(spoken.length).toBe(2));
    expect(spoken[1].text).toBe("Start with the Fourier notes (link) and email your tutor (link).");
    expect(spoken.map((u) => u.text).join(" ")).not.toContain("example.org");
  });

  it("finds no axe violations with links and the links list open", async () => {
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "links.html", "text/html");
    await user.click(screen.getByRole("button", { name: "Links in this document (2)" }));
    within(reading()).getByRole("link", { name: /Fourier notes/ }).focus();
    expect(await axeViolations()).toEqual([]);
  });
});
