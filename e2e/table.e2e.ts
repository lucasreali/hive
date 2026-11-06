import { expect, test } from "@playwright/test";

test("table: a CSV file opens as its table, with its header in view, and as text (23.6)", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("region", { name: "Files" });
  await panel.getByRole("treeitem", { name: "data" }).click();
  await panel.getByRole("treeitem", { name: "orders.csv" }).click();

  const view = page.getByRole("region", { name: "data/orders.csv" });
  const toggle = view.getByRole("button", { name: "Show as a table" });
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  const grid = view.getByRole("table", { name: "data/orders.csv" });
  const header = grid.getByRole("columnheader");
  await expect(header).toHaveText(["id", "customer", "total", "status"]);
  await expect(grid.getByRole("cell", { name: "Customer 1", exact: true })).toBeVisible();
  // The short last row is padded to the header's cells.
  const scroller = view.locator(".table-view");
  await scroller.evaluate((e) => e.scrollTo(0, e.scrollHeight));
  const last = grid.getByRole("row").filter({ hasText: "Late" });
  await expect(last.getByRole("cell")).toHaveText(["1041", "Late", "", ""]);
  // The header stays in view, at the top of the scroller.
  await expect(header.first()).toBeInViewport();
  const top = async (l: typeof scroller) => (await l.boundingBox())?.y;
  expect(await top(grid.getByRole("row").first())).toBe(await top(scroller));
  await page.screenshot({ path: "target/e2e/table.png" });

  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  await expect(grid).toHaveCount(0);
  await expect(view.locator(".cm-content")).toContainText("id,customer,total,status");
  await toggle.click();
  await expect(header.first()).toBeVisible();
});
