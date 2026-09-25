import { expect, type Locator, test } from "@playwright/test";

/** Expands the folders `names` of the files tree, in order (they start collapsed). */
async function open(files: Locator, ...names: string[]) {
  for (const name of names) {
    await files.getByRole("treeitem", { name, exact: true }).click();
  }
}

test("files panel: shows the selected worktree's changes; Ctrl+Shift+B toggles it", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "refactor-auth" }).click();

  // The side panel starts open (4.21).
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await expect(panel).toBeVisible();
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Diff" }).click();
  await expect(panel.locator(".files-summary")).toHaveText("5 files changed+24−49");
  const files = panel.getByRole("tree", { name: "Files" });
  // Only the changed files; folders start collapsed.
  await expect(files.getByRole("treeitem")).toHaveCount(3);
  await expect(files.getByRole("treeitem", { name: "README.md" })).toHaveCount(0);

  await open(files, "src", "auth");
  await files.getByRole("treeitem", { name: /token\.ts/ }).click();
  // The file opens in its tab, in place of the terminal.
  const view = page.getByRole("region", { name: "src/auth/token.ts" });
  await expect(view).toBeVisible();
  await expect(page.getByRole("tab", { name: "token.ts" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await page.screenshot({ path: "target/e2e/files-panel.png" });

  // Selecting another worktree asks the service again; the file's tab goes with its worktree.
  await tree.getByRole("button", { name: "fix-login" }).click();
  await expect(panel.locator(".files-summary")).toHaveText("2 files changed+4−1");
  await expect(view).toBeHidden();
  const tab = page.getByRole("tab", { name: "token.ts" });
  await expect(tab).toHaveCount(0);
  await tree.getByRole("button", { name: "refactor-auth" }).click();
  await expect(tab).toBeVisible();
  await tree.getByRole("button", { name: "fix-login" }).click();

  await page.keyboard.press("Control+Shift+B");
  await expect(panel).toBeHidden();

  // Files: every file of the worktree, unchanged ones without a letter.
  await page.keyboard.press("Control+Shift+B");
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Files" }).click();
  const all = panel.getByRole("region", { name: "Files" });
  const readme = all.getByRole("treeitem", { name: "README.md" });
  await expect(readme).toBeVisible();
  await expect(readme.locator(".status-letter")).toHaveCount(0);
});

test("files panel: a changed file shows as a read-only unified diff", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  const files = panel.getByRole("tree", { name: "Files" });

  await open(files, "src", "auth");
  await files.getByRole("treeitem", { name: /session\.ts/ }).click();
  const view = page.getByRole("region", { name: "src/auth/session.ts" });
  await expect(view.locator(".cm-editor")).toBeVisible();
  // The removed line above the added ones, long unchanged stretches collapsed.
  await expect(view.locator(".cm-deletedChunk").nth(1)).toHaveText(/const ttl = SESSION_TTL;/);
  await expect(view.locator(".cm-changedLine").nth(1)).toHaveText(/opts\.rememberMe/);
  await expect(view.locator(".cm-collapsedLines").first()).toBeVisible();
  // Highlighted as TypeScript once its language loads.
  await expect(view.locator(".cm-line span").first()).toBeVisible();
  await page.screenshot({ path: "target/e2e/file-diff.png" });
  // Read-only: typing changes nothing.
  await view.locator(".cm-line").first().click();
  await page.keyboard.type("zzz");
  await expect(view.locator(".cm-content")).not.toContainText("zzz");

  // A deleted file shows all removed, a new one all added, a binary one a message.
  await tree.getByRole("button", { name: "refactor-auth" }).click();
  await open(files, "src", "legacy");
  await files.getByRole("treeitem", { name: /jwt\.ts/ }).click();
  const deleted = page.getByRole("region", { name: "src/legacy/jwt.ts" });
  await expect(deleted.locator(".cm-deletedChunk")).toHaveText(/export const value = 1;/);
  await open(files, "assets");
  await files.getByRole("treeitem", { name: /logo\.png/ }).click();
  await expect(page.getByRole("region", { name: "assets/logo.png" })).toContainText(
    "Binary file not shown.",
  );
  await tree.getByRole("button", { name: "feat-checkout" }).click();
  await open(files, "src", "checkout");
  await files.getByRole("treeitem", { name: /shipping\.ts/ }).click();
  const added = page.getByRole("region", { name: "src/checkout/shipping.ts" });
  await expect(added.locator(".cm-changedLine")).toHaveCount(2);
  // Nothing was removed: the chunk's removed part is empty.
  await expect(added.locator(".cm-deletedChunk")).toHaveText("");
});
