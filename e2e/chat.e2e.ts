import { expect, test } from "@playwright/test";

test("chat: confirm the folder, send, entries arrive, a tool opens, Stop while busy, close", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New agent chat").click();

  // The first chat in a folder asks first.
  const dialog = page.getByRole("dialog", { name: "Chat in this folder?" });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(dialog).toBeHidden();

  const chat = page.getByRole("region", { name: "Chat" });
  await expect(chat.getByText("Send a message to start.")).toBeVisible();
  await expect(tabs.getByRole("tab", { name: "fix-login" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  const input = chat.getByRole("textbox", { name: "Message" });
  await input.fill("list the files");
  await input.press("Shift+Enter");
  await input.press("Enter");
  await expect(input).toHaveValue("");
  // While the turn runs, Send turns into Stop.
  await expect(chat.getByRole("button", { name: "Stop" })).toBeVisible();
  const entries = chat.locator(".transcript-entry");
  await expect(entries.first()).toHaveText("Youlist the files");
  const tool = chat.locator('.transcript-entry[data-role="tool"]');
  await expect(tool).toHaveAttribute("data-status", "ok");
  // Live text grows in place, then the turn ends with its usage.
  await expect(chat.locator('[data-role="assistant"]').last()).toContainText(
    "a README.md and a src folder.",
  );
  await expect(chat.locator('[data-role="usage"]')).toBeVisible();
  await expect(chat.getByRole("button", { name: "Send" })).toBeVisible();
  // Thinking is collapsed; a finished tool opens to its output.
  await expect(chat.locator('[data-role="thinking"] details')).not.toHaveAttribute("open");
  await expect(tool.locator(".tool-output")).toBeHidden();
  await tool.locator("summary").click();
  await expect(tool.locator(".tool-output")).toHaveText("README.md\nsrc");

  // Stop ends a running turn.
  await input.fill("again");
  await input.press("Enter");
  await chat.getByRole("button", { name: "Stop" }).click();
  await expect(chat.locator('[data-role="note"]')).toHaveText("Interrupted");
  await expect(chat.getByRole("button", { name: "Send" })).toBeVisible();

  await page.getByRole("button", { name: "Close chat fix-login" }).click();
  await expect(chat).toBeHidden();
  await expect(tabs.getByRole("tab")).toHaveCount(0);
});
