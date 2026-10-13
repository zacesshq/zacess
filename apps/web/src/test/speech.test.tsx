import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { availableVoices, pickVoice, sentencesOf } from "@/lib/speech";
import { nearestScroll, revealSentence } from "@/lib/scroll";
import { openFixture, reading } from "./helpers";
import { installSpeech, LOCAL, LOCAL_2, ONLINE, removeSpeech } from "./speech-mock";

afterEach(() => removeSpeech());

describe("voices", () => {
  it("offers only on-device voices unless online voices are allowed", () => {
    expect(availableVoices([ONLINE, LOCAL, LOCAL_2], false)).toEqual([LOCAL_2, LOCAL]);
    expect(availableVoices([ONLINE, LOCAL], true).map((v) => v.voiceURI)).toEqual(["local-daniel", "online-english"]);
    expect(availableVoices([ONLINE], false)).toEqual([]);
  });

  it("keeps the saved voice and otherwise matches the language", () => {
    expect(pickVoice([LOCAL_2, LOCAL], "local-karen", "en-GB")).toBe(LOCAL_2);
    expect(pickVoice([LOCAL_2, LOCAL], "gone", "en-GB")).toBe(LOCAL);
    expect(pickVoice([], null, "en-GB")).toBeNull();
  });

  it("lists on-device voices by default and online ones only after the labelled opt-in", async () => {
    installSpeech();
    const user = userEvent.setup();
    render(<App />);
    const select = screen.getByRole("combobox", { name: "Voice" });
    expect(within(select).getAllByRole("option").map((o) => o.textContent)).toEqual(["Karen (en-AU)", "Daniel (en-GB)"]);
    const online = screen.getByRole("switch", { name: "Show online voices" });
    expect(online).not.toBeChecked();
    expect(online).toHaveAccessibleDescription(/sends the text it reads/);
    await user.click(online);
    expect(within(select).getAllByRole("option").map((o) => o.textContent)).toContain("Cloud English (en-GB), online");
  });
});

describe("read aloud", () => {
  it("splits the reading view into sentences without the hidden screen reader prefix", () => {
    const root = document.createElement("article");
    root.innerHTML =
      '<h2>Title</h2><p>One sentence. Two <span class="zc-uncertain"><span class="zc-sr">unclear: </span>kQ</span> here.</p>' +
      '<ul><li>Item<ul><li>Nested</li></ul></li></ul><figure><img alt="A graph"></figure>';
    document.body.append(root);
    expect(sentencesOf(root, "en").map((s) => s.text)).toEqual([
      "Title",
      "One sentence.",
      "Two kQ here.",
      "Item",
      "Nested",
      "Image: A graph",
    ]);
    root.remove();
  });

  it("reads sentence by sentence with the chosen voice and speed and pauses in place", async () => {
    const { synth, spoken } = installSpeech();
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "notes.md");
    screen.getByRole("slider", { name: "Speed" }).focus();
    await user.keyboard("{ArrowRight}{ArrowRight}");
    await user.click(screen.getByRole("button", { name: "Play" }));
    expect(spoken[0].text).toBe("Fourier series");
    expect(spoken[0].voice).toBe(LOCAL);
    expect(spoken[0].rate).toBeCloseTo(1.2);
    expect(reading().querySelector(".zc-reading-block")).toHaveTextContent("Fourier series");
    await act(async () => spoken[0].onend?.());
    expect(spoken[1].text).toBe("Any periodic signal can be written as a sum of sines.");
    expect(screen.getByText("Reading sentence 2 of 7")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Pause" }));
    expect(synth.cancel).toHaveBeenCalled();
    expect(screen.getByText("Paused at sentence 2 of 7")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Resume" }));
    expect(spoken[spoken.length - 1].text).toBe("Any periodic signal can be written as a sum of sines.");

    await user.click(screen.getByRole("button", { name: "Stop" }));
    expect(screen.getByText("Stopped")).toBeInTheDocument();
    expect(reading().querySelector(".zc-reading-block")).toBeNull();
  });

  it("starts from the paragraph the person clicked", async () => {
    const { spoken } = installSpeech();
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "notes.md");
    await user.click(within(reading()).getByText("Harmonics are whole multiples of it."));
    await user.click(screen.getByRole("button", { name: "Play" }));
    expect(spoken[0].text).toBe("Harmonics are whole multiples of it.");
  });

  it("says so when the browser has no speech", () => {
    render(<App />);
    expect(screen.getByText(/no speech voices/)).toBeInTheDocument();
  });
});

