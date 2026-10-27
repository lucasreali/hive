import { expect, type Locator, type Page, test } from "@playwright/test";
import { mod } from "./host";

// D18.10 (a, b, d): a window that tiles, a System theme and the launch colour, and macOS' own
// scrollbars and cursor.

const MAC_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)";
const LINUX_UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

const box = async (l: Locator) => {
  const b = await l.boundingBox();
  if (!b) throw new Error("not visible");
  return b;
};

/** Waits for the mock service's settings and projects, then opens `n` terminals. */
async function terminals(page: Page, n: number) {
  const tree = page.getByRole("navigation", { name: "Projects" });
  await expect(tree.getByRole("button", { name: "fix-login" })).toBeVisible();
  for (let i = 0; i < n; i++) {
    await tree.getByRole("button", { name: "main", exact: true }).first().click();
    await page.getByTitle("New terminal or file").click();
    await page.getByRole("menuitem", { name: "Terminal" }).click();
  }
}

test.describe("half a 13-inch screen (18.32)", () => {
  test.skip(({ browserName }) => browserName !== "webkit", "a WebKit check");
  test.use({ userAgent: MAC_UA, viewport: { width: 700, height: 800 } });

  test("at 700 px the sidebar, the terminals and the panel sit side by side", async ({ page }) => {
    await page.goto("/");
    await terminals(page, 6);
    const sidebar = await box(page.getByRole("navigation", { name: "Projects" }));
    const area = await box(page.getByRole("region", { name: "Terminals" }));
    const panel = await box(page.getByRole("complementary", { name: "Side panel" }));
    expect(sidebar.x).toBe(0);
    expect(sidebar.x + sidebar.width).toBeLessThanOrEqual(area.x);
    expect(area.width).toBeGreaterThan(0);
    expect(area.x + area.width).toBeLessThanOrEqual(panel.x);
    expect(panel.x + panel.width).toBeLessThanOrEqual(700);
    // The page itself never scrolls sideways.
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(700);
    // Six tabs do not fit: the tab bar scrolls, within the terminals' width.
    const tabs = page.locator(".tabs");
    const bar = await box(tabs);
    expect(bar.x + bar.width).toBeLessThanOrEqual(area.x + area.width);
    const sizes = await tabs.evaluate((e) => [e.scrollWidth, e.clientWidth]);
    expect(sizes[0]).toBeGreaterThan(sizes[1] as number);
    await tabs.evaluate((e) => {
      e.scrollLeft = e.scrollWidth;
    });
    expect(await tabs.evaluate((e) => e.scrollLeft)).toBeGreaterThan(0);
  });
});

test("the page launches in the last run's theme, before the service's settings", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("hive.theme", "one-light");
    // The theme when the app's first render reaches the page.
    document.addEventListener("DOMContentLoaded", () => {
      const root = document.getElementById("root") as HTMLElement;
      const first = new MutationObserver(() => {
        (window as { launch?: string }).launch = document.documentElement.dataset.theme;
        first.disconnect();
      });
      first.observe(root, { childList: true });
    });
  });
  // A service that never answers: no settings arrive, the last run's theme stays.
  await page.goto("/?mock=disconnected");
  await expect(page.getByRole("alertdialog")).toBeVisible();
  expect(await page.evaluate(() => (window as { launch?: string }).launch)).toBe("one-light");
  await expect(page.locator("body")).toHaveCSS("background-color", "rgb(250, 250, 250)");
});

test("System follows the system's appearance, live (18.33)", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/");
  await expect(page.getByRole("navigation", { name: "Projects" })).toBeVisible();
  const root = page.locator("html");
  await expect(root).toHaveAttribute("data-theme", "one-dark");
  await page.keyboard.press(`${mod}+Comma`);
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog.getByRole("button", { name: "Appearance" }).click();
  await dialog.getByRole("combobox", { name: "Theme" }).click();
  await page.getByRole("option", { name: "System" }).click();
  await expect(root).toHaveAttribute("data-theme", "one-light");
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(root).toHaveAttribute("data-theme", "one-dark");
  await page.emulateMedia({ colorScheme: "light" });
  await expect(root).toHaveAttribute("data-theme", "one-light");
  // Kept for the next launch.
  expect(await page.evaluate(() => localStorage.getItem("hive.theme"))).toBe("system");
});

for (const [platform, userAgent] of [
  ["macOS", MAC_UA],
  ["Linux", LINUX_UA],
] as const) {
  test.describe(`scrollbars and cursor on ${platform} (18.36)`, () => {
    test.use({ userAgent });
    const mac = platform === "macOS";

    test(`${mac ? "native" : "the prototype's"} scrollbars, ${mac ? "the arrow" : "a pointer"} over controls`, async ({
      page,
      browserName,
    }) => {
      await page.goto("/");
      const tree = page.getByRole("navigation", { name: "Projects" });
      await expect(tree.getByRole("button", { name: "fix-login" })).toBeVisible();
      expect(await page.locator("html").getAttribute("data-mac")).toBe(mac ? "" : null);
      const pointer = mac ? "default" : "pointer";
      await expect(tree.getByRole("button", { name: "fix-login" })).toHaveCSS("cursor", pointer);
      await page.keyboard.press(mac ? "Meta+Comma" : "Control+Comma");
      const dialog = page.getByRole("dialog", { name: "Settings" });
      await expect(dialog.getByRole("checkbox").first()).toHaveCSS("cursor", pointer);
      // An overflowing box's scrollbar: the prototype's is 10 px; macOS' is its own. (Headless
      // Chromium hides every scrollbar.)
      if (browserName !== "webkit") return;
      const width = await page.evaluate(() => {
        const outer = document.createElement("div");
        outer.style.cssText = "position:fixed;width:100px;height:100px;overflow:scroll";
        outer.innerHTML = '<div style="height:300px"></div>';
        document.body.append(outer);
        const w = outer.offsetWidth - outer.clientWidth;
        outer.remove();
        return w;
      });
      if (mac) expect(width).not.toBe(10);
      else expect(width).toBe(10);
    });
  });
}
