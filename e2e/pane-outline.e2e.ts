import { expect, type Page, test } from "@playwright/test";

/** Sends the app what the service would say: agent `id` runs in terminal `channel`, in `state`. */
function agent(page: Page, id: string, channel: number, state: string) {
  return page.evaluate(
    async ({ id, channel, state }) => {
      const [reduce, mock] = ["/src/reduce.ts", "/src/transport/mock.ts"];
      const { apply } = await import(/* @vite-ignore */ reduce);
      const { agentStatus } = await import(/* @vite-ignore */ mock);
      apply({ type: "agent_detected", channel, id, project: null, worktree: null, cwd: null });
      apply({ type: "agent_state", id, ...agentStatus(state), subagents: [] });
    },
    { id, channel, state },
  );
}

// 17.26: the state tokens of `src/styles.css`, as computed colours.
const COLOURS = {
  "one-dark": { permission: "rgb(222, 193, 132)", error: "rgb(208, 114, 119)" },
  "one-light": { permission: "rgb(164, 136, 25)", error: "rgb(211, 97, 81)" },
};

for (const [theme, colour] of Object.entries(COLOURS)) {
  test(`pane outline: in a split, the pane whose agent needs you is outlined in ${theme} (17.26)`, async ({
    page,
  }) => {
    await page.goto("/");
    const tree = page.getByRole("navigation", { name: "Projects" });
    await tree.getByRole("button", { name: "fix-login" }).click();
    await page.getByTitle("New terminal or file").click();
    await page.getByRole("menuitem", { name: "Terminal" }).click();
    await expect(page.locator(".terminal-pane:visible")).toHaveCount(1);
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);

    // Alone, a pending pane is not outlined: the outline tells two panes apart.
    await agent(page, "a", 1, "waiting_permission");
    const left = page.locator('.terminal-pane[data-pane="left"]');
    const right = page.locator('.terminal-pane[data-pane="right"]');
    const alone = page.locator(".terminal-pane:visible");
    await expect(alone).toHaveCSS("outline-style", "none");

    const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
    await tabs.getByRole("tab").first().click({ button: "right" });
    await page.getByRole("menuitem", { name: "Split right" }).click();
    await expect(page.locator(".terminal-pane:visible")).toHaveCount(2);
    // 🟡 outlines its pane in the permission colour.
    await expect(left).toHaveCSS("outline-style", "solid");
    await expect(left).toHaveCSS("outline-color", colour.permission);
    await expect(left).toHaveCSS("outline-width", "2px");
    await expect(right).toHaveCSS("outline-style", "none");

    // 🔴 in the other pane: the error colour; a 🟠 that finished leaves its pane plain.
    await agent(page, "b", 2, "error");
    await expect(right).toHaveCSS("outline-color", colour.error);
    await agent(page, "a", 1, "waiting_you");
    await expect(left).toHaveCSS("outline-style", "none");
    await expect(right).toHaveCSS("outline-style", "solid");
    await page.screenshot({ path: `target/e2e/pane-outline-${theme}.png` });
  });
}
