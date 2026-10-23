import { expect, type Page, test } from "@playwright/test";
import { mod } from "./host";

// 18.24: the window's chrome (rows, buttons, headings) never selects, as in a Mac app; content
// (the Markdown preview, paths and code in dialogs) still does.
const selection = (page: Page) => page.evaluate(() => getSelection()?.toString().trim() ?? "");

test("select all on a tree row selects nothing; the preview and dialogs still select", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  const row = page.getByRole("region", { name: "Files" }).getByRole("treeitem", {
    name: "README.md",
  });
  await row.focus();
  await page.keyboard.press(`${mod}+a`);
  expect(await selection(page)).toBe("");
  await row.click();

  // The rendered Markdown selects.
  const view = page.getByRole("region", { name: "README.md" });
  await view.getByRole("button", { name: "Show rendered Markdown" }).click();
  const word = view.locator(".markdown-view p").first();
  await word.dblclick();
  expect(await selection(page)).not.toBe("");

  // In a dialog, a heading does not; its code does.
  await page.keyboard.press(`${mod}+Comma`);
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog.getByRole("button", { name: "Accounts" }).click();
  await dialog.getByRole("heading").first().dblclick();
  expect(await selection(page)).toBe("");
  await dialog.locator("code", { hasText: "CLAUDE_CONFIG_DIR" }).dblclick();
  expect(await selection(page)).toBe("CLAUDE_CONFIG_DIR");
});
