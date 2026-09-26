import { expect, type Page, test } from "@playwright/test";

/** Records what the page writes into terminals; returns a reader of those writes. */
async function recordWrites(page: Page): Promise<() => Promise<string[]>> {
  await page.evaluate(async () => {
    const url = "/src/transport/index.ts";
    const { transport } = await import(/* @vite-ignore */ url);
    const write = transport.writeTerminal.bind(transport);
    const w = window as unknown as { writes: string[] };
    w.writes = [];
    transport.writeTerminal = (id: number, data: string) => {
      w.writes.push(data);
      return write(id, data);
    };
  });
  return () => page.evaluate(() => (window as unknown as { writes: string[] }).writes);
}

test("review comments on the diff are pasted into the terminal as one text", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal (Ctrl+Shift+T)").click();
  await expect(page.locator(".terminal-pane:visible textarea")).toBeFocused();
  // The program in the terminal asks for bracketed paste, as Claude Code does.
  await page.evaluate(async () => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    terminal(1).write("\x1b[?2004h");
  });
  const writes = await recordWrites(page);

  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Diff" }).click();
  for (const name of ["src", "auth"]) {
    await panel.getByRole("treeitem", { name, exact: true }).click();
  }
  await panel.getByRole("treeitem", { name: /session\.ts/ }).click();
  const view = page.getByRole("region", { name: "src/auth/session.ts" });
  const comment = view.getByRole("button", { name: "Comment", exact: true });
  await expect(comment).toBeDisabled();

  // Lines 39–41 (the added ones), then Ctrl+Shift+M, a comment and Enter.
  const added = view.locator(".cm-changedLine");
  await added.nth(1).click();
  await page.keyboard.press("Home");
  await added.nth(3).click({ modifiers: ["Shift"] });
  await page.keyboard.press("Control+Shift+M");
  const input = view.getByLabel("Lines 39–41");
  await expect(input).toBeFocused();
  await page.keyboard.type("handle the expired token");
  await page.keyboard.press("Enter");
  await expect(input).toBeHidden();
  await expect(view.locator(".cm-line.cm-commented")).toHaveCount(3);

  // One line, with the button; Escape drops a comment being written.
  await added.nth(1).click();
  await page.keyboard.press("Shift+Home");
  await comment.click();
  await page.keyboard.type("dropped");
  await page.keyboard.press("Escape");
  await added.nth(1).click();
  await page.keyboard.press("Shift+Home");
  await comment.click();
  await page.keyboard.type("rename this");
  await page.keyboard.press("Enter");

  const review = view.getByRole("region", { name: "Review comments" });
  await expect(review.getByRole("listitem")).toHaveText([
    "src/auth/session.ts:39–41handle the expired token",
    "src/auth/session.ts:39rename this",
  ]);
  await page.screenshot({ path: "target/e2e/review.png" });

  // One text, bracketed so its newline does not submit; no Enter; the terminal is shown.
  expect(await writes()).toEqual([]);
  await review.getByRole("button", { name: "Send review (2)" }).click();
  await expect
    .poll(writes)
    .toEqual([
      "\x1b[200~@src/auth/session.ts (lines 39–41) — handle the expired token\r" +
        "@src/auth/session.ts (line 39) — rename this\x1b[201~",
    ]);
  await expect(view).toBeHidden();
  await expect(page.locator(".terminal-pane:visible textarea")).toBeFocused();
  await page.getByRole("tab", { name: "session.ts" }).click();
  await expect(review).toBeHidden();
  await expect(view.locator(".cm-line.cm-commented")).toHaveCount(0);
});
