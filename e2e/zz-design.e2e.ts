// THROWAWAY (25.3 design): screenshots and fit measurements. Never merged.
import { mkdirSync, writeFileSync } from "node:fs";
import { expect, type Page, test } from "@playwright/test";

const OUT = "/var/tmp/hive-25.3-design";
const LAYOUTS = ["baseline", "strip", "more", "rail", "menu"] as const;
const ALL = ["Files", "Diff", "Sessions", "PRs", "Actions", "Scripts"];
mkdirSync(OUT, { recursive: true });

async function open(page: Page, layout: string, width: number, flags: object = {}) {
  await page.goto("/");
  await page.evaluate(
    ([w, d]) => {
      localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: w }));
      localStorage.setItem("hive.design", JSON.stringify(d));
    },
    [width, { layout: layout === "baseline" ? undefined : layout, ...flags }] as const,
  );
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "refactor-auth" })
    .click();
  await page.waitForTimeout(250);
}
const panel = (page: Page) => page.getByRole("complementary", { name: "Side panel" });
const shot = async (page: Page, name: string, full = false) => {
  await page.waitForTimeout(120);
  const box = await panel(page).boundingBox();
  if (!box) throw new Error("no panel");
  await page.screenshot({
    path: `${OUT}/${name}.png`,
    ...(full ? {} : { clip: { x: box.x - 40, y: 0, width: box.width + 40, height: 420 } }),
  });
};
async function pick(page: Page, layout: string, label: string) {
  const p = panel(page);
  const tab = p.getByRole("tab", { name: new RegExp(`^${label}`) });
  if (await tab.count()) return tab.click();
  if (layout === "more") await p.locator(".d-more-button").click();
  if (layout === "menu") await p.locator(".d-picker").click();
  await page.getByRole("menuitem", { name: new RegExp(`^${label}`) }).click();
}
/** Everything in the switcher inside the bar, with nothing clipped. */
const fit = (page: Page) =>
  panel(page).evaluate((aside) => {
    const a = aside.getBoundingClientRect();
    const bar = aside.querySelector(":scope > .bar") as HTMLElement;
    const items = [
      ...aside.querySelectorAll('[role="tab"], .d-more-button, .d-picker, .ghost'),
    ].filter((e) => e.closest(".bar, .d-rail")) as HTMLElement[];
    const out = items.filter((e) => {
      const r = e.getBoundingClientRect();
      return (
        r.width <= 0 ||
        r.left < a.left - 0.5 ||
        r.right > a.right + 0.5 ||
        e.scrollWidth > e.clientWidth + 1
      );
    });
    const coll = bar.querySelector(".ghost")?.getBoundingClientRect();
    const overlap = items.some((e) => {
      if (e.classList.contains("ghost")) return false;
      const r = e.getBoundingClientRect();
      return !!coll && !e.closest(".d-rail") && r.right > coll.left + 0.5;
    });
    const content = aside.querySelector(".panel-view")?.getBoundingClientRect().width ?? 0;
    return {
      clipped: out.length,
      overlap,
      tabs: aside.querySelectorAll('[role="tab"]').length,
      labels: [...aside.querySelectorAll('[role="tab"] .tab-label')].filter(
        (e) => (e as HTMLElement).offsetWidth > 0,
      ).length,
      content: Math.round(content),
    };
  });

test("rail icons only, wide", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium");
  await open(page, "rail", 640, { iconsOnly: true });
  await shot(page, "rail-640-iconsonly-files");
});

