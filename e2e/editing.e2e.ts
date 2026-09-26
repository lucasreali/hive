import { expect, type Page, test } from "@playwright/test";

/** Opens the right panel's Files on fix-login with README.md (no changes): editable text. */
async function openReadme(page: Page) {
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("region", { name: "Files" });
  await panel.getByRole("treeitem", { name: "README.md" }).click();
  const view = page.getByRole("region", { name: "README.md" });
  await expect(view.locator(".cm-content")).toContainText("export const value = 1;");
  return { panel, view };
}

test("editing: closing unsaved edits asks in a Hive dialog, never the browser's (8.20)", async ({
  page,
}) => {
  const native: string[] = [];
  page.on("dialog", (d) => {
    native.push(d.message());
    void d.dismiss();
  });
  await page.goto("/");
  const { view } = await openReadme(page);
  await view.locator(".cm-line").first().click();
  await page.keyboard.type("// mine\n");
  const unsaved = page.getByRole("button", { name: "Close file README.md (unsaved changes)" });
  await unsaved.click();
  const asked = page.getByRole("dialog", { name: "Discard changes?" });
  await expect(asked).toContainText("Your unsaved changes to README.md will be lost.");
  await expect(asked.getByRole("button", { name: "Cancel" })).toBeFocused();
  await page.keyboard.press("Enter"); // Cancel has the focus.
  await expect(asked).toBeHidden();
  await expect(view).toBeVisible();
  await unsaved.click();
  await asked.getByRole("button", { name: "Discard" }).click();
  await expect(view).toBeHidden();
  expect(native).toEqual([]);
});

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
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
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

test("editing: the tree's menu renames the open file and creates a new one", async ({ page }) => {
  await page.goto("/");
  const { panel } = await openReadme(page);
  const dialog = page.getByRole("dialog");
  await panel.getByRole("treeitem", { name: "README.md" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Rename…" }).click();
  await expect(dialog.getByLabel("Name")).toHaveValue("README.md");
  // Taken names are refused by the (fake) service, and the reason shows.
  await dialog.getByLabel("Name").fill("package.json");
  await page.keyboard.press("Enter");
  await expect(dialog.getByRole("alert")).toHaveText("package.json already exists");
  await dialog.getByLabel("Name").fill("NOTES.md");
  await page.keyboard.press("Enter");
  await expect(dialog).toBeHidden();
  // The open file follows the rename.
  const notes = page.getByRole("region", { name: "NOTES.md" });
  await expect(notes.locator(".cm-content")).toContainText("export const value = 1;");
  await expect(panel.getByRole("treeitem", { name: "NOTES.md" })).toBeVisible();
  await expect(panel.getByRole("treeitem", { name: "README.md" })).toHaveCount(0);

  await panel.getByRole("treeitem", { name: "NOTES.md" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "New File…" }).click();
  await dialog.getByLabel("Name").fill("todo.md");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("region", { name: "todo.md" })).toBeVisible();
  await expect(panel.getByRole("treeitem", { name: "todo.md" })).toBeVisible();
});

test("files: New Folder shows an empty folder; a dragged file moves into it, its tab follows", async ({
  page,
}) => {
  await page.goto("/");
  const { panel } = await openReadme(page);
  await panel.getByRole("treeitem", { name: "README.md" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "New Folder…" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading")).toHaveText("New folder");
  await dialog.getByLabel("Name").fill("notes");
  await page.keyboard.press("Enter");
  await expect(dialog).toBeHidden();
  const folder = panel.getByRole("treeitem", { name: "notes" });
  await expect(folder).toHaveAttribute("aria-expanded", "false");

  // Held over the closed folder, it opens; dropped there, the file moves in.
  const from = await panel.getByRole("treeitem", { name: "README.md" }).boundingBox();
  const to = await folder.boundingBox();
  if (!from || !to) throw new Error("the rows are not shown");
  await page.mouse.move(from.x + 40, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(to.x + 40, to.y + to.height / 2);
  await page.mouse.move(to.x + 41, to.y + to.height / 2);
  await expect(folder).toHaveAttribute("data-drop", "true");
  await expect(folder).toHaveAttribute("aria-expanded", "true");
  await page.mouse.up();
  await expect(page.getByRole("region", { name: "notes/README.md" })).toBeVisible();
  await expect(panel.getByRole("treeitem", { name: "README.md" })).toHaveAttribute(
    "title",
    "notes/README.md",
  );
});
