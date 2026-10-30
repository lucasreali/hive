import { expect, type Page, test } from "@playwright/test";
import { mac } from "./host";

// 21.1: clearing a terminal left it blank and dead. `clear` in fish erases the screen while the
// command runs, its own prompt's mark with it; the command's end (OSC 133 `D`) then threw inside
// xterm's parser, which drew nothing more. ⌘K and the menu's Clear kept only the cursor's line,
// so fish's prompt of two lines lost its first.

/** The lines of terminal `id`'s buffer, scrollback first, trailing blank lines dropped. */
const text = (page: Page, id: number) =>
  page.evaluate(async (id) => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const buffer = terminal(id).buffer.active;
    const lines: string[] = [];
    for (let i = 0; i < buffer.length; i++) lines.push(buffer.getLine(i).translateToString(true));
    return lines.join("\n").trimEnd();
  }, id);

/** Writes `data` to terminal `id` as its shell would, once xterm has drawn it. */
const shell = (page: Page, id: number, data: string) =>
  page.evaluate(
    async ([id, data]) => {
      const url = "/src/terminals.ts";
      const { terminal } = await import(/* @vite-ignore */ url);
      await new Promise<void>((resolve) => terminal(id).write(data, resolve));
    },
    [id, data] as const,
  );

async function run(page: Page, line: string) {
  await page.keyboard.type(line);
  await page.keyboard.press("Enter");
}

/** A terminal in `fix-login`, focused, with two commands and their prompt bars above. */
async function open(page: Page) {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  const pane = page.locator(".terminal-pane:not([hidden])");
  await expect(pane).toHaveCount(1);
  const id = Number(await pane.getAttribute("data-terminal"));
  await run(page, "true");
  await run(page, "false");
  await expect(pane.locator(".shell-mark")).toHaveCount(2);
  return { pane, id };
}

test("`clear` typed in the shell: its prompt redrawn at the top, focused, taking input", async ({
  page,
}) => {
  const { pane, id } = await open(page);
  await run(page, "clear");
  await expect.poll(() => text(page, id)).toBe("mock$");
  await expect(pane.locator(".xterm-helper-textarea")).toBeFocused();
  await run(page, "echo after-clear");
  await expect.poll(() => text(page, id)).toBe("mock$ echo after-clear\necho after-clear\nmock$");
  // Twice in a row, and the prompt bars come back with the next commands.
  await run(page, "clear");
  await expect.poll(() => text(page, id)).toBe("mock$");
  await run(page, "false");
  await expect(pane.locator(".shell-mark")).toHaveCount(1);
});

const ways = [
  { name: "the menu's Clear", mac: false },
  { name: "⌘K", mac: true },
];

for (const way of ways) {
  test(`${way.name}: the whole prompt stays at the top, focused, taking input`, async ({
    page,
  }) => {
    test.skip(way.mac && !mac, "⌘K clears on macOS only");
    const { pane, id } = await open(page);
    // fish's prompt of two lines, from its OSC 133 mark.
    await shell(page, id, "\r\n\x1b]133;A\x07hive on main via bun\r\n❯ ");
    if (way.mac) await page.keyboard.press("Meta+K");
    else {
      await pane.click({ button: "right" });
      await page
        .getByRole("menu", { name: "Terminal" })
        .getByRole("menuitem", { name: "Clear" })
        .click();
    }
    await expect.poll(() => text(page, id)).toBe("hive on main via bun\n❯");
    await expect(pane.locator(".xterm-helper-textarea")).toBeFocused();
    await page.keyboard.type("typed");
    await expect.poll(() => text(page, id)).toBe("hive on main via bun\n❯ typed");
  });
}
