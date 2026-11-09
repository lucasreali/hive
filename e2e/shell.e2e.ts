import { expect, test } from "@playwright/test";
import { CONNECTION, keyLabel } from "./host";

test("app shell renders with bundled fonts and toggles the right panel", async ({ page }) => {
  const external: string[] = [];
  page.on("request", (r) => {
    if (new URL(r.url()).hostname !== "localhost") external.push(r.url());
  });
  await page.goto("/");

  await expect(page.getByRole("banner")).toHaveText("Hive");
  // Outside Tauri the UI connects through the mock transport.
  await expect(page.getByTitle(CONNECTION)).toHaveText("WSL: Ubuntuconnected");
  // The app version sits at the right end of the same line (7.8).
  await expect(page.getByRole("contentinfo")).toContainText(/v\d+\.\d+\.\d+$/);
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  // Fonts load lazily, so wait for it rather than checking once.
  await expect
    .poll(() => page.evaluate(() => document.fonts.check("13px 'IBM Plex Sans'")))
    .toBe(true);
  // The side panel starts open (4.21).
  await expect(page.getByRole("complementary", { name: "Side panel" })).toBeVisible();
  await page.screenshot({ path: "target/e2e/shell-right-panel.png" });
  // The terminal bar has no button for it (26.12); the X closes the view, the rail stays.
  await expect(page.getByTitle(/Files, diff and sessions/)).toHaveCount(0);
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await page.getByTitle(keyLabel("Close (Ctrl+Shift+B)")).click();
  await expect(panel.locator(".panel-title")).toHaveCount(0);
  await expect(panel.getByRole("tablist", { name: "Panel" })).toBeVisible();
  await page.screenshot({ path: "target/e2e/shell.png" });
  await panel.getByRole("tab", { name: "Files" }).click();
  await expect(panel.locator(".panel-title")).toBeVisible();

  expect(external).toEqual([]);
});