describe("read aloud bar and scrolling", () => {
  it("keeps the controls together in one bar above the document", async () => {
    installSpeech();
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "notes.md");
    const bar = screen.getByRole("group", { name: "Read aloud controls" });
    expect(bar.className).toContain("sticky");
    for (const name of ["Play", "Stop", "Previous sentence", "Next sentence"]) {
      expect(within(bar).getByRole("button", { name })).toBeInTheDocument();
    }
    expect(within(bar).getByRole("slider", { name: "Speed" })).toBeInTheDocument();
    // the bar comes before the document in the tab order
    expect(bar.compareDocumentPosition(reading()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // the settings panel no longer repeats them
    expect(screen.getAllByRole("slider", { name: "Speed" })).toHaveLength(1);
  });

  it("keeps the focus on the control that was used and does not scroll while the sentence is in view", async () => {
    installSpeech();
    const scrollBy = vi.spyOn(window, "scrollBy").mockImplementation(() => {});
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "notes.md");
    await user.click(screen.getByRole("button", { name: "Play" }));
    const pause = screen.getByRole("button", { name: "Pause" });
    expect(document.activeElement).toBe(pause);
    await user.click(screen.getByRole("button", { name: "Next sentence" }));
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Next sentence" }));
    expect(scrollBy).not.toHaveBeenCalled();
    scrollBy.mockRestore();
  });

  it("scrolls by the least needed once the sentence has left the viewport", async () => {
    installSpeech();
    const scrollBy = vi.spyOn(window, "scrollBy").mockImplementation(() => {});
    const rect = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      const below = this.classList.contains("zc-reading-block");
      const box = below ? { top: window.innerHeight + 40, bottom: window.innerHeight + 70 } : { top: 0, bottom: 0 };
      return { ...box, left: 0, right: 0, width: 0, height: box.bottom - box.top, x: 0, y: box.top, toJSON() {} } as DOMRect;
    });
    const user = userEvent.setup();
    render(<App />);
    await openFixture(user, "notes.md");
    await user.click(screen.getByRole("button", { name: "Play" }));
    expect(scrollBy).toHaveBeenCalledTimes(1);
    expect(scrollBy).toHaveBeenCalledWith({ top: 70, behavior: "auto" });
    rect.mockRestore();
    scrollBy.mockRestore();
  });
});

describe("revealSentence", () => {
  const view = { top: 0, bottom: 800 };
  it("does nothing while the sentence is inside the viewport", () => {
    const scrollBy = vi.fn();
    revealSentence({ top: 120, bottom: 150 }, { top: 0, bottom: 60 }, 800, scrollBy);
    expect(scrollBy).not.toHaveBeenCalled();
    expect(nearestScroll({ top: 0, bottom: 800 }, view)).toBe(0);
  });
  it("scrolls up or down by the least distance and clears the sticky bar", () => {
    const scrollBy = vi.fn();
    revealSentence({ top: 820, bottom: 850 }, null, 800, scrollBy);
    expect(scrollBy).toHaveBeenLastCalledWith(50);
    revealSentence({ top: 10, bottom: 40 }, { top: 0, bottom: 90 }, 800, scrollBy);
    expect(scrollBy).toHaveBeenLastCalledWith(-80);
  });
  it("aligns the top of a sentence taller than the viewport", () => {
    expect(nearestScroll({ top: 100, bottom: 1200 }, view)).toBe(100);
  });
});
