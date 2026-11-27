import { expect, type Page, test } from "@playwright/test";
import { HOME, mod } from "./host";

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

test("selected lines of the diff go to the agent as a reference", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect.poll(() => screen(page, 1)).toBe("mock$");
  // Claude runs in it: a reference goes to an agent (28.21).
  await page.keyboard.type("claude");
  await page.keyboard.press("Enter");
  await expect(tree.locator(".tree-row.agent")).toHaveCount(1);

  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Diff" }).click();
  for (const name of ["src", "auth"]) {
    await panel
      .getByRole("treeitem")
      .filter({ has: page.getByText(name, { exact: true }) })
      .click();
  }
  await panel.getByRole("treeitem", { name: /session\.ts/ }).click();
  const view = page.getByRole("region", { name: "src/auth/session.ts" });
  const send = view.getByRole("button", { name: "Send to agent" });
  await expect(send).toBeDisabled();
  await expect(send).toHaveAttribute("title", "Select lines to send their reference");

  // The three added lines, 39–41 of the new file, from the start of the first to the end of
  // the last; the removed line above them is not part of the selection.
  const added = view.locator(".cm-changedLine");
  await added.nth(1).click();
  await page.keyboard.press(HOME);
  await added.nth(3).click({ modifiers: ["Shift"] });
  await expect(send).toBeEnabled();
  await page.keyboard.press(`${mod}+Shift+L`);
  // Written as input, no Enter, and the terminal has the focus: typing goes on after it.
  await page.keyboard.type("why?");
  // The fake service's agent echoes the line that started it.
  const started = "mock$ claude\nclaude\nmock$ ";
  await expect
    .poll(() => screen(page, 1))
    .toBe(`${started}@src/auth/session.ts (lines 39–41) why?`);

  // Sending showed the terminal; the file's tab is still there. One line, with the button.
  await page.getByRole("tab", { name: "session.ts" }).click();
  await added.nth(1).click();
  await page.keyboard.press(`Shift+${HOME}`);
  await send.click();
  await expect
    .poll(() => screen(page, 1))
    .toBe(`${started}@src/auth/session.ts (lines 39–41) why?@src/auth/session.ts (line 39)`);
  await page.screenshot({ path: "target/e2e/reference.png" });

  // In another worktree, fix-login's terminal is not shown (tabs belong to their worktree, 4.7):
  // there is no terminal to send to.
  await tree.getByRole("button", { name: "refactor-auth" }).click();
  for (const name of ["src", "auth"]) {
    await panel
      .getByRole("treeitem")
      .filter({ has: page.getByText(name, { exact: true }) })
      .click();
  }
  await panel.getByRole("treeitem", { name: /token\.ts/ }).click();
  const other = page.getByRole("region", { name: "src/auth/token.ts" });
  await other.locator(".cm-line").first().click();
  await page.keyboard.press(`Shift+${HOME}`);
  const blocked = other.getByRole("button", { name: "Send to agent" });
  await expect(blocked).toBeDisabled();
  await expect(blocked).toHaveAttribute("title", "No agent in this worktree");
});
