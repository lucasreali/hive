import { expect, test } from "@playwright/test";

// Uninstall Hive (23.5): the app menu's "Uninstall Hive…" (`menu uninstall` typed in a mock
// terminal) opens the dialog with the service's list; the mock removes nothing and answers
// `uninstalled`, after which the window closes (`data-closed` outside Tauri).

test("uninstall: the dialog lists what goes, its boxes ask again, and confirming quits", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await page.keyboard.type("menu uninstall");
  await page.keyboard.press("Enter");

  const dialog = page.getByRole("dialog", { name: "Uninstall Hive?" });
  await expect(dialog).toBeVisible();
  // The open terminal is said to close.
  await expect(dialog).toContainText("1 terminal is open: uninstalling closes them");
  const data = dialog.getByRole("region", { name: "Settings, spaces and accounts" });
  await expect(data).toContainText("/home/mock/.hive/settings.json");
  await expect(dialog.getByRole("region", { name: "The app" })).toContainText(
    "/Applications/Hive.app",
  );
  const accounts = dialog.getByRole("checkbox", { name: /Also remove the Claude account/ });
  await expect(accounts).not.toBeChecked();
  const folders = dialog.getByRole("region", { name: "Claude account folders Hive made" });
  await expect(folders).toBeHidden();
  await accounts.check();
  await expect(folders).toContainText("/home/mock/.hive/accounts/work");

  const keep = dialog.getByRole("checkbox", { name: "Keep my settings, spaces and accounts" });
  await keep.check();
  await expect(data).toBeHidden();
  await expect(folders).toBeHidden();
  await expect(accounts).toBeDisabled();
  await expect(dialog).toContainText("/home/mock/.hive/bin");
  await expect(dialog).toContainText("run /logout in claude first");

  // Cancel keeps everything.
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toBeHidden();
  const closed = () => page.evaluate(() => document.documentElement.dataset.closed !== undefined);
  expect(await closed()).toBe(false);

  // Back in the terminal.
  await page.locator(".xterm-screen").first().click();
  await page.keyboard.type("menu uninstall");
  await page.keyboard.press("Enter");
  await expect(dialog).toBeVisible();
  // Opened again, the boxes are off.
  await expect(keep).not.toBeChecked();
  await dialog.getByRole("button", { name: "Uninstall", exact: true }).click();
  await expect.poll(closed).toBe(true);
});
