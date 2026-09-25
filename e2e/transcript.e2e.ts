import { expect, test } from "@playwright/test";

test("transcript: clicking a subagent shows its conversation; Back returns to the terminal", async ({
  page,
}) => {
  await page.goto("/?mock=states");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
  await tree.getByRole("button", { name: "fix-login", exact: true }).click();
  await page.getByTitle("New terminal (Ctrl+Shift+T)").click();
  const tab = tabs.getByRole("tab", { name: "fix-login" });
  await expect(tab).toHaveAttribute("aria-selected", "true");
  const terminal = page.locator(".terminal-host");
  await expect(terminal).toBeVisible();

  // fix-login's agent runs a4 (Explore); shop's main one runs another Explore first.
  await tree
    .getByRole("button", { name: /subagent: Explore/ })
    .nth(1)
    .click();
  const view = page.getByRole("region", { name: "Subagent conversation" });
  await expect(view.locator(".file-view-bar")).toContainText("subagent: Explore");
  await expect(view.locator(".state-icon")).toHaveAttribute("data-state", "idle");
  const entries = view.locator(".transcript-entry");
  await expect(entries).toHaveCount(4);
  await expect(entries.first()).toContainText("Find where the login form is handled (a4).");
  await expect(entries.nth(2)).toContainText("Grep");
  await expect(terminal).toBeHidden();
  await expect(tab).toHaveAttribute("aria-selected", "false");

  await view.getByRole("button", { name: "Back to terminal" }).click();
  await expect(view).toBeHidden();
  await expect(terminal).toBeVisible();
  await expect(tab).toHaveAttribute("aria-selected", "true");
});
