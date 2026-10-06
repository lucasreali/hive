import { expect, type Locator, type Page, test } from "@playwright/test";

/** The colour `var(<token>)` resolves to in the current theme, as computed styles report it. */
const token = (page: Page, name: string) =>
  page.evaluate((name) => {
    const probe = document.createElement("span");
    probe.style.color = `var(${name})`;
    document.body.append(probe);
    const color = getComputedStyle(probe).color;
    probe.remove();
    return color;
  }, name);

const color = (el: Locator) => el.evaluate((e) => getComputedStyle(e).color);

// 14.5: the search fields draw the app's clear button, never the WebView's blue one.
for (const theme of ["one-dark", "one-light"]) {
  test(`the search fields' clear button uses the theme's colours (${theme})`, async ({ page }) => {
    await page.goto("/");
    await page
      .getByRole("navigation", { name: "Projects" })
      .getByRole("button", { name: "fix-login" })
      .click();
    await page.evaluate((theme) => {
      document.documentElement.dataset.theme = theme;
    }, theme);
    expect(await token(page, "--bg")).toBe(
      theme === "one-light" ? "rgb(250, 250, 250)" : "rgb(40, 44, 51)",
    );
    const search = page.getByRole("searchbox", { name: "Find files" });
    const clear = page.getByRole("button", { name: "Clear search" });
    await expect(clear).toHaveCount(0);

    // The native button is not drawn: with text, the input's right end (where the WebView puts
    // it) looks as it does empty. Computed styles do not report that pseudo-element.
    await search.focus();
    const end = async () => {
      const box = await search.boundingBox();
      if (!box) throw new Error("search not visible");
      return page.screenshot({
        clip: { x: box.x + box.width - 24, y: box.y, width: 24, height: box.height },
      });
    };
    const empty = await end();
    await search.fill("session");
    expect(await end()).toEqual(empty);

    // The app's: muted, brighter on hover, the magnifying glass's size.
    await expect(clear).toBeVisible();
    await search.hover();
    expect(await color(clear)).toBe(await token(page, "--text-3"));
    await clear.hover();
    await expect.poll(() => color(clear)).toBe(await token(page, "--text"));
    const glass = await page.locator(".files-search-field > svg").first().boundingBox();
    const x = await clear.locator("svg").boundingBox();
    expect([x?.width, x?.height]).toEqual([glass?.width, glass?.height]);

    // A click empties the field and leaves the focus in it; Esc still clears.
    await clear.click();
    await expect(search).toHaveValue("");
    await expect(search).toBeFocused();
    await expect(clear).toHaveCount(0);
    await search.fill("session");
    await search.press("Escape");
    await expect(search).toHaveValue("");
  });
}
