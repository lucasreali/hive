import { expect, test } from "@playwright/test";
import { mod } from "./host";

test("diff: files marked seen, counted, filtered, marked all, changed since seen (27.1)", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Diff" }).click();
  const files = panel.getByRole("tree", { name: "Files" });
  const count = panel.locator(".files-seen-count");
  await expect(count).toHaveText("0 of 2 seen");
  for (const name of ["src", "auth"]) {
    await files
      .getByRole("treeitem")
      .filter({ has: page.getByText(name, { exact: true }) })
      .click();
  }
  const box = (name: string) => files.getByRole("button", { name: `Seen: src/auth/${name}` });
  const row = (name: string) => files.getByRole("treeitem", { name: new RegExp(name) });

  // A row's eye, at its right end, marks the file without opening it.
  await box("session.ts").click();
  await expect(box("session.ts")).toHaveAttribute("aria-pressed", "true");
  await expect(count).toHaveText("1 of 2 seen");
  await expect(page.getByRole("region", { name: "src/auth/session.ts" })).toHaveCount(0);
  const rowEnd = await row("session.ts").evaluate((r) => r.getBoundingClientRect().right);
  const boxEnd = await box("session.ts").evaluate((b) => b.getBoundingClientRect().right);
  expect(rowEnd - boxEnd).toBeLessThan(16);

  // "Unseen only" leaves the seen one out.
  const filter = panel.getByRole("button", { name: "Unseen only" });
  await filter.click();
  await expect(files.getByRole("treeitem", { name: /session\.ts/ })).toHaveCount(0);
  await expect(row("constants.ts")).toBeVisible();

  // The palette marks all of them.
  await page.keyboard.press(`${mod}+Shift+P`);
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await palette.getByRole("textbox", { name: /Search commands/ }).fill("mark all as seen");
  await page.keyboard.press("Enter");
  await expect(count).toHaveText("2 of 2 seen");
  await expect(panel.getByText("Every changed file is seen.")).toBeVisible();
  await filter.click();

  // The diff's header has the open file's box; an agent's edit makes it changed since seen.
  await row("session.ts").click();
  const view = page.getByRole("region", { name: "src/auth/session.ts" });
  await expect(view.getByRole("checkbox", { name: "Seen" })).toBeChecked();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await page.keyboard.type("write src/auth/session.ts edited");
  await page.keyboard.press("Enter");
  await expect(count).toHaveText("1 of 2 seen");
  await expect(box("session.ts")).toHaveAttribute("aria-pressed", "false");
  await expect(row("session.ts").locator(".seen-changed")).toHaveAttribute(
    "title",
    "Changed since seen",
  );
  await page.screenshot({ path: "target/e2e/seen.png" });
});

test("diff: a row's staging box and eye, by mouse and by keyboard (28.20)", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Diff" }).click();
  const files = panel.getByRole("tree", { name: "Files" });
  const seenCount = panel.locator(".files-seen-count");
  const stagedCount = panel.getByRole("region", { name: "Commit" }).locator(".commit-box-bar");
  await expect(seenCount).toHaveText("0 of 2 seen");
  await expect(stagedCount).toHaveText("0 of 2 staged");
  for (const name of ["src", "auth"]) {
    await files
      .getByRole("treeitem")
      .filter({ has: page.getByText(name, { exact: true }) })
      .click();
  }
  const row = (name: string) => files.getByRole("treeitem", { name: new RegExp(name) });
  // One checkbox per row: the staging one; "seen" is the eye button.
  await expect(row("session.ts").getByRole("checkbox")).toHaveCount(1);
  const stage = row("session.ts").getByRole("checkbox", { name: "Stage src/auth/session.ts" });
  const eye = row("session.ts").getByRole("button", { name: "Seen: src/auth/session.ts" });
  await expect(eye).toHaveAttribute("title", "Mark as seen");

  // The counts sit over the controls they count: "staged" starts at the boxes' column, "seen"
  // ends at the eyes' column.
  const left = (l: typeof stage) => l.evaluate((e) => e.getBoundingClientRect());
  const header = await left(panel.locator(".commit-box-bar input"));
  const box = await left(stage);
  expect(Math.abs(header.x + header.width / 2 - (box.x + box.width / 2))).toBeLessThan(2);
  const headerEye = await left(seenCount.locator("svg"));
  const rowEye = await left(eye.locator("svg"));
  expect(Math.abs(headerEye.right - rowEye.right)).toBeLessThan(2);

  // By mouse: each control acts alone, opening nothing.
  await stage.click();
  await expect(stagedCount).toHaveText("1 of 2 staged");
  await expect(seenCount).toHaveText("0 of 2 seen");
  await eye.click();
  await expect(seenCount).toHaveText("1 of 2 seen");
  await expect(stagedCount).toHaveText("1 of 2 staged");
  await expect(eye).toHaveAttribute("aria-pressed", "true");
  await expect(eye).toHaveAttribute("title", "Seen");
  await expect(page.getByRole("region", { name: "src/auth/session.ts" })).toHaveCount(0);
  // The click made its row the active one; Space then unstages it and marks nothing.
  await page.keyboard.press(" ");
  await expect(stagedCount).toHaveText("0 of 2 staged");
  await expect(seenCount).toHaveText("1 of 2 seen");

  // By keyboard on the other row: Space stages the active file, Enter opens its diff, and Space
  // on its header's "Seen" box marks it (the tree's eyes take no Tab stop, 27.1).
  await files.focus();
  await page.keyboard.press("ArrowUp");
  const constants = await row("constants.ts").getAttribute("id");
  await expect(files).toHaveAttribute("aria-activedescendant", constants ?? "");
  await page.keyboard.press(" ");
  await expect(stagedCount).toHaveText("1 of 2 staged");
  await page.keyboard.press("Enter");
  const view = page.getByRole("region", { name: "src/auth/constants.ts" });
  const seen = view.getByRole("checkbox", { name: "Seen" });
  await seen.focus();
  await page.keyboard.press(" ");
  await expect(seen).toBeChecked();
  await expect(seenCount).toHaveText("2 of 2 seen");
  await expect(
    row("constants.ts").getByRole("button", { name: "Seen: src/auth/constants.ts" }),
  ).toHaveAttribute("aria-pressed", "true");
});
