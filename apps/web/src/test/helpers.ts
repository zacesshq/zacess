import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import type { UserEvent } from "@testing-library/user-event";
import axe from "axe-core";

export function fixture(name: string, type = ""): File {
  const bytes = readFileSync(resolve(import.meta.dirname, "fixtures", name));
  return new File([bytes], name, { type });
}

export async function openFixture(user: UserEvent, name: string, type = "") {
  const input = screen.getByTestId("file-input") as HTMLInputElement;
  await user.upload(input, fixture(name, type));
  await waitFor(() => expect(screen.getByText(new RegExp(`Opened ${name.replace(".", "\\.")}`))).toBeInTheDocument());
  // opening moves focus to the document, which has to happen before a test moves it elsewhere
  await waitFor(() => expect(document.activeElement?.id).toBe("document"));
}

/** The reading view's content. */
export function reading(): HTMLElement {
  return document.querySelector(".zc-view") as HTMLElement;
}

/** Runs axe with the WCAG 2.0, 2.1 and 2.2 A and AA rules. jsdom has no layout, so colour contrast is checked in a real browser by scripts/check-browser.mjs. */
export async function axeViolations(root: Element = document.body) {
  const results = await axe.run(root, {
    runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"] },
    rules: { "color-contrast": { enabled: false } },
  });
  return results.violations.map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).join(", ")})`);
}
