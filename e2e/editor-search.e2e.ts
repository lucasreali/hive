import { expect, test } from "@playwright/test";
import { mod } from "./host";

test("editor search: ⌘F (Ctrl+F off a Mac) finds the next and previous match (18.38)", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page
    .getByRole("region", { name: "Files" })
    .getByRole("treeitem", { name: "README.md" })
    .click();
  const view = page.getByRole("region", { name: "README.md" });
  await expect(view.locator(".cm-content")).toContainText("export const value = 1;");
  await view.locator(".cm-line").first().click();
  await page.keyboard.press(`${mod}+Home`);
  await page.keyboard.type("foo one\nfoo two\n");
  await page.keyboard.press(`${mod}+Home`);

  await page.keyboard.press(`${mod}+f`);
  const field = view.locator(".cm-search input[name=search]");
  await expect(field).toBeFocused();
  // The terminal's search bar stays closed: the key was the editor's.
  await expect(page.locator(".terminal-search")).toHaveCount(0);
  // Typed, not filled: the panel takes the query on keyup.
  await page.keyboard.type("foo");
  await page.keyboard.press("Enter");
  const active = view.locator(".cm-activeLine");
  await expect(active).toHaveText("foo one");
  await page.keyboard.press("Enter");
  await expect(active).toHaveText("foo two");
  await page.keyboard.press("Shift+Enter");
  await expect(active).toHaveText("foo one");
  await expect(view.locator(".cm-searchMatch")).toHaveCount(2);
  await page.screenshot({ path: "target/e2e/editor-search.png" });

  await page.keyboard.press("Escape");
  await expect(view.locator(".cm-search")).toHaveCount(0);
});
