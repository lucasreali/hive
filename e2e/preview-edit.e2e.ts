import { expect, type Page, test } from "@playwright/test";
import { mod } from "./host";

/** Opens `name` from fix-login's Files, inside the folder `folder` when given. */
async function open(page: Page, name: string, path: string, folder?: string) {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("region", { name: "Files" });
  if (folder) await panel.getByRole("treeitem", { name: folder }).click();
  await panel.getByRole("treeitem", { name }).click();
  return page.getByRole("region", { name: path });
}

test("preview edit: a Markdown block opens as its source, alone, and saves (25.4)", async ({
  page,
}) => {
  const view = await open(page, "README.md", "README.md");
  const unsaved = page.getByRole("button", { name: "Close file README.md (unsaved changes)" });
  await view.locator(".cm-line").first().click();
  await page.keyboard.press(`${mod}+Home`);
  await page.keyboard.type("# Title\n\nOne paragraph\nof two lines.\n\n");
  await page.keyboard.press(`${mod}+s`);
  await expect(unsaved).toBeHidden();
  await view.getByRole("button", { name: "Show rendered Markdown" }).click();

  const rendered = view.locator(".markdown-view");
  const heading = rendered.getByRole("heading", { name: "Title" });
  await rendered.getByText("One paragraph").click();
  const box = rendered.getByRole("textbox");
  await expect(box).toHaveValue("One paragraph\nof two lines.");
  await expect(box).toBeFocused();
  await expect(heading).toBeVisible();
  await expect(rendered.locator("p").filter({ hasText: "One paragraph" })).toBeHidden();
  await page.screenshot({ path: "target/e2e/preview-edit-block.png" });

  await box.fill("**Bold** now.");
  await expect(unsaved).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(box).toBeHidden();
  await expect(rendered.locator("strong")).toHaveText("Bold");
  await expect(heading).toBeVisible();

  // A click outside renders it too; on a block, that one opens with the next click.
  await heading.click();
  await expect(box).toHaveValue("# Title");
  await rendered.locator("strong").click();
  await expect(box).toBeHidden();

  await page.keyboard.press(`${mod}+s`);
  await expect(unsaved).toBeHidden();
  await expect(rendered).toBeVisible();

  // The text holds the edit and every other line.
  await view.getByRole("button", { name: "Show rendered Markdown" }).click();
  const text = view.locator(".cm-content");
  await expect(text).toContainText("**Bold** now.");
  await expect(text).toContainText("# Title");
  await expect(text).toContainText("export const value = 1;");
  await expect(text).not.toContainText("One paragraph");
});

test("preview edit: a table's cells, rows and columns, then Save (25.4)", async ({ page }) => {
  const view = await open(page, "orders.csv", "data/orders.csv", "data");
  const unsaved = page.getByRole("button", { name: "Close file orders.csv (unsaved changes)" });
  const grid = view.getByRole("table", { name: "data/orders.csv" });
  const customer = grid.getByRole("cell", { name: "Customer 1", exact: true });

  await customer.dblclick();
  const input = view.getByRole("textbox", { name: "Cell value" });
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("Customer 1");
  await page.screenshot({ path: "target/e2e/preview-edit-cell.png" });
  await input.fill("Ann");
  await page.keyboard.press("Enter");
  await expect(input).toBeHidden();
  await expect(grid.getByRole("cell", { name: "Ann", exact: true })).toBeVisible();
  await expect(unsaved).toBeVisible();

  // The keys: the selected cell moves, Enter edits it, Escape drops the text.
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Enter");
  await expect(input).toHaveValue("0.00");
  await page.keyboard.type("nope");
  await page.keyboard.press("Escape");
  await expect(input).toBeHidden();
  await expect(grid.getByRole("cell", { name: "nope" })).toHaveCount(0);

  // A new row under the first one, and a column removed.
  const rows = grid.getByRole("row");
  await expect(rows.nth(1).getByRole("cell")).toHaveText(["1001", "Ann", "0.00", "paid"]);
  await grid.getByRole("cell", { name: "Ann", exact: true }).click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Table" });
  await menu.getByRole("menuitem", { name: "Insert Row Below" }).click();
  await expect(rows.nth(2).getByRole("cell")).toHaveText(["", "", "", ""]);
  await grid.getByRole("cell", { name: "Ann", exact: true }).click({ button: "right" });
  await menu.getByRole("menuitem", { name: "Delete Column" }).click();
  await expect(grid.getByRole("columnheader")).toHaveText(["id", "total", "status"]);
  await rows.nth(2).getByRole("cell").first().click({ button: "right" });
  await menu.getByRole("menuitem", { name: "Delete Row" }).click();
  await expect(rows.nth(2).getByRole("cell").first()).not.toHaveText("");

  await page.keyboard.press(`${mod}+s`);
  await expect(unsaved).toBeHidden();
  await expect(grid.getByRole("columnheader")).toHaveText(["id", "total", "status"]);

  const toggle = view.getByRole("button", { name: "Show as a table" });
  await toggle.click();
  const text = view.locator(".cm-content");
  await expect(text).toContainText("id,total,status");
  await expect(text).toContainText("1001,0.00,paid");
  await toggle.click();
  await expect(grid.getByRole("columnheader")).toHaveText(["id", "total", "status"]);
});

test("preview edit: ⌘S with a cell open saves its text; the text view can go back (25.4)", async ({
  page,
}) => {
  const view = await open(page, "orders.csv", "data/orders.csv", "data");
  const unsaved = page.getByRole("button", { name: "Close file orders.csv (unsaved changes)" });
  const grid = view.getByRole("table", { name: "data/orders.csv" });
  await grid.getByRole("cell", { name: "Customer 2", exact: true }).dblclick();
  await view.getByRole("textbox", { name: "Cell value" }).fill("Bo");
  // Unsaved, to the text and back: the table shows the buffer.
  await page.keyboard.press("Enter");
  await expect(unsaved).toBeVisible();
  const toggle = view.getByRole("button", { name: "Show as a table" });
  await toggle.click();
  await expect(view.locator(".cm-content")).toContainText("1002,Bo,");
  await toggle.click();
  await expect(grid.getByRole("cell", { name: "Bo", exact: true })).toBeVisible();

  await grid.getByRole("cell", { name: "Customer 3", exact: true }).dblclick();
  await view.getByRole("textbox", { name: "Cell value" }).fill("Cy");
  await page.keyboard.press(`${mod}+s`);
  await expect(unsaved).toBeHidden();
  await expect(grid.getByRole("cell", { name: "Cy", exact: true })).toBeVisible();
  await toggle.click();
  await expect(view.locator(".cm-content")).toContainText("1003,Cy,");
});
