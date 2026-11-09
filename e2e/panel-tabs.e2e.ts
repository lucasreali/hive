import { expect, type Page, test } from "@playwright/test";
import { keyLabel, mod } from "./host";

const LABELS = ["Files", "Diff", "Scripts", "PRs", "Actions", "Sessions"];
const RAIL_WIDTH = 36;

const rightPanel = (page: Page) => page.getByRole("complementary", { name: "Side panel" });

async function openAt(page: Page, panelWidth: number, worktree = "fix-login") {
  await page.addInitScript(
    (w) => localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: w })),
    panelWidth,
  );
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: worktree, exact: true }).first().click();
}

/**
 * How far the shown view spills sideways, in pixels: a box past the view's right edge, or a text
 * past its own box (boxes alone missed "Preview" spilling out of a 24 px button). A text inside
 * a box that clips it is shortened on purpose, not spilled.
 */
const sideways = (page: Page) =>
  rightPanel(page)
    .locator(".panel-view")
    .evaluate((view) => {
      const edge = view.getBoundingClientRect().right;
      let worst = 0;
      for (const e of view.querySelectorAll("*")) {
        const r = e.getBoundingClientRect();
        if (r.width > 0) worst = Math.max(worst, r.right - edge);
      }
      const clipped = (from: Element) => {
        for (let e: Element | null = from; e && e !== view; e = e.parentElement) {
          // Not a scroller: it holds the view's lists, and clips nothing on purpose.
          if (["hidden", "clip"].includes(getComputedStyle(e).overflowX)) return true;
        }
        return false;
      };
      const texts = document.createTreeWalker(view, NodeFilter.SHOW_TEXT);
      for (let n = texts.nextNode(); n; n = texts.nextNode()) {
        const owner = n.parentElement as Element;
        if (!n.textContent?.trim() || clipped(owner)) continue;
        const range = document.createRange();
        range.selectNodeContents(n);
        const text = range.getBoundingClientRect();
        if (text.width > 0) {
          worst = Math.max(
            worst,
            text.right - owner.getBoundingClientRect().right,
            text.right - edge,
          );
        }
      }
      return Math.round(worst);
    });

/** What each view shows once the mock's data has arrived (it answers on a later tick). */
const CONTENT: Record<string, string> = {
  Files: '[role="treeitem"]',
  Diff: '[role="treeitem"]',
  Scripts: ".scripts ul li",
  PRs: ".pull",
  Actions: ".session.run",
  Sessions: '.sessions-count:has-text("shown")',
};

// 25.3: the rail is the same at every width, the ones that clipped the old bar (420, 440) too;
// nothing in any view reaches past the panel.
for (const panelWidth of [280, 320, 380, 400, 420, 440, 460, 520, 640]) {
  test(`right panel: the rail and every view fit at ${panelWidth} px`, async ({ page }) => {
    await openAt(page, panelWidth);
    const panel = rightPanel(page);
    const rail = panel.getByRole("tablist", { name: "Panel" });
    await expect(rail.getByRole("tab")).toHaveCount(LABELS.length);
    const box = await panel.boundingBox();
    const railBox = await rail.boundingBox();
    expect(Math.round(box?.width ?? 0)).toBe(panelWidth);
    expect(railBox?.width).toBe(RAIL_WIDTH);
    expect(Math.round((railBox?.x ?? 0) + RAIL_WIDTH)).toBe(Math.round((box?.x ?? 0) + panelWidth));
    const places: (number | undefined)[] = [];
    for (const label of LABELS) {
      const tab = rail.getByRole("tab", { name: label });
      await expect(tab).toHaveAttribute("title", label);
      await tab.click();
      await expect(tab).toHaveAttribute("aria-selected", "true");
      await expect(panel.locator(".panel-title")).toHaveText(label);
      const region = panel.getByRole("region", { name: label });
      await expect(region).toBeVisible();
      await expect(region.locator(CONTENT[label] as string).first()).toBeVisible();

      // The bar: as wide as the panel leaves it, its title and Collapse inside and apart.
      const bar = await panel.locator(":scope > .bar").evaluate((b) => {
        const box = b.getBoundingClientRect();
        const title = (b.querySelector(".panel-title") as HTMLElement).getBoundingClientRect();
        const collapse = (b.querySelector(".ghost") as HTMLElement).getBoundingClientRect();
        return {
          width: Math.round(box.width),
          unscrolled: b.scrollWidth <= b.clientWidth,
          title: title.width > 0 && title.left >= box.left && title.right <= collapse.left,
          collapse: collapse.width > 0 && collapse.right <= box.right,
        };
      });
      expect(bar, label).toEqual({
        // The panel's border and the rail's 36 px.
        width: panelWidth - 1 - RAIL_WIDTH,
        unscrolled: true,
        title: true,
        collapse: true,
      });

      // Every icon fills the rail, its own 1 px border aside.
      const place = (await tab.boundingBox()) as { x: number; y: number; width: number };
      expect(place.width).toBe(RAIL_WIDTH - 1);
      expect(place.x).toBe((railBox?.x ?? 0) + 1);
      expect(place.x + place.width).toBe((railBox?.x ?? 0) + RAIL_WIDTH);
      places.push(place.y);
      await expect.poll(() => sideways(page), { message: label }).toBe(0);
    }
    // In their order, each below the last, with the separators between the groups.
    for (let i = 1; i < places.length; i++) {
      expect(places[i] ?? 0).toBeGreaterThan(places[i - 1] ?? 0);
    }
    await expect(rail.locator(".panel-rail-separator")).toHaveCount(2);
    // Icons stay put whichever is selected.
    await rail.getByRole("tab", { name: "Files" }).click();
    expect((await rail.getByRole("tab", { name: "Sessions" }).boundingBox())?.y).toBe(places[5]);
  });
}

