import { expect, test } from "@playwright/test";

test("add a project from the empty state and see its worktrees", async ({ page }) => {
  await page.goto("/?mock=empty");
  const terminals = page.getByRole("region", { name: "Terminals" });
  await expect(terminals).toContainText("No project open");
  await expect(page.getByRole("navigation", { name: "Projects" })).toContainText("No projects");
  await page.screenshot({ path: "target/e2e/empty-state.png" });

  await terminals.getByRole("button", { name: "Add project" }).click();
  const dialog = page.getByRole("dialog", { name: "Add project" });
  const field = dialog.getByLabel("Folder in WSL");
  await expect(field).toBeFocused();
  await field.fill("/home/user/nowhere");
  await field.press("Enter");
  await expect(dialog.getByRole("alert")).toContainText("cannot open /home/user/nowhere");
  await page.screenshot({ path: "target/e2e/add-project-error.png" });

  await field.fill("/home/user/projects/shop");
  await field.press("Enter");
  await expect(dialog).toHaveCount(0);
  await expect(terminals).not.toContainText("No project open");
  const tree = page.getByRole("navigation", { name: "Projects" });
  for (const name of ["shop", "main", "fix-login", "feat-checkout"]) {
    await expect(tree.getByRole("button", { name, exact: true })).toBeVisible();
  }
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.screenshot({ path: "target/e2e/project-tree.png" });

  await tree.getByRole("button", { name: "Collapse shop" }).click();
  await expect(tree.getByRole("button", { name: "fix-login" })).toHaveCount(0);
});

test("Esc closes the add-project dialog", async ({ page }) => {
  await page.goto("/");
  await page.getByTitle("Add project (Ctrl+Shift+O)").click();
  const dialog = page.getByRole("dialog", { name: "Add project" });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});
