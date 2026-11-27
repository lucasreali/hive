import { expect, type Page, test } from "@playwright/test";

/** Records each terminal write the page makes, as [terminal, text]; returns a reader. */
async function recordWrites(page: Page): Promise<() => Promise<[number, string][]>> {
  await page.evaluate(async () => {
    const url = "/src/transport/index.ts";
    const { transport } = await import(/* @vite-ignore */ url);
    const write = transport.writeTerminal.bind(transport);
    const w = window as unknown as { writes: [number, string][] };
    w.writes = [];
    transport.writeTerminal = (id: number, data: string) => {
      w.writes.push([id, data]);
      return write(id, data);
    };
  });
  return () => page.evaluate(() => (window as unknown as { writes: [number, string][] }).writes);
}

test("review comments go to the agent picked, never the active dev server's terminal", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const terminal = async (...lines: string[]) => {
    await page.getByTitle("New terminal or file").click();
    await page.getByRole("menuitem", { name: "Terminal" }).click();
    await expect(page.locator(".terminal-pane:visible textarea")).toBeFocused();
    for (const line of lines) {
      await page.keyboard.type(line);
      await page.keyboard.press("Enter");
    }
  };
  // Two agents, named as their sessions; then a dev server, the active tab.
  await terminal("claude", "title fix the login");
  await terminal("claude", "title write the tests");
  await terminal("pnpm dev");
  await expect(tabs.getByRole("tab", { name: "write the tests" })).toBeVisible();
  await expect(tabs.getByRole("tab", { name: "fix-login" })).toHaveAttribute(
    "aria-selected",
    "true",
  );

  // A comment of the worktree, as the service keeps it.
  await page.evaluate(async () => {
    const [store, index] = ["/src/store.ts", "/src/transport/index.ts"];
    const { useHive } = await import(/* @vite-ignore */ store);
    const { transport } = await import(/* @vite-ignore */ index);
    const worktree = useHive.getState().selection;
    await transport.addComment(worktree, "src/auth/session.ts", 39, 39, "rename this");
  });
  const writes = await recordWrites(page);
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel
    .getByRole("tablist", { name: "Panel" })
    .getByRole("tab", { name: "Comments" })
    .click();
  const send = panel.getByRole("region", { name: "Open" }).getByRole("button", { name: "Send" });

  // Two agents: a menu by the button lists them, with their state; the dev server is not there.
  await send.click();
  const menu = page.getByRole("menu", { name: "Send to agent" });
  // The title line set each one working, as a prompt would.
  await expect(menu.getByRole("menuitem")).toHaveText([
    /fix the login.*working · title fix the login/,
    /write the tests.*working/,
  ]);
  await page.screenshot({ path: "target/e2e/comments-to-agent.png" });
  await menu.getByRole("menuitem", { name: /write the tests/ }).click();
  await expect(menu).toBeHidden();

  // The comment lands in the picked agent's terminal, whose tab is shown and focused.
  const agentTerminal = 2;
  await expect
    .poll(writes)
    .toEqual([[agentTerminal, "@src/auth/session.ts (line 39) — rename this"]]);
  await expect(tabs.getByRole("tab", { name: "write the tests" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.locator(".terminal-pane:visible textarea")).toBeFocused();

  // The pick is remembered: it comes first next time.
  await send.click();
  await expect(menu.getByRole("menuitem").first()).toHaveText(/write the tests/);
  await page.keyboard.press("Escape");
  await expect(menu).toBeHidden();
});
