import { expect, type Page, test } from "@playwright/test";
import { mac, mod } from "./host";

/** The search key: ⌘F on macOS; elsewhere Ctrl+F is the shell's, so Ctrl+Shift+F (#35). */
const FIND = mac ? "Meta+F" : "Control+Shift+F";

/** Writes `text` into terminal 1, as its program's output would arrive. */
const print = (page: Page, text: string) =>
  page.evaluate(async (text) => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    await new Promise<void>((resolve) => terminal(1).write(text, resolve));
  }, text);

/** Terminal 1's first row in view, and the text selected in it. */
const view = (page: Page) =>
  page.evaluate(async () => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const term = terminal(1);
    return { top: term.buffer.active.viewportY, selected: term.getSelection() };
  });

test("⌘F (Ctrl+Shift+F off macOS) finds a line far up the scrollback (17.21)", async ({ page }) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  const lines = Array.from({ length: 600 }, (_, i) => `output line ${i}`);
  await print(page, `\r\nbuild started: needle-42\r\n${lines.join("\r\n")}\r\n`);
  // The view is at the bottom, far below the line.
  expect((await view(page)).top).toBeGreaterThan(500);

  await page.keyboard.press(FIND);
  const field = page.getByRole("textbox", { name: "Search the terminal" });
  await expect(field).toBeFocused();
  await field.fill("NEEDLE-42");
  const bar = page.getByRole("search", { name: "Search the terminal" });
  await expect(bar).toContainText("1 of 1");
  await expect.poll(async () => (await view(page)).top).toBeLessThan(30);
  expect((await view(page)).selected).toBe("needle-42");
  // Match case: the line has it in lower case.
  await bar.getByRole("button", { name: "Match case" }).click();
  await expect(bar).toContainText("No results");
  await bar.getByRole("button", { name: "Match case" }).click();
  await expect(bar).toContainText("1 of 1");

  // Two matches: Enter goes up to the older one, Shift+Enter back down.
  await field.fill("line 59");
  await expect(bar).toContainText("11 of 11");
  await field.press("Enter");
  await expect(bar).toContainText("10 of 11");
  await field.press("Shift+Enter");
  await expect(bar).toContainText("11 of 11");

  // Esc closes it: the keyboard is back in the terminal.
  await field.press("Escape");
  await expect(bar).toBeHidden();
  await expect(page.locator(".terminal-pane:not([hidden]) textarea")).toBeFocused();
  // Opened again: empty, the old search gone.
  await page.keyboard.press(FIND);
  await expect(field).toBeFocused();
  await expect(field).toHaveValue("");

  // From the command palette too, the field taking the keyboard.
  await field.press("Escape");
  await page.keyboard.press(`${mod}+Shift+P`);
  await page.keyboard.type("Search the terminal");
  await page.keyboard.press("Enter");
  await expect(field).toBeFocused();
});
