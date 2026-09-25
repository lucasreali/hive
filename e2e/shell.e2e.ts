import { expect, test } from "@playwright/test";

test("app shell renders with bundled fonts and toggles the right panel", async ({ page }) => {
  const external: string[] = [];
  page.on("request", (r) => {
    if (new URL(r.url()).hostname !== "localhost") external.push(r.url());
  });
  await page.goto("/");

  await expect(page.getByRole("banner")).toHaveText("Hive");
  // Outside Tauri the UI connects through the mock transport.
  await expect(page.getByTitle("WSL connection")).toHaveText("WSL: Ubuntuconnected");
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  // Fonts load lazily, so wait for it rather than checking once.
  await expect
    .poll(() => page.evaluate(() => document.fonts.check("13px 'IBM Plex Sans'")))
    .toBe(true);
  // The side panel starts open (4.21).
  await expect(page.getByRole("complementary", { name: "Side panel" })).toBeVisible();
  await page.screenshot({ path: "target/e2e/shell-right-panel.png" });
  await page.getByTitle("Collapse (Ctrl+Shift+B)").click();
  await expect(page.getByRole("complementary")).toHaveCount(0);
  await page.screenshot({ path: "target/e2e/shell.png" });
  await page.getByTitle("Files, diff and sessions (Ctrl+Shift+B)").click();
  await expect(page.getByRole("complementary", { name: "Side panel" })).toBeVisible();

  expect(external).toEqual([]);
});
