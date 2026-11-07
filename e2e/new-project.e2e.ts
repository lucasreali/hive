import { expect, test } from "@playwright/test";
import { keyLabel } from "./host";

test("a new project is made from Hive: a new folder, the project in it, claude in its terminal", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await page.getByTitle(keyLabel("Add project (Ctrl+Shift+O)")).click();
  const dialog = page.getByRole("dialog", { name: "Add project" });
  await dialog.getByRole("button", { name: "New project" }).click();
  const parent = dialog.getByLabel("Parent folder");
  await expect(parent).toHaveValue("/home/user/");

  // "New folder" in the browser: made, then entered.
  await dialog.getByRole("button", { name: "New folder" }).click();
  const folderName = dialog.getByLabel("New folder name");
  await expect(folderName).toBeFocused();
  await folderName.fill("clients");
  await folderName.press("Enter");
  await expect(parent).toHaveValue("/home/user/clients/");
  await expect(dialog).toBeVisible();

  // A taken name is the service's refusal, under the name.
  const name = dialog.getByLabel("Name", { exact: true });
  const create = dialog.getByRole("button", { name: "Create project" });
  await expect(create).toBeDisabled();
  await parent.fill("/home/user/");
  await name.fill("projects");
  await create.click();
  await expect(dialog.getByRole("alert")).toHaveText("/home/user/projects already exists");

  // With git and claude (both on by default): a project, its terminal running claude.
  await parent.fill("/home/user/clients/");
  await name.fill("acme");
  await expect(dialog.getByRole("alert")).toHaveCount(0);
  await expect(dialog.getByRole("checkbox", { name: "Initialize git" })).toBeChecked();
  await expect(
    dialog.getByRole("checkbox", { name: "Open a terminal running claude" }),
  ).toBeChecked();
  await page.screenshot({ path: "target/e2e/new-project.png" });
  await create.click();
  await expect(dialog).toHaveCount(0);
  await expect(tree.locator(".tree-row.project", { hasText: /^acme/ })).toBeVisible();
  // Its terminal, in its main worktree; `claude` was typed in it: its agent is under the project.
  const terminals = page.getByRole("tablist", { name: "Open terminals and files" });
  await expect(terminals.getByRole("tab", { name: /main$/ })).toBeVisible();
  await expect(tree.locator(".tree-row.agent")).toHaveCount(1);

  // Without git: a folder project; the dialog starts at the last parent used.
  await page.getByTitle(keyLabel("Add project (Ctrl+Shift+O)")).click();
  await dialog.getByRole("button", { name: "New project" }).click();
  await expect(parent).toHaveValue("/home/user/clients/");
  await name.fill("notes-2");
  await dialog.getByRole("checkbox", { name: "Initialize git" }).uncheck();
  await dialog.getByRole("checkbox", { name: "Open a terminal running claude" }).uncheck();
  await create.click();
  await expect(dialog).toHaveCount(0);
  await expect(tree.locator(".tree-row.folder", { hasText: "notes-2" })).toBeVisible();
});

test("New folder's button: a toggle, its icon centred, its separator short of both lines (24.10)", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTitle(keyLabel("Add project (Ctrl+Shift+O)")).click();
  const dialog = page.getByRole("dialog", { name: "Add project" });
  await dialog.getByRole("button", { name: "New project" }).click();
  await expect(dialog.getByLabel("Parent folder")).toHaveValue("/home/user/");
  const button = dialog.getByRole("button", { name: "New folder" });
  const folderName = dialog.getByLabel("New folder name");

  // Clicked again, the row closes; a typed name stays when the name is left.
  await button.click();
  await expect(button).toHaveAttribute("aria-pressed", "true");
  await button.click();
  await expect(folderName).toHaveCount(0);
  await button.click();
  await folderName.fill("clients");
  const parent = dialog.getByLabel("Parent folder");
  await parent.click();
  await expect(folderName).toHaveValue("clients");
  // Typing in the path keeps the focus there, and the button still closes the name.
  await parent.press("End");
  await page.keyboard.type("x");
  await expect(parent).toHaveValue("/home/user/x");
  await expect(parent).toBeFocused();
  await button.click();
  await expect(folderName).toHaveCount(0);

  const geometry = await button.evaluate((el) => {
    const box = el.getBoundingClientRect();
    const icon = (el.querySelector("svg") as SVGElement).getBoundingClientRect();
    const path = (el.parentElement as HTMLElement).getBoundingClientRect();
    const line = getComputedStyle(el, "::before");
    return {
      box: { x: box.x + box.width / 2, y: box.y + box.height / 2 },
      icon: { x: icon.x + icon.width / 2, y: icon.y + icon.height / 2 },
      field: { top: path.top, bottom: path.bottom, buttonTop: box.top, buttonBottom: box.bottom },
      line: { top: Number.parseFloat(line.top), bottom: Number.parseFloat(line.bottom) },
      width: line.borderLeftWidth,
    };
  });
  // The button fills the path's row, the icon at its centre.
  expect(geometry.field.buttonTop).toBe(geometry.field.top);
  expect(geometry.field.buttonBottom).toBe(geometry.field.bottom);
  expect(Math.abs(geometry.icon.x - geometry.box.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(geometry.icon.y - geometry.box.y)).toBeLessThanOrEqual(0.5);
  // The separator reaches neither line, with the same gap above and below.
  expect(geometry.width).toBe("1px");
  expect(geometry.line.top).toBeGreaterThan(0);
  expect(geometry.line.bottom).toBe(geometry.line.top);
});
