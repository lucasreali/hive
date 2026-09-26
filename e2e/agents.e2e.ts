import { expect, test } from "@playwright/test";

test("agents: placed by their cwd, clicking one shows its terminal, exiting removes it", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const tabs = page.getByRole("tablist");
  const plus = page.getByTitle("New terminal, agent or file");
  const agents = tree.locator(".tree-row.agent");
  const rows = () => tree.locator(".tree-row").allTextContents();

  // A terminal opened in shop's main worktree.
  await tree.getByRole("button", { name: "main", exact: true }).first().click();
  await plus.click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect(tabs.getByRole("tab", { name: "main" })).toHaveAttribute("aria-selected", "true");
  // The fake service detects an agent where `claude` runs, not where the terminal opened.
  await page.keyboard.type("cd .claude/worktrees/fix-login");
  await page.keyboard.press("Enter");
  await page.keyboard.type("claude");
  await page.keyboard.press("Enter");
  await expect(agents).toHaveCount(1);
  expect((await rows()).slice(0, 5)).toEqual([
    "shopNew worktree",
    "main",
    "fix-login↑3↓1●2",
    // The icon's name, the title, the state's name and the time in it.
    expect.stringMatching(/^idleClaudeidle\ds$/),
    "feat-checkout↑1●2",
  ]);

  await tree.getByRole("button", { name: "refactor-auth" }).click();
  await plus.click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect(tabs.getByRole("tab", { name: "refactor-auth" })).toHaveAttribute(
    "aria-selected",
    "true",
  );

  // Clicking the agent shows its terminal.
  await tree.getByRole("button", { name: "Claude" }).click();
  await expect(tabs.getByRole("tab", { name: "main" })).toHaveAttribute("aria-selected", "true");
  await expect(tree.getByRole("button", { name: "Claude" })).toHaveAttribute(
    "aria-current",
    "true",
  );

  // Exiting the terminal removes its agent.
  await page.keyboard.type("exit");
  await page.keyboard.press("Enter");
  await expect(agents).toHaveCount(0);
});
