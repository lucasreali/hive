import { expect, test } from "@playwright/test";

// 27.6: commit, push and run a workflow from Hive, each a click asked first, on the mock.

test("commit: stage a file by its box, commit it asked first, then push the branch", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Diff" }).click();
  const box = panel.getByRole("region", { name: "Commit" });
  await expect(box).toContainText("0 of 2 staged");
  const files = panel.getByRole("tree", { name: "Files" });
  await files.getByRole("treeitem").filter({ hasText: "src" }).first().click();
  await files.getByRole("treeitem").filter({ hasText: "auth" }).first().click();
  // The box stages the file without opening it.
  await files.getByRole("checkbox", { name: "Stage src/auth/session.ts" }).click();
  await expect(files.getByRole("checkbox", { name: "Unstage src/auth/session.ts" })).toBeChecked();
  await expect(box).toContainText("1 of 2 staged");
  await expect(page.getByRole("tab", { name: "session.ts" })).toHaveCount(0);

  const commit = box.getByRole("button", { name: "Commit" });
  await expect(commit).toBeDisabled();
  await box.getByLabel("Commit message").fill("Keep the session longer");
  await commit.click();
  const confirm = page.getByRole("dialog", { name: "Commit?" });
  await expect(confirm).toContainText(
    "Commit 1 staged file on worktree-fix-login as “Keep the session longer”?",
  );
  await confirm.getByRole("button", { name: "Commit" }).click();
  await expect(page.getByRole("status", { name: "Messages" })).toContainText(
    "Committed 4f2a9c1: Keep the session longer",
  );
  await expect(files.getByRole("checkbox", { name: /^(Un)?[Ss]tage / })).toHaveCount(1);
  await expect(box.getByLabel("Commit message")).toHaveValue("");

  // Push: where it goes first, then pushed once asked.
  await box.getByRole("button", { name: "Push…" }).click();
  const push = page.getByRole("dialog", { name: "Push" });
  await expect(push).toContainText(
    "Push worktree-fix-login to origin/worktree-fix-login, and make it its upstream?",
  );
  await push.getByRole("button", { name: "Push" }).click();
  await expect(push).toHaveCount(0);
  await expect(page.getByRole("status", { name: "Messages" })).toContainText(
    "Pushed worktree-fix-login to origin/worktree-fix-login",
  );
});

test("push: a rejected push shows git's message and pushes nothing", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "refactor-auth" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Push…" }).click();
  const push = page.getByRole("dialog", { name: "Push" });
  await push.getByRole("button", { name: "Push" }).click();
  await expect(push.getByRole("alert")).toContainText("! [rejected]");
  await expect(push.getByRole("button", { name: "Push" })).toBeEnabled();
  await push.getByRole("button", { name: "Cancel" }).click();
  await expect(push).toHaveCount(0);
});

test("run workflow: inputs asked, then the new run followed in the Actions view", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const side = page.getByRole("complementary", { name: "Side panel" });
  await side.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Actions" }).click();
  const actions = page.getByRole("region", { name: "Actions" });
  await actions.getByRole("button", { name: "Run workflow…" }).click();
  const dialog = page.getByRole("dialog", { name: "Run workflow" });
  await dialog.getByRole("combobox", { name: "Workflow" }).click();
  await page.getByRole("option", { name: "Release (release.yml)" }).click();
  await dialog.getByRole("combobox", { name: "level *" }).click();
  await page.getByRole("option", { name: "minor" }).click();
  await dialog.getByLabel("Branch").fill("main");
  await dialog.getByRole("button", { name: "Run" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("status", { name: "Messages" })).toContainText(
    "Started release.yml on main",
  );
  // The new run's jobs show, as a re-run's do.
  await expect(actions.locator(".pull-heading")).toContainText("Release");
});
