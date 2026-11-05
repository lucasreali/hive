import { expect, type Locator, type Page, test } from "@playwright/test";
import { mod } from "./host";

// 23.11: the editor is its own stacking context, so its find bar (CodeMirror's panels, z-index
// 300 inside it) never covers a menu, a select list or their items.

/** Whether the point at the middle of `item` hits the item itself, not something over it. */
const uncovered = (item: Locator) =>
  item.evaluate((el) => {
    const box = el.getBoundingClientRect();
    const hit = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
    return hit !== null && el.contains(hit);
  });

/** Whether `a` and `b` overlap on screen. */
async function overlap(a: Locator, b: Locator) {
  const [x, y] = [await a.boundingBox(), await b.boundingBox()];
  if (!x || !y) return false;
  return x.x < y.x + y.width && y.x < x.x + x.width && x.y < y.y + y.height && y.y < x.y + x.height;
}

/** README.md of fix-login open with its find bar (⌘F), which is returned. */
async function findBar(page: Page) {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page
    .getByRole("region", { name: "Files" })
    .getByRole("treeitem", { name: "README.md" })
    .click();
  const view = page.getByRole("region", { name: "README.md" });
  await expect(view.locator(".cm-content")).toContainText("export const value = 1;");
  await view.locator(".cm-line").first().click();
  await page.keyboard.press(`${mod}+f`);
  await expect(view.locator(".cm-search input[name=search]")).toBeFocused();
  return view.locator(".cm-panels");
}

/** Every item of `menu` is on top where it overlaps `bar`, which it does. */
async function allUncovered(menu: Locator, items: Locator, bar: Locator) {
  await expect(menu).toBeVisible();
  expect(await overlap(menu, bar)).toBe(true);
  const count = await items.count();
  expect(count).toBeGreaterThan(1);
  for (let i = 0; i < count; i++) expect(await uncovered(items.nth(i))).toBe(true);
}

test('the "+" menu opens over the editor\'s find bar, every item clickable (23.11)', async ({
  page,
}) => {
  const bar = await findBar(page);
  await page.getByTitle("New terminal or file").click();
  const menu = page.getByRole("menu", { name: "New tab" });
  await allUncovered(menu, menu.getByRole("menuitem"), bar);
  await menu.getByRole("menuitem", { name: "Terminal" }).click();
  await expect(menu).toHaveCount(0);
  await expect(
    page.getByRole("tablist", { name: "Open terminals and files" }).getByRole("tab", {
      name: "fix-login",
    }),
  ).toHaveAttribute("aria-selected", "true");
});

test("a worktree's context menu over the find bar has every item on top (23.11)", async ({
  page,
}) => {
  const bar = await findBar(page);
  const box = await bar.boundingBox();
  if (!box) throw new Error("no find bar");
  // Opened where the find bar is (the row's own spot would be beside it, in the sidebar).
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .evaluate((row, at) => {
      const init = { bubbles: true, cancelable: true, clientX: at.x + 40, clientY: at.y + 4 };
      row.dispatchEvent(new MouseEvent("contextmenu", init));
    }, box);
  const menu = page.getByRole("menu", { name: "Worktree fix-login" });
  await allUncovered(menu, menu.getByRole("menuitem").and(page.locator(":enabled")), bar);
  await menu.getByRole("menuitem", { name: "Rename…" }).click();
  await expect(page.getByRole("dialog", { name: /Rename/ })).toBeVisible();
});

test("a select list over the find bar has every option on top (23.11)", async ({ page }) => {
  const bar = await findBar(page);
  const box = await bar.boundingBox();
  if (!box) throw new Error("no find bar");
  const space = page.getByRole("navigation", { name: "Projects" }).getByRole("combobox", {
    name: "Space",
  });
  await space.click();
  const list = page.getByRole("listbox");
  // A list stays under its field (in the sidebar): moved over the find bar, as a field beside
  // an editor would place it.
  await list.evaluate((el, at) => {
    el.style.left = `${at.x + 40}px`;
    el.style.top = `${at.y}px`;
    el.style.bottom = "auto";
  }, box);
  await allUncovered(list, list.getByRole("option"), bar);
  await list.getByRole("option", { name: "New space…" }).click();
  await expect(page.getByRole("dialog", { name: "New space" })).toBeVisible();
});
