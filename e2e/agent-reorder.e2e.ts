import { expect, type Page, test } from "@playwright/test";

/** A terminal in the worktree `row`, where `claude` runs after `cd`, labelled `badge`. */
async function agent(page: Page, row: string, badge: string, cd?: string) {
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: row, exact: true }).first().click();
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect(page.locator(".xterm-helper-textarea").last()).toBeFocused();
  for (const line of [cd && `cd ${cd}`, `hive badge ${badge}`, "claude"]) {
    if (!line) continue;
    await page.keyboard.type(line);
    await page.keyboard.press("Enter");
  }
  await expect(tree.locator(".tree-row.agent", { hasText: badge })).toBeVisible();
}

test("agents: dragged within their worktree, never into another", async ({ page }) => {
  await page.goto("/");
  await agent(page, "main", "one");
  await agent(page, "main", "two");
  await agent(page, "main", "far", ".claude/worktrees/fix-login");
  const rows = page.locator(".tree-row.agent");
  const badges = () => rows.locator(".label-badge").allTextContents();
  const row = (badge: string) => rows.filter({ hasText: badge });
  await expect.poll(badges).toEqual(["one", "two", "far"]);

  // Onto the upper edge of "one": "two" lands before it.
  await row("two").dragTo(row("one"), { targetPosition: { x: 40, y: 2 } });
  await expect.poll(badges).toEqual(["two", "one", "far"]);
  // Remembered for a reload, by session id.
  const saved = await page.evaluate(() =>
    JSON.parse(localStorage.getItem("hive.agentOrder") ?? ""),
  );
  expect(saved).toHaveLength(2);

  // Another worktree's agent refuses it.
  await row("two").dragTo(row("far"), { targetPosition: { x: 40, y: 20 } });
  await expect.poll(badges).toEqual(["two", "one", "far"]);
  await expect(page.locator("[data-drop]")).toHaveCount(0);

  // Alt+↓ from the keyboard.
  await row("two").locator(".row-main").focus();
  await page.keyboard.press("Alt+ArrowDown");
  await expect.poll(badges).toEqual(["one", "two", "far"]);
  await expect(row("two").locator(".row-main")).toBeFocused();
});
