import { expect, test } from "@playwright/test";

test("markdown: the eye and Ctrl+Shift+V show a Markdown file rendered; Ctrl+S shows the text (11.2)", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("region", { name: "Files" });

  // Not a Markdown file: no eye.
  await panel.getByRole("treeitem", { name: "package.json" }).click();
  const pkg = page.getByRole("region", { name: "package.json" });
  await expect(pkg.locator(".cm-content")).toBeVisible();
  await expect(pkg.getByRole("button", { name: "Show rendered Markdown" })).toHaveCount(0);

  await panel.getByRole("treeitem", { name: "README.md" }).click();
  const view = page.getByRole("region", { name: "README.md" });
  const eye = view.getByRole("button", { name: "Show rendered Markdown" });
  await expect(eye).toHaveAttribute("aria-pressed", "false");
  await expect(eye).toHaveAttribute("title", "Show rendered (Ctrl+Shift+V)");
  await view.locator(".cm-line").first().click();
  await page.keyboard.press("Control+Home");
  await page.keyboard.type("# Hello\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n");

  // The unsaved text, rendered.
  await eye.click();
  await expect(eye).toHaveAttribute("aria-pressed", "true");
  const rendered = view.locator(".markdown-view");
  await expect(rendered.getByRole("heading", { name: "Hello" })).toBeVisible();
  await expect(rendered.getByRole("cell", { name: "2" })).toBeVisible();
  await expect(view.locator(".cm-editor")).toHaveCount(0);
  await page.screenshot({ path: "target/e2e/markdown.png" });

  await page.keyboard.press("Control+Shift+V");
  await expect(eye).toHaveAttribute("aria-pressed", "false");
  await expect(view.locator(".cm-content")).toContainText("# Hello");
  await page.keyboard.press("Control+Shift+V");
  await expect(rendered).toBeVisible();

  // Ctrl+S saves and shows the text again.
  await page.keyboard.press("Control+s");
  await expect(eye).toHaveAttribute("aria-pressed", "false");
  await expect(view.getByRole("button", { name: "Save" })).toBeDisabled();
  await expect(view.locator(".cm-content")).toContainText("# Hello");
});

// 14.6: the preview's document style, compared with its reference in each theme. A fixed text
// with every kind of block, so only a change of style changes the picture.
const DOCUMENT = `# Hive document

A paragraph with **bold**, *emphasis*, ~~struck~~, a [link](https://example.com) and \`inline code\`.

## Lists

- [x] A task that is done
- [ ] A task to do
  - a nested item

1. First
2. Second

> A quote: Hive only *observes* agents.

### A table

| Key | Value | Count |
|---|---|--:|
| COVERAGE_EXCLUSIONS.md | Approved coverage exclusions, created when the first one is approved | 1 |
| \`docs/hive.md\` | Single source of truth for every decision | 22 |

#### Code

\`\`\`ts
// Coloured by the editor's parsers.
export function safeUrl(url: string): string {
  return /^https?:/i.test(url) ? url : "";
}
\`\`\`

\`\`\`
plain text, no language
\`\`\`

---

![A screenshot](shot.png)
`;

test("markdown: the preview's document style in light and dark (14.6)", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page
    .getByRole("region", { name: "Files" })
    .getByRole("treeitem", { name: "README.md" })
    .click();
  const view = page.getByRole("region", { name: "README.md" });
  await view.locator(".cm-line").first().click();
  await page.keyboard.press("Control+a");
  await page.keyboard.insertText(DOCUMENT);
  await view.getByRole("button", { name: "Show rendered Markdown" }).click();
  const rendered = view.locator(".markdown-view");
  await expect(rendered.locator(".tok-keyword").first()).toHaveText("export");

  for (const theme of ["dark", "light"]) {
    await page.evaluate((light) => {
      if (light) document.documentElement.dataset.theme = "one-light";
      else delete document.documentElement.dataset.theme;
    }, theme === "light");
    await expect(rendered).toHaveScreenshot(`markdown-document-${theme}.png`);
  }
});

// 13.1: the rendered Markdown never scrolls sideways; a wide table, a long inline path and a long
// code line wrap, in the editor area beside the right panel at its narrowest and its widest.
for (const panelWidth of [280, 640]) {
  test(`markdown: nothing in the preview scrolls sideways beside a ${panelWidth} px panel (13.1)`, async ({
    page,
  }) => {
    await page.addInitScript(
      (w) => localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: w })),
      panelWidth,
    );
    await page.goto("/");
    const tree = page.getByRole("navigation", { name: "Projects" });
    await tree.getByRole("button", { name: "fix-login" }).click();
    const panel = page.getByRole("region", { name: "Files" });
    await panel.getByRole("treeitem", { name: "docs" }).click();
    await panel.getByRole("treeitem", { name: "api.md" }).click();
    const view = page.getByRole("region", { name: "docs/api.md" });
    await view.getByRole("button", { name: "Show rendered Markdown" }).click();
    const rendered = view.locator(".markdown-view");
    await expect(rendered.getByRole("table")).toBeVisible();
    await expect(rendered.locator("p code")).toBeVisible();
    await expect(rendered.locator("pre")).toBeVisible();

    const fit = await rendered.evaluate((root) => {
      const right = root.getBoundingClientRect().right;
      return [root, ...root.querySelectorAll(".md-table, table, p, pre")].map((el) => ({
        el: el.tagName,
        fits: el.scrollWidth <= el.clientWidth && el.getBoundingClientRect().right <= right,
      }));
    });
    expect(fit).toEqual(["DIV", "DIV", "TABLE", "P", "PRE"].map((el) => ({ el, fits: true })));
    await page.screenshot({ path: `target/e2e/markdown-wrap-${panelWidth}.png` });
  });
}
