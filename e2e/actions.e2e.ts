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
  await expect(page.getByRole("status", { name: "Messages" })).toContainText(
    "Re-running the failed jobs",
  );
  await panel.getByRole("button", { name: "Cancel", exact: true }).click();
  const confirm = page.getByRole("dialog", { name: "Cancel run?" });
  await expect(confirm).toContainText("Cancel CI #101 on worktree-fix-login?");
  await confirm.getByRole("button", { name: "Cancel run" }).click();
  await expect(page.getByRole("status", { name: "Messages" })).toContainText("Cancelling the run");
  await expect(panel.getByRole("button", { name: "Re-run all jobs" })).toBeVisible();

  // Back to the list, filtered on the worktree's branch; all branches on demand.
  await panel.getByRole("button", { name: "Runs" }).click();
  await expect(panel.locator(".run .label")).toHaveText(["Fix the login redirect"]);
  await panel.getByRole("combobox", { name: "Branch" }).click();
  await page.getByRole("option", { name: "All branches" }).click();
  await expect(panel.locator(".run")).toHaveCount(4);
});

// 10.1/10.2: the details' back button shows whole, and the runs list's branch picker fits its
// bar like Refresh (height, alignment, inside the padding), at every right-panel width.
for (const panelWidth of [280, 640]) {
  test(`actions: the bar's controls fit it at ${panelWidth} px`, async ({ page }) => {
    await page.addInitScript(
      (w) => localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: w })),
      panelWidth,
    );
    await page.goto("/");
    const tree = page.getByRole("navigation", { name: "Projects" });
    await tree
      .locator(".tree-row.worktree", { hasText: "fix-login" })
      .locator(".run-badge")
      .click();
    const panel = page.getByRole("region", { name: "Actions" });
    await expect(panel.locator(".pull-heading")).toBeVisible();

    const back = panel.getByRole("button", { name: "Runs" });
    const whole = await back.evaluate((button) => {
      const bar = button.closest(".pulls-bar") as HTMLElement;
      const box = bar.getBoundingClientRect();
      const inner = box.left + Number.parseFloat(getComputedStyle(bar).paddingLeft);
      const self = button.getBoundingClientRect();
      const icon = (button.querySelector("svg") as SVGElement).getBoundingClientRect();
      return {
        inside: self.left >= inner && self.right <= box.right,
        icon: icon.left >= self.left && icon.right <= self.right,
        whole: button.scrollWidth <= button.clientWidth,
      };
    });
    expect(whole).toEqual({ inside: true, icon: true, whole: true });
    await expect(back).toHaveText("Runs");

    await back.click();
    const picker = panel.getByRole("combobox", { name: "Branch" });
    // As it shows, then with a branch as long as an agent worktree's, which must shorten rather
    // than push the bar wider than the panel (and the panel sideways).
    for (const label of ["", "worktree-agent-a7caf550bef42b7ca-and-then-some"]) {
      const fit = await picker.evaluate((trigger, label) => {
        if (label) (trigger.querySelector(".select-value") as HTMLElement).textContent = label;
        const bar = trigger.closest(".pulls-bar") as HTMLElement;
        const box = bar.getBoundingClientRect();
        const inner = box.left + Number.parseFloat(getComputedStyle(bar).paddingLeft);
        const self = trigger.getBoundingClientRect();
        const other = (bar.querySelector(".ghost.icon") as HTMLElement).getBoundingClientRect();
        const middle = (r: DOMRect) => r.top + r.height / 2;
        return {
          width: Math.round(box.width),
          height: box.height <= 33 && self.height === other.height,
          inside: self.left >= inner && self.right <= other.left && other.right <= box.right,
          aligned: Math.abs(middle(self) - middle(other)) <= 0.5,
          unscrolled: bar.scrollWidth <= bar.clientWidth,
        };
      }, label);
      expect(fit).toEqual({
        width: panelWidth - 1,
        height: true,
        inside: true,
        aligned: true,
        unscrolled: true,
      });
    }
  });
}
