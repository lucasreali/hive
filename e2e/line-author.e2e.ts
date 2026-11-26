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
  await expect(row.locator(".session-edit")).toHaveText(
    /^Edit · \d+h ago · lines 39–41 · you changed it before accepting$/,
  );
  const files = row.getByRole("list", { name: "Files changed" });
  await expect(files.locator(".session-file[data-focused] .session-file-path")).toHaveText(
    "src/auth/session.ts",
  );
  await page.screenshot({ path: "target/e2e/line-author-session.png" });

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
