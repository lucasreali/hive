import { expect, test } from "@playwright/test";

test("agents: placed by their cwd, clicking one shows its terminal, exiting removes it", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const tabs = page.getByRole("tablist");
  const newTerminal = page.getByTitle("New terminal (Ctrl+Shift+T)");
  const agents = tree.locator(".tree-row.agent");
  const rows = () => tree.locator(".tree-row").allTextContents();

  // A terminal opened in shop's main worktree.
  await tree.getByRole("button", { name: "main", exact: true }).first().click();
  await newTerminal.click();
  await expect(tabs.getByRole("tab", { name: "main" })).toHaveAttribute("aria-selected", "true");
  // The fake service detects an agent where `claude` runs, not where the terminal opened.
  await page.keyboard.type("cd .claude/worktrees/fix-login");
  await page.keyboard.press("Enter");
  await page.keyboard.type("claude");
  await page.keyboard.press("Enter");
  await expect(agents).toHaveCount(1);
  expect((await rows()).slice(0, 5)).toEqual([
    "shop",
    "main",
    "fix-login↑3↓1●2",
    // The icon's name, the title, the state's name and the time in it.
    expect.stringMatching(/^idleClaudeidle\ds$/),
    "feat-checkout↑1●2",
  ]);
  // A worktree with agents (chevron) and one without line up their branch icons (7.13).
  const iconX = async (name: string) =>
    (
      await tree
        .locator(".tree-row.worktree", { hasText: name })
        .locator(".row-main > svg")
        .boundingBox()
    )?.x;
  await expect(tree.getByRole("button", { name: "Collapse fix-login" })).toBeVisible();
  await expect(tree.getByRole("button", { name: "Collapse feat-checkout" })).toHaveCount(0);
  expect(await iconX("fix-login")).toBe(await iconX("feat-checkout"));
  expect(await iconX("fix-login")).toBeGreaterThan(0);

  await tree.getByRole("button", { name: "refactor-auth" }).click();
  await newTerminal.click();
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
