import { expect, type Page, test } from "@playwright/test";

/** The text of a terminal's buffer, read from the page's own terminal manager. */
function screen(page: Page, id: number): Promise<string> {
  return page.evaluate(async (id) => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const buffer = terminal(id).buffer.active;
    const lines: string[] = [];
    for (let y = 0; y < buffer.length; y++) lines.push(buffer.getLine(y).translateToString(true));
    return lines.join("\n").trimEnd();
  }, id);
}

test("selected lines of the diff go to the terminal as a reference", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal (Ctrl+Shift+T)").click();
  await expect.poll(() => screen(page, 1)).toBe("mock$");

  await page.keyboard.press("Control+Shift+B");
  const panel = page.getByRole("complementary", { name: "Changes" });
  for (const name of ["src", "auth"]) {
    await panel.getByRole("treeitem", { name, exact: true }).click();
  }
  await panel.getByRole("treeitem", { name: /session\.ts/ }).click();
  const view = page.getByRole("region", { name: "src/auth/session.ts" });
  const send = view.getByRole("button", { name: "Send to terminal" });
  await expect(send).toBeDisabled();
  await expect(send).toHaveAttribute("title", "Select lines to send their reference");

  // The three added lines, 39–41 of the new file, from the start of the first to the end of
  // the last; the removed line above them is not part of the selection.
  const added = view.locator(".cm-changedLine");
  await added.nth(1).click();
  await page.keyboard.press("Home");
  await added.nth(3).click({ modifiers: ["Shift"] });
  await expect(send).toBeEnabled();
  await page.keyboard.press("Control+Shift+L");
  // Written as input, no Enter, and the terminal has the focus: typing goes on after it.
  await page.keyboard.type("why?");
  await expect.poll(() => screen(page, 1)).toBe("mock$ @src/auth/session.ts (lines 39–41) why?");

  // Sending showed the terminal; the file's tab is still there. One line, with the button.
  await page.getByRole("tab", { name: "session.ts" }).click();
  await added.nth(1).click();
  await page.keyboard.press("Shift+Home");
  await send.click();
  await expect
    .poll(() => screen(page, 1))
    .toBe("mock$ @src/auth/session.ts (lines 39–41) why?@src/auth/session.ts (line 39)");
  await page.screenshot({ path: "target/e2e/reference.png" });

  // A file of another worktree cannot go to this terminal.
  await tree.getByRole("button", { name: "refactor-auth" }).click();
  for (const name of ["src", "auth"]) {
    await panel.getByRole("treeitem", { name, exact: true }).click();
  }
  await panel.getByRole("treeitem", { name: /token\.ts/ }).click();
  const other = page.getByRole("region", { name: "src/auth/token.ts" });
  await other.locator(".cm-line").first().click();
  await page.keyboard.press("Shift+Home");
  const blocked = other.getByRole("button", { name: "Send to terminal" });
  await expect(blocked).toBeDisabled();
  await expect(blocked).toHaveAttribute("title", "The active terminal is in another worktree");
});
