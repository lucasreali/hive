import { expect, test } from "@playwright/test";
import { mod } from "./host";

test("a new space starts empty, takes new projects, and F8 goes back to another space's agent", async ({
  page,
}) => {
  await page.goto("/?mock=states");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const space = tree.getByRole("combobox", { name: "Space" });
  await expect(space).toHaveText("Default");
  await expect(tree.getByRole("button", { name: "shop", exact: true })).toBeVisible();

  // A new space, with its terminals' git identity.
  await space.click();
  await page.getByRole("option", { name: "New space…" }).click();
  const dialog = page.getByRole("dialog", { name: "New space" });
  await dialog.getByLabel("Name", { exact: true }).fill("Work");
  await dialog.getByLabel("Git email").fill("me@work.example");
  await page.screenshot({ path: "target/e2e/space-dialog.png" });
  await dialog.getByLabel("Name", { exact: true }).press("Enter");
  await expect(dialog).toHaveCount(0);
  await expect(space).toHaveText("Work");
  await expect(tree).toContainText("No projects");

  // Its projects only; a project of another space is refused.
  await page.keyboard.press(`${mod}+Shift+O`);
  const add = page.getByRole("dialog", { name: "Add project" });
  const field = add.getByLabel("Folder", { exact: true });
  await field.fill("/home/user/projects/shop");
  await expect(add.getByRole("button", { name: /^Add project/ })).toBeEnabled();
  await field.press("Enter");
  await expect(add.getByRole("alert")).toContainText("is already in the space Default");
  await field.fill("/home/user/dotfiles");
  await expect(add.getByRole("button", { name: /^Add project/ })).toBeEnabled();
  await field.press("Enter");
  await expect(add).toHaveCount(0);
  await expect(tree.getByRole("button", { name: "dotfiles", exact: true })).toBeVisible();
  await expect(tree.getByRole("button", { name: "shop", exact: true })).toHaveCount(0);
  await page.screenshot({ path: "target/e2e/space-work.png" });

  // Agents of the other space still count, and F8 goes to one there.
  await expect(page.getByRole("button", { name: /pending: notifications/ })).toBeVisible();
  await page.keyboard.press("F8");
  await expect(space).toHaveText("Default");
  await expect(tree.getByRole("button", { name: "shop", exact: true })).toBeVisible();
});

test("the current space is edited; one with projects cannot be deleted", async ({ page }) => {
  await page.goto("/");
  const space = page.getByRole("combobox", { name: "Space" });
  await space.click();
  await page.getByRole("option", { name: "Edit space…" }).click();
  const dialog = page.getByRole("dialog", { name: "Edit space" });
  await expect(dialog.getByRole("button", { name: "Delete space" })).toBeDisabled();
  await dialog.getByLabel("Name", { exact: true }).fill("Personal");
  await dialog.getByRole("button", { name: /^Save/ }).click();
  await expect(dialog).toHaveCount(0);
  await expect(space).toHaveText("Personal");
});

test("the space's Claude account shows in the status bar and is changed in its dialog (22.5)", async ({
  page,
}) => {
  await page.goto("/");
  const bar = page.locator(".statusbar");
  await expect(bar.getByRole("button", { name: /^Session 42%/ })).toBeVisible();
  // An account besides the default one.
  await page.keyboard.press(`${mod}+Comma`);
  const settings = page.getByRole("dialog", { name: "Settings" });
  await settings.getByRole("button", { name: "Accounts" }).click();
  await settings.getByRole("button", { name: "New account" }).click();
  await settings.getByLabel("New account name").fill("Work");
  await settings.getByLabel("Use an existing folder").check();
  await settings.getByLabel("New account folder").fill("/home/user/.claude-work");
  await settings.getByRole("button", { name: "Add", exact: true }).click();
  await page.keyboard.press("Escape");
  await expect(settings).toBeHidden();

  const account = bar.getByRole("button", { name: "Default", exact: true });
  await expect(account).toHaveAttribute(
    "title",
    "Claude account of Default; change it in the space's settings",
  );
  await expect(bar.getByRole("combobox")).toHaveCount(0);
  // Its click opens the space's dialog, where the account is picked.
  await account.click();
  const dialog = page.getByRole("dialog", { name: "Edit space" });
  await dialog.getByRole("combobox", { name: "Claude account" }).click();
  await page.getByRole("option", { name: "Work" }).click();
  await dialog.getByRole("button", { name: /^Save/ }).click();
  await expect(dialog).toHaveCount(0);
  await expect(bar.getByRole("button", { name: "Work", exact: true })).toBeVisible();
  // The usage follows the space's account: none yet for this one.
  await expect(bar.getByRole("button", { name: /^No usage yet/ })).toBeVisible();
});

test("switching space shows the place last selected there, with its tabs", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const space = tree.getByRole("combobox", { name: "Space" });
  const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
  const plus = page.getByTitle("New terminal or file");
  const pick = async (name: string) => {
    await space.click();
    await page.getByRole("option", { name }).click();
  };
  const terminal = async () => {
    await plus.click();
    await page.getByRole("menuitem", { name: "Terminal" }).click();
  };

  await tree.getByRole("button", { name: "fix-login" }).click();
  await terminal();
  await expect(tabs.getByRole("tab")).toHaveText(["fix-login"]);

  // A new space is empty: nothing selected, no tabs.
  await pick("New space…");
  const dialog = page.getByRole("dialog", { name: "New space" });
  await dialog.getByLabel("Name", { exact: true }).fill("Work");
  await dialog.getByLabel("Name", { exact: true }).press("Enter");
  await expect(space).toHaveText("Work");
  await expect(tabs.getByRole("tab")).toHaveCount(0);
  await page.keyboard.press(`${mod}+Shift+O`);
  const add = page.getByRole("dialog", { name: "Add project" });
  await add.getByLabel("Folder", { exact: true }).fill("/home/user/dotfiles");
  await expect(add.getByRole("button", { name: /^Add project/ })).toBeEnabled();
  await add.getByLabel("Folder", { exact: true }).press("Enter");
  await expect(add).toHaveCount(0);
  await tree.getByRole("button", { name: "dotfiles", exact: true }).click();
  await terminal();
  await expect(tabs.getByRole("tab")).toHaveCount(1);
  const work = (await tabs.getByRole("tab").textContent()) as string;

  // Each space shows its place again, with that place's tabs.
  await pick("Default");
  await expect(space).toHaveText("Default");
  await expect(tabs.getByRole("tab")).toHaveText(["fix-login"]);
  await expect(tree.getByRole("button", { name: "fix-login" })).toHaveAttribute(
    "aria-current",
    "true",
  );
  await page.screenshot({ path: "target/e2e/space-place.png" });
  await pick("Work");
  await expect(tabs.getByRole("tab")).toHaveText([work]);
  await expect(tree.getByRole("button", { name: "dotfiles", exact: true })).toHaveAttribute(
    "aria-current",
    "true",
  );
});
