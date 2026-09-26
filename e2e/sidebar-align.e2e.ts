import { expect, type Locator, test } from "@playwright/test";

test("sidebar: a collapsed worktree's rollup lines up with the badges' right column", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const row = (name: string) => tree.locator(".tree-row.worktree", { hasText: name });
  const box = async (l: Locator) => {
    const b = await l.boundingBox();
    if (!b) throw new Error("not visible");
    return b;
  };
  const right = async (l: Locator) => {
    const b = await box(l);
    return b.x + b.width;
  };

  // Agents in shop's main (no badges) and in fix-login (badges ↑3 ↓1 ●2).
  const newTerminal = page.getByTitle("New terminal (Ctrl+Shift+T)");
  await tree.getByRole("button", { name: "main", exact: true }).first().click();
  await newTerminal.click();
  await page.keyboard.type("claude");
  await page.keyboard.press("Enter");
  await tree.getByRole("button", { name: "main", exact: true }).first().click();
  await newTerminal.click();
  await page.keyboard.type("cd .claude/worktrees/fix-login");
  await page.keyboard.press("Enter");
  await page.keyboard.type("claude");
  await page.keyboard.press("Enter");
  await expect(tree.locator(".tree-row.agent")).toHaveCount(2);

  await tree.getByRole("button", { name: "Collapse main" }).first().click();
  await tree.getByRole("button", { name: "Collapse fix-login" }).click();
  // Away from the rows, so no "+" shows.
  await page.mouse.move(900, 600);

  // No badges: the rollup ends where another row's badges end.
  const mainRollup = row("main").first().locator(".state-icon");
  const badges = row("feat-checkout").locator(".health");
  expect(await right(mainRollup)).toBe(await right(badges));
  // Badges and rollup: the badges first, the rollup in the right column, no overlap.
  const fixRollup = row("fix-login").locator(".state-icon");
  expect(await right(fixRollup)).toBe(await right(badges));
  expect((await box(fixRollup)).x).toBeGreaterThanOrEqual(
    await right(row("fix-login").locator(".health")),
  );

  // Hovered, the "+" comes after the rollup without covering it.
  await row("fix-login").hover();
  const newChat = row("fix-login").locator(".new-chat");
  await expect(newChat).toBeVisible();
  expect((await box(newChat)).x).toBeGreaterThanOrEqual(await right(fixRollup));

  // A collapsed project's rollup takes the same column.
  await tree.getByRole("button", { name: "Collapse shop" }).click();
  await page.mouse.move(900, 600);
  // Focus in a project row still shows its "New worktree" button (7.12 removes it).
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  const shopRollup = tree.locator(".tree-row.project", { hasText: "shop" }).locator(".state-icon");
  expect(await right(shopRollup)).toBe(await right(row("refactor-auth").locator(".health")));
});
