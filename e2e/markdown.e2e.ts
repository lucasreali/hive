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
