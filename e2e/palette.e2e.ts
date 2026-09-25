import { expect, test } from "@playwright/test";

test("palette: Ctrl+Shift+P runs commands, goes to worktrees and opens files", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await expect(tree.getByRole("button", { name: "fix-login" })).toBeVisible();
  const palette = page.getByRole("dialog", { name: "Command palette" });
  const field = palette.getByRole("textbox", { name: /Search commands/ });

  // A command, with its shortcut shown, run with Enter.
  await page.keyboard.press("Control+Shift+P");
  await expect(field).toBeFocused();
  await expect(palette.getByRole("button", { name: /Open settings/ })).toContainText("Ctrl+,");
  await page.screenshot({ path: "target/e2e/palette.png" });
  await page.keyboard.type("opset");
  await page.keyboard.press("Enter");
  await expect(palette).toBeHidden();
  const settings = page.getByRole("dialog", { name: "Settings" });
  await expect(settings).toBeVisible();
  // Esc, then at once Ctrl+Shift+P: the settings let go on `cancel`, before their `close`.
  await page.keyboard.press("Escape");

  // A worktree: Enter selects it.
  await page.keyboard.press("Control+Shift+P");
  await page.keyboard.type("fix-login");
  await expect(palette.getByRole("region", { name: "Agents and worktrees" })).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(palette).toBeHidden();
  await expect(tree.getByRole("button", { name: "fix-login" })).toHaveAttribute(
    "aria-current",
    "true",
  );

  // Text in the selected worktree's files: a click opens the file at that line.
  await page.keyboard.press("Control+Shift+P");
  await page.keyboard.type("rememberMe");
  const files = palette.getByRole("region", { name: "Files" });
  await files
    .getByRole("button", { name: /src\/auth\/session\.ts:\d+/ })
    .first()
    .click();
  await expect(palette).toBeHidden();
  await expect(page.getByRole("tab", { name: /session\.ts/ })).toHaveAttribute(
    "aria-selected",
    "true",
  );

  // Nothing matches: said so; Esc closes.
  await page.keyboard.press("Control+Shift+P");
  await page.keyboard.type("zzzqqqxxx");
  await expect(palette.getByText("No matches")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(palette).toBeHidden();
});
