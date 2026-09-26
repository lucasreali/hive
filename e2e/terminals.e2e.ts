import { expect, type Page, test } from "@playwright/test";

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

test("terminals: tabs, typing, switching, copy and paste, exit and close", async ({
  page,
  context,
}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
  const plus = page.getByTitle("New terminal, agent or file");
  await expect(tree.getByRole("button", { name: "fix-login" })).toBeVisible();
  // Nothing selected, so there is no worktree to open a terminal in.
  await expect(plus).toBeDisabled();

  await tree.getByRole("button", { name: "fix-login" }).click();
  await plus.click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect(tabs.getByRole("tab", { name: "fix-login" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect.poll(() => screen(page, 1)).toBe("mock$");
  // Keys go straight to the focused terminal.
  await page.keyboard.type("echo one");
  await page.keyboard.press("Enter");
  await expect.poll(() => screen(page, 1)).toBe("mock$ echo one\necho one\nmock$");

  await tree.getByRole("button", { name: "refactor-auth" }).click();
  await plus.click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect(tabs.getByRole("tab", { name: "refactor-auth" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await page.keyboard.type("two");
  await expect.poll(() => screen(page, 2)).toBe("mock$ two");
  // Only the shown terminal renders, with WebGL; the hidden one keeps no renderer canvas.
  await expect(page.locator(".terminal-pane:visible")).toHaveCount(1);
  await expect(page.locator(".terminal-pane:visible canvas").first()).toBeAttached();
  await expect(page.locator(".terminal-pane[hidden] canvas")).toHaveCount(0);
  // Fitted to the area (wider than the 80 columns it opened with), in the bundled font.
  expect(
    await page.evaluate(async () => {
      const url = "/src/terminals.ts";
      const { terminal } = await import(/* @vite-ignore */ url);
      return terminal(2).cols;
    }),
  ).toBeGreaterThan(80);
  expect(await page.evaluate(() => document.fonts.check("13px 'IBM Plex Mono'"))).toBe(true);
  await page.screenshot({ path: "target/e2e/terminals.png" });

  // Tabs belong to their worktree: only refactor-auth's shows now.
  await expect(tabs.getByRole("tab")).toHaveText(["refactor-auth"]);
  // Back to fix-login: its tab shows again, with its output still there.
  await tree.getByRole("button", { name: "fix-login" }).click();
  await expect(tabs.getByRole("tab")).toHaveText(["fix-login"]);
  await expect(tabs.getByRole("tab", { name: "fix-login" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  expect(await screen(page, 1)).toBe("mock$ echo one\necho one\nmock$");

  // Ctrl+Shift+C copies the selection; Ctrl+Shift+V pastes it into the terminal.
  await page.evaluate(async () => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    terminal(1).select(0, 1, 8); // "echo one" on the second line
  });
  await page.keyboard.press("Control+Shift+C");
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe("echo one");
  await page.keyboard.press("Control+Shift+V");
  await expect.poll(() => screen(page, 1)).toBe("mock$ echo one\necho one\nmock$ echo one");

  // `exit` ends the shell: the tab stays, marked exited, until it is closed.
  await tree.getByRole("button", { name: "refactor-auth" }).click();
  await page.keyboard.press("Enter");
  await page.keyboard.type("exit");
  await page.keyboard.press("Enter");
  await expect(tabs.getByRole("tab", { name: "refactor-auth exited" })).toBeVisible();

  await page.getByRole("button", { name: "Close terminal refactor-auth" }).click();
  await expect(tabs.getByRole("tab")).toHaveCount(0);
  await expect(page.getByText("No terminal in refactor-auth")).toBeVisible();
  await tree.getByRole("button", { name: "fix-login" }).click();
  await expect(tabs.getByRole("tab")).toHaveText(["fix-login"]);
  await expect(tabs.getByRole("tab", { name: "fix-login" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await page.getByRole("button", { name: "Close terminal fix-login" }).click();
  await expect(tabs.getByRole("tab")).toHaveCount(0);
});
