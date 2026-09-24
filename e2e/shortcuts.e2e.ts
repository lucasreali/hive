import { expect, type Page, test } from "@playwright/test";

/** Records every write the page's terminals send to the (mock) service. */
async function recordWrites(page: Page): Promise<() => Promise<string[]>> {
  await page.evaluate(async () => {
    const url = "/src/transport/index.ts";
    const { transport } = await import(/* @vite-ignore */ url);
    const write = transport.writeTerminal.bind(transport);
    const w = window as unknown as { writes: string[] };
    w.writes = [];
    transport.writeTerminal = (id: number, data: string) => {
      w.writes.push(data);
      return write(id, data);
    };
  });
  return () => page.evaluate(() => (window as unknown as { writes: string[] }).writes);
}

test("shortcuts: taken with the focus in a terminal; every other key reaches it", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const tabs = page.getByRole("tablist");
  await expect(tree.getByRole("button", { name: "fix-login" })).toBeVisible();
  const writes = await recordWrites(page);

  // Ctrl+Shift+T with nothing focused: the picker; filter, arrows, Enter.
  await page.keyboard.press("Control+Shift+T");
  const picker = page.getByRole("dialog", { name: "Open a terminal in worktree" });
  await expect(picker).toBeVisible();
  await expect(picker.getByPlaceholder("Open a terminal in worktree…")).toBeFocused();
  await page.screenshot({ path: "target/e2e/worktree-picker.png" });
  await page.keyboard.type("shop");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(picker).toBeHidden();
  await expect(tabs.getByRole("tab", { name: "fix-login" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  const input = page.locator(".terminal-pane:visible textarea");
  await expect(input).toBeFocused();

  // Plain keys, Ctrl+O and Alt+B go to the terminal.
  await page.keyboard.type("ab");
  await page.keyboard.press("Control+O");
  await page.keyboard.press("Alt+B");
  await page.keyboard.press("Control+T");
  await expect.poll(writes).toEqual(["a", "b", "\x0f", "\x1bb", "\x14"]);

  // Ctrl+Shift+B toggles the files panel, and never reaches the terminal.
  const panel = page.getByRole("complementary", { name: "Changes" });
  await page.keyboard.press("Control+Shift+B");
  await expect(panel).toBeVisible();
  await input.focus();
  await page.keyboard.press("Control+Shift+B");
  await expect(panel).toBeHidden();

  // F8: no agent is pending, so nothing happens and nothing reaches the shell.
  await input.focus();
  await page.keyboard.press("F8");

  // Ctrl+Shift+N: the new worktree dialog for the selected worktree's project.
  await input.focus();
  await page.keyboard.press("Control+Shift+N");
  const dialog = page.getByRole("dialog", { name: "New worktree" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByLabel("Project")).toHaveValue(/shop/);
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();

  // Ctrl+Shift+O: add project.
  await input.focus();
  await page.keyboard.press("Control+Shift+O");
  await expect(page.getByRole("dialog", { name: "Add project" })).toBeVisible();
  await page.keyboard.press("Escape");

  expect(await writes()).toEqual(["a", "b", "\x0f", "\x1bb", "\x14"]);
  await expect(tabs.getByRole("tab")).toHaveCount(1);
});
