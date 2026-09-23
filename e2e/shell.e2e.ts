import { expect, test } from "@playwright/test";

test("app shell renders with bundled fonts and toggles the right panel", async ({ page }) => {
  const external: string[] = [];
  page.on("request", (r) => {
    if (!r.url().startsWith("http://localhost:1420")) external.push(r.url());
  });
  await page.goto("/");

  await expect(page.getByRole("banner")).toHaveText("Hive");
  await expect(page.getByRole("contentinfo")).toContainText("WSL");
  expect(await page.evaluate(() => document.fonts.check("13px 'IBM Plex Sans'"))).toBe(true);
  await page.screenshot({ path: "target/e2e/shell.png" });

  await page.getByTitle("Files and diff (Ctrl+Shift+B)").click();
  await expect(page.getByRole("complementary", { name: "Files and diff" })).toBeVisible();
  await page.screenshot({ path: "target/e2e/shell-right-panel.png" });
  await page.getByTitle("Collapse (Ctrl+Shift+B)").click();
  await expect(page.getByRole("complementary")).toHaveCount(0);

  expect(external).toEqual([]);
});
