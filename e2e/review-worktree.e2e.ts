import { expect, test } from "@playwright/test";

test("a review worktree: made under its worktree, brought back on the click, discarded", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const main = tree.getByRole("button", { name: "main", exact: true }).first();

  await main.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Review in a new worktree…" }).click();
  const create = page.getByRole("dialog", { name: "Review in a new worktree" });
  await expect(create).toContainText("from the tip of main");
  await create.getByRole("button", { name: "Create review" }).click();
  await expect(create).toHaveCount(0);

  // Under main, marked, and selected.
  const review = tree.getByRole("button", { name: /^review-main/ });
  await expect(review).toHaveAttribute("aria-current", "true");
  await expect(review.locator(".review-badge")).toHaveAttribute("title", "A review of main");
  const rows = tree.locator(".tree-row.worktree .label");
  await expect(rows).toHaveText([
    "main",
    "review-main",
    "fix-login",
    "feat-checkout",
    "main",
    "refactor-auth",
  ]);

  // Nothing merges until "Bring back" is clicked (#12).
  await review.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Bring changes back…" }).click();
  const bring = page.getByRole("dialog", { name: "Bring changes back" });
  await expect(bring).toContainText("2 commits of review-main go into main, with a merge commit:");
  await expect(bring.locator(".review-commits li")).toHaveText([
    "9f1c2abKeep the redirect after a login",
    "4d7e013Test the expired session",
  ]);
  await bring.getByRole("button", { name: "Bring back" }).click();
  await expect(bring).toHaveCount(0);
  await expect(page.getByText("Brought 2 commits back into main (merge commit).")).toBeVisible();

  // A review of a worktree with changes is refused with the service's words.
  await tree.getByRole("button", { name: /^fix-login/ }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Review in a new worktree…" }).click();
  await page.getByRole("button", { name: "Create review" }).click();
  await tree.getByRole("button", { name: /^review-fix-login/ }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Bring changes back…" }).click();
  await page.getByRole("button", { name: "Bring back" }).click();
  await expect(
    page.getByRole("dialog", { name: "Bring changes back" }).getByRole("alert"),
  ).toHaveText("fix-login has uncommitted changes: commit or discard them first");
  await page.keyboard.press("Escape");

  await review.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Discard review…" }).click();
  const discard = page.getByRole("dialog", { name: "Discard review" });
  await discard.getByRole("button", { name: "Discard", exact: true }).click();
  await expect(discard).toHaveCount(0);
  await expect(tree.getByRole("button", { name: /^review-main/ })).toHaveCount(0);
});
