import { expect, test } from "@playwright/test";

test("who wrote a line: a session's edit opens its row; an unknown line answers by the line (27.5)", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  const tabs = panel.getByRole("tablist", { name: "Panel" });
  await tabs.getByRole("tab", { name: "Diff" }).click();
  for (const name of ["src", "auth"]) {
    await panel
      .getByRole("treeitem")
      .filter({ has: page.getByText(name, { exact: true }) })
      .click();
  }
  await panel.getByRole("treeitem", { name: /session\.ts/ }).click();
  const view = page.getByRole("region", { name: "src/auth/session.ts" });
  await view.getByRole("button", { name: "Edit", exact: true }).click();
  const number = (n: number) =>
    view.locator(".cm-lineNumbers .cm-gutterElement").filter({ hasText: new RegExp(`^${n}$`) });

  // The text keeps its native menu: only the line numbers have Hive's.
  await number(40).scrollIntoViewIfNeeded();
  await number(40).click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Line 40" });
  await menu.getByRole("menuitem", { name: "Who wrote this line?" }).click();
  await expect(menu).toBeHidden();

  // The mock's rememberMe lines are fix-login's session's, followed live.
  await expect(tabs.getByRole("tab", { name: "Sessions" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  const row = panel.locator(".session[data-focused]");
  await expect(row.locator(".session-title .label")).toHaveText("Fix the login redirect");
  const told = row.locator(".session-edit");
  await expect(told).toHaveText(
    /^Edited lines 39–41 · \d+h ago · you changed it before accepting$/,
  );
  const files = row.getByRole("list", { name: "Files changed" });
  await expect(files.locator(".session-file[aria-current]")).toHaveAttribute(
    "title",
    "src/auth/session.ts",
  );
  // One line per file (28.22), inside the card: a deep folder is cut, not wrapped.
  const rows = files.locator(".session-file");
  await expect(rows).toHaveCount(5);
  const card = await row.boundingBox();
  for (const r of await rows.all()) {
    const box = await r.boundingBox();
    expect(box?.height).toBe(22);
    expect(box && card && box.x > card.x + 12 && box.x + box.width < card.x + card.width - 8).toBe(
      true,
    );
  }
  const deep = rows.filter({ hasText: "intent.ts" });
  const head = deep.locator(".session-file-head");
  expect(await head.evaluate((e) => e.clientWidth > 0 && e.scrollWidth > e.clientWidth)).toBe(true);
  const tail = deep.locator(".session-file-tail");
  await expect(tail).toHaveText("/handlers");
  expect(await tail.evaluate((e) => e.scrollWidth <= e.clientWidth)).toBe(true);
  await page.screenshot({ path: "target/e2e/line-author-session.png" });
  for (const theme of ["one-dark", "one-light"]) {
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    await panel.screenshot({ path: `target/e2e/line-author-session-${theme}.png` });
  }
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "one-dark";
  });

  // The edit's line opens the file at its first line: the cursor goes there.
  await number(1).scrollIntoViewIfNeeded();
  await told.click();
  await expect(view.locator(".cm-activeLineGutter", { hasText: /^39$/ })).toBeVisible();

  // Any other line has only its commit: the popover by the line says so.
  await number(10).scrollIntoViewIfNeeded();
  await number(10).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Who wrote this line?" }).click();
  const popover = page.getByRole("dialog", { name: "Who wrote this line?" });
  await expect(popover).toContainText("No session recorded for this line.");
  await expect(popover).toContainText(
    "4f2a9c1 · Ada Lovelace · 2d ago · Keep sessions in the store",
  );
  await page.screenshot({ path: "target/e2e/line-author-unknown.png" });
  await page.keyboard.press("Escape");
  await expect(popover).toBeHidden();
});

test("who wrote a line: at the narrowest panel each file's line fits, its name and numbers readable (28.22)", async ({
  page,
}) => {
  await page.addInitScript(() =>
    localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: 280 })),
  );
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Diff" }).click();
  for (const name of ["src", "auth"]) {
    await panel
      .getByRole("treeitem")
      .filter({ has: page.getByText(name, { exact: true }) })
      .click();
  }
  await panel.getByRole("treeitem", { name: /session\.ts/ }).click();
  const view = page.getByRole("region", { name: "src/auth/session.ts" });
  await view.getByRole("button", { name: "Edit", exact: true }).click();
  const line = view.locator(".cm-lineNumbers .cm-gutterElement").filter({ hasText: /^40$/ });
  await line.scrollIntoViewIfNeeded();
  await line.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Who wrote this line?" }).click();
  const rows = panel.locator(".session[data-focused] .session-file");
  await expect(rows).toHaveCount(5);

  // Another worktree's file: its worktree's name gives way first, cut with an ellipsis.
  const other = rows.filter({ hasText: "Cart.tsx" });
  const worktree = other.locator(".session-file-worktree");
  await expect(worktree).toHaveText("feat-checkout ·");
  expect(await worktree.evaluate((e) => e.scrollWidth > e.clientWidth)).toBe(true);
  await expect(other.locator(".session-file-numbers")).toHaveText(
    /^3 edits · 128 lines · \d+h ago$/,
  );
  // Every line fits its row; the numbers are whole; the file's name shows at least its first
  // 6 characters (the whole name when shorter).
  for (const r of await rows.all()) {
    const fits = await r.evaluate((e) => {
      const name = e.querySelector(".session-file-name") as HTMLElement;
      const numbers = e.querySelector(".session-file-numbers") as HTMLElement;
      const text = name.firstChild as Text;
      const range = document.createRange();
      range.setStart(text, 0);
      range.setEnd(text, Math.min(6, text.length));
      return {
        row: e.scrollWidth <= e.clientWidth,
        numbers:
          numbers.scrollWidth <= numbers.clientWidth &&
          numbers.getBoundingClientRect().right <= e.getBoundingClientRect().right,
        name: range.getBoundingClientRect().width <= name.clientWidth,
      };
    });
    expect(fits).toEqual({ row: true, numbers: true, name: true });
  }
  for (const theme of ["one-dark", "one-light"]) {
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    await panel.screenshot({ path: `target/e2e/line-author-session-280-${theme}.png` });
  }
});
