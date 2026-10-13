import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { availableVoices, pickVoice, sentencesOf } from "@/lib/speech";
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
