import { expect, type Page, test } from "@playwright/test";
import { keyLabel, mac } from "./host";

// The terminal's own menu (17.20): a right click (a two-finger tap on a Mac) in a terminal opens
// Hive's menu, never the WebView's (#53), with Copy, Paste, Clear and Split.

/** The text of terminal `id`'s buffer, read from the page's own terminal manager. */
function screen(page: Page, id = 1): Promise<string> {
  return page.evaluate(async (id) => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const buffer = terminal(id).buffer.active;
    const lines: string[] = [];
    for (let y = 0; y < buffer.length; y++) lines.push(buffer.getLine(y).translateToString(true));
    return lines.join("\n").trimEnd();
  }, id);
}

/** The page position of the middle of terminal 1's cell (`col`, `row`). */
function cell(page: Page, col: number, row: number): Promise<{ x: number; y: number }> {
  return page.evaluate(
    async ([col, row]) => {
      const url = "/src/terminals.ts";
      const { terminal } = await import(/* @vite-ignore */ url);
      const term = terminal(1);
      const box = term.element.querySelector(".xterm-screen").getBoundingClientRect();
      const width = box.width / term.cols;
      const height = box.height / term.rows;
      return { x: box.left + width * (col + 0.5), y: box.top + height * (row + 0.5) };
    },
    [col, row],
  );
}

const clipboard = (page: Page) => page.evaluate(() => navigator.clipboard.readText());

test.beforeEach(async ({ context, browserName }) => {
  // WebKit knows only clipboard-read; it lets a page write the clipboard without a permission.
  await context.grantPermissions(
    browserName === "webkit" ? ["clipboard-read"] : ["clipboard-read", "clipboard-write"],
  );
});

test("terminal menu: a right click opens Copy, Paste, Clear and Split, and each does its job", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect.poll(() => screen(page)).toBe("mock$");
  await page.keyboard.type("echo one");
  await page.keyboard.press("Enter");
  await expect.poll(() => screen(page)).toBe("mock$ echo one\necho one\nmock$");
  // Whether the WebView's own menu would open: the contextmenu event left uncancelled.
  await page.evaluate(() => {
    const seen: boolean[] = [];
    Object.assign(window, { seen });
    window.addEventListener("contextmenu", (event) => seen.push(event.defaultPrevented));
  });
  const prevented = () =>
    page.evaluate(() => (window as unknown as { seen: boolean[] }).seen.splice(0));
  const menu = page.getByRole("menu", { name: "Terminal" });
  const item = (name: string) => menu.getByRole("menuitem", { name });
  const rightClick = async (col: number, row: number) => {
    const at = await cell(page, col, row);
    await page.mouse.click(at.x, at.y, { button: "right" });
  };

  // Copy what the mouse selected ("echo one" on the second line).
  const from = await cell(page, 0, 1);
  const to = await cell(page, 8, 1);
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 4 });
  await page.mouse.up();
  await page.evaluate(() => navigator.clipboard.writeText("before"));
  await rightClick(4, 1);
  await expect(menu).toBeVisible();
  expect(await prevented()).toEqual([true]);
  await expect(menu.getByRole("menuitem")).toHaveText([
    `Copy ${mac ? "⌘C" : "Ctrl+Shift+C"}`,
    `Paste ${mac ? "⌘V" : "Ctrl+Shift+V"}`,
    mac ? "Clear ⌘K" : "Clear",
    `Split right ${keyLabel("Ctrl+Shift+D")}`,
  ]);
  // A right click inside the selection keeps it (outside it, on a Mac, xterm selects a word).
  await item("Copy").click();
  await expect(menu).toBeHidden();
  await expect.poll(() => clipboard(page)).toBe("echo one");

  // Paste: the clipboard reaches the shell, and the terminal keeps the keyboard.
  await page.evaluate(() => navigator.clipboard.writeText("pasted"));
  await rightClick(10, 2);
  await item("Paste").click();
  await expect.poll(() => screen(page)).toBe("mock$ echo one\necho one\nmock$ pasted");
  await expect(page.locator(".terminal-pane textarea")).toBeFocused();

  // Clear: the scrollback and the screen go, the prompt line stays; nothing reaches the shell.
  await rightClick(10, 2);
  await item("Clear").click();
  await expect.poll(() => screen(page)).toBe("mock$ pasted");

  // Esc closes it doing nothing; Split right shows a second terminal beside it.
  await rightClick(2, 0);
  await page.keyboard.press("Escape");
  await expect(menu).toBeHidden();
  await rightClick(2, 0);
  await item("Split right").click();
  await expect(page.locator(".terminal-pane:not([hidden])")).toHaveCount(2);
  await expect.poll(() => screen(page, 2)).toBe("mock$");
});
