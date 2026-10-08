import { expect, test } from "@playwright/test";

// 15.3: the session usage as a ring in the status bar, its tooltip on hover and keyboard focus.
for (const theme of ["one-dark", "one-light"]) {
  test(`usage: the session ring and its tooltip in ${theme} (15.3)`, async ({ page }) => {
    await page.goto("/");
    // The mock's windows: 42% of the session, 41% of the week.
    const name = /^Session 42% · resets \d\d:\d\d, Week 41% · resets \w{3} \d\d:\d\d$/;
    const ring = page.locator(".statusbar").getByRole("button", { name });
    await expect(ring).toBeVisible();
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    const token = (name: string) =>
      page.evaluate(
        (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim(),
        name,
      );
    const style = (selector: string, property: string) =>
      page
        .locator(selector)
        .evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), property);
    const rgb = (hex: string) =>
      `rgb(${[1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16)).join(", ")})`;

    // The icons' size, a track and an arc to 42%, in the theme's colours.
    const box = await ring.locator("svg").boundingBox();
    expect([box?.width, box?.height]).toEqual([14, 14]);
    expect(await style(".usage-track", "stroke")).toBe(rgb(await token("--text-4")));
    expect(await style(".usage-arc", "stroke")).toBe(rgb(await token("--text-2")));
    await expect(ring.locator(".usage-arc")).toHaveAttribute("stroke-dasharray", "42 100");

    // Hidden until hovered; then above the bar, inside the window, in the panel colours.
    const tip = ring.locator(".usage-tip");
    await expect(tip).toBeHidden();
    await ring.hover();
    await expect(tip).toBeVisible();
    await expect(tip.locator("span")).toHaveText([
      /^Session 42% · resets \d\d:\d\d$/,
      /^Week 41% · resets \w{3} \d\d:\d\d$/,
    ]);
    const [shown, bar, view] = [
      await tip.boundingBox(),
      await page.locator(".statusbar").boundingBox(),
      page.viewportSize(),
    ];
    if (!shown || !bar || !view) throw new Error("no layout");
    expect(shown.y + shown.height).toBeLessThanOrEqual(bar.y);
    expect(shown.x + shown.width).toBeLessThanOrEqual(view.width);
    expect(await style(".usage-tip", "background-color")).toBe(rgb(await token("--panel")));
    expect(await style(".usage-tip", "color")).toBe(rgb(await token("--text")));
    await page.screenshot({ path: `target/e2e/usage-${theme}.png` });
    await page.mouse.move(0, 0);
    await expect(tip).toBeHidden();

    // From the keyboard too.
    await ring.focus();
    await expect(tip).toBeVisible();
    await ring.blur();
    await expect(tip).toBeHidden();
  });
}
