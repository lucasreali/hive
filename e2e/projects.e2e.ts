import { expect, test } from "@playwright/test";

test("add a project from the empty state and see its worktrees", async ({ page }) => {
  await page.goto("/?mock=empty");
  const terminals = page.getByRole("region", { name: "Terminals" });
  await expect(terminals).toContainText("No project open");
  await expect(page.getByRole("navigation", { name: "Projects" })).toContainText("No projects");
  await page.screenshot({ path: "target/e2e/empty-state.png" });

  await terminals.getByRole("button", { name: "Add project" }).click();
  const dialog = page.getByRole("dialog", { name: "Add project" });
  const field = dialog.getByLabel("Folder", { exact: true });
  const add = dialog.getByRole("button", { name: /^Add project/ });
  await expect(field).toBeFocused();
  // The field starts at the home folder, its subfolders listed under it; a click enters one.
  await expect(field).toHaveValue("/home/user/");
  const folders = dialog.getByRole("list", { name: "Folders" });
  await folders.getByText("projects", { exact: true }).click();
  await expect(field).toHaveValue("/home/user/projects/");
  await expect(folders.getByRole("button", { name: "Repository shop" })).toBeVisible();

  // Enter adds once the typed folder is listed.
  await field.fill("/home/user/nowhere");
  await expect(add).toBeEnabled();
  await field.press("Enter");
  await expect(dialog.getByRole("alert")).toContainText("cannot open /home/user/nowhere");
  await page.screenshot({ path: "target/e2e/add-project-error.png" });

  await field.fill("/home/user/projects/shop");
  await expect(add).toBeEnabled();
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

test("the folder kind select opens the app's own list over the dialog", async ({ page }) => {
  await page.goto("/");
  await page.getByTitle("Add project (Ctrl+Shift+O)").click();
  const dialog = page.getByRole("dialog", { name: "Add project" });
  const kind = dialog.getByRole("combobox", { name: "Folder kind" });
  await expect(kind).toHaveText("WSL");
  await expect(kind).toHaveCSS("cursor", "pointer");

  await kind.click();
  const list = page.getByRole("listbox", { name: "Folder kind" });
  const windows = list.getByRole("option", { name: "Windows" });
  await expect(windows).toHaveCSS("cursor", "pointer");
  // Under the trigger, and on top: nothing of the dialog covers or clips it.
  const trigger = await kind.boundingBox();
  const box = await list.boundingBox();
  expect(box?.y).toBeGreaterThanOrEqual((trigger?.y ?? 0) + (trigger?.height ?? 0));
  const hit = await windows.evaluate((el) => {
    const r = el.getBoundingClientRect();
    return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
  });
  expect(hit).toBe(true);
  await page.screenshot({ path: "target/e2e/add-project-select.png" });

  // Esc closes the list and leaves the dialog open, the focus on the trigger.
  await page.keyboard.press("Escape");
  await expect(list).toHaveCount(0);
  await expect(dialog).toBeVisible();
  await expect(kind).toBeFocused();

  // The keyboard picks too: the first ↑ opens the list on the value, the next one moves.
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Enter");
  await expect(kind).toHaveText("Windows");
  await expect(dialog.getByLabel("Folder", { exact: true })).toHaveValue("C:\\Users\\user\\");

  // A click picks, and one outside closes the list.
  await kind.click();
  await list.getByRole("option", { name: "WSL" }).click();
  await expect(kind).toHaveText("WSL");
  await kind.click();
  await dialog.getByRole("heading", { name: "Add project" }).click();
  await expect(list).toHaveCount(0);
  await expect(dialog).toBeVisible();
});