test("the rail: one Tab stop, then ↑/↓, Home and End, on the keyboard", async ({
  page,
  browserName,
}) => {
  await openAt(page, 380);
  const panel = rightPanel(page);
  const rail = panel.getByRole("tablist", { name: "Panel" });
  await panel.getByRole("separator", { name: "Resize the side panel" }).focus();
  await page.keyboard.press("Tab");
  await expect(rail.getByRole("tab", { name: "Files" })).toBeFocused();
  // The next Tab leaves the rail. WebKit's Tab skips a button with no tabindex by default, as
  // Safari's does, so it goes past Collapse to the view's first field.
  await page.keyboard.press("Tab");
  if (browserName === "chromium") {
    await expect(panel.getByTitle(keyLabel("Collapse (Ctrl+Shift+B)"))).toBeFocused();
  } else {
    await expect(panel.getByLabel("Find files")).toBeFocused();
  }
  await rail.getByRole("tab", { name: "Files" }).focus();
  await page.keyboard.press("ArrowDown");
  const diff = rail.getByRole("tab", { name: "Diff" });
  await expect(diff).toBeFocused();
  await expect(diff).toHaveAttribute("aria-selected", "true");
  await expect(panel.locator(".panel-title")).toHaveText("Diff");
  await page.keyboard.press("End");
  await expect(rail.getByRole("tab", { name: "Sessions" })).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(rail.getByRole("tab", { name: "Files" })).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await expect(rail.getByRole("tab", { name: "Sessions" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
});

test("the rail follows the sidebar's badges and the palette's commands", async ({ page }) => {
  await openAt(page, 280);
  const panel = rightPanel(page);
  const rail = panel.getByRole("tablist", { name: "Panel" });
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.locator(".tree-row.worktree", { hasText: "fix-login" }).locator(".pull-badge").click();
  await expect(rail.getByRole("tab", { name: "PRs" })).toHaveAttribute("aria-selected", "true");
  await expect(panel.locator(".panel-title")).toHaveText("PRs");

  // The palette shows a view, opening the panel first when it is closed.
  await panel.getByTitle(keyLabel("Collapse (Ctrl+Shift+B)")).click();
  await expect(panel).toHaveCount(0);
  await page.keyboard.press(`${mod}+Shift+P`);
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await expect(palette.getByRole("textbox", { name: /Search commands/ })).toBeFocused();
  await page.keyboard.type("Show Diff");
  await page.keyboard.press("Enter");
  await expect(rail.getByRole("tab", { name: "Diff" })).toHaveAttribute("aria-selected", "true");
  await expect(panel.locator(".panel-title")).toHaveText("Diff");
});

test("Actions' toolbar reads whole at the panel's narrowest", async ({ page }) => {
  await openAt(page, 280);
  const panel = rightPanel(page);
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Actions" }).click();
  const updated = panel.locator(".pulls-updated");
  await expect(updated).toContainText("Updated");
  // Not cut: the text's own width fits its box.
  expect(await updated.evaluate((e) => e.scrollWidth <= e.clientWidth)).toBe(true);
  const refresh = (await panel.getByTitle("Refresh").boundingBox()) as { x: number; width: number };
  const view = (await panel.locator(".panel-view").boundingBox()) as { x: number; width: number };
  expect(refresh.x + refresh.width).toBeLessThanOrEqual(view.x + view.width);
  const box = (await updated.boundingBox()) as { x: number; width: number };
  expect(box.x + box.width).toBeLessThanOrEqual(refresh.x);
});

test("a long repository name shortens in the PRs bar; 'Updated …' stays whole", async ({
  page,
}) => {
  await openAt(page, 280);
  const panel = rightPanel(page);
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "PRs" }).click();
  const bar = panel.locator(".pulls-bar");
  await expect(bar.locator(".pulls-updated")).toContainText("Updated");
  const fit = await bar.evaluate((b) => {
    const link = b.querySelector(".link") as HTMLElement;
    link.textContent = "a-long-organisation-name/a-repository-with-a-very-long-name-indeed";
    const box = b.getBoundingClientRect();
    const updated = b.querySelector(".pulls-updated") as HTMLElement;
    const range = document.createRange();
    range.selectNodeContents(updated);
    const text = range.getBoundingClientRect();
    const own = updated.getBoundingClientRect();
    const refresh = (b.querySelector(".ghost.icon") as HTMLElement).getBoundingClientRect();
    return {
      shortened: link.scrollWidth > link.clientWidth,
      nameFirst: link.getBoundingClientRect().right <= own.left,
      wholeText: text.left >= own.left && text.right <= own.right,
      beforeRefresh: own.right <= refresh.left,
      refreshInside: refresh.right <= box.right,
      unscrolled: b.scrollWidth <= b.clientWidth,
    };
  });
  expect(fit).toEqual({
    shortened: true,
    nameFirst: true,
    wholeText: true,
    beforeRefresh: true,
    refreshInside: true,
    unscrolled: true,
  });
});

// Linux WebKit's search field asks for more room than macOS' does: a longer count takes the
// field down to where every engine must give way.
test("the Sessions search field gives way at the panel's narrowest", async ({ page }) => {
  await openAt(page, 280);
  const panel = rightPanel(page);
  await panel
    .getByRole("tablist", { name: "Panel" })
    .getByRole("tab", { name: "Sessions" })
    .click();
  await expect(panel.locator(".sessions-count")).toContainText("shown");
  const fit = await panel.locator(".sessions-search").evaluate((row) => {
    (row.querySelector(".sessions-count") as HTMLElement).textContent = "12345 of 67890 shown";
    const box = row.getBoundingClientRect();
    const field = (row.querySelector(".files-search-field") as HTMLElement).getBoundingClientRect();
    const refresh = (row.querySelector(".ghost") as HTMLElement).getBoundingClientRect();
    return { field: field.width > 0, inside: refresh.right <= box.right };
  });
  expect(fit).toEqual({ field: true, inside: true });
});

test("the Scripts view's ports read whole at the panel's narrowest", async ({ page }) => {
  await openAt(page, 280);
  const panel = rightPanel(page);
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Scripts" }).click();
  const row = panel.locator(".scripts-ports");
  await expect(row).toContainText("Ports");
  // Each item is on its own lines inside the row, none cut and none over another.
  const fit = await row.evaluate((r) => {
    const box = r.getBoundingClientRect();
    const items = [...r.children].map((c) => c.getBoundingClientRect());
    return {
      inside: items.every(
        (i) =>
          i.left >= box.left && i.right <= box.right && i.top >= box.top && i.bottom <= box.bottom,
      ),
      apart: items.every((a, n) =>
        items
          .slice(n + 1)
          .every((b) => a.right <= b.left || b.right <= a.left || a.bottom <= b.top),
      ),
      lines: [...r.querySelectorAll("span")].every((s) => s.getClientRects().length === 1),
      // "Preview" is as wide as its word.
      word: [...r.querySelectorAll("button")].every((b) => b.scrollWidth <= b.clientWidth),
    };
  });
  expect(fit).toEqual({ inside: true, apart: true, lines: true, word: true });
});

test("a folder project's rail has its two icons", async ({ page }) => {
  await page.addInitScript(() =>
    localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: 280 })),
  );
  await page.goto("/");
  await page.getByTitle(keyLabel("Add project (Ctrl+Shift+O)")).click();
  const dialog = page.getByRole("dialog", { name: "Add project" });
  const field = dialog.getByLabel("Folder", { exact: true });
  await field.fill("/home/user/projects/notes");
  await expect(dialog.getByRole("button", { name: /^Add project/ })).toBeEnabled();
  await field.press("Enter");
  await expect(dialog).toHaveCount(0);
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "notes", exact: true })
    .click();
  const panel = rightPanel(page);
  const rail = panel.getByRole("tablist", { name: "Panel" });
  await expect(rail.getByRole("tab")).toHaveCount(2);
  await rail.getByRole("tab", { name: "Sessions" }).click();
  await expect(panel.locator(".panel-title")).toHaveText("Sessions");
  expect(await sideways(page)).toBe(0);
});
