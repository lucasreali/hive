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

// 12.3: "Create pull request from <branch>" keeps to one line in the bar's font at every panel
// width; only a long branch is cut, with an ellipsis, and the tooltip has the whole label.
for (const panelWidth of [280, 640]) {
  test(`pull requests: the create button fits its panel at ${panelWidth} px`, async ({ page }) => {
    await page.addInitScript(
      (w) => localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: w })),
      panelWidth,
    );
    await page.goto("/");
    const tree = page.getByRole("navigation", { name: "Projects" });
    await tree.getByRole("button", { name: "shop", exact: true }).click({ button: "right" });
    await page.getByRole("menuitem", { name: /^New worktree…/ }).click();
    const dialog = page.getByRole("dialog", { name: "New worktree" });
    const name = "linkedin-profile-sync-for-the-new-onboarding";
    await dialog.getByLabel("Worktree name").fill(name);
    await dialog.getByRole("button", { name: "Create worktree" }).click();
    await expect(tree.getByRole("button", { name, exact: true })).toHaveAttribute(
      "aria-current",
      "true",
    );

    const side = page.getByRole("complementary", { name: "Side panel" });
    await side.getByRole("tab", { name: "PRs" }).click();
    const panel = page.getByRole("region", { name: "PRs" });
    const label = `Create pull request from worktree-${name}`;
    const create = panel.getByRole("button", { name: label });
    await expect(create).toBeVisible();
    await page.screenshot({ path: `target/e2e/pulls-create-${panelWidth}.png` });
    await expect(create).toHaveAttribute("title", label);
    const fit = await create.evaluate((button) => {
      const pulls = button.closest(".pulls") as HTMLElement;
      const box = pulls.getBoundingClientRect();
      const self = button.getBoundingClientRect();
      const branch = button.querySelector(".pulls-create-branch") as HTMLElement;
      const bar = pulls.querySelector(".pulls-bar") as HTMLElement;
      return {
        inside: self.left >= box.left + 12 && self.right <= box.right - 12,
        oneLine: self.height === 28,
        font: getComputedStyle(button).fontSize === getComputedStyle(bar).fontSize,
        cut: branch.scrollWidth > branch.clientWidth,
      };
    });
    expect(fit).toEqual({ inside: true, oneLine: true, font: true, cut: panelWidth === 280 });
  });
}
