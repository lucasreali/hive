import { expect, test } from "@playwright/test";

test("actions: badge, a failed run's jobs and log, a re-run, a cancel asked first", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  // The fake service's shop: fix-login's latest run failed.
  const row = tree.locator(".tree-row.worktree", { hasText: "fix-login" });
  const badge = row.locator(".run-badge");
  await expect(badge).toHaveText("✗");
  await badge.click();

  const panel = page.getByRole("region", { name: "Actions" });
  await expect(panel.locator(".pull-heading")).toHaveText("Fix the login redirect #101");
  const jobs = panel.getByRole("region", { name: "Jobs (2)" });
  await jobs.getByRole("button", { name: "Show the end of its log" }).click();
  const log = panel.locator(".run-log");
  await expect(log).toContainText("##[error]Process completed with exit code 1.");
  await expect(log).toHaveCSS("white-space", "pre");

  // Re-run its failed jobs: running now, so it can be cancelled, once confirmed.
  await panel.getByRole("button", { name: "Re-run failed jobs" }).click();
  await expect(page.locator(".statusbar")).toContainText("Re-running the failed jobs");
  await panel.getByRole("button", { name: "Cancel", exact: true }).click();
  const confirm = page.getByRole("dialog", { name: "Cancel run?" });
  await expect(confirm).toContainText("Cancel CI #101 on worktree-fix-login?");
  await confirm.getByRole("button", { name: "Cancel run" }).click();
  await expect(page.locator(".statusbar")).toContainText("Cancelling the run");
  await expect(panel.getByRole("button", { name: "Re-run all jobs" })).toBeVisible();

  // Back to the list, filtered on the worktree's branch; all branches on demand.
  await panel.getByRole("button", { name: "Runs" }).click();
  await expect(panel.locator(".run .label")).toHaveText(["Fix the login redirect"]);
  await panel.getByRole("combobox", { name: "Branch" }).click();
  await page.getByRole("option", { name: "All branches" }).click();
  await expect(panel.locator(".run")).toHaveCount(4);
});
