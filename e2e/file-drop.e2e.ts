import { expect, type Page, test } from "@playwright/test";
import { mod } from "./host";

// Files dropped from Finder on a terminal (18.1). On macOS the app side takes the drop before
// WebKit does and sends `files_dropped {text, x, y}`; the mock transport sends the same for
// `drop-files <x> <y> <text>` typed in a terminal. The text is typed into the terminal whose
// pane is at that point, never with Enter; a drop anywhere else is ignored.

/** The text of a terminal's buffer, read from the page's own terminal manager. */
function screen(page: Page, id: number): Promise<string> {
  return page.evaluate(async (id) => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const buffer = terminal(id).buffer.active;
    const lines: string[] = [];
    for (let y = 0; y < buffer.length; y++) lines.push(buffer.getLine(y).translateToString(true));
    return lines.join("\n").trimEnd();
  }, id);
}

/** The middle of `selector`'s first element, in the window, as `x y`. */
async function middle(page: Page, selector: string): Promise<string> {
  const box = await page.locator(selector).first().boundingBox();
  if (!box) throw new Error(`no ${selector}`);
  return `${Math.round(box.x + box.width / 2)} ${Math.round(box.y + box.height / 2)}`;
}

test("files dropped on a terminal pane type their quoted paths there; elsewhere nothing", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect.poll(() => screen(page, 1)).toBe("mock$");
  // Two panes: the drop goes to the one under it, not to the focused one.
  await page.keyboard.press(`${mod}+Shift+D`);
  await expect(page.locator(".terminal-pane:visible")).toHaveCount(2);
  await expect.poll(() => screen(page, 2)).toBe("mock$");
  const pane = (side: string) => `.terminal-pane[data-pane="${side}"] .xterm-screen`;
  const left = await middle(page, pane("left"));
  const right = await middle(page, pane("right"));
  await page.locator(pane("left")).click();

  // The mock echoes each line typed, then shows a new prompt; `last` is the prompt line.
  const last = async (id: number) => (await screen(page, id)).split("\n").at(-1);
  const echoed = async (line: string) =>
    (await screen(page, 1)).split("\n").filter((l) => l === line).length;

  // `drop-files` typed in the left terminal stands in for a drop on the right one.
  const text = "/a\\ b.png /c ";
  await page.keyboard.type(`drop-files ${right} ${text}`);
  await page.keyboard.press("Enter");
  // The paths wait on the right terminal's prompt line: no Enter after them.
  await expect.poll(() => screen(page, 2)).toBe(`mock$ ${text.trimEnd()}`);
  expect(await last(1)).toBe("mock$");
  // That terminal took the keyboard: the next key goes after the paths.
  await page.keyboard.type("x");
  await expect.poll(() => screen(page, 2)).toBe(`mock$ ${text}x`);

  // Dropped on the left pane: typed on its new prompt line.
  await page.locator(pane("left")).click();
  await page.keyboard.type(`drop-files ${left} ${text}`);
  await page.keyboard.press("Enter");
  await expect.poll(() => last(1)).toBe(`mock$ ${text.trimEnd()}`);
  await page.keyboard.press("Enter");
  await expect.poll(() => last(1)).toBe("mock$");

  // On the sidebar, the tab bar or the status bar: nothing typed anywhere.
  const right2 = await screen(page, 2);
  const outside = ['nav[aria-label="Projects"]', '[role="tablist"]', ".statusbar"];
  let line = "";
  for (const selector of outside) {
    line = `drop-files ${await middle(page, selector)} /e `;
    await page.keyboard.type(line);
    await page.keyboard.press("Enter");
  }
  // The last one was run (its echo shows).
  await expect.poll(() => echoed(line)).toBe(1);
  // Time for a misrouted drop to land before checking that none did.
  await page.waitForTimeout(300);
  expect(await screen(page, 2)).toBe(right2);
  expect(await last(1)).toBe("mock$");
  // The page never left the app.
  expect(new URL(page.url()).pathname).toBe("/");
});
