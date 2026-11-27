import { expect, test } from "@playwright/test";

test("closing a tab shows the one used before it; a worktree and a space shown again keep their tab and view, after a reload too (28.19)", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const space = tree.getByRole("combobox", { name: "Space" });
  const bar = page.getByRole("tablist", { name: "Open terminals and files" });
  const rail = page.getByRole("tablist", { name: "Panel" });
  const shown = bar.getByRole("tab", { selected: true });
  const view = rail.getByRole("tab", { selected: true });
  const tab = (name: string) => bar.getByRole("tab", { name });
  /** Opens `name` from the Files view in a tab of its own (a double click keeps it, 11.1). */
  const open = async (name: string) => {
    await page
      .getByRole("region", { name: "Files" })
      .getByRole("treeitem", { name, exact: true })
      .dblclick();
    await expect(page.getByRole("region", { name }).locator(".cm-content")).not.toBeEmpty();
  };
  const pick = async (name: string) => {
    await space.click();
    await page.getByRole("option", { name }).click();
  };

  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await open("README.md");
  await open("package.json");
  // The bar is fix-login, README.md, package.json; shown README.md, the terminal, package.json.
  await tab("README.md").click();
  await tab("fix-login").click();
  await tab("package.json").click();
  // Closed, package.json gives way to the terminal shown before it, not README.md beside it.
  await page.getByRole("button", { name: "Close file package.json" }).click();
  await expect(shown).toHaveText("fix-login");

  // fix-login left on README.md in the Diff view; feat-checkout on the Files view.
  await tab("README.md").click();
  await rail.getByRole("tab", { name: "Diff" }).click();
  await tree.getByRole("button", { name: "feat-checkout" }).click();
  await expect(view).toHaveAccessibleName("Diff");
  await rail.getByRole("tab", { name: "Files" }).click();
  await tree.getByRole("button", { name: "fix-login" }).click();
  await expect(shown).toHaveText("README.md");
  await expect(view).toHaveAccessibleName("Diff");
  await page.screenshot({ path: "target/e2e/last-place.png" });
  await tree.getByRole("button", { name: "feat-checkout" }).click();
  await expect(view).toHaveAccessibleName("Files");

  // Another space and back: the place, its tab and its view.
  await tree.getByRole("button", { name: "fix-login" }).click();
  await pick("New space…");
  const dialog = page.getByRole("dialog", { name: "New space" });
  await dialog.getByLabel("Name", { exact: true }).fill("Work");
  await dialog.getByLabel("Name", { exact: true }).press("Enter");
  await expect(space).toHaveText("Work");
  await expect(bar.getByRole("tab")).toHaveCount(0);
  await pick("Default");
  await expect(tree.getByRole("button", { name: "fix-login" })).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(shown).toHaveText("README.md");
  await expect(view).toHaveAccessibleName("Diff");

  // After a reload each worktree shows the view it was left with.
  await page.reload();
  await tree.getByRole("button", { name: "fix-login" }).click();
  await expect(view).toHaveAccessibleName("Diff");
  await tree.getByRole("button", { name: "feat-checkout" }).click();
  await expect(view).toHaveAccessibleName("Files");
});
