import { expect, type Page, test } from "@playwright/test";
import { CONNECTION, mac } from "./host";

// Outside Tauri a close that went through marks the page (`closeWindow` in window.ts).
const closed = (page: Page) =>
  page.evaluate(() => document.documentElement.dataset.closed !== undefined);

/**
 * The title bar's Close. On macOS the native traffic lights replace it: their close request is
 * the one the button sends (`windowAction`).
 */
const closeButton = (page: Page) => ({
  click: () =>
    mac
      ? page.evaluate(async () => {
          const url = "/src/window.ts";
          const { windowAction } = await import(/* @vite-ignore */ url);
          windowAction("close");
        })
      : page.getByTitle("Close", { exact: true }).click(),
});

test("closing with no agent closes at once", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTitle(CONNECTION)).toHaveAttribute("data-status", "connected");
  await closeButton(page).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await closed(page)).toBe(true);
});

test("closing with a working agent asks first: cancel keeps the app, confirm closes it", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "main", exact: true }).first().click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await page.keyboard.type("claude");
  await page.keyboard.press("Enter");
  await expect(tree.getByRole("img", { name: "idle" })).toHaveCount(1);

  // An idle agent loses nothing: the app closes at once (#18).
  const close = closeButton(page);
  const dialog = page.getByRole("dialog", { name: "Close Hive?" });
  await close.click();
  await expect(dialog).toHaveCount(0);
  expect(await closed(page)).toBe(true);
  await page.evaluate(() => delete document.documentElement.dataset.closed);

  // A prompt sets the fake agent working.
  await page.locator(".xterm").click();
  await page.keyboard.type("fix the login bug");
  await page.keyboard.press("Enter");
  await expect(tree.getByRole("img", { name: "working" })).toHaveCount(1);
  await close.click();
  await expect(dialog).toContainText("1 agent is running.");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
  expect(await closed(page)).toBe(false);

  await close.click();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  expect(await closed(page)).toBe(false);

  // The confirmation has the focus: Enter closes.
  await close.click();
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Enter");
  expect(await closed(page)).toBe(true);
});
