import { expect, type Page, test } from "@playwright/test";

/** Opens a terminal in `worktree`, starts a fake agent there and moves it to `state`. */
async function agentIn(page: Page, worktree: string, state: string) {
  const tree = page.getByRole("navigation", { name: "Projects" });
  const agents = tree.locator(".tree-row.agent");
  const count = await agents.count();
  await tree.getByRole("button", { name: worktree, exact: true }).click();
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  const tab = page.getByRole("tablist").getByRole("tab", { name: worktree });
  await expect(tab).toHaveAttribute("aria-selected", "true");
  await page.keyboard.type("claude");
  await page.keyboard.press("Enter");
  await expect(agents).toHaveCount(count + 1);
  await page.keyboard.type(`state ${state}`);
  await page.keyboard.press("Enter");
}

test("inbox: the bell lists pending agents and past alerts; an item goes to its agent", async ({
  page,
}) => {
  await page.goto("/");
  const tabs = page.getByRole("tablist");
  const bell = page.locator(".pending-bell");
  const dot = bell.locator(".unread-dot");
  await expect(bell).toHaveAccessibleName("Notifications");
  await expect(dot).toHaveCount(0);

  await agentIn(page, "fix-login", "waiting_permission");
  await agentIn(page, "feat-checkout", "waiting_you");
  await expect(bell).toHaveAccessibleName("2 pending: notifications");
  await expect(dot).toBeVisible();

  // Keyboard only: Enter opens it on its first item and marks everything read.
  await bell.focus();
  await page.keyboard.press("Enter");
  const menu = page.getByRole("menu", { name: "Notifications" });
  await expect(dot).toHaveCount(0);
  const items = menu.getByRole("menuitem");
  await expect(items).toHaveText([
    /Claude\s*waiting for permission$/,
    /Claude\s*waiting for you$/,
    /Claude finished\s*\d+s ago$/,
    /Claude is waiting for permission\s*\d+s ago$/,
  ]);
  await expect(items.first()).toBeFocused();
  // Esc closes it.
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);

  // The oldest alert (fix-login's) goes to its agent's terminal.
  await bell.click();
  for (let i = 0; i < 3; i++) await page.keyboard.press("ArrowDown");
  await expect(items.nth(3)).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(menu).toHaveCount(0);
  await expect(tabs.getByRole("tab", { name: "fix-login" })).toHaveAttribute(
    "aria-selected",
    "true",
  );

  // F8 still goes to the next pending agent; a click outside closes the inbox.
  await page.keyboard.press("F8");
  await expect(tabs.getByRole("tab", { name: "feat-checkout" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await bell.click();
  await expect(menu).toBeVisible();
  await page.getByRole("navigation", { name: "Projects" }).click({ position: { x: 5, y: 5 } });
  await expect(menu).toHaveCount(0);
});
