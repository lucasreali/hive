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

/** How far anything in the shown view reaches past the view's right edge, in pixels. */
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
      return Math.round(worst);
    });

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
      await expect(panel.getByRole("region", { name: label })).toBeVisible();
      const place = await tab.boundingBox();
      // Every icon is inside the rail, as wide as it.
      expect(place?.width).toBeLessThanOrEqual(RAIL_WIDTH);
      expect(place?.x).toBeGreaterThanOrEqual(railBox?.x ?? 0);
      places.push(place?.y);
      expect(await sideways(page), label).toBe(0);
    }
    // In their order, with the separators between the groups.
    expect(places).toEqual([...places].sort((a, b) => (a ?? 0) - (b ?? 0)));
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
  // Safari's does, so only Chromium lands on Collapse.
  await page.keyboard.press("Tab");
  expect(await rail.evaluate((r) => r.contains(document.activeElement))).toBe(false);
  if (browserName === "chromium") {
    await expect(panel.getByTitle(keyLabel("Collapse (Ctrl+Shift+B)"))).toBeFocused();
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
