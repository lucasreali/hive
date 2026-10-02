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
