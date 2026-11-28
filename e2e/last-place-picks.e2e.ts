import { expect, test } from "@playwright/test";
import { mod } from "./host";

// 28.27: the picks of 28.19's audit (docs/review/last-place.md), on a switch and on a restart
// (a reload of the page).

test("a reload keeps the place, the file tabs and their pair, the diff base, the trees and the panel (28.27)", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const bar = page.getByRole("tablist", { name: "Open terminals and files" });
  const shown = bar.getByRole("tab", { selected: true });
  const panel = page.getByRole("complementary", { name: "Side panel" });
  const rail = panel.getByRole("tablist", { name: "Panel" });
  const tab = (name: string) => bar.getByRole("tab", { name });
  const find = panel.getByRole("searchbox", { name: "Find files" });
  const open = async (name: string) => {
    await panel
      .getByRole("region", { name: "Files" })
      .getByRole("treeitem", { name, exact: true })
      .dblclick();
    await expect(page.getByRole("region", { name }).locator(".cm-content")).not.toBeEmpty();
  };

  // The Files search is each worktree's (row 11).
  await tree.getByRole("button", { name: "fix-login" }).click();
  await find.fill("read");
  await tree.getByRole("button", { name: "feat-checkout" }).click();
  await expect(find).toHaveValue("");
  await tree.getByRole("button", { name: "fix-login" }).click();
  await expect(find).toHaveValue("read");
  await find.fill("");

  // Two files paired, package.json shown last (rows 4, 5).
  await open("README.md");
  await open("package.json");
  await tab("README.md").click({ button: "right" });
  await page.getByRole("menuitem", { name: "Split right" }).click();
  await expect(page.getByRole("region", { name: "README.md" })).toBeVisible();
  await tab("package.json").click();

  // The Diff view on HEAD, its folders open, Unseen only (rows 6, 8, 9); api collapsed (10).
  await rail.getByRole("tab", { name: "Diff" }).click();
  const base = panel.getByRole("combobox", { name: "Compare with" });
  await base.click();
  await page.getByRole("option", { name: "HEAD" }).click();
  await expect(base).toHaveText("HEAD");
  const files = panel.getByRole("tree", { name: "Files" });
  for (const name of ["src", "auth"]) {
    await files
      .getByRole("treeitem")
      .filter({ has: page.getByText(name, { exact: true }) })
      .click();
  }
  await expect(files.getByRole("treeitem", { name: /session\.ts/ })).toBeVisible();
  const unseen = panel.getByRole("button", { name: "Unseen only" });
  await unseen.click();
  await expect(unseen).toHaveAttribute("aria-pressed", "true");
  await tree.getByRole("button", { name: "Collapse api" }).click();

  await page.reload();
  // The place is selected again (row 1), with its files reopened and paired, the last shown.
  await expect(tree.getByRole("button", { name: "fix-login" })).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(tab("README.md")).toBeVisible();
  await expect(shown).toHaveText("package.json");
  await expect(page.getByRole("region", { name: "README.md" })).toBeVisible();
  await expect(rail.getByRole("tab", { selected: true })).toHaveAccessibleName("Diff");
  await expect(panel.getByRole("combobox", { name: "Compare with" })).toHaveText("HEAD");
  await expect(files.getByRole("treeitem", { name: /session\.ts/ })).toBeVisible();
  await expect(panel.getByRole("button", { name: "Unseen only" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(tree.getByRole("button", { name: "Expand api" })).toBeVisible();
  await page.screenshot({ path: "target/e2e/last-place-picks.png" });

  // The right panel closed stays closed (row 17).
  await page.keyboard.press(`${mod}+Shift+B`);
  await expect(panel.getByRole("searchbox")).toHaveCount(0);
  await page.reload();
  await expect(tree.getByRole("button", { name: "fix-login" })).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(rail).toBeVisible();
  await expect(panel.getByRole("tree")).toHaveCount(0);
});

test("Settings reopens on the section picked last (28.27)", async ({ page }) => {
  await page.goto("/");
  const place = page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login", exact: true });
  await place.click();
  await page.keyboard.press(`${mod}+Comma`);
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog.getByRole("button", { name: "Appearance" }).click();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await place.click();
  await page.keyboard.press(`${mod}+Comma`);
  await expect(dialog.getByRole("button", { name: "Appearance" })).toHaveAttribute(
    "aria-current",
    "page",
  );
});
