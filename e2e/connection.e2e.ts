import { expect, test } from "@playwright/test";

test("a version mismatch blocks the workspace", async ({ page }) => {
  await page.goto("/?mock=mismatch");
  const block = page.getByRole("alertdialog");
  await expect(block).toContainText("The app and the hive service versions differ");
  await expect(block).toContainText("cargo install --path crates/hive");
  await expect(page.getByTitle("WSL connection")).toHaveText("WSLversion mismatch");
  await expect(page.getByRole("button", { name: "Reconnect" })).toBeFocused();
  await page.screenshot({ path: "target/e2e/version-mismatch.png" });
  // The workspace under the block takes no clicks.
  await page.getByTitle("Files, diff and sessions (Ctrl+Shift+B)").click({ force: true });
  await expect(page.getByRole("complementary")).toHaveCount(0);
});

test("a disconnect shows the reason and can reconnect", async ({ page }) => {
  await page.goto("/?mock=disconnected");
  const block = page.getByRole("alertdialog");
  await expect(block).toContainText("mock: the hive bridge exited");
  await expect(page.getByTitle("WSL connection")).toHaveText("WSLdisconnected");
  await page.screenshot({ path: "target/e2e/disconnected.png" });
  await page.getByRole("button", { name: "Reconnect" }).click();
  // The mock keeps failing the same way, so the block comes back.
  await expect(block).toContainText("mock: the hive bridge exited");
});
