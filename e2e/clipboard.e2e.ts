import { expect, type Page, test } from "@playwright/test";
import { mac, mod } from "./host";

// Terminal copy and paste without WebKit's "Paste" bubble (17.8): copy on select copies on
// mouse-up (a user gesture), and on macOS Cmd+V is the engine's own paste, not a script read.

/** The text of terminal 1's buffer, read from the page's own terminal manager. */
function screen(page: Page): Promise<string> {
  return page.evaluate(async () => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const buffer = terminal(1).buffer.active;
    const lines: string[] = [];
    for (let y = 0; y < buffer.length; y++) lines.push(buffer.getLine(y).translateToString(true));
    return lines.join("\n").trimEnd();
  });
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

/** Opens a terminal in fix-login and runs `echo one` in it. */
async function echoOne(page: Page): Promise<void> {
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
}

const clipboard = (page: Page) => page.evaluate(() => navigator.clipboard.readText());

test.beforeEach(async ({ context, browserName }) => {
  // WebKit knows only clipboard-read; it lets a page write the clipboard without a permission.
  await context.grantPermissions(
    browserName === "webkit" ? ["clipboard-read"] : ["clipboard-read", "clipboard-write"],
  );
});

test("clipboard: copy on select copies what the mouse selected, on release", async ({ page }) => {
  await echoOne(page);
  await page.evaluate(() => navigator.clipboard.writeText("before"));
  await page.keyboard.press(`${mod}+Comma`);
  const dialog = page.getByRole("dialog", { name: "Settings" });
  // Checked once the service has saved the setting and sent it back.
  await dialog.getByLabel("Copy on select").click();
  await expect(dialog.getByLabel("Copy on select")).toBeChecked();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();

  // Drag over "echo one" on the second line (to the middle of the cell after it).
  const from = await cell(page, 0, 1);
  const to = await cell(page, 8, 1);
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 4 });
  expect(await clipboard(page)).toBe("before");
  await page.mouse.up();
  await expect.poll(() => clipboard(page)).toBe("echo one");
});

test.describe("on macOS", () => {
  test.use({
    userAgent:
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
  });

  // Cmd+V is left to the engine: in WKWebView the Edit menu's Paste fires a trusted paste event
  // on xterm's textarea, as a Mac host's WebKit does. Linux WebKit has no Cmd+V paste, so there
  // the check is in two halves: the key is neither cancelled nor typed, and the paste event the
  // engine would fire reaches the shell.
  test("clipboard: Cmd+V is the engine's paste, bracketed when the shell asks", async ({
    page,
    browserName,
  }) => {
    test.skip(browserName !== "webkit", "WKWebView's engine only");
    await echoOne(page);
    await page.evaluate(async () => {
      const url = "/src/terminals.ts";
      const { terminal } = await import(/* @vite-ignore */ url);
      const seen = { data: [] as string[], cancelled: [] as boolean[] };
      Object.assign(window, { seen });
      terminal(1).onData((data: string) => seen.data.push(data));
      // The shell asks for bracketed paste.
      await new Promise((resolve) => terminal(1).write("\x1b[?2004h", resolve));
      window.addEventListener("keydown", (event) => seen.cancelled.push(event.defaultPrevented));
      await navigator.clipboard.writeText("pasted");
    });
    const seen = () =>
      page.evaluate(
        () => (window as unknown as { seen: { data: string[]; cancelled: boolean[] } }).seen,
      );
    await page.keyboard.press("Meta+V");
    // Meta's keydown and V's: neither cancelled (as a script paste would), nothing typed; on a
    // Mac the engine's paste follows.
    expect(await seen()).toEqual({
      data: mac ? ["\x1b[200~pasted\x1b[201~"] : [],
      cancelled: [false, false],
    });

    if (!mac)
      await page.evaluate(async () => {
        const url = "/src/terminals.ts";
        const { terminal } = await import(/* @vite-ignore */ url);
        const clipboardData = new DataTransfer();
        clipboardData.setData("text/plain", await navigator.clipboard.readText());
        const paste = new ClipboardEvent("paste", {
          clipboardData,
          bubbles: true,
          cancelable: true,
        });
        terminal(1).textarea.dispatchEvent(paste);
      });
    expect((await seen()).data).toEqual(["\x1b[200~pasted\x1b[201~"]);
    await expect.poll(() => screen(page)).toBe("mock$ echo one\necho one\nmock$ pasted");
  });
});
