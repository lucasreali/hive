import { expect, test } from "@playwright/test";

test("worktree health: badges with tooltips, and removing merged worktrees", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const row = (name: string) => tree.locator(".tree-row.worktree", { hasText: name });

  // The fake service's statuses: shop's fix-login is ahead, behind and changed.
  const badges = row("fix-login").locator(".health > span");
  await expect(badges).toHaveText(["↑3", "↓1", "●2"]);
  await expect(badges.first()).toHaveAttribute(
    "title",
    "3 commits not on the main worktree's branch",
  );
  // Merged, but with changes: not one to remove.
  await expect(row("refactor-auth").locator(".health > span")).toHaveText(["↓2", "●6", "merged"]);

  // A new worktree has nothing of its own yet, so it counts as merged and clean.
  await tree.getByRole("button", { name: "api", exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: /^New worktree…/ }).click();
  const create = page.getByRole("dialog", { name: "New worktree" });
  await create.getByLabel("Worktree name").fill("done");
  await create.getByLabel("Open a terminal in the new worktree").uncheck();
  await create.getByRole("button", { name: "Create worktree Enter" }).click();
  await expect(row("done").locator(".health > span")).toHaveText(["merged"]);

  await tree.getByRole("button", { name: "api", exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Remove merged worktrees…" }).click();
  const dialog = page.getByRole("dialog", { name: "Remove merged worktrees" });
  await expect(dialog.getByRole("checkbox")).toHaveCount(1);
  await expect(dialog.getByRole("checkbox", { name: /^done/ })).toBeChecked();
  await dialog.getByRole("button", { name: /^Remove \(1\)/ }).click();
  await expect(dialog.getByRole("status")).toHaveText("Removed");
  await expect(row("done")).toHaveCount(0);
  await dialog.getByRole("button", { name: "Close Esc" }).click();
  await expect(dialog).toHaveCount(0);
  // Nothing else went.
  await expect(row("refactor-auth")).toBeVisible();
  await expect(row("fix-login")).toBeVisible();
});
