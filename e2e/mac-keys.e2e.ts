import { expect, type Page, test } from "@playwright/test";

// 18.34 (D18.10): macOS' ⌃Tab / ⌃⇧Tab and the trackpad pinch, in WebKit (WKWebView's engine).
// WKWebView's user agent on every host, so Hive takes macOS' keys here off a Mac too.
test.use({
  userAgent:
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)",
});

/** The font size of the terminal shown. */
const fontSize = (page: Page) =>
  page.evaluate(async () => {
    const [store, terminals] = ["/src/store.ts", "/src/terminals.ts"];
    const { useHive } = await import(/* @vite-ignore */ store);
    const { terminal } = await import(/* @vite-ignore */ terminals);
    return terminal(useHive.getState().activeTab)?.options.fontSize;
  });

test.beforeEach(async ({ page, browserName }) => {
  test.skip(browserName !== "webkit", "macOS' keys, in WKWebView's engine");
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
});

test("⌃Tab and ⌃⇧Tab cycle the tabs, with the focus in a terminal, and never reach it", async ({
  page,
}) => {
  const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
  const plus = page.getByTitle("New terminal or file");
  for (let i = 0; i < 2; i++) {
    await plus.click();
    await page.getByRole("menuitem", { name: "Terminal" }).click();
  }
  const [one, two] = [tabs.getByRole("tab").nth(0), tabs.getByRole("tab").nth(1)];
  await expect(two).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".terminal-pane:visible textarea")).toBeFocused();
  // Every write a terminal sends the (mock) service from now on.
  await page.evaluate(async () => {
    const url = "/src/transport/index.ts";
    const { transport } = await import(/* @vite-ignore */ url);
    const write = transport.writeTerminal.bind(transport);
    const w = window as unknown as { writes: string[] };
    w.writes = [];
    transport.writeTerminal = (id: number, data: string) => {
      w.writes.push(data);
      return write(id, data);
    };
  });
  await page.keyboard.press("Control+Tab");
  await expect(one).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Control+Tab");
  await expect(two).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Control+Shift+Tab");
  await expect(one).toHaveAttribute("aria-selected", "true");
  // Only a plain Tab reaches the terminal.
  await page.keyboard.press("Tab");
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { writes: string[] }).writes))
    .toEqual(["\t"]);
});

test("a pinch on the terminal zooms its font, as ⌘= and ⌘- do", async ({ page }) => {
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  const pane = page.locator(".terminal-pane:visible");
  await expect(pane).toBeVisible();
  const start = await fontSize(page);
  // WebKit's pinch: gesture events, by their scale.
  await pane.evaluate((el) => {
    for (const [type, scale] of [
      ["gesturestart", 1],
      ["gesturechange", 1.2],
    ] as const)
      el.dispatchEvent(
        Object.assign(new Event(type, { bubbles: true, cancelable: true }), { scale }),
      );
  });
  await expect.poll(() => fontSize(page)).toBe(start + 1);
  // Chromium's: a Ctrl+wheel; inward (positive) smaller.
  const box = (await pane.boundingBox()) as { x: number; y: number };
  await page.mouse.move(box.x + 50, box.y + 50);
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, 40);
  await page.keyboard.up("Control");
  await expect.poll(() => fontSize(page)).toBe(start);
});
