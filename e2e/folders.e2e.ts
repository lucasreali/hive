import { expect, test } from "@playwright/test";
import { keyLabel } from "./host";

test("a plain folder is a folder project: its files, no diff, then git init makes it a project", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const rows = () => tree.locator(".tree-row").allTextContents();

  // Added as any project: the fake service follows a folder in no repository as a folder project.
  await page.getByTitle(keyLabel("Add project (Ctrl+Shift+O)")).click();
  const dialog = page.getByRole("dialog", { name: "Add project" });
  const field = dialog.getByLabel("Folder", { exact: true });
  await field.fill("/home/user/projects/notes");
  await expect(dialog.getByRole("button", { name: /^Add project/ })).toBeEnabled();
  await field.press("Enter");
  await expect(dialog).toHaveCount(0);
  const row = tree.locator(".tree-row.folder");
  await expect(row).toHaveText("notes");
  // No worktree rows under it.
  expect((await rows()).at(-1)).toBe("notes");

  // Its panel: Files and Sessions only, with its files.
  await tree.getByRole("button", { name: "notes", exact: true }).click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  const tabs = panel.getByRole("tablist", { name: "Panel" }).getByRole("tab");
  await expect
    .poll(() => tabs.evaluateAll((all) => all.map((t) => t.getAttribute("aria-label"))))
    .toEqual(["Files", "Sessions"]);
  await expect(panel.getByRole("treeitem", { name: "todo.md" })).toBeVisible();
  await page.screenshot({ path: "target/e2e/folder-project.png" });

  // A terminal in it, and its agent under its row.
  await page.getByRole("button", { name: "New terminal", exact: true }).click();
  await expect(page.getByRole("tablist").getByRole("tab", { name: "notes" })).toBeVisible();
  await page.keyboard.type("claude");
  await page.keyboard.press("Enter");
  await expect(tree.locator(".tree-row.agent")).toHaveCount(1);

  // "Initialize git" in its menu: a project now, with its main worktree and every panel view.
  await tree.getByRole("button", { name: "notes", exact: true }).click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Project" });
  await expect(menu.getByRole("menuitem")).toHaveText(["Initialize git", "Remove project…"]);
  await menu.getByRole("menuitem", { name: "Initialize git" }).click();
  await expect(row).toHaveCount(0);
  await expect(tree.locator(".tree-row.project", { hasText: /^notes/ })).toBeVisible();
  await expect(tree.locator(".tree-row.worktree").last()).toHaveText(/^main/);
  await tree
    .getByRole("button", { name: /^notes/ })
    .first()
    .click();
  await expect(tabs).toHaveCount(6);
});