for (const layout of LAYOUTS) {
  for (const width of [280, 380, 640]) {
    test(`shots ${layout} ${width}`, async ({ page, browserName }) => {
      test.skip(browserName !== "chromium");
      await open(page, layout, width);
      await shot(page, `${layout}-${width}-files`);
      await pick(page, layout, "Sessions");
      await shot(page, `${layout}-${width}-sessions`);
      if (width === 280 && (layout === "more" || layout === "menu")) {
        await panel(page)
          .locator(layout === "more" ? ".d-more-button" : ".d-picker")
          .click();
        await shot(page, `${layout}-${width}-menu-open`);
      }
    });
  }
  test(`full window ${layout}`, async ({ page, browserName }) => {
    test.skip(browserName !== "chromium");
    await open(page, layout, 380);
    await shot(page, `${layout}-380-window`, true);
  });
  test(`future view and badges ${layout}`, async ({ page, browserName }) => {
    test.skip(browserName !== "chromium" || layout === "baseline");
    await open(page, layout, 280, { extra: true, badges: true });
    await pick(page, layout, "Sessions");
    await shot(page, `${layout}-280-7views-badges`);
  });
  test(`folder ${layout}`, async ({ page, browserName }) => {
    test.skip(browserName !== "chromium");
    await open(page, layout, 280);
    await page
      .getByTitle(/Add project/)
      .first()
      .click();
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
    await page.waitForTimeout(200);
    await shot(page, `${layout}-280-folder`);
  });
  test(`probe ${layout}`, async ({ page, browserName }) => {
    test.skip(browserName !== "chromium");
    test.setTimeout(120_000);
    await open(page, layout, 280);
    const p = panel(page);
    const report: Record<string, unknown> = {};
    // Free room in the bar at the narrowest, with the widest label selected.
    await pick(page, layout, "Sessions");
    report.free280 = await p.evaluate((a) => {
      const bar = a.querySelector(":scope > .bar") as HTMLElement;
      const coll = bar.querySelector(".ghost")?.getBoundingClientRect().left ?? 0;
      const right = Math.max(
        ...[...bar.querySelectorAll('[role="tab"], .d-more-button, .d-picker')].map(
          (e) => e.getBoundingClientRect().right,
        ),
      );
      return Math.round(coll - right);
    });
    // How far the other tabs move when the selection changes.
    const lefts: Record<string, Record<string, number>> = {};
    if (await p.getByRole("tab").count()) {
      for (const label of ALL) {
        const tab = p.getByRole("tab", { name: new RegExp(`^${label}`) });
        if (!(await tab.count())) continue;
        await tab.click();
        await page.waitForTimeout(30);
        lefts[label] = await p.evaluate((a) =>
          Object.fromEntries(
            [...a.querySelectorAll('[role="tab"]')].map((t) => [
              t.getAttribute("title"),
              Math.round(t.getBoundingClientRect().left),
            ]),
          ),
        );
      }
    }
    const moved: Record<string, number> = {};
    for (const label of ALL) {
      const xs = Object.values(lefts)
        .map((l) => l[label])
        .filter((x): x is number => x !== undefined);
      if (xs.length) moved[label] = Math.max(...xs) - Math.min(...xs);
    }
    report.moved = moved;
    // What overflows sideways in each view, at the narrowest.
    const over: Record<string, number> = {};
    for (const label of ALL) {
      await pick(page, layout, label);
      await page.waitForTimeout(150);
      over[label] = await p.evaluate((a) => {
        const v = a.querySelector(".panel-view") as HTMLElement;
        const box = v.getBoundingClientRect();
        let worst = 0;
        for (const e of v.querySelectorAll("*")) {
          const r = e.getBoundingClientRect();
          if (r.width > 0) worst = Math.max(worst, r.right - box.right);
        }
        return Math.round(worst);
      });
      if (layout === "rail") await shot(page, `rail-280-view-${label.toLowerCase()}`);
    }
    report.overflow = over;
    // The names a screen reader gets.
    report.aria = await p
      .locator(":scope > .bar, :scope > .d-rail")
      .evaluateAll((els) => els.map((e) => e.getAttribute("class")));
    report.snapshot = await p.ariaSnapshot();
    // The keyboard: Tab into the switcher, then the arrows.
    await p.getByRole("button", { name: "Resize the side panel" }).count();
    await p.locator('[role="separator"]').focus();
    const stops: string[] = [];
    for (let i = 0; i < 8; i++) {
      await page.keyboard.press("Tab");
      const s = await page.evaluate(() => {
        const e = document.activeElement as HTMLElement;
        return e.getAttribute("aria-label") ?? e.getAttribute("title") ?? e.tagName;
      });
      stops.push(s);
      if (s.startsWith("Collapse")) break;
    }
    report.tabStops = stops;
    writeFileSync(`${OUT}/probe-${layout}.json`, JSON.stringify(report, null, 1));
  });
  test(`measure ${layout}`, async ({ page, browserName }) => {
    test.setTimeout(240_000);
    const rows: unknown[] = [];
    for (const extra of [false, true]) {
      if (extra && layout === "baseline") continue;
      for (const width of [280, 320, 360, 380, 400, 420, 440, 460, 480, 500, 520, 560, 640]) {
        await open(page, layout, width, { extra, badges: extra });
        for (const label of [...ALL, ...(extra ? ["Notes"] : [])]) {
          await pick(page, layout, label);
          await page.waitForTimeout(30);
          rows.push({ extra, width, label, ...(await fit(page)) });
        }
      }
    }
    writeFileSync(`${OUT}/measure-${layout}-${browserName}.json`, JSON.stringify(rows));
  });
}
