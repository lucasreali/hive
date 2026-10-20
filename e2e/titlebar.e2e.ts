import { expect, test } from "@playwright/test";

// 17.15: the macOS title bar as WKWebView's engine draws it, in both themes: room on the left
// for the native traffic lights, the brand, the bell, no window buttons of Hive's own. A Mac
// user agent turns the macOS layout on (window.ts `isMac`); this spec presses no keys.
test.skip(({ browserName }) => browserName !== "webkit", "a WebKit check");
test.use({
  userAgent:
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)",
});

for (const theme of ["one-dark", "one-light"]) {
  test(`the macOS title bar (${theme})`, async ({ page }) => {
    await page.goto("/");
    // Once the settings are in (they set the theme), the theme under test.
    const tree = page.getByRole("navigation", { name: "Projects" });
    await expect(tree.getByRole("button", { name: "fix-login" })).toBeVisible();
    await page.evaluate((theme) => {
      document.documentElement.dataset.theme = theme;
    }, theme);
    const bar = page.locator("header.titlebar");
    await expect(bar).toHaveCSS(
      "background-color",
      theme === "one-light" ? "rgb(220, 220, 221)" : "rgb(59, 65, 77)",
    );
    await expect(bar).toHaveAttribute("data-mac", "true");
    await expect(bar.getByRole("button", { name: "Close" })).toHaveCount(0);
    // The brand starts after the traffic lights' 70 px.
    const box = await bar.locator(".titlebar-brand").boundingBox();
    expect(box?.x).toBe(70);
    // The bell's count follows the mock agents' replay, so it is left out of the picture.
    await expect(bar).toHaveScreenshot(`titlebar-${theme}.png`, {
      mask: [bar.locator(".pending-bell")],
    });
  });
}
