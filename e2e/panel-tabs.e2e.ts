import { expect, test } from "@playwright/test";

const LABELS = ["Files", "Diff", "Sessions", "PRs", "Actions"];

// 10.4: every view tab and the collapse button fit the right panel's bar at every width; a
// narrow panel shows the tabs' icons only.
for (const panelWidth of [280, 640]) {
  test(`right panel: the view tabs fit their bar at ${panelWidth} px`, async ({ page }) => {
    await page.addInitScript(
      (w) => localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: w })),
      panelWidth,
    );
    await page.goto("/");
    const tree = page.getByRole("navigation", { name: "Projects" });
    await tree.getByRole("button", { name: "refactor-auth" }).click();
    const panel = page.getByRole("complementary", { name: "Side panel" });
    const tabs = panel.getByRole("tablist", { name: "Panel" });
    await expect(tabs.getByRole("tab")).toHaveCount(LABELS.length);
    const fit = await panel.locator(":scope > .bar").evaluate((bar) => {
      const box = bar.getBoundingClientRect();
      const strip = bar.querySelector(".panel-views") as HTMLElement;
      const inside = (el: Element) => {
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.left >= box.left && r.right <= box.right;
      };
      return {
        width: box.width,
        strip: strip.scrollWidth <= strip.clientWidth,
        tabs: [...bar.querySelectorAll('[role="tab"]')].map(
          (t) => inside(t) && t.scrollWidth <= t.clientWidth,
        ),
        collapse: inside(bar.querySelector(".ghost") as Element),
      };
    });
    expect(Math.round(fit.width)).toBe(panelWidth - 1);
    expect(fit).toEqual({
      width: fit.width,
      strip: true,
      tabs: LABELS.map(() => true),
      collapse: true,
    });
    for (const label of LABELS) {
      const tab = tabs.getByRole("tab", { name: label });
      await expect(tab).toHaveAttribute("title", label);
      await tab.click();
      await expect(tab).toHaveAttribute("aria-selected", "true");
      await expect(panel.getByRole("region", { name: label })).toBeVisible();
      if (panelWidth === 640) await expect(tab).toHaveText(label);
      else await expect(tab.locator(".tab-label")).toBeHidden();
    }
  });
}
