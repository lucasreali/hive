import { expect, type Page, test } from "@playwright/test";

// 20.3: `/clear` in Claude Code ends its session and starts a new one in the same terminal
// (the mock sends `agent_removed` then `agent_detected` with a new id, and clears the screen and
// its scrollback); the terminal stays shown and keeps taking input.

/** The text on terminal `id`'s screen and scrollback. */
const text = (page: Page, id: number) =>
  page.evaluate(async (id) => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const buffer = terminal(id).buffer.active;
    const lines: string[] = [];
    for (let i = 0; i < buffer.length; i++) lines.push(buffer.getLine(i).translateToString(true));
    return lines.join("\n");
  }, id);

async function run(page: Page, line: string) {
  await page.keyboard.type(line);
  await page.keyboard.press("Enter");
}

for (const how of ["its tab", "its agent (the inbox, the palette)"]) {
  test(`clear: the terminal still renders and takes input, shown from ${how}`, async ({ page }) => {
    await page.goto("/");
    const tree = page.getByRole("navigation", { name: "Projects" });
    const agents = tree.locator(".tree-row.agent");
    await tree.getByRole("button", { name: "fix-login" }).click();
    await page.getByTitle("New terminal or file").click();
    await page.getByRole("menuitem", { name: "Terminal" }).click();
    const pane = page.locator(".terminal-pane:not([hidden])");
    await expect(pane).toHaveCount(1);
    const id = Number(await pane.getAttribute("data-terminal"));
    await run(page, "claude");
    await expect(agents).toHaveCount(1);
    // Prompts with a command's status: decorations in the scrollback `/clear` erases.
    await run(page, "true");
    await run(page, "false");
    await expect(page.locator(".terminal-pane .shell-mark")).toHaveCount(3);
    if (how !== "its tab") {
      // As the inbox and the palette go to an agent: the agent is the selection.
      await page.evaluate(async () => {
        const [store, shortcuts] = ["/src/store.ts", "/src/shortcuts.ts"];
        const { useHive } = await import(/* @vite-ignore */ store);
        const { goToAgent } = await import(/* @vite-ignore */ shortcuts);
        goToAgent(Object.values(useHive.getState().agents)[0]);
      });
      await pane.click();
    }

    await run(page, "/clear");
    // The new session is the terminal's agent.
    await expect(agents).toHaveCount(1);
    await expect(pane).toHaveCount(1);
    await expect(pane).toBeVisible();
    expect(await text(page, id)).not.toContain("false");
    // The keyboard still reaches the terminal, and its echo is drawn.
    await run(page, "echo after-clear");
    await expect.poll(() => text(page, id)).toContain("echo after-clear");
    await expect(pane.locator(".xterm-helper-textarea")).toBeFocused();
  });
}
