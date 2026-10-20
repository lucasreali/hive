import { expect, type Page, test } from "@playwright/test";
import { mac } from "./host";

// 17.25 against the mock transport, whose shell marks its prompts and commands with OSC 133
// (`false` fails) and whose typed stand-ins send what the service reads from OSC 9 / 777 / 9;4.

/** The top line of terminal `id`'s view. */
const top = (page: Page, id: number) =>
  page.evaluate(async (id) => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    return terminal(id).buffer.active.viewportY as number;
  }, id);

async function run(page: Page, line: string) {
  await page.keyboard.type(line);
  await page.keyboard.press("Enter");
}

test("shell marks: each prompt's bar, a program's progress and notification", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const tab = page.getByRole("tablist").getByRole("tab", { name: "fix-login" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect(tab).toHaveAttribute("aria-selected", "true");

  // How each command ended shows on its prompt's line.
  await run(page, "true");
  await run(page, "false");
  const marks = page.locator(".terminal-pane .shell-mark");
  await expect(marks).toHaveCount(2);
  await expect(page.locator('.shell-mark[data-status="ok"]')).toHaveCount(1);
  await expect(page.locator('.shell-mark[data-status="failed"]')).toHaveCount(1);

  // A program's progress on the tab, until it clears.
  await run(page, "progress normal 40");
  const progress = tab.getByRole("progressbar");
  await expect(progress).toHaveAccessibleName("Progress 40%");
  await run(page, "progress none");
  await expect(progress).toHaveCount(0);

  // A program's notification is in the inbox; its item shows the terminal.
  await run(page, "osc-notify make;done");
  await tree.getByRole("button", { name: "refactor-auth" }).click();
  await page.locator(".pending-bell").click();
  await page.getByRole("menuitem", { name: "make: done" }).click();
  await expect(tab).toHaveAttribute("aria-selected", "true");
});

test("shell marks: ⌘↑ and ⌘↓ jump between prompts (macOS)", async ({ page }) => {
  test.skip(!mac, "the prompt keys are macOS' (D17.2)");
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  // Enough commands to scroll: a prompt on every other line (0, 2, 4…).
  for (let i = 0; i < 40; i++) await run(page, `echo ${i}`);
  await expect.poll(() => top(page, 1)).toBeGreaterThan(4);
  const end = await top(page, 1);
  // The prompt above the view's top line comes to the top, then the one above it.
  const above = end - 1 - ((end - 1) % 2);
  await page.keyboard.press("Meta+ArrowUp");
  await expect.poll(() => top(page, 1)).toBe(above);
  await page.keyboard.press("Meta+ArrowUp");
  await expect.poll(() => top(page, 1)).toBe(above - 2);
  await page.keyboard.press("Meta+ArrowDown");
  await expect.poll(() => top(page, 1)).toBe(above);
  // The next prompts are on the last screen: the end.
  await page.keyboard.press("Meta+ArrowDown");
  await expect.poll(() => top(page, 1)).toBe(end);
});
