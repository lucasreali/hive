import { expect, test } from "@playwright/test";

test("create a worktree from a project row: name check, branch filter, tree update", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const row = tree.locator(".tree-row.project", { hasText: "shop" });
  // The row's affordance shows on hover, as in the prototype.
  await expect(row.getByTitle("New worktree (Ctrl+Shift+N)")).toBeHidden();
  await row.hover();
  await row.getByTitle("New worktree (Ctrl+Shift+N)").click();

  const dialog = page.getByRole("dialog", { name: "New worktree" });
  const name = dialog.getByLabel("Worktree name");
  await expect(name).toBeFocused();
  await expect(dialog.getByRole("button", { name: "main default" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  // The service's (here the mock's) verdict, worded as the CLI says it.
  await name.fill("Fix Cart");
  await expect(dialog.getByRole("alert")).toContainText('invalid worktree name "Fix Cart"');
  await expect(dialog.getByRole("button", { name: "Create worktree Enter" })).toBeDisabled();
  await page.screenshot({ path: "target/e2e/new-worktree-error.png" });
  await name.fill("fix-login");
  await expect(dialog.getByRole("alert")).toContainText('worktree "fix-login" already exists');

  await name.fill("fix-cart");
  await expect(dialog.getByRole("alert")).toHaveCount(0);
  const filter = dialog.getByLabel("Base branch");
  await filter.fill("release");
  await expect(dialog.locator(".branch-name")).toHaveText(["origin/release/2.4"]);
  await filter.fill("dependency-2");
  await filter.press("ArrowDown");
  await expect(dialog.locator(".plan")).toContainText(
    "Branch: worktree-fix-cart (from origin/renovate/dependency-20)",
  );
  await expect(dialog.locator(".plan")).toContainText("Folder: .claude/worktrees/fix-cart/");
  await page.screenshot({ path: "target/e2e/new-worktree.png" });

  await name.press("Enter");
  await expect(dialog).toHaveCount(0);
  const created = tree.getByRole("button", { name: "fix-cart", exact: true });
  await expect(created).toBeVisible();
  await expect(created).toHaveAttribute("aria-current", "true");
  // Its terminal opened as a shown tab, with claude started in it (the dialog's default):
  // typed at once, so the mock echoes it before its first prompt.
  await expect(page.getByRole("tab", { name: "fix-cart" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const url = "/src/terminals.ts";
        const { terminal } = await import(/* @vite-ignore */ url);
        return terminal(1).buffer.active.getLine(0).translateToString(true);
      }),
    )
    .toBe("claude");
  await expect(tree.locator(".tree-row.agent")).toHaveCount(1);
  await page.screenshot({ path: "target/e2e/new-worktree-created.png" });
});

test("the long remote branch list scrolls with the arrow keys", async ({ page }) => {
  await page.goto("/");
  const row = page.locator(".tree-row.project", { hasText: "shop" });
  await row.hover();
  await row.getByTitle("New worktree (Ctrl+Shift+N)").click();
  const filter = page.getByLabel("Base branch");
  await filter.fill("renovate");
  for (let i = 0; i < 40; i++) await filter.press("ArrowDown");
  const pressed = page.locator(".branch-row[aria-pressed=true]");
  await expect(pressed).toHaveText("origin/renovate/dependency-41");
  await expect(pressed).toBeInViewport();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
});
