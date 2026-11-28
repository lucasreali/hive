import { expect, type Page, test } from "@playwright/test";
import { keyLabel, mac, mod } from "./host";

// The keymap (28.7): a chord recorded in Settings → Keyboard runs its command, also with the
// focus in a terminal; the chord it no longer has goes back to the terminal; a conflict shows
// on both rows; Reset brings the default back.

/** Records every write the page's terminals send to the (mock) service. */
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

const row = (page: Page, label: string) =>
  page.locator(".keymap-row").filter({ has: page.locator(".keymap-label", { hasText: label }) });

async function keyboardSettings(page: Page) {
  await page.keyboard.press(`${mod}+,`);
  const settings = page.getByRole("dialog", { name: "Settings" });
  await settings.getByRole("button", { name: "Keyboard" }).click();
  return settings;
}

test("a rebound shortcut runs from a terminal, its old chord reaches the terminal, Reset undoes it", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await expect(tree.getByRole("button", { name: "fix-login" })).toBeVisible();
  await page.keyboard.press(`${mod}+Shift+T`);
  await page.keyboard.type("shop");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  const input = page.locator(".terminal-pane:visible textarea");
  await expect(input).toBeFocused();
  const writes = await recordWrites(page);

  const settings = await keyboardSettings(page);
  const palette = row(page, "Command palette");
  await expect(palette.locator("kbd")).toHaveText([keyLabel("Ctrl+Shift+P")]);
  // Record ⇧⌘K (Ctrl+Shift+K elsewhere); Esc while recording ends the recording only.
  await palette.getByRole("button", { name: "Add a shortcut to Command palette" }).click();
  await page.keyboard.press("Escape");
  await expect(settings).toBeVisible();
  await palette.getByRole("button", { name: "Add a shortcut to Command palette" }).click();
  await expect(
    palette.getByRole("textbox", { name: "New shortcut for Command palette" }),
  ).toBeFocused();
  await page.keyboard.press(`${mod}+Shift+K`);
  await expect(palette.locator("kbd")).toHaveText([
    keyLabel("Ctrl+Shift+P"),
    keyLabel("Ctrl+Shift+K"),
  ]);
  // A chord another command has: both rows say so once kept.
  await palette.getByRole("button", { name: "Add a shortcut to Command palette" }).click();
  await page.keyboard.press(`${mod}+Shift+M`);
  await expect(palette.getByRole("status")).toContainText(
    "is also the shortcut of Comment on the selected lines",
  );
  await palette.getByRole("button", { name: "Keep both" }).click();
  const comment = row(page, "Comment on the selected lines");
  await expect(comment.locator(".field-warning")).toHaveText(
    `${keyLabel("Ctrl+Shift+M")} runs Command palette`,
  );
  await expect(palette.locator(".field-warning")).toContainText("this one runs");
  await page.screenshot({ path: "target/e2e/keymap-conflict.png" });
  // Only ⇧⌘K left.
  for (const chord of ["Ctrl+Shift+M", "Ctrl+Shift+P"]) {
    await palette
      .getByRole("button", { name: `Remove ${keyLabel(chord)} from Command palette` })
      .click();
  }
  await expect(palette.locator("kbd")).toHaveText([keyLabel("Ctrl+Shift+K")]);
  await expect(comment.locator(".field-warning")).toHaveCount(0);
  // F8 taken from its command goes to the terminal.
  const pending = row(page, "Go to the next pending agent");
  await pending
    .getByRole("button", { name: "Remove F8 from Go to the next pending agent" })
    .click();
  await expect(pending.locator("kbd")).toHaveText(mac ? ["⇧⌘J"] : []);
  await page.keyboard.press("Escape");
  await expect(settings).toBeHidden();

  // In the terminal: the new chord opens the palette; the old one no longer does, and F8 is
  // the terminal's (its escape sequence).
  await input.focus();
  await page.keyboard.press(`${mod}+Shift+K`);
  const dialog = page.getByRole("dialog", { name: "Command palette" });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await input.focus();
  await page.keyboard.press(`${mod}+Shift+P`);
  await expect(dialog).toBeHidden();
  await page.keyboard.press("F8");
  await expect.poll(writes).toContain("\x1b[19~");

  // Reset: the default again.
  await keyboardSettings(page);
  await palette.getByRole("button", { name: "Reset Command palette" }).click();
  await expect(palette.locator("kbd")).toHaveText([keyLabel("Ctrl+Shift+P")]);
  await page.keyboard.press("Escape");
  await input.focus();
  await page.keyboard.press(`${mod}+Shift+P`);
  await expect(dialog).toBeVisible();
});
