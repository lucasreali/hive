import { expect, test } from "@playwright/test";

test("removed worktrees: the row disappears, its project is selected, its tab stays", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const fixLogin = tree.getByRole("button", { name: "fix-login" });

  await fixLogin.click();
  await page.getByTitle("New terminal (Ctrl+Shift+T)").click();
  const tab = page.getByRole("tablist").getByRole("tab", { name: "fix-login" });
  await expect(tab).toHaveAttribute("aria-selected", "true");
  await expect(fixLogin).toHaveAttribute("aria-current", "true");

  // The fake service's stand-in for a `WorktreeRemove` hook.
  await page.keyboard.type("worktree-remove fix-login");
  await page.keyboard.press("Enter");
  await expect(fixLogin).toHaveCount(0);
  await expect(tree.getByRole("button", { name: "shop", exact: true })).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(tree.getByRole("button", { name: "feat-checkout" })).toBeVisible();
  await expect(tab).toBeVisible();
});
