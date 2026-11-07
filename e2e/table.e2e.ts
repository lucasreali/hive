import { expect, test } from "@playwright/test";
import { mod } from "./host";

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

test("table: ⌘F finds in the cells, ↓ scrolls a match far down into view (24.12)", async ({
  page,
}) => {
  // A narrow window: the table's last column is out of view to the right.
  await page.setViewportSize({ width: 900, height: 700 });
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  const panel = page.getByRole("region", { name: "Files" });
  await panel.getByRole("treeitem", { name: "data" }).click();
  await panel.getByRole("treeitem", { name: "orders.csv" }).click();
  const view = page.getByRole("region", { name: "data/orders.csv" });
  const grid = view.getByRole("table", { name: "data/orders.csv" });
  const scroller = view.locator(".table-view");
  await grid.getByRole("cell", { name: "Customer 1", exact: true }).click();
  await expect(scroller).toBeFocused();

  await page.keyboard.press(`${mod}+f`);
  const bar = view.getByRole("search", { name: "Find in the table" });
  const field = bar.getByRole("textbox", { name: "Find" });
  await expect(field).toBeFocused();
  await expect(bar.getByRole("textbox", { name: "Replace" })).toHaveCount(0);
  const status = bar.locator(".find-status");
  const current = view.locator(".table-match-current");
  // The current match, whole in view and not under the header kept in view: its row and text.
  const shown = async () => {
    await expect(current).toHaveCount(1);
    await expect(current).toBeInViewport({ ratio: 1 });
    const hit = await current.evaluate((e) => {
      const r = e.getBoundingClientRect();
      return document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) === e;
    });
    expect(hit).toBe(true);
    return [
      await current.locator("xpath=..").getAttribute("aria-rowindex"),
      await current.textContent(),
    ];
  };

  await page.keyboard.type("paid");
  await expect(status).toHaveText("40 matches");
  await expect(grid.getByRole("cell", { name: "paid", exact: true }).first()).toHaveClass(
    /table-match/,
  );
  // The status column is out of view to the right, until ↓ makes its first cell current.
  const statusHeader = grid.getByRole("columnheader", { name: "status" });
  await expect(statusHeader).not.toBeInViewport({ ratio: 1 });
  await bar.getByRole("button", { name: "Next match" }).click();
  await expect(status).toHaveText("1 of 40");
  expect(await shown()).toEqual(["2", "paid"]);

  // A match far down, out of the rows rendered: ↓ scrolls it into view.
  await bar.getByRole("button", { name: "Use regular expression" }).click();
  await field.fill("^10[34]0$");
  await expect(status).toHaveText("2 matches");
  await expect(grid.getByRole("cell", { name: "1040", exact: true })).toHaveCount(0);
  await field.press("Enter");
  expect(await shown()).toEqual(["31", "1030"]);
  await field.press("Enter");
  await expect(status).toHaveText("2 of 2");
  expect(await shown()).toEqual(["41", "1040"]);
  await page.screenshot({ path: "target/e2e/table-find.png" });
  // ↓ wraps to the first, back up.
  await field.press("Enter");
  await expect(status).toHaveText("1 of 2");
  expect(await shown()).toEqual(["31", "1030"]);

  await page.keyboard.press("Escape");
  await expect(bar).toHaveCount(0);
  await expect(scroller).toBeFocused();
});
