import { expect, type Page, test } from "@playwright/test";
import { HOME, mod } from "./host";

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

test("review comments on the diff are kept, listed, pasted into the agent and resolved", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect(page.locator(".terminal-pane:visible textarea")).toBeFocused();
  // Claude runs in it: comments go to an agent (28.21).
  await page.keyboard.type("claude");
  await page.keyboard.press("Enter");
  await expect(tree.locator(".tree-row.agent")).toHaveCount(1);
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
    await panel
      .getByRole("treeitem")
      .filter({ has: page.getByText(name, { exact: true }) })
      .click();
  }
  await panel.getByRole("treeitem", { name: /session\.ts/ }).click();
  const view = page.getByRole("region", { name: "src/auth/session.ts" });
  const comment = view.getByRole("button", { name: "Comment", exact: true });
  await expect(comment).toBeDisabled();

  // Lines 39–41 (the added ones), then Ctrl+Shift+M, a comment and Enter.
  const added = view.locator(".cm-changedLine");
  await added.nth(1).click();
  await page.keyboard.press(HOME);
  await added.nth(3).click({ modifiers: ["Shift"] });
  await page.keyboard.press(`${mod}+Shift+M`);
  const input = view.getByLabel("Lines 39–41");
  await expect(input).toBeFocused();
  await page.keyboard.type("handle the expired token");
  await page.keyboard.press("Enter");
  await expect(input).toBeHidden();
  await expect(view.locator(".cm-line.cm-commented")).toHaveCount(3);

  // One line, with the button; Escape drops a comment being written.
  await added.nth(1).click();
  await page.keyboard.press(`Shift+${HOME}`);
  await comment.click();
  await page.keyboard.type("dropped");
  await page.keyboard.press("Escape");
  await added.nth(1).click();
  await page.keyboard.press(`Shift+${HOME}`);
  await comment.click();
  await page.keyboard.type("rename this");
  await page.keyboard.press("Enter");

  // The service keeps them: the right panel's Comments view lists them, open.
  await panel
    .getByRole("tablist", { name: "Panel" })
    .getByRole("tab", { name: "Comments" })
    .click();
  const open = panel.getByRole("region", { name: "Open" });
  await expect(open.locator(".comment-body")).toHaveText([
    "handle the expired token",
    "rename this",
  ]);
  await expect(open.locator(".comment-where")).toHaveText([
    "src/auth/session.ts:39–41",
    "src/auth/session.ts:39",
  ]);
  await page.screenshot({ path: "target/e2e/review.png" });

  // One text, bracketed so its newline does not submit; no Enter; the terminal is shown.
  expect(await writes()).toEqual([]);
  await panel.getByRole("button", { name: "Send all open (2)" }).click();
  await expect
    .poll(writes)
    .toEqual([
      "\x1b[200~@src/auth/session.ts (lines 39–41) — handle the expired token\r" +
        "@src/auth/session.ts (line 39) — rename this\x1b[201~",
    ]);
  await expect(view).toBeHidden();
  await expect(page.locator(".terminal-pane:visible textarea")).toBeFocused();

  // Sent, they stay open; one sent alone, then resolved: it leaves the open ones and its mark.
  const second = open.getByRole("listitem").nth(1);
  await second.getByRole("button", { name: "Send" }).click();
  await expect
    .poll(async () => (await writes()).at(-1))
    .toBe("\x1b[200~@src/auth/session.ts (line 39) — rename this\x1b[201~");
  await second.getByRole("button", { name: "Resolve" }).click();
  await expect(open.locator(".comment-body")).toHaveText(["handle the expired token"]);
  const resolved = panel.getByRole("region", { name: "Resolved" });
  await expect(resolved.locator(".comment-body")).toHaveText(["rename this"]);
  await page.getByRole("tab", { name: "session.ts" }).click();
  await expect(view.locator(".cm-line.cm-commented")).toHaveCount(3);
  // Reopened, then deleted.
  await resolved.getByRole("button", { name: "Reopen" }).click();
  await expect(open.locator(".comment-body")).toHaveCount(2);
  await open.getByRole("button", { name: "Delete comment" }).first().click();
  await expect(open.locator(".comment-body")).toHaveText(["rename this"]);
  await expect(view.locator(".cm-line.cm-commented")).toHaveCount(1);
});
