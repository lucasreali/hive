import { expect, test } from "@playwright/test";

test("pull requests: list, badge, details, a merge asked first and a refused one", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  // The fake service's shop: fix-login is #12's branch.
  const badge = tree.locator(".tree-row.worktree", { hasText: "fix-login" }).locator(".pull-badge");
  await expect(badge).toHaveText("#12");
  await badge.click();

  const panel = page.getByRole("region", { name: "PRs" });
  await expect(panel.locator(".pull-heading")).toHaveText("Fix the login redirect #12");
  await expect(panel.getByRole("region", { name: "Checks (1)" })).toContainText("test");

  // Back to the list: the worktree's own pull request is marked.
  await panel.getByRole("button", { name: "Pull requests" }).click();
  const yours = panel.getByRole("region", { name: "Yours" });
  await expect(yours.locator(".label")).toHaveText([
    "Fix the login redirect",
    "New checkout flow",
    "Bump dependencies",
  ]);
  await expect(yours.locator(".pull").first()).toHaveAttribute("data-here", "true");

  // GitHub refuses #14's merge: its message shows; nothing merged.
  await panel.getByRole("button", { name: /Rate limit the API/ }).click();
  await panel.getByRole("button", { name: "Merge", exact: true }).click();
  const confirm = page.getByRole("dialog", { name: "Merge pull request?" });
  await expect(confirm).toContainText("(squash and merge)");
  await confirm.getByRole("button", { name: "Merge" }).click();
  await expect(panel.getByRole("alert")).toContainText("At least 1 approving review is required");

  // #12 merges once confirmed.
  await panel.getByRole("button", { name: "Pull requests" }).click();
  await panel.getByRole("button", { name: /Fix the login redirect/ }).click();
  await panel.getByRole("button", { name: "Merge", exact: true }).click();
  await page
    .getByRole("dialog", { name: "Merge pull request?" })
    .getByRole("button", { name: "Merge" })
    .click();
  await expect(panel.locator(".pull-heading + .session-meta .pull-state")).toHaveText("Merged");
  await expect(page.getByRole("status", { name: "Messages" })).toContainText(
    "Merged pull request #12",
  );
  await expect(badge).toHaveAttribute("data-state", "merged");
});

// 10.1: the details' back button shows whole, icon and label, at every right-panel width.
for (const panelWidth of [280, 640]) {
  test(`pull requests: the back button fits its bar at ${panelWidth} px`, async ({ page }) => {
    await page.addInitScript(
      (w) => localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: w })),
      panelWidth,
    );
    await page.goto("/");
    const tree = page.getByRole("navigation", { name: "Projects" });
    await tree
      .locator(".tree-row.worktree", { hasText: "fix-login" })
      .locator(".pull-badge")
      .click();
    const panel = page.getByRole("region", { name: "PRs" });
    await expect(panel.locator(".pull-heading")).toBeVisible();
    const back = panel.getByRole("button", { name: "Pull requests" });
    const fit = await back.evaluate((button) => {
      const bar = button.closest(".pulls-bar") as HTMLElement;
      const box = bar.getBoundingClientRect();
      const inner = box.left + Number.parseFloat(getComputedStyle(bar).paddingLeft);
      const self = button.getBoundingClientRect();
      const icon = (button.querySelector("svg") as SVGElement).getBoundingClientRect();
      return {
        width: box.width,
        inside: self.left >= inner && self.right <= box.right,
        icon: icon.left >= self.left && icon.right <= self.right,
        whole: button.scrollWidth <= button.clientWidth,
      };
    });
    expect(Math.round(fit.width)).toBe(panelWidth - 1);
    expect(fit).toMatchObject({ inside: true, icon: true, whole: true });
    await expect(back).toHaveText("Pull requests");
  });
}
