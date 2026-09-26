import { expect, type Page, test } from "@playwright/test";

/**
 * Terminal 1's font size and columns, read from the page's own terminal manager (undefined
 * until it exists: it waits for the bundled fonts).
 */
function terminal(page: Page): Promise<{ fontSize?: number; cols?: number }> {
  return page.evaluate(async () => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const term = terminal(1);
    return { fontSize: term?.options.fontSize, cols: term?.cols };
  });
}

const cssVar = (page: Page, name: string) =>
  page.evaluate((name) => getComputedStyle(document.documentElement).getPropertyValue(name), name);

test("settings: Ctrl+, opens them; font size and theme apply live", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect.poll(async () => (await terminal(page)).fontSize).toBe(13);
  const before = await terminal(page);

  // Taken even with the focus in the terminal.
  await page.keyboard.press("Control+Comma");
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("Font size", { exact: true }).fill("19");
  // The stepper's own button, not the WebView's spin button.
  await dialog.getByRole("button", { name: "Increase Font size" }).click();
  await expect(dialog.getByLabel("Font size", { exact: true })).toHaveValue("20");
  await expect.poll(async () => (await terminal(page)).fontSize).toBe(20);
  // Bigger cells: the terminal refits to fewer columns.
  expect((await terminal(page)).cols).toBeLessThan(before.cols as number);

  expect(await cssVar(page, "--bg")).toBe("#282c33");
  await dialog.getByRole("button", { name: "Appearance" }).click();
  await dialog.getByRole("combobox", { name: "Theme" }).click();
  await page.getByRole("option", { name: "One Light" }).click();
  await expect.poll(() => cssVar(page, "--bg")).toBe("#fafafa");

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
});
