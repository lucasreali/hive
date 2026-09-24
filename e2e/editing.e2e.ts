import { expect, type Page, test } from "@playwright/test";

/** Opens the files panel on fix-login with README.md (no changes): editable text. */
async function openReadme(page: Page) {
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.keyboard.press("Control+Shift+B");
  const panel = page.getByRole("complementary", { name: "Files and diff" });
  await panel.getByRole("treeitem", { name: "README.md" }).click();
  const view = page.getByRole("region", { name: "README.md" });
  await expect(view.locator(".cm-content")).toContainText("export const value = 1;");
  return { panel, view };
}

test("editing: a file without changes is edited and saved with Ctrl+S", async ({ page }) => {
  await page.goto("/");
  const { panel, view } = await openReadme(page);
  // The file's tab marks unsaved edits: a dot in place of the ×, the × again on hover.
  const unsaved = page.getByRole("button", { name: "Close file README.md (unsaved changes)" });
  await expect(unsaved).toBeHidden();

  await view.locator(".cm-line").first().click();
  await page.keyboard.press("Control+Home");
  await page.keyboard.type("// edited\n");
  await expect(unsaved).toBeVisible();
  await expect(unsaved.locator(".dirty")).toBeVisible();
  await expect(unsaved.locator("svg")).toBeHidden();
  await unsaved.hover();
  await expect(unsaved.locator(".dirty")).toBeHidden();
  await expect(unsaved.locator("svg")).toBeVisible();
  await view.locator(".cm-line").first().hover();
  await page.screenshot({ path: "target/e2e/editing.png" });
  await page.keyboard.press("Control+s");
  await expect(unsaved).toBeHidden();
  await expect(view.getByRole("button", { name: "Save" })).toBeDisabled();

  // Saved in the (fake) service: opened again, the file has the edit.
  await page.getByRole("button", { name: "Close file README.md" }).click();
  await panel.getByRole("treeitem", { name: "README.md" }).click();
  await expect(view.locator(".cm-line").first()).toHaveText("// edited");

  // Outside the app nothing opens, and the view says so.
  await view.getByRole("button", { name: "Open in external editor" }).click();
  await expect(view).toContainText(
    "Only the Hive app opens an external editor: \\\\wsl.localhost\\Ubuntu",
  );
});

test("editing: an agent writing the file under unsaved edits shows the conflict", async ({
  page,
}) => {
  await page.goto("/");
  const { view } = await openReadme(page);
  await page.getByTitle("New terminal (Ctrl+Shift+T)").click();
  await expect(page.getByRole("tab", { name: "fix-login" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  // The fake service's stand-in for an agent writing the file, typed in the terminal's tab;
  // then the file's tab is shown again.
  const agentWrites = async (text: string) => {
    await page.getByRole("tab", { name: "fix-login" }).click();
    await page.locator(".xterm").click();
    await page.keyboard.type(`write README.md ${text}`);
    await page.keyboard.press("Enter");
    await page.getByRole("tab", { name: /^README\.md/ }).click();
  };

  // A clean buffer follows the disk.
  await agentWrites("first agent edit");
  await expect(view.locator(".cm-content")).toHaveText("first agent edit");

  await view.locator(".cm-line").first().click();
  await page.keyboard.press("End");
  await page.keyboard.type(" and mine");
  await agentWrites("second agent edit");
  const banner = view.getByRole("alert");
  await expect(banner).toContainText("Changed on disk.");
  await expect(view.locator(".cm-content")).toContainText("first agent edit and mine");

  // The edits against the disk, read-only.
  await banner.getByRole("button", { name: "View diff" }).click();
  await expect(view.locator(".cm-deletedChunk")).toContainText("second agent edit");
  await page.screenshot({ path: "target/e2e/conflict.png" });

  // Keep mine: the next save overwrites the agent's text.
  await banner.getByRole("button", { name: "Keep mine" }).click();
  await expect(banner).toBeHidden();
  await view.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("button", { name: /\(unsaved changes\)$/ })).toBeHidden();

  // Reload: the agent's text replaces the edits.
  await view.locator(".cm-line").first().click();
  await page.keyboard.type("!");
  await agentWrites("third agent edit");
  await view.getByRole("alert").getByRole("button", { name: "Reload" }).click();
  await expect(view.locator(".cm-content")).toHaveText("third agent edit");
  await expect(view.getByRole("alert")).toBeHidden();
});
