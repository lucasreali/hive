import { expect, test } from "@playwright/test";

test("worktree health: badges with tooltips, and removing merged worktrees", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const row = (name: string) => tree.locator(".tree-row.worktree", { hasText: name });

  // The fake service's statuses: shop's fix-login is ahead, behind and changed.
  const badges = row("fix-login").locator(".health > span");
  await expect(badges).toHaveText(["↑3", "↓1", "+4−1"]);
  await expect(badges.first()).toHaveAttribute(
    "title",
    "3 commits not on the main worktree's branch",
  );
  // Its changes as lines, the file count in the tooltip.
  await expect(badges.nth(2)).toHaveAttribute("title", "2 changed files, +4 −1 lines");
  // Merged, but with changes: not one to remove.
  await expect(row("refactor-auth").locator(".health > span")).toHaveText([
    "↓2",
    "+24−49",
    "merged",
  ]);

  // A new worktree has nothing of its own yet, so it counts as merged and clean.
  await tree.getByRole("button", { name: "api", exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: /^New worktree…/ }).click();
  const create = page.getByRole("dialog", { name: "New worktree" });
  await create.getByLabel("Worktree name").fill("done");
  await create.getByLabel("Open a terminal in the new worktree").uncheck();
  await create.getByRole("button", { name: "Create worktree" }).click();
  await expect(row("done").locator(".health > span")).toHaveText(["merged"]);

  await tree.getByRole("button", { name: "api", exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Remove merged worktrees…" }).click();
  const dialog = page.getByRole("dialog", { name: "Remove merged worktrees" });
  await expect(dialog.getByRole("checkbox")).toHaveCount(1);
  await expect(dialog.getByRole("checkbox", { name: /^done/ })).toBeChecked();
  await dialog.getByRole("button", { name: /^Remove \(1\)/ }).click();
  await expect(dialog.getByRole("status")).toHaveText("Removed");
  await expect(row("done")).toHaveCount(0);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  // Nothing else went.
  await expect(row("refactor-auth")).toBeVisible();
  await expect(row("fix-login")).toBeVisible();
});

test("a failed removal's long message sits under its row's name (24.9)", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  // A terminal stays in the new worktree, so the fake service refuses to remove it.
  await tree.getByRole("button", { name: "api", exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: /^New worktree…/ }).click();
  const create = page.getByRole("dialog", { name: "New worktree" });
  await create.getByLabel("Worktree name").fill("a-worktree-with-a-rather-long-name");
  await expect(create.getByLabel("Open a terminal in the new worktree")).toBeChecked();
  await create.getByRole("button", { name: "Create worktree" }).click();
  await expect(create).toHaveCount(0);

  await tree.getByRole("button", { name: "api", exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Remove merged worktrees…" }).click();
  const dialog = page.getByRole("dialog", { name: "Remove merged worktrees" });
  await dialog.getByRole("button", { name: /^Remove \(1\)/ }).click();
  const error = dialog.locator(".field-error");
  await expect(error).toContainText("close its terminals first");

  const item = dialog.locator(".merged-item");
  const name = await item.locator(".label").boundingBox();
  const date = await item.locator("label .field-help").boundingBox();
  const below = await error.boundingBox();
  if (!name || !date || !below) throw new Error("the row is not laid out");
  // The error starts below the name and the date, never over them.
  expect(below.y).toBeGreaterThanOrEqual(Math.max(name.y + name.height, date.y + date.height));
});
