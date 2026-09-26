import { expect, test } from "@playwright/test";

test('the "+" menu: from the keyboard, an agent and a new file', async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
  const plus = page.getByTitle("New terminal, agent or file");
  const item = (name: string) => page.getByRole("menuitem", { name });
  await tree.getByRole("button", { name: "fix-login" }).click();

  // Enter opens it on its first item, arrows move, Esc closes it back on "+".
  await plus.focus();
  await page.keyboard.press("Enter");
  await expect(item("Terminal")).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(item("Agent")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(plus).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(item("Terminal")).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(tabs.getByRole("tab", { name: "fix-login" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.getByRole("menu")).toHaveCount(0);

  // Agent: the in-app chat (7.3), after the first chat's folder confirmation.
  await plus.click();
  await item("Agent").click();
  await page.getByRole("button", { name: "Start chat Enter" }).click();
  await expect(page.getByRole("region", { name: "Chat" })).toBeVisible();

  // New file: its name first, then it opens in the editor.
  await plus.click();
  await item("New file…").click();
  const dialog = page.getByRole("dialog", { name: "New file" });
  await expect(dialog.getByText("(worktree root)")).toBeVisible();
  await page.keyboard.type("notes.md");
  await page.keyboard.press("Enter");
  await expect(dialog).toHaveCount(0);
  await expect(tabs.getByRole("tab", { name: "notes.md" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
});
