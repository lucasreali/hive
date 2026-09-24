import { expect, test } from "@playwright/test";

test("files panel: Ctrl+Shift+B shows the selected worktree's changes", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "refactor-auth" }).click();

  await page.keyboard.press("Control+Shift+B");
  const panel = page.getByRole("complementary", { name: "Files and diff" });
  await expect(panel).toBeVisible();
  await expect(panel.locator(".files-summary")).toHaveText("5 files changed+24−49");
  const files = panel.getByRole("tree", { name: "Files" });
  // "All": every file of the watched worktree, unchanged ones without a letter.
  const readme = files.getByRole("treeitem", { name: "README.md" });
  await expect(readme).toBeVisible();
  await expect(readme.locator(".status-letter")).toHaveCount(0);
  await panel.getByRole("button", { name: "Changed" }).click();
  await expect(files.getByRole("treeitem")).toHaveCount(10);
  await expect(readme).toBeHidden();

  await files.getByRole("treeitem", { name: /token\.ts/ }).click();
  const view = panel.getByRole("region", { name: "src/auth/token.ts" });
  await expect(view).toBeVisible();
  await page.screenshot({ path: "target/e2e/files-panel.png" });

  // Selecting another worktree asks the service again.
  await tree.getByRole("button", { name: "fix-login" }).click();
  await expect(panel.locator(".files-summary")).toHaveText("2 files changed+4−1");
  await expect(view).toBeHidden();

  await page.keyboard.press("Control+Shift+B");
  await expect(panel).toBeHidden();
});
