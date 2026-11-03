/** The project's website. A plain link, so it needs nothing in the Content Security Policy. */
export const SITE = "https://zacess.com";

/** The source repository on GitHub. */
export const REPO = "https://github.com/zacesshq/zacess";

/** The year the app was first published, which starts the copyright range. */
export const COPYRIGHT_FROM = 2026;

/** The footer links, in the order the website's own footer uses. Every one leaves the app for the website or GitHub. */
export const footerLinks: [string, string][] = [
  ["Docs", `${SITE}/docs/`],
  ["Privacy", `${SITE}/privacy/`],
  ["Accessibility", `${SITE}/accessibility/`],
  ["Security", `${SITE}/security/`],
  ["Contact", `${SITE}/contact/`],
  ["Source code", REPO],
  ["Report a problem", `${REPO}/issues/new/choose`],
];

/** "2026" in the first year, then "2026 to 2027" and so on, matching the website's footer. */
export function copyrightYears(from: number, now: number = new Date().getFullYear()): string {
  return now > from ? `${from} to ${now}` : `${from}`;
}
