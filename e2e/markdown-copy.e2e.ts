import { expect, type Page, test } from "@playwright/test";
import { mod } from "./host";

// 23.8: a code block of a file's preview has a copy button, shown on hover; the scroll-sync of
// 21.3 still lands on the same block with the button's box around each code block.

test.beforeEach(async ({ context, browserName }) => {
  // WebKit knows only clipboard-read; it lets a page write the clipboard without a permission.
  await context.grantPermissions(
    browserName === "webkit" ? ["clipboard-read"] : ["clipboard-read", "clipboard-write"],
  );
});

/** README.md of fix-login with `text` in place of its own, shown rendered. */
async function rendered(page: Page, text: string) {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page
    .getByRole("region", { name: "Files" })
    .getByRole("treeitem", { name: "README.md" })
    .dblclick();
  const view = page.getByRole("region", { name: "README.md" });
  await view.locator(".cm-line").first().click();
  await page.keyboard.press(`${mod}+a`);
  await page.keyboard.insertText(text);
  const eye = view.getByRole("button", { name: "Show rendered Markdown" });
  await eye.click();
  return { view, eye, preview: view.locator(".markdown-view") };
}

test("a code block's copy button shows on hover and copies the block's text (23.8)", async ({
  page,
}) => {
  const code = 'export function f() {\n  return "a";\n}';
  const { preview } = await rendered(page, `# Copy\n\nText.\n\n\`\`\`ts\n${code}\n\`\`\`\n`);
  const box = preview.locator(".md-code");
  const button = box.getByRole("button", { name: "Copy code" });
  const opacity = () => button.evaluate((el) => getComputedStyle(el).opacity);
  // Hidden until the pointer is over the block.
  await preview.getByRole("heading", { name: "Copy" }).hover();
  await expect.poll(opacity).toBe("0");
  await box.hover();
  await expect.poll(opacity).toBe("1");
  // At the block's top right.
  const [b, p] = [await button.boundingBox(), await box.boundingBox()];
  if (!b || !p) throw new Error("no box");
  expect(p.x + p.width - (b.x + b.width)).toBeLessThan(12);
  expect(b.y - p.y).toBeLessThan(12);

  await button.click();
  await expect(button).toHaveAttribute("data-copied", "true");
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(code);
  // The check stays visible with the pointer gone, then the copy icon is back.
  await preview.getByRole("heading", { name: "Copy" }).hover();
  await expect.poll(opacity).toBe("1");
  await expect(button).not.toHaveAttribute("data-copied", { timeout: 4000 });
});

// Sections with a code block each, so blocks with a copy box are between every heading.
const LONG = Array.from(
  { length: 60 },
  (_, k) =>
    `## Section ${k}\n\nThe text of section ${k}.\n\n\`\`\`\ncode ${k}\nmore ${k}\n\`\`\`\n\n`,
).join("");

/** The text of the first top-level block at the top of the rendered view. */
const topBlock = (page: Page) =>
  page.evaluate(() => {
    const view = document.querySelector(".file-view .markdown-view");
    if (!view) return null;
    const top = view.getBoundingClientRect().top;
    const blocks = [...view.querySelectorAll(".markdown > [data-line]")];
    return blocks.find((b) => b.getBoundingClientRect().bottom > top + 4)?.textContent ?? null;
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

test("the text and rendered views still show the same code block (21.3, 23.8)", async ({
  page,
}) => {
  const { view, eye, preview } = await rendered(page, LONG);
  await expect(preview.getByRole("heading", { name: "Section 0" })).toBeVisible();
  // A code block of section 30 at the top of the rendered view.
  await page.evaluate(() => {
    const scroller = document.querySelector(".file-view .markdown-view") as HTMLElement;
    const block = [...scroller.querySelectorAll(".md-code")].find((b) =>
      b.textContent?.startsWith("code 30"),
    ) as HTMLElement;
    scroller.scrollTop += block.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
  });
  await expect.poll(() => topBlock(page)).toBe("code 30\nmore 30");
  // The text: the block's opening fence on top; rendered again, the same block.
  await eye.click();
  await expect(view.locator(".cm-editor")).toBeVisible();
  await expect.poll(() => topLine(page)).toBe("```");
  const below = await page.evaluate(() => {
    const scroller = document.querySelector(".file-view .cm-scroller") as HTMLElement;
    const top = scroller.getBoundingClientRect().top;
    const lines = [...scroller.querySelectorAll(".cm-line")];
    const at = lines.findIndex((l) => l.getBoundingClientRect().bottom > top + 4);
    return lines[at + 1]?.textContent;
  });
  expect(below).toBe("code 30");
  await eye.click();
  await expect.poll(() => topBlock(page)).toBe("code 30\nmore 30");
});
