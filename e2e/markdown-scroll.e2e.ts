import { expect, type Page, test } from "@playwright/test";
import { mod } from "./host";

// 21.3: the rendered Markdown keeps its scroll, and the text and rendered show the same place.

// 120 sections: a heading, a paragraph and a list, 8 lines each.
const LONG = Array.from(
  { length: 120 },
  (_, k) => `## Section ${k}\n\nThe text of section ${k}.\n\n- one\n- two\n\n`,
).join("");

/** The text of the first heading at the top of the rendered view (or under it, by 4 px). */
const topHeading = (page: Page) =>
  page.evaluate(() => {
    const view = document.querySelector(".file-view .markdown-view");
    if (!view) return null;
    const top = view.getBoundingClientRect().top;
    const headings = [...view.querySelectorAll("h2")];
    return headings.find((h) => h.getBoundingClientRect().bottom > top + 4)?.textContent ?? null;
  });

/** The text of the first line at the top of the text view. */
const topLine = (page: Page) =>
  page.evaluate(() => {
    const scroller = document.querySelector(".file-view .cm-scroller");
    if (!scroller) return null;
    const top = scroller.getBoundingClientRect().top;
    const lines = [...scroller.querySelectorAll(".cm-line")];
    return lines.find((l) => l.getBoundingClientRect().bottom > top + 4)?.textContent ?? null;
  });

/** Waits for a smooth scroll (WebKit's wheel) to stop, and gives what `read` reads then. */
async function settled(read: () => Promise<string | null>): Promise<string | null> {
  let seen: string | null = null;
  await expect
    .poll(async () => {
      const before = seen;
      seen = await read();
      return seen !== null && seen === before;
    })
    .toBe(true);
  return seen;
}

test("the rendered Markdown keeps its scroll, and switching to the text keeps the place (21.3)", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  // A terminal in the worktree, to leave the file by.
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await page
    .getByRole("region", { name: "Files" })
    .getByRole("treeitem", { name: "README.md" })
    .dblclick();
  const view = page.getByRole("region", { name: "README.md" });
  await view.locator(".cm-line").first().click();
  await page.keyboard.press(`${mod}+a`);
  await page.keyboard.insertText(LONG);
  const eye = view.getByRole("button", { name: "Show rendered Markdown" });
  await eye.click();
  const rendered = view.locator(".markdown-view");
  await expect(rendered.getByRole("heading", { name: "Section 0" })).toBeVisible();

  // Far down, by the wheel.
  await rendered.hover();
  await page.mouse.wheel(0, 2000);
  await settled(() => topHeading(page));
  // Lands on a heading exactly, as the human scrolled to it.
  await page.evaluate(() => {
    const view = document.querySelector(".file-view .markdown-view") as HTMLElement;
    const top = view.getBoundingClientRect().top;
    const heading = [...view.querySelectorAll("h2")].find(
      (h) => h.getBoundingClientRect().top >= top,
    ) as HTMLElement;
    view.scrollTop += heading.getBoundingClientRect().top - top;
  });
  const left = await settled(() => topHeading(page));
  expect(left).toMatch(/Section ([1-9]\d)/);

  // A terminal tab, and back: the same place.
  await page.getByRole("tab", { name: "fix-login" }).click();
  await page.getByRole("tab", { name: "README.md" }).click();
  await expect.poll(() => topHeading(page)).toBe(left);

  // The text: the same section on top, then rendered again: the same heading.
  await eye.click();
  await expect.poll(() => topLine(page)).toBe(`## ${left}`);
  await eye.click();
  await expect.poll(() => topHeading(page)).toBe(left);

  // Scrolled in the text, then rendered: the heading on the text's top, and back.
  await eye.click();
  await view.locator(".cm-scroller").hover();
  await page.mouse.wheel(0, 1500);
  await settled(() => topLine(page));
  // Lands the text on a heading's line exactly, as the human scrolled to it.
  await page.evaluate(() => {
    const scroller = document.querySelector(".file-view .cm-scroller") as HTMLElement;
    const top = scroller.getBoundingClientRect().top;
    const heading = [...scroller.querySelectorAll(".cm-line")].find(
      (l) => l.getBoundingClientRect().top >= top && l.textContent?.startsWith("## "),
    ) as HTMLElement;
    scroller.scrollTop += heading.getBoundingClientRect().top - top;
  });
  const heading = await settled(() => topLine(page));
  expect(heading).toMatch(/^## Section \d+$/);
  await eye.click();
  await expect.poll(() => topHeading(page)).toBe(heading?.slice(3));
  await eye.click();
  await expect.poll(() => topLine(page)).toBe(heading);
});
