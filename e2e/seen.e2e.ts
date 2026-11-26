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
  const count = panel.locator(".files-seen > span");
  await expect(count).toHaveText("0 of 2 seen");
  for (const name of ["src", "auth"]) {
    await files
      .getByRole("treeitem")
      .filter({ has: page.getByText(name, { exact: true }) })
      .click();
  }
  const box = (name: string) => files.getByRole("checkbox", { name: `Seen: src/auth/${name}` });
  const row = (name: string) => files.getByRole("treeitem", { name: new RegExp(name) });

  // A row's box, at its right end, marks the file without opening it.
  await box("session.ts").click();
  await expect(box("session.ts")).toBeChecked();
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
  await expect(box("session.ts")).not.toBeChecked();
  await expect(row("session.ts").locator(".seen-changed")).toHaveAttribute(
    "title",
    "Changed since seen",
  );
  await page.screenshot({ path: "target/e2e/seen.png" });
});
