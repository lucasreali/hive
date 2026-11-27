import { expect, type Page, test } from "@playwright/test";

// 28.8 against the mock transport: its typed stand-ins `bg-start <id> <command>` and
// `bg-end <id> <status> [exit]` send what the service makes of an agent's background commands.

async function run(page: Page, line: string) {
  await page.keyboard.type(line);
  await page.keyboard.press("Enter");
}

test("an agent's background commands show under it and their end lands in the inbox", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const tab = page.getByRole("tablist").getByRole("tab", { name: "fix-login" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect(tab).toHaveAttribute("aria-selected", "true");
  await run(page, "claude");
  await expect(tree.locator(".tree-row.agent")).toHaveCount(1);

  // Two commands start: one line each under the agent, the full command in its title.
  await run(page, "bg-start b1 npm run dev");
  await run(page, "bg-start b2 make build");
  const lines = tree.locator(".background-line");
  await expect(lines).toHaveCount(2);
  await expect(lines.nth(0)).toHaveAttribute("title", "npm run dev");
  await expect(lines.nth(1).locator(".subagent-activity")).toHaveText("make build");

  // One ends well (quiet), one fails: both stay listed with their result, the failure red.
  await run(page, "bg-end b1 completed 0");
  await run(page, "bg-end b2 completed 3");
  await expect(lines.nth(0).locator(".subagent-time")).toHaveText("done");
  await expect(lines.nth(1).locator(".subagent-time")).toHaveText("exit 3");
  await expect(lines.nth(1)).toHaveAttribute("data-failed", "true");
  await expect(lines.nth(0)).not.toHaveAttribute("data-failed");

  // Both are inbox items; the failure's is red, and its click shows the agent's terminal.
  await tree.getByRole("button", { name: "refactor-auth" }).click();
  await page.locator(".pending-bell").click();
  await expect(
    page.getByRole("menuitem", { name: "Background command finished: npm run dev" }),
  ).toBeVisible();
  const failed = page.getByRole("menuitem", {
    name: "Background command failed (exit 3): make build",
  });
  await expect(failed.locator(".inbox-terminal")).toHaveAttribute("data-failed", "true");
  await failed.click();
  await expect(tab).toHaveAttribute("aria-selected", "true");

  // Ten seconds after they ended, they leave the row.
  await expect(lines).toHaveCount(0, { timeout: 15_000 });
});
