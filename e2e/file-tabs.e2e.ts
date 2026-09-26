import { expect, type Page, test } from "@playwright/test";

/** The names of the tab bar's tabs, left to right. */
const names = (page: Page) =>
  page
    .getByRole("tablist", { name: "Open terminals and files" })
    .locator(".tab-name")
    .allTextContents();

/** Opens `name` from the Files tree of the selected worktree, as editable text. */
async function openFile(page: Page, name: string) {
  const panel = page.getByRole("region", { name: "Files" });
  await panel.getByRole("treeitem", { name, exact: true }).click();
  const view = page.getByRole("region", { name });
  await expect(view.locator(".cm-content")).not.toBeEmpty();
  return view;
}

test("file tabs: each file keeps its edits; tabs reorder by drag, remembered after a reload", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();

  // Two files, each in its own tab, each with its own unsaved edits.
  const readme = await openFile(page, "README.md");
  await readme.locator(".cm-line").first().click();
  await page.keyboard.press("Control+Home");
  await page.keyboard.type("// readme\n");
  const pkg = await openFile(page, "package.json");
  await pkg.locator(".cm-line").first().click();
  await page.keyboard.press("Control+Home");
  await page.keyboard.type("// pkg\n");
  await expect.poll(() => names(page)).toEqual(["README.md", "package.json"]);
  await page.getByRole("tab", { name: "README.md" }).click();
  await expect(readme.locator(".cm-line").first()).toHaveText("// readme");
  await page.getByRole("tab", { name: "package.json" }).click();
  await expect(pkg.locator(".cm-line").first()).toHaveText("// pkg");
  await expect(page.getByRole("button", { name: /\(unsaved changes\)$/ })).toHaveCount(2);

  // A new terminal goes last; dragged onto the left edge of README.md it lands first.
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect.poll(() => names(page)).toEqual(["README.md", "package.json", "fix-login"]);
  const tab = (name: string) => page.locator(".tab", { hasText: name });
  await tab("fix-login").dragTo(tab("README.md"), { targetPosition: { x: 2, y: 10 } });
  await expect.poll(() => names(page)).toEqual(["fix-login", "README.md", "package.json"]);
  // package.json before README.md.
  await tab("package.json").dragTo(tab("README.md"), { targetPosition: { x: 2, y: 10 } });
  await expect.poll(() => names(page)).toEqual(["fix-login", "package.json", "README.md"]);
  await expect(page.locator("[data-drop]")).toHaveCount(0);

  // After a reload the files keep their order, whatever order they open in.
  await page.reload();
  await tree.getByRole("button", { name: "fix-login" }).click();
  await openFile(page, "README.md");
  await openFile(page, "package.json");
  await expect.poll(() => names(page)).toEqual(["package.json", "README.md"]);
});
