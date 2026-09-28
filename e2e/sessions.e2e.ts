import { expect, test } from "@playwright/test";

test("sessions panel: the shown worktree's sessions, one under the other", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "main", exact: true }).first().click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel
    .getByRole("tablist", { name: "Panel" })
    .getByRole("tab", { name: "Sessions" })
    .click();
  const list = panel.getByRole("list", { name: "Sessions" });
  const rows = list.locator(".session");
  await expect(rows.locator(".session-title .label")).toHaveText([
    "Checkout totals",
    "Untitled session",
  ]);
  // The virtualizer measures each row and places the next right below it.
  const box = async (n: number) => (await rows.nth(n).boundingBox()) ?? { y: 0, height: 0 };
  const [first, second] = [await box(0), await box(1)];
  expect(first.height).toBeGreaterThan(20);
  expect(second.y).toBeCloseTo(first.y + first.height, 1);
  const height = await list.evaluate((ul) => ul.getBoundingClientRect().height);
  expect(height).toBeCloseTo(first.height + second.height, 1);
});
