import { expect, type Page, test } from "@playwright/test";
import { mod } from "./host";

// The app menu (17.13). Its Edit items are AppKit's: in WKWebView ⌘C/⌘V in a text field and the
// editor are the engine's own copy and paste, which Hive's keys must leave alone (Playwright's
// WebKit on a Mac runs them as the Edit menu's commands; elsewhere they are Ctrl+C/Ctrl+V).
// The menu's own items reach the UI as `menu {item}`: `menu <item>` typed in a mock terminal.

const clipboard = (page: Page) => page.evaluate(() => navigator.clipboard.readText());

test.beforeEach(async ({ context, browserName }) => {
  // WebKit knows only clipboard-read; it lets a page write the clipboard without a permission.
  await context.grantPermissions(
    browserName === "webkit" ? ["clipboard-read"] : ["clipboard-read", "clipboard-write"],
  );
});

test("menu: copy and paste in a text field and the editor", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await expect(tree.getByRole("button", { name: "fix-login" })).toBeVisible();
  // A text field: the settings' search.
  await page.keyboard.press(`${mod}+Comma`);
  const search = page.getByRole("dialog", { name: "Settings" }).getByLabel("Search settings");
  await search.fill("theme");
  await search.press(`${mod}+A`);
  await search.press(`${mod}+C`);
  await expect.poll(() => clipboard(page)).toBe("theme");
  await search.press("ArrowRight");
  await search.press(`${mod}+V`);
  await expect(search).toHaveValue("themetheme");
  await page.keyboard.press("Escape");

  // The editor: README.md of fix-login.
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("region", { name: "Files" });
  await panel.getByRole("treeitem", { name: "README.md" }).click();
  const view = page.getByRole("region", { name: "README.md" });
  const line = view.locator(".cm-line", { hasText: "export const value = 1;" });
  await expect(line).toBeVisible();
  await line.click();
  // The line's text, from its start to its end.
  await page.keyboard.press(mod === "Meta" ? "Meta+ArrowLeft" : "Home");
  await page.keyboard.press(mod === "Meta" ? "Meta+Shift+ArrowRight" : "Shift+End");
  await page.keyboard.press(`${mod}+C`);
  await expect.poll(() => clipboard(page)).toBe("export const value = 1;");
  await page.keyboard.press(mod === "Meta" ? "Meta+ArrowRight" : "End");
  await page.keyboard.press(`${mod}+V`);
  await expect(view.locator(".cm-content")).toContainText(
    "export const value = 1;export const value = 1;",
  );
});

test("menu: Toggle Sidebar and Toggle Right Panel", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  const files = page.getByRole("region", { name: "Files" });
  await expect(files).toBeVisible();
  const menu = async (item: string) => {
    await page.keyboard.type(`menu ${item}`);
    await page.keyboard.press("Enter");
  };
  await menu("toggle-sidebar");
  await expect(tree).toBeHidden();
  await menu("toggle-panel");
  await expect(files).toBeHidden();
  await menu("toggle-sidebar");
  await expect(tree).toBeVisible();
});
